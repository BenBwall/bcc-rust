//! The table of `#pragma GCC diagnostic` and `#pragma clang diagnostic`
//! changes. Preprocessing finishes before parsing starts, so no mutable state
//! can serve later phases: each change records where it falls in the output
//! tokens, and a diagnostic looks up the settings at its first token, or at
//! the place the compiler it follows asks them for a warning it reports late
//! (`semantic_analysis/unused.rs`).
//!
//! Output-token order is the order GCC's and Clang's own lookups give for
//! everything but a directive inside macro arguments. It covers each inclusion
//! of a header apart, a macro expanded under other settings than its body was
//! defined under, and a `_Pragma` in a macro expansion, which governs the
//! tokens after it in that expansion: `QUIET(a = 3;)`, where `QUIET` wraps its
//! argument in `_Pragma` operators, is silent in both compilers.
//!
//! A diagnostic of phases 1-6 is reported as its token is produced, so the
//! latest change governs it. A later phase's location indexes the parser-token
//! provenance, which is in output order, directly, or a retained copy, which
//! [`DiagnosticPragmas::retained`] maps back. A unit without such a pragma has
//! an empty table and looks nothing up.
//!
//! GCC: <https://gcc.gnu.org/onlinedocs/gcc/Diagnostic-Pragmas.html>.
//! Clang: <https://clang.llvm.org/docs/UsersManual.html#controlling-diagnostics-via-pragmas>.
//! C99: implementation-defined pragma behavior, §6.10.6 paragraph 1, p. 159;
//! PDF p. 171; §6.10.9 paragraph 1, p. 161; PDF p. 173.

use super::Context;
use crate::{
    configuration::{
        CL_CODED_WARNINGS,
        CompilerConfiguration,
        DiagnosticGroup,
        PragmaClass,
    },
    translation_phases::{
        ErrorSeverity,
        GetSeverity,
        TranslationError,
        errors::ClassifiedError,
        preprocessing::{
            PreprocessorError,
            WarningSpecifier,
        },
        provenance::{
            SourceArena,
            SourceVectors,
        },
    },
    util::bump::{
        ArenaQueue,
        ArenaVec,
        Bump,
    },
};

/// What the diagnostic pragmas of one translation unit changed, in order.
pub(crate) struct DiagnosticPragmas<'tu> {
    changes:  ArenaVec<'tu, Change<'tu>>,
    /// The settings each `push` not yet popped saved.
    stack:    ArenaVec<'tu, &'tu PragmaState>,
    /// Each retained copy of parser-token provenance made while the table
    /// has changes: where the copy starts, and the parser-token index its
    /// first vector came from. Retained ranges only grow, so this is sorted.
    retained: ArenaVec<'tu, (u32, u32)>,
}

/// One change: the settings from `token`, an index into the parser-token
/// provenance, on.
#[derive(Clone, Copy)]
struct Change<'tu> {
    token: u32,
    state: &'tu PragmaState,
}

/// The warning settings in force at some point of a translation unit.
#[derive(Clone, Copy)]
struct PragmaState {
    configuration:    CompilerConfiguration,
    /// One bit per group Clang's `fatal` made fatal.
    fatal:            u128,
    /// `warning "-Wpedantic"`: `-Werror` no longer promotes extension
    /// diagnostics.
    pedantic_warning: bool,
    /// The class Clang's `-Weverything` last set, which reaches diagnostics
    /// that belong to no other group.
    everything:       Option<PragmaClass>,
}

impl<'tu> DiagnosticPragmas<'tu> {
    pub(super) fn new_in(tu: &'tu Bump) -> Self {
        Self {
            changes:  ArenaVec::new_in(tu),
            stack:    ArenaVec::new_in(tu),
            retained: ArenaVec::new_in(tu),
        }
    }
}

impl<'tu> Context<'tu> {
    /// The warning settings that govern a diagnostic at `source`: the
    /// command line's, until a diagnostic pragma says otherwise.
    #[inline]
    pub(crate) fn configuration_at(&self, source: SourceVectors) -> CompilerConfiguration {
        match self.pragma_state_at(source) {
            | Some(state) => state.configuration,
            | None => self.configuration,
        }
    }

    /// Whether any diagnostic pragma has changed the settings, so that a
    /// warning the command line leaves off may still be on somewhere.
    #[inline]
    pub(crate) fn has_diagnostic_pragmas(&self) -> bool {
        !self.diagnostic_pragmas.changes.is_empty()
    }

    /// The warning settings in force at the end of the unit, as far as the
    /// output tokens go: once analysis runs, the latest change's.
    pub(crate) fn final_configuration(&self) -> CompilerConfiguration {
        self.current_pragma_state().configuration
    }

    /// Queues a diagnostic of a warning group as the settings at `source`
    /// classify it, which may be not at all.
    #[inline]
    pub(crate) fn classified_error(&mut self, source: SourceVectors, error: TranslationError<'tu>) {
        if self.diagnostic_pragmas.changes.is_empty() {
            self.pending_errors.push_back(error);
        } else {
            self.classify_with_pragmas(source, error);
        }
    }

    /// Queues a diagnostic of `group` that the caller has already asked is on,
    /// for one whose kind [`TranslationError::group`] does not name.
    #[inline]
    pub(crate) fn classified_group_error(
        &mut self,
        group: DiagnosticGroup,
        source: SourceVectors,
        error: TranslationError<'tu>,
    ) {
        if self.diagnostic_pragmas.changes.is_empty() {
            self.pending_errors.push_back(error);
        } else {
            self.classify_in_group(Some(group), source, error);
        }
    }

    /// Whether `warning "-Wpedantic"` governs `source`, which keeps a
    /// `-Wpedantic` diagnostic there from `-Werror` and `-pedantic-errors`.
    pub(crate) fn pedantic_warning_at(&self, source: SourceVectors) -> bool {
        self.pragma_state_at(source)
            .is_some_and(|state| state.pedantic_warning)
    }

    /// The class Clang's `-Weverything` set where `source` is, if a pragma set
    /// one.
    pub(crate) fn everything_class_at(&self, source: SourceVectors) -> Option<PragmaClass> {
        self.pragma_state_at(source)
            .and_then(|state| state.everything)
    }

    /// Queues an extension diagnostic, which `warning "-Wpedantic"` keeps from
    /// `-Werror`. A parser diagnostic records that itself, since the parser
    /// edits it after queueing it and the reporter folds it by its variant.
    pub(crate) fn classified_extension(
        &mut self,
        source: SourceVectors,
        mut error: TranslationError<'tu>,
    ) {
        if error.severity() == ErrorSeverity::Warning && self.pedantic_warning_at(source) {
            if let TranslationError::Parsing(parsing) = &mut error {
                parsing.severity_is_final = true;
                self.pending_errors.push_back(error);
                return;
            }
            let error = self.tu.alloc(error);
            self.pending_errors
                .push_back(TranslationError::Classified(ClassifiedError {
                    error,
                    severity: ErrorSeverity::Warning,
                    fatal: false,
                }));
        } else {
            self.classified_error(source, error);
        }
    }

    /// What the settings at `source` make of a diagnostic of `group`, for a
    /// caller that holds part of the context while it queues one with
    /// [`Self::raw_classified_preprocessor_error`]: `None` where only the
    /// command line's settings hold, or where they silence it, which the
    /// caller has already asked [`Self::group_severity`].
    pub(crate) fn pragma_classification(
        &self,
        group: DiagnosticGroup,
        source: SourceVectors,
    ) -> Option<(&'tu Bump, ErrorSeverity, bool)> {
        let state = self.pragma_state_at(source)?;
        let severity = state.configuration.diagnostic_severity(group)?;
        Some((self.tu, severity, state.fatal & (1 << group as u32) != 0))
    }

    /// Queues `error` as [`Self::pragma_classification`] classified it.
    #[cold]
    #[inline(never)]
    pub(crate) fn raw_classified_preprocessor_error(
        pending_errors: &mut ArenaQueue<'tu, TranslationError<'tu>>,
        classified: Option<(&'tu Bump, ErrorSeverity, bool)>,
        error: PreprocessorError<'tu>,
    ) {
        let error = TranslationError::Preprocessing(error);
        pending_errors.push_back(match classified {
            | Some((tu, severity, fatal)) => TranslationError::Classified(ClassifiedError {
                severity,
                fatal,
                error: tu.alloc(error),
            }),
            | None => error,
        });
    }

    #[cold]
    #[inline(never)]
    fn classify_with_pragmas(&mut self, source: SourceVectors, error: TranslationError<'tu>) {
        self.classify_in_group(error.group(), source, error);
    }

    fn classify_in_group(
        &mut self,
        group: Option<DiagnosticGroup>,
        source: SourceVectors,
        error: TranslationError<'tu>,
    ) {
        let Some(state) = self.pragma_state_at(source).copied() else {
            self.pending_errors.push_back(error);
            return;
        };
        let (severity, fatal) = if let Some(group) = group {
            let Some(severity) = state.configuration.diagnostic_severity(group) else {
                return;
            };
            (severity, state.fatal & (1 << group as u32) != 0)
        } else {
            self.pending_errors.push_back(error);
            return;
        };
        let error = self.tu.alloc(error);
        self.pending_errors
            .push_back(TranslationError::Classified(ClassifiedError {
                error,
                severity,
                fatal,
            }));
    }

    /// `#pragma ... diagnostic <class> "-W<name>"`, with Clang's `fatal`.
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_diagnostic_class(&mut self, name: &str, class: PragmaClass, fatal: bool) {
        let mut state = self.current_pragma_state();
        state.configuration = state.configuration.with_pragma_class(name, class);
        let mask = self.configuration.pragma_group_mask(name);
        state.fatal = if fatal {
            state.fatal | mask
        } else {
            state.fatal & !mask
        };
        // Clang's `-Weverything` holds `-Wpedantic`.
        let everything = name == "everything"
            && self.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang;
        if name == "pedantic" || everything {
            state.pedantic_warning = class == PragmaClass::Warning;
        }
        if everything {
            state.everything = Some(class);
        }
        self.record_pragma_state(state);
    }

    /// One number of `#pragma warning(<specifier>: <number>)` under the `cl`
    /// identity, for the warnings bcc gives `cl`'s code. Microsoft Learn:
    /// "warning pragma".
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_warning_number(&mut self, number: u32, specifier: WarningSpecifier) {
        let group = CL_CODED_WARNINGS
            .iter()
            .find(|&&(code, ..)| u32::from(code) == number)
            .and_then(|&(_, _, name)| DiagnosticGroup::parse(name));
        let Some(group) = group else {
            return;
        };
        let class = match specifier {
            | WarningSpecifier::Disable => PragmaClass::Ignored,
            | WarningSpecifier::Error => PragmaClass::Error,
            | WarningSpecifier::Enable => PragmaClass::Warning,
            | WarningSpecifier::Default => {
                if self.configuration.diagnostic_severity(group).is_some() {
                    PragmaClass::Warning
                } else {
                    PragmaClass::Ignored
                }
            },
            | WarningSpecifier::Suppress => return,
        };
        let mut state = self.current_pragma_state();
        state.configuration = state.configuration.with_pragma_group(group, class);
        self.record_pragma_state(state);
    }

    /// `#pragma ... diagnostic push`.
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_diagnostic_push(&mut self) {
        let state = self.current_pragma_state();
        let state = &*self.tu.alloc(state);
        self.diagnostic_pragmas.stack.push(state);
    }

    /// `#pragma ... diagnostic pop`, which restores the settings the matching
    /// `push` saved. Without one, GCC goes back to the command line's, and
    /// Clang changes nothing and returns false to report it.
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_diagnostic_pop(&mut self) -> bool {
        let state = match self.diagnostic_pragmas.stack.pop() {
            | Some(state) => *state,
            | None if self.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang =>
                return false,
            | None => Self::initial_pragma_state(self.configuration),
        };
        self.record_pragma_state(state);
        true
    }

    /// Notes that a copy of `source` now starts at `start` in the retained
    /// provenance. `source` is in the parser-token or retained provenance.
    #[cold]
    #[inline(never)]
    pub(super) fn record_retained_pragma_copy(&mut self, start: u32, source: SourceVectors) {
        let Some(token) = self.pragma_token_of(source) else {
            return;
        };
        self.diagnostic_pragmas.retained.push((start, token));
    }

    /// Notes that `anchor`, a zero-width location just made for syntax the
    /// input left out, stands where `token` is, or at the end of the output.
    /// It copies no provenance, so nothing else says which settings hold there.
    #[inline]
    pub(crate) fn record_retained_pragma_anchor(
        &mut self,
        anchor: SourceVectors,
        token: Option<SourceVectors>,
    ) {
        if self.records_retained_pragma_copies() {
            self.record_anchor_place(anchor, token);
        }
    }

    #[cold]
    #[inline(never)]
    fn record_anchor_place(&mut self, anchor: SourceVectors, token: Option<SourceVectors>) {
        let (_, start) = SourceArena::decode(anchor);
        let place = match token {
            | Some(token) => self.pragma_token_of(token),
            | None => u32::try_from(self.parser_token_vectors.len()).ok(),
        };
        if let Some(place) = place {
            self.diagnostic_pragmas.retained.push((start, place));
        }
    }

    /// Whether retained copies of parser-token provenance need recording.
    #[inline]
    pub(super) fn records_retained_pragma_copies(&self) -> bool {
        !self.diagnostic_pragmas.changes.is_empty()
    }

    fn record_pragma_state(&mut self, state: PragmaState) {
        let token = u32::try_from(self.parser_token_vectors.len())
            .expect("parser provenance fits its 32-bit source ranges");
        let state = &*self.tu.alloc(state);
        self.diagnostic_pragmas
            .changes
            .push(Change { token, state });
    }

    fn current_pragma_state(&self) -> PragmaState {
        self.diagnostic_pragmas
            .changes
            .last()
            .map_or(Self::initial_pragma_state(self.configuration), |change| {
                *change.state
            })
    }

    const fn initial_pragma_state(configuration: CompilerConfiguration) -> PragmaState {
        PragmaState {
            configuration,
            fatal: 0,
            pedantic_warning: false,
            everything: None,
        }
    }

    /// The settings at `source`, or `None` where only the command line's
    /// hold: in a unit without diagnostic pragmas, or before the first one.
    #[inline]
    fn pragma_state_at(&self, source: SourceVectors) -> Option<&'tu PragmaState> {
        if self.diagnostic_pragmas.changes.is_empty() {
            return None;
        }
        self.pragma_state_in_table(source)
    }

    #[cold]
    #[inline(never)]
    fn pragma_state_in_table(&self, source: SourceVectors) -> Option<&'tu PragmaState> {
        let changes = &self.diagnostic_pragmas.changes;
        // A location no output token has yet was reported while its token was
        // being produced, under the latest change.
        let Some(token) = self.pragma_token_of(source) else {
            return changes.last().map(|change| change.state);
        };
        let after = changes.partition_point(|change| change.token <= token);
        after.checked_sub(1).map(|last| changes[last].state)
    }

    /// The parser-token index of the first token of `source`, if it has one.
    fn pragma_token_of(&self, source: SourceVectors) -> Option<u32> {
        let (arena, start) = SourceArena::decode(source);
        match arena {
            | SourceArena::Preprocessor => None,
            | SourceArena::ParserTokens => Some(start),
            | SourceArena::Retained => {
                let retained = &self.diagnostic_pragmas.retained;
                let after = retained.partition_point(|&(copy, _)| copy <= start);
                after.checked_sub(1).map(|last| retained[last].1)
            },
        }
    }
}
