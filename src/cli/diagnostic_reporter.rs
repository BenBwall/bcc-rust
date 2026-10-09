//! Folding, ordering, and rendering of the diagnostics one compilation
//! reports, for the CLI's token and syntax-tree output.

use std::io::{
    self,
    Write,
};

use rustc_hash::FxBuildHasher;

use crate::{
    diagnostics::{
        ColorChoice as RenderColor,
        Diagnostic,
        Renderer,
        ToDiagnostic,
        count_of,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        preprocessing::PreprocessorErrorType,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Renders diagnostics to a caller's writer (stderr, for the CLI) and
/// summarizes them at the end, like `N errors and M warnings generated`.
///
/// An error reported at exactly the same place as an earlier error from the
/// same mistake is folded into it rather than printed again: a parser error
/// into the parser error just before it when no input was consumed between
/// them, otherwise into a lexing or preprocessing error at that place; a
/// preprocessing error into the preprocessing error just before it. The
/// parser and the preprocessor are considered separately.
///
/// Pending diagnostics are built in the translation-unit arena (`'tu`). The
/// pending list, the folding map, and the locations live in the reporter's
/// own arena (`'r`). Parser mode keeps it for the whole compilation; token
/// mode resets it between flushed batches.
pub(super) struct DiagnosticReporter<'r, 'tu> {
    arena:               &'r Bump,
    diagnostics:         &'tu Bump,
    renderer:            Renderer,
    pending:             ArenaVec<'r, PendingDiagnostic<'r, 'tu>>,
    /// The parser diagnostic reported last: where it was folded or stored,
    /// and the input the parser had consumed by then.
    last_parser:         Option<(usize, usize)>,
    /// Where the preprocessing diagnostic reported last was folded or stored.
    last_other:          Option<usize>,
    /// The latest pending preprocessing error at each location.
    other_errors:        ArenaMap<'r, &'r [SourceVector], usize>,
    pub(super) errors:   usize,
    pub(super) warnings: usize,
}

/// One diagnostic awaiting rendering, with any later errors folded in.
struct PendingDiagnostic<'r, 'tu> {
    diagnostic: Diagnostic<'tu>,
    location: &'r [SourceVector],
    ordering_location: Option<(u32, u32)>,
    /// Its place among the pending diagnostics when it was reported, which
    /// keeps the order of diagnostics at one location stable.
    sequence: usize,
    /// Whether errors at the same place may be folded into this one.
    foldable: bool,
    /// An unclosed angle header and an empty translation unit remain two
    /// separate diagnostics even when they point at the same EOF position.
    preserve_empty_translation_unit: bool,
}

impl PendingDiagnostic<'_, '_> {
    fn absorbs(&self, location: &[SourceVector]) -> bool {
        self.foldable
            && self.diagnostic.severity == ErrorSeverity::Error
            && self.location == location
    }
}

impl<'r, 'tu> DiagnosticReporter<'r, 'tu> {
    pub(super) fn new(arena: &'r Bump, diagnostics: &'tu Bump, color: RenderColor) -> Self {
        Self {
            arena,
            diagnostics,
            renderer: Renderer::new(color),
            pending: ArenaVec::new_in(arena),
            last_parser: None,
            last_other: None,
            other_errors: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            errors: 0,
            warnings: 0,
        }
    }

    pub(super) fn report(&mut self, error: &TranslationError<'_>, context: &mut Context<'_>) {
        let source = error.source_vectors(context);
        let diagnostic = error.diagnostic_in(context, source, self.diagnostics);
        let location = context.get_source_vectors(source);
        let ordering_location = match error {
            | TranslationError::Parsing(error) => error.ordering_location,
            // Preprocessing errors may still point into a macro definition.
            | _ if diagnostic.severity != ErrorSeverity::Warning => None,
            | _ => location
                .first()
                .map(|source| (source.source_file_index, source.index)),
        };
        let (parser, consumed, foldable) = match error {
            | TranslationError::Parsing(error) => (true, error.consumed_tokens, error.may_fold()),
            | _ => (false, 0, true),
        };
        let empty_translation_unit =
            matches!(error, TranslationError::Parsing(error) if error.is_empty_translation_unit());
        let target =
            if diagnostic.severity == ErrorSeverity::Error && foldable && !location.is_empty() {
                self.fold_target(parser.then_some(consumed), empty_translation_unit, location)
            } else {
                None
            };
        let index = if let Some(index) = target {
            self.pending[index].diagnostic.absorb(diagnostic, context);
            index
        } else {
            let location: &'r [SourceVector] =
                self.arena.alloc_slice_fill_iter(location.iter().cloned());
            if !parser && diagnostic.severity == ErrorSeverity::Error && !location.is_empty() {
                _ = self.other_errors.insert(location, self.pending.len());
            }
            self.pending.push(PendingDiagnostic {
                diagnostic,
                location,
                ordering_location,
                sequence: self.pending.len(),
                foldable,
                preserve_empty_translation_unit: matches!(
                    error,
                    TranslationError::Preprocessing(error)
                        if matches!(error.error_type, PreprocessorErrorType::UnterminatedHeaderName('>'))
                ),
            });
            self.pending.len() - 1
        };
        if parser {
            self.last_parser = Some((index, consumed));
        } else {
            self.last_other = Some(index);
        }
    }

    /// The pending error that a foldable error at `location` folds into. A
    /// parser error, which `parser_consumed` gives the input consumed before,
    /// folds into the parser error just before it when nothing was consumed
    /// between them, or else into a preprocessing error at that place; any
    /// other error folds into the preprocessing error just before it.
    fn fold_target(
        &self,
        parser_consumed: Option<usize>,
        empty_translation_unit: bool,
        location: &[SourceVector],
    ) -> Option<usize> {
        let Some(consumed) = parser_consumed else {
            return self
                .last_other
                .filter(|&index| self.pending[index].absorbs(location));
        };
        self.last_parser
            .filter(|&(index, last_consumed)| {
                last_consumed == consumed && self.pending[index].absorbs(location)
            })
            .map(|(index, _)| index)
            .or_else(|| {
                self.other_errors.get(location).copied().filter(|&index| {
                    self.pending[index].absorbs(location)
                        && !(empty_translation_unit
                            && self.pending[index].preserve_empty_translation_unit)
                })
            })
    }

    /// Reports every pending diagnostic of a parsed translation unit, in
    /// source order, and writes them to `out`.
    pub(super) fn report_pending(
        &mut self,
        context: &mut Context<'_>,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        while let Some(error) = context.pop_pending_error() {
            self.report(&error, context);
        }
        self.order_source_runs();
        self.flush(context, out)
    }

    /// Lookahead can fetch a warning beyond the current parser error. Order
    /// each file run only after folding; preprocessing errors, file
    /// transitions and unknown locations remain barriers, and macro
    /// diagnostics use their captured invocation. Must be followed by
    /// [`Self::flush`], since the pending indices that folding remembers no
    /// longer hold.
    fn order_source_runs(&mut self) {
        for run in self.pending.chunk_by_mut(|left, right| {
            left.ordering_location.is_some()
                && left.ordering_location.map(|(file, _)| file)
                    == right.ordering_location.map(|(file, _)| file)
        }) {
            // Diagnostics at one location keep the order they were reported
            // in. An unstable sort with that tiebreak needs no buffer.
            run.sort_unstable_by_key(|diagnostic| {
                (diagnostic.ordering_location, diagnostic.sequence)
            });
        }
    }

    /// Renders and counts every pending diagnostic. Later errors can no
    /// longer fold into them, so the folding targets are forgotten.
    pub(super) fn flush(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
        for PendingDiagnostic { diagnostic, .. } in self.pending.drain(..) {
            match diagnostic.severity {
                | ErrorSeverity::Error => self.errors += 1,
                | ErrorSeverity::Warning => self.warnings += 1,
                | ErrorSeverity::Note => {},
            }
            out.write_all(self.renderer.render_text(&diagnostic, context).as_bytes())?;
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.flush(context, out)?;
        let errors = count_of(self.errors, "error");
        let warnings = count_of(self.warnings, "warning");
        match (self.errors, self.warnings) {
            | (0, 0) => Ok(()),
            | (_, 0) => writeln!(out, "{errors} generated."),
            | (0, _) => writeln!(out, "{warnings} generated."),
            | _ => writeln!(out, "{errors} and {warnings} generated."),
        }
    }
}
