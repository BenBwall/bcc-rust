//! Folding, ordering, and rendering of the diagnostics one compilation
//! reports, for the CLI's output and the explorer.

use std::io::{
    self,
    Write,
};

use rustc_hash::FxBuildHasher;

use super::{
    ColorChoice as RenderColor,
    Diagnostic,
    Explanation,
    Renderer,
    ToDiagnostic,
    TreatedAsError,
    count_of,
    format_in,
};
use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        SourceVectors,
        TranslationError,
        preprocessing::PreprocessorErrorType,
        semantic_analysis::ReportOrder,
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
/// parser and the preprocessor are considered separately. Only errors in the
/// input's syntax fold ([`TranslationError::may_fold`]): a semantic error, an
/// extension report or a warning made an error never does.
///
/// Pending diagnostics are built in the translation-unit arena (`'tu`). The
/// pending list, the folding map, and the locations live in the reporter's
/// own arena (`'r`). Parser mode keeps it for the whole compilation; token
/// mode resets it between flushed batches.
pub(crate) struct DiagnosticReporter<'r, 'tu> {
    arena:               &'r Bump,
    /// Sees each diagnostic as it is rendered, with its text.
    observer:            Option<&'r mut Observer<'r>>,
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
    pub(crate) errors:   usize,
    pub(crate) warnings: usize,
    /// A fatal diagnostic was written, so nothing further is.
    fatal_reported:      bool,
    /// A warning `cl`'s `/WX` made an error was written, after `cl`'s
    /// error C2220.
    treated_as_error:    bool,
    /// The warnings written as errors that GCC's format counts
    /// (`DK_WERROR`), which end its diagnostics with a line saying so.
    pub(crate) promoted: usize,
}

/// What a compiler says about its own command line with each translation
/// unit it compiles, which bcc says with the unit's diagnostics: Clang's
/// front end warns about the warning options it does not know before
/// anything else and counts the warnings with the others, and GCC's
/// compiler notes the `-Wno-` options it did not recognize after the
/// others, only when there were others.
#[derive(Debug, Default)]
pub(crate) struct CommandLineDiagnostics<'tu> {
    /// Clang's warnings, each message ending with its `[-W…]`, at the
    /// severity the options give it; the first reporter of the unit takes
    /// them.
    pub(crate) leading:      std::cell::Cell<&'tu [(ErrorSeverity, &'tu str)]>,
    /// GCC's unrecognized `-Wno-` options, in command-line order.
    pub(crate) unrecognized: &'tu [&'tu str],
    /// How many of [`Self::leading`] are errors, which leave the unit
    /// without code.
    pub(crate) errors:       usize,
}

impl<'r, 'tu> DiagnosticReporter<'r, 'tu> {
    pub(crate) fn report(&mut self, error: &TranslationError<'_>, context: &mut Context<'_>) {
        let source = error.source_vectors(context);
        // Folded parser recovery keeps spelling provenance so its secondary
        // ranges remain relative to the preprocessing error it joins.
        let diagnostic = error.diagnostic_in(context, source, self.diagnostics);
        let location = context.get_source_vectors(source);
        let tcc = context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Tcc;
        // TCC reports in one pass, so a semantic warning `-Werror` makes an
        // error keeps its place among the others: TCC stops at the first
        // error in the unit, whichever phase reports it.
        let warning = diagnostic.severity == ErrorSeverity::Warning
            || (tcc && is_semantic(error) && error.group().is_some());
        let ordering_location = match error {
            | TranslationError::Parsing(error) => error.ordering_location,
            // Preprocessing errors may still point into a macro definition.
            | _ if !warning => None,
            | _ => match report_order(error) {
                | ReportOrder::AtSite => location
                    .first()
                    .map(|source| (source.source_file_index, source.index)),
                | ReportOrder::After(token) => context
                    .get_source_vectors(token)
                    .first()
                    .map(|source| (source.source_file_index, source.index)),
                // Last in its file's run, which ends the diagnostics.
                | ReportOrder::UnitEnd => location
                    .first()
                    .map(|source| (source.source_file_index, u32::MAX)),
            },
        };
        // TCC reports as it reads, in one pass, so the warnings of one line
        // come in the order it compiles them, which semantic analysis
        // follows, rather than in the order of the places they point at:
        // `return c ? p : q;` reports the conditional's mismatch before the
        // conversion the `return` makes, whose range starts first. A line
        // that also has a warning of the other phases keeps the order of
        // places ([`Self::order_source_runs`]).
        let tcc_line = ordering_location
            .filter(|_| warning && tcc && report_order(error) == ReportOrder::AtSite)
            .map(|(file, index)| {
                let line_start = context
                    .source_text(file)
                    .and_then(|text| text.get(..index as usize))
                    .and_then(|before| before.rfind('\n'))
                    .map_or(0, |newline| newline + 1);
                (
                    u32::try_from(line_start).unwrap_or(index),
                    is_semantic(error),
                )
            });
        let (parser, consumed) = match error {
            | TranslationError::Parsing(error) => (true, error.consumed_tokens),
            | _ => (false, 0),
        };
        let foldable = error.may_fold();
        let empty_translation_unit =
            matches!(error, TranslationError::Parsing(error) if error.is_empty_translation_unit());
        let target =
            if diagnostic.severity == ErrorSeverity::Error && foldable && !location.is_empty() {
                self.fold_target(parser.then_some(consumed), empty_translation_unit, location)
                    // A file read twice puts two mistakes at one place.
                    .filter(|&index| {
                        self.pending[index]
                            .diagnostic
                            .reading
                            .is_none_or(|reading| context.same_reading(reading, source))
                    })
            } else {
                None
            };
        let index = if let Some(index) = target {
            self.pending[index].diagnostic.absorb(diagnostic, context);
            index
        } else {
            let location: &'r [SourceVector] =
                self.arena.alloc_slice_fill_iter(location.iter().cloned());
            let diagnostic = if parser {
                let primary = context.file_location(source);
                if primary == source {
                    diagnostic
                } else {
                    error.diagnostic_in(context, primary, self.diagnostics)
                }
            } else {
                diagnostic
            };
            let mut diagnostic = diagnostic;
            diagnostic.reading = diagnostic.reading.or(Some(source));
            if !parser && diagnostic.severity == ErrorSeverity::Error && !location.is_empty() {
                _ = self.other_errors.insert(location, self.pending.len());
            }
            self.pending.push(PendingDiagnostic {
                diagnostic,
                location,
                ordering_location,
                tcc_line,
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

    /// Reports an error found after semantic analysis, such as a construct
    /// lowering does not handle, at `source` (or at no place, for the whole
    /// unit). Such errors never fold into another diagnostic.
    pub(crate) fn report_late(
        &mut self,
        error: &dyn ToDiagnostic,
        source: Option<SourceVectors>,
        context: &Context<'_>,
    ) {
        let diagnostic = error.diagnostic_in(context, source.unwrap_or_default(), self.diagnostics);
        self.pending.push(PendingDiagnostic {
            diagnostic,
            location: &[],
            ordering_location: None,
            tcc_line: None,
            sequence: self.pending.len(),
            foldable: false,
            preserve_empty_translation_unit: false,
        });
    }

    /// Renders and counts every pending diagnostic. Later errors can no
    /// longer fold into them, so the folding targets are forgotten.
    pub(crate) fn flush(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
        // TCC stops compiling at the first error, as GCC's `-Wfatal-errors`
        // does, so nothing follows it.
        let stops_at_error = context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Tcc
            || context.configuration.fatal_errors();
        let leading = context.command_line.leading.take();
        if !leading.is_empty() {
            let pending = std::mem::replace(&mut self.pending, ArenaVec::new_in(self.arena));
            self.pending
                .extend(leading.iter().map(|&(severity, message)| {
                    PendingDiagnostic {
                        diagnostic: Explanation::new(
                            self.diagnostics,
                            self.diagnostics.alloc_str(message),
                        )
                        .option_suffix()
                        .at(severity, SourceVectors::default()),
                        location: &[],
                        ordering_location: None,
                        tcc_line: None,
                        sequence: 0,
                        foldable: false,
                        preserve_empty_translation_unit: false,
                    }
                }));
            self.pending.extend(pending);
        }
        let gcc = context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Gcc;
        let errors_fatal = context.configuration.errors_fatal();
        for PendingDiagnostic { mut diagnostic, .. } in self.pending.drain(..) {
            // Clang's `-Wfatal-errors` leaves an error a pragma made one
            // (`DiagnosticIDs::getDiagnosticSeverity`).
            if errors_fatal
                && diagnostic.severity == ErrorSeverity::Error
                && !diagnostic.fatal
                && !diagnostic.pragma_mapped
            {
                diagnostic.fatal = true;
            }
            // Clang's fatal error names the warning option alone, without
            // the `-Werror` it writes for an error
            // (`printDiagnosticOptions` in `TextDiagnosticPrinter.cpp`).
            if diagnostic.fatal
                && diagnostic.option_suffix
                && let Some(at) = diagnostic.message.rfind(" [-Werror,-W")
            {
                let message = diagnostic.message;
                diagnostic.message = format_in!(
                    self.diagnostics,
                    "{} [{}",
                    &message[..at],
                    &message[at + " [-Werror,".len()..]
                );
            }
            if diagnostic.severity == ErrorSeverity::Warning
                && context.configuration.suppress_warnings()
            {
                continue;
            }
            // Clang's `fatal` ends the compilation, counts and all.
            if self.fatal_reported {
                continue;
            }
            self.fatal_reported = diagnostic.fatal;
            let stopped = stops_at_error && self.errors != 0;
            match diagnostic.severity {
                | ErrorSeverity::Error => self.errors += 1,
                | ErrorSeverity::Warning => self.warnings += 1,
            }
            if stopped {
                continue;
            }
            // `cl` announces only a unit's first such warning, and none after
            // an error it reported (this one is counted already); its code
            // generator announces each of its own.
            if diagnostic.treated_as_error != TreatedAsError::No && !diagnostic.back_end {
                diagnostic.treated_as_error = if self.treated_as_error || self.errors > 1 {
                    TreatedAsError::Later
                } else {
                    TreatedAsError::First
                };
                self.treated_as_error = true;
            }
            if gcc && diagnostic.severity == ErrorSeverity::Error && diagnostic.option_suffix {
                let message = diagnostic.message;
                if message.ends_with(" [-Werror]") || message.contains(" [-Werror=") {
                    self.promoted += 1;
                }
            }
            let text = self.renderer.render_text(&diagnostic, context);
            if let Some(observer) = &mut self.observer {
                observer(&diagnostic, text, context);
            }
            out.write_all(text.as_bytes())?;
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.flush(context, out)?;
        let gcc = context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Gcc;
        let terminated = (context.configuration.fatal_errors() || (gcc && self.fatal_reported))
            && self.errors != 0;
        if terminated {
            // GCC names `-Wfatal-errors` when it, not a fatal error, ends
            // the compilation (`diagnostic_context::action_after_output`).
            writeln!(
                out,
                "{}",
                if gcc && !self.fatal_reported {
                    "compilation terminated due to -Wfatal-errors."
                } else {
                    "compilation terminated."
                }
            )?;
        }
        if gcc {
            self.finish_gcc(context, out, terminated)?;
        }
        // GCC, MSVC and TCC print no summary, and Clang prints it only with
        // its snippets (`CompilerInstance::printDiagnosticStats` checks
        // `ShowCarets`), warnings first.
        let format = context.configuration.diagnostic_format();
        let clang = format == crate::configuration::DiagnosticFormat::Clang;
        if matches!(
            format,
            crate::configuration::DiagnosticFormat::Gcc
                | crate::configuration::DiagnosticFormat::Msvc { .. }
                | crate::configuration::DiagnosticFormat::Tcc
        ) || (clang && !context.configuration.diagnostic_presentation().caret)
        {
            return Ok(());
        }
        let errors = count_of(self.errors, "error");
        let warnings = count_of(self.warnings, "warning");
        match (self.errors, self.warnings) {
            | (0, 0) => Ok(()),
            | (_, 0) => writeln!(out, "{errors} generated."),
            | (0, _) => writeln!(out, "{warnings} generated."),
            | _ if clang => writeln!(out, "{warnings} and {errors} generated."),
            | _ => writeln!(out, "{errors} and {warnings} generated."),
        }
    }

    /// GCC's last lines for a translation unit. Unless an error ended the
    /// compilation, a note for each `-Wno-` option it did not recognize,
    /// last first, when it reported anything (`print_ignored_options` in
    /// `opts-global.cc`), at top level; then whether `-Werror` or a
    /// `-Werror=` made the warnings it wrote as errors so
    /// (`diagnostic_context::finish`).
    fn finish_gcc(
        &self,
        context: &Context<'_>,
        out: &mut dyn Write,
        terminated: bool,
    ) -> io::Result<()> {
        let unrecognized = context.command_line.unrecognized;
        if !terminated && self.errors + self.warnings != 0 && !unrecognized.is_empty() {
            if self.renderer.leave_gcc_function() {
                writeln!(out, "At top level:")?;
            }
            for option in unrecognized.iter().rev() {
                writeln!(
                    out,
                    "cc1: note: unrecognized command-line option '{option}' may have been \
                     intended to silence earlier diagnostics"
                )?;
            }
        }
        if self.promoted != 0 {
            writeln!(
                out,
                "cc1: {} warnings being treated as errors",
                if context.configuration.warnings_are_errors() {
                    "all"
                } else {
                    "some"
                }
            )?;
        }
        Ok(())
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
    pub(crate) fn report_pending(
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
        // Under TCC's format, a line whose warnings all come from semantic
        // analysis orders them as they were reported.
        let mut mixed_lines = ArenaMap::with_hasher_in(FxBuildHasher, self.arena);
        for pending in &self.pending {
            if let (Some((file, _)), Some((line, semantic))) =
                (pending.ordering_location, pending.tcc_line)
            {
                *mixed_lines.entry((file, line)).or_insert(false) |= !semantic;
            }
        }
        for pending in &mut self.pending {
            if let (Some((file, _)), Some((line, true))) =
                (pending.ordering_location, pending.tcc_line)
                && mixed_lines.get(&(file, line)) == Some(&false)
            {
                pending.ordering_location = Some((file, line));
            }
        }
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

    pub(crate) fn new(arena: &'r Bump, diagnostics: &'tu Bump, color: RenderColor) -> Self {
        Self {
            arena,
            observer: None,
            diagnostics,
            renderer: Renderer::new(color),
            pending: ArenaVec::new_in(arena),
            last_parser: None,
            last_other: None,
            other_errors: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            errors: 0,
            warnings: 0,
            fatal_reported: false,
            treated_as_error: false,
            promoted: 0,
        }
    }
}

/// Whether semantic analysis reported `error`.
fn is_semantic(error: &TranslationError<'_>) -> bool {
    match error {
        | TranslationError::Semantic(_) => true,
        | TranslationError::Classified(classified) => is_semantic(classified.error),
        | _ => false,
    }
}

/// Where a diagnostic falls among the others: some of GCC's semantic warnings
/// come after the place they name.
fn report_order(error: &TranslationError<'_>) -> ReportOrder {
    match error {
        | TranslationError::Semantic(error) => error.order,
        | TranslationError::Classified(classified) => report_order(classified.error),
        | _ => ReportOrder::AtSite,
    }
}

/// Receives each rendered diagnostic, for callers that keep diagnostics
/// rather than print them.
pub(crate) type Observer<'r> = dyn FnMut(&Diagnostic<'_>, &str, &Context<'_>) + 'r;

impl<'r> DiagnosticReporter<'r, '_> {
    /// Also hands each diagnostic to `observer` as it is rendered.
    pub(crate) fn observed(mut self, observer: &'r mut Observer<'r>) -> Self {
        self.observer = Some(observer);
        self
    }
}

/// One diagnostic awaiting rendering, with any later errors folded in.
struct PendingDiagnostic<'r, 'tu> {
    diagnostic: Diagnostic<'tu>,
    location: &'r [SourceVector],
    ordering_location: Option<(u32, u32)>,
    /// Under TCC's format, a warning's line, by where it starts, and whether
    /// semantic analysis reported it.
    tcc_line: Option<(u32, bool)>,
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
