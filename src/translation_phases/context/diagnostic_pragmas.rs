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
        ClWarningState,
        ClWarnings,
        CompilerConfiguration,
        DiagnosticGroup,
        ExtensionGroup,
        ExtensionPolicy,
        PragmaClass,
        PragmaExtensionReach,
    },
    translation_phases::{
        ErrorSeverity,
        ExtensionOption,
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
    changes:      ArenaVec<'tu, Change<'tu>>,
    /// The lines `cl`'s `#pragma warning(suppress: N)` withholds a warning
    /// on, which apply over the settings `changes` record.
    suppressions: ArenaVec<'tu, Suppression>,
    /// The settings each `push` not yet popped saved.
    stack:        ArenaVec<'tu, &'tu PragmaState>,
    /// Each retained copy of parser-token provenance made while the table
    /// has changes: where the copy starts, and the parser-token index its
    /// first vector came from. Retained ranges only grow, so this is sorted.
    retained:     ArenaVec<'tu, (u32, u32)>,
}

/// `cl`'s warning `code`, which `suppress` withholds on lines `first` to
/// `last` of the source file `file`.
#[derive(Clone, Copy)]
struct Suppression {
    file:  u32,
    first: u32,
    last:  u32,
    code:  u16,
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
    configuration: CompilerConfiguration,
    /// One bit per group Clang's `fatal` made fatal.
    fatal:         crate::configuration::GroupMask,
    /// One bit per group any pragma classified, whose errors Clang's
    /// `-Wfatal-errors` then leaves errors: `makeUserMapping` sets
    /// `NoErrorAsFatal` on a pragma's mapping (and `NoWarningAsError`, which
    /// `configuration` already holds).
    mapped:        crate::configuration::GroupMask,
    /// What the latest pragma naming `-Wpedantic`, or Clang's
    /// `-Weverything`, which holds it, made of the extension diagnostics:
    /// `warning` keeps them from `-Werror`, and `error` from Clang's
    /// `-Wfatal-errors`.
    pedantic:      Option<PragmaSetting>,
    /// What Clang's `-Weverything` last set, which reaches diagnostics
    /// that belong to no other group.
    everything:    Option<PragmaSetting>,
    /// Which extension diagnostics the pragmas in force reach
    /// ([`CompilerConfiguration::pragma_extension_reach`]). The command
    /// line alone classifies the others, after any other pragma or a
    /// balanced `push` and `pop`.
    extensions:    ExtensionPragmas,
}

/// The extension diagnostics some pragma in force reaches: all of them, or
/// those filed under the GCC options of the groups set in `groups`, one bit
/// each.
#[derive(Clone, Copy)]
struct ExtensionPragmas {
    all:    bool,
    groups: u16,
}

impl ExtensionPragmas {
    const NONE: Self = Self {
        all:    false,
        groups: 0,
    };

    const fn with(self, reach: PragmaExtensionReach) -> Self {
        match reach {
            | PragmaExtensionReach::Nothing => self,
            | PragmaExtensionReach::Group(group) => Self {
                groups: self.groups | 1 << group as u16,
                ..self
            },
            | PragmaExtensionReach::All => Self { all: true, ..self },
        }
    }

    const fn reach(self, group: ExtensionGroup) -> bool {
        self.all || self.groups & 1 << group as u16 != 0
    }
}

/// A pragma's class for an option, with Clang's `fatal`.
#[derive(Clone, Copy)]
struct PragmaSetting {
    class: PragmaClass,
    fatal: bool,
}

/// What the settings at a diagnostic's place made of it.
#[derive(Clone, Copy)]
pub(crate) struct PragmaClassification {
    severity: ErrorSeverity,
    /// Clang's `fatal` named its group.
    fatal:    bool,
    /// A pragma named its group (see [`PragmaState::mapped`]).
    mapped:   bool,
}

impl PragmaState {
    fn classification(
        &self,
        group: DiagnosticGroup,
        severity: ErrorSeverity,
    ) -> PragmaClassification {
        PragmaClassification {
            severity,
            fatal: self.fatal.contains(group),
            mapped: self.mapped.contains(group),
        }
    }
}

impl<'tu> DiagnosticPragmas<'tu> {
    pub(super) fn new_in(tu: &'tu Bump) -> Self {
        Self {
            changes:      ArenaVec::new_in(tu),
            suppressions: ArenaVec::new_in(tu),
            stack:        ArenaVec::new_in(tu),
            retained:     ArenaVec::new_in(tu),
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
            .and_then(|state| state.pedantic)
            .is_some_and(|setting| setting.class == PragmaClass::Warning)
    }

    /// The class Clang's `-Weverything` set where `source` is, if a pragma set
    /// one.
    pub(crate) fn everything_class_at(&self, source: SourceVectors) -> Option<PragmaClass> {
        self.pragma_state_at(source)
            .and_then(|state| state.everything)
            .map(|setting| setting.class)
    }

    /// [`Self::everything_class_at`] for an extension diagnostic of `option`,
    /// if only Clang's `-Weverything` reaches it: a pragma naming `-Wpedantic`
    /// does not, before or after that one.
    pub(crate) fn everything_class_outside_pedantic(
        &self,
        option: ExtensionOption,
        source: SourceVectors,
    ) -> Option<PragmaClass> {
        if option == ExtensionOption::Everything {
            self.everything_class_at(source)
        } else {
            None
        }
    }

    /// The extension policy that decides an extension diagnostic of `option`
    /// at `source` where no `-Weverything` pragma set its class
    /// ([`Self::everything_class_outside_pedantic`]). GCC files what Clang
    /// warns of by default outside `-Wpedantic` under that option, so a pragma
    /// naming it maps it there. Clang's maps none of it: `-pedantic-errors`
    /// still makes it an error, and `error "-Wpedantic"` alone does not. So
    /// under Clang's rules the command line's policy decides it, but where
    /// that one leaves it unreported, which Clang's default does not, a
    /// pragma that turns `-Wpedantic` on reports it as the warning Clang
    /// gives by default.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    pub(crate) fn extension_policy_at(
        &self,
        option: ExtensionOption,
        source: SourceVectors,
    ) -> ExtensionPolicy {
        let policy = self.configuration_at(source).extension_policy();
        if option != ExtensionOption::Everything
            || self.configuration.compiler_identity().rules()
                != crate::configuration::RuleSet::Clang
        {
            return policy;
        }
        match self.configuration.extension_policy() {
            | ExtensionPolicy::Allow if policy != ExtensionPolicy::Allow => ExtensionPolicy::Warn,
            | command_line => command_line,
        }
    }

    /// Whether the latest pragma naming `-Wpedantic`, or with `everything`
    /// Clang's `-Weverything`, made an extension diagnostic at `source` an
    /// error, and if so whether as Clang's `fatal`. Such an error is a
    /// pragma's mapping, which Clang's `-Wfatal-errors` leaves an error (see
    /// [`PragmaState::mapped`]).
    pub(crate) fn pedantic_error_at(
        &self,
        source: SourceVectors,
        everything: bool,
    ) -> Option<bool> {
        let state = self.pragma_state_at(source)?;
        let setting = if everything {
            state.everything
        } else {
            state.pedantic
        }?;
        (setting.class == PragmaClass::Error).then_some(setting.fatal)
    }

    /// Queues an extension diagnostic of the warning `option`, which
    /// `warning "-Wpedantic"` keeps from `-Werror`. One whose severity is the
    /// extension policy's and that no group of its own classifies is an error
    /// of that pragma's `error` or `fatal` making, which Clang's
    /// `-Wfatal-errors` leaves an error. What Clang warns of by default
    /// outside `-Wpedantic` answers to its `-Weverything` alone, which also
    /// gave it its severity. A parser diagnostic records either itself, since
    /// the parser edits it after queueing it and the reporter folds it by its
    /// variant.
    pub(crate) fn classified_extension(
        &mut self,
        source: SourceVectors,
        mut error: TranslationError<'tu>,
        option: ExtensionOption,
    ) {
        let outside_pedantic = option == ExtensionOption::Everything
            && self.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang;
        let everything = self.everything_class_outside_pedantic(option, source);
        let (severity, fatal) = match error.severity() {
            | ErrorSeverity::Warning if everything == Some(PragmaClass::Warning) =>
                (ErrorSeverity::Warning, false),
            | ErrorSeverity::Warning if !outside_pedantic && self.pedantic_warning_at(source) =>
                (ErrorSeverity::Warning, false),
            | ErrorSeverity::Error
                if error.group().is_none()
                    && (option == ExtensionOption::Pedantic || everything.is_some()) =>
            {
                let Some(fatal) = self.pedantic_error_at(source, everything.is_some()) else {
                    self.classified_error(source, error);
                    return;
                };
                (ErrorSeverity::Error, fatal)
            },
            | _ => {
                self.classified_error(source, error);
                return;
            },
        };
        if let TranslationError::Parsing(parsing) = &mut error {
            parsing.severity_is_final = true;
            parsing.pragma_mapped = true;
            parsing.fatal = fatal;
            self.pending_errors.push_back(error);
            return;
        }
        let error = self.tu.alloc(error);
        self.pending_errors
            .push_back(TranslationError::Classified(ClassifiedError {
                error,
                severity,
                fatal,
                mapped: true,
            }));
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
    ) -> Option<(&'tu Bump, PragmaClassification)> {
        let state = self.pragma_state_at(source)?;
        let severity = state.configuration.diagnostic_severity(group)?;
        Some((self.tu, state.classification(group, severity)))
    }

    /// Queues `error` as [`Self::pragma_classification`] classified it.
    #[cold]
    #[inline(never)]
    pub(crate) fn raw_classified_preprocessor_error(
        pending_errors: &mut ArenaQueue<'tu, TranslationError<'tu>>,
        classified: Option<(&'tu Bump, PragmaClassification)>,
        error: PreprocessorError<'tu>,
    ) {
        let error = TranslationError::Preprocessing(error);
        pending_errors.push_back(match classified {
            | Some((tu, classification)) => TranslationError::Classified(ClassifiedError {
                severity: classification.severity,
                fatal:    classification.fatal,
                mapped:   classification.mapped,
                error:    tu.alloc(error),
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
        let Some(state) = self.pragma_state_at(source) else {
            self.pending_errors.push_back(error);
            return;
        };
        let classification = if let Some(group) = group {
            let Some(severity) = state.configuration.diagnostic_severity(group) else {
                return;
            };
            state.classification(group, severity)
        } else {
            self.pending_errors.push_back(error);
            return;
        };
        let error = self.tu.alloc(error);
        self.pending_errors
            .push_back(TranslationError::Classified(ClassifiedError {
                error,
                severity: classification.severity,
                fatal: classification.fatal,
                mapped: classification.mapped,
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
            state.fatal.union(mask)
        } else {
            state.fatal.difference(mask)
        };
        state.mapped = state.mapped.union(mask);
        state.extensions = state
            .extensions
            .with(self.configuration.pragma_extension_reach(name));
        // Clang's `-Weverything` holds `-Wpedantic`.
        let everything = name == "everything"
            && self.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang;
        let setting = Some(PragmaSetting { class, fatal });
        if name == "pedantic" || everything {
            state.pedantic = setting;
        }
        if everything {
            state.everything = setting;
        }
        self.record_pragma_state(&state);
    }

    /// One number of `#pragma warning(<specifier>: <number>)` under the `cl`
    /// identity, for the warnings bcc gives `cl`'s code: `disable` and
    /// `error` set it so, and `default` or a level shows it from that level
    /// again, undoing a `/wd` or `/we`. Microsoft Learn: "warning pragma".
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_warning_number(
        &mut self,
        number: u32,
        specifier: WarningSpecifier,
        line: (u32, u32),
    ) {
        let Some((code, default)) = u16::try_from(number)
            .ok()
            .and_then(|code| Some((code, ClWarnings::default_level(code)?)))
        else {
            return;
        };
        let setting = match specifier {
            | WarningSpecifier::Disable => ClWarningState::Disabled,
            | WarningSpecifier::Error => ClWarningState::Error,
            | WarningSpecifier::Default => ClWarningState::Level(default),
            | WarningSpecifier::Once => ClWarningState::Once(default),
            | WarningSpecifier::Level(level) => ClWarningState::Level(level),
            | WarningSpecifier::Suppress => {
                self.pragma_warning_suppress(code, line);
                return;
            },
        };
        let mut state = self.current_pragma_state();
        let warnings = state.configuration.cl_warnings().with_state(code, setting);
        state.configuration = state.configuration.with_cl_pragma(warnings, Some(code));
        // An error the pragma made is `cl`'s `error CNNNN`, not a warning
        // `/WX` makes an error after C2220.
        for &group in CL_CODED_WARNINGS
            .iter()
            .filter(|warning| warning.code == code)
            .flat_map(|warning| warning.groups)
        {
            let mask = crate::configuration::GroupMask::of(group);
            state.mapped = if setting == ClWarningState::Error {
                state.mapped.union(mask)
            } else {
                state.mapped.difference(mask)
            };
        }
        self.record_pragma_state(&state);
    }

    /// `#pragma warning(suppress: code)` under the `cl` identity on `line`
    /// of a source file: `cl` withholds the warning on that line and the
    /// next. A
    /// `suppress` on a line another one reaches holds that one's warnings
    /// to its own next line too. Its settings are those of the lines, where
    /// the other changes are those of the tokens after them; it gives the
    /// table a change, which a diagnostic then looks up.
    /// Microsoft Learn: "warning pragma".
    fn pragma_warning_suppress(&mut self, code: u16, (file, line): (u32, u32)) {
        let next = line.saturating_add(1);
        for suppression in self.diagnostic_pragmas.suppressions.iter_mut().rev() {
            if suppression.file != file || suppression.last < line {
                break;
            }
            suppression.last = next;
        }
        self.diagnostic_pragmas.suppressions.push(Suppression {
            file,
            first: line,
            last: next,
            code,
        });
        if self.diagnostic_pragmas.changes.is_empty() {
            let state = self.current_pragma_state();
            self.record_pragma_state(&state);
        }
    }

    /// `#pragma warning(push, n)` under the `cl` identity, after its push:
    /// level `n` until the matching `pop`.
    #[cold]
    #[inline(never)]
    pub(crate) fn pragma_warning_level(&mut self, level: u8) {
        let mut state = self.current_pragma_state();
        let warnings = state.configuration.cl_warnings().with_level(level);
        state.configuration = state.configuration.with_cl_pragma(warnings, None);
        self.record_pragma_state(&state);
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
        self.record_pragma_state(&state);
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

    fn record_pragma_state(&mut self, state: &PragmaState) {
        let token = u32::try_from(self.parser_token_vectors.len())
            .expect("parser provenance fits its 32-bit source ranges");
        let state = &*self.tu.alloc(*state);
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
            fatal: crate::configuration::GroupMask::EMPTY,
            mapped: crate::configuration::GroupMask::EMPTY,
            pedantic: None,
            everything: None,
            extensions: ExtensionPragmas::NONE,
        }
    }

    /// Whether a pragma in force at `source` reaches an extension diagnostic
    /// whose tag names `group` (see [`PragmaState::extensions`]), rather
    /// than the command line's classification of it.
    #[inline]
    pub(crate) fn extension_pragma_governs(
        &self,
        group: ExtensionGroup,
        source: SourceVectors,
    ) -> bool {
        self.pragma_state_at(source)
            .is_some_and(|state| state.extensions.reach(group))
    }

    /// The settings at `source`, or `None` where only the command line's
    /// hold: in a unit without diagnostic pragmas, or before the first one.
    #[inline]
    fn pragma_state_at(&self, source: SourceVectors) -> Option<PragmaState> {
        if self.diagnostic_pragmas.changes.is_empty() {
            return None;
        }
        let state = *self.pragma_state_in_table(source)?;
        if self.diagnostic_pragmas.suppressions.is_empty() || source.length() == 0 {
            return Some(state);
        }
        Some(self.suppressed_state(state, source))
    }

    /// `state` with the warnings `suppress` withholds on the line of
    /// `source`.
    #[cold]
    #[inline(never)]
    fn suppressed_state(&self, mut state: PragmaState, source: SourceVectors) -> PragmaState {
        let vector = self.file_vector(source);
        let (file, line) = (vector.source_file_index, vector.line);
        for suppression in &self.diagnostic_pragmas.suppressions {
            if suppression.file == file && (suppression.first..=suppression.last).contains(&line) {
                let code = suppression.code;
                let warnings = state
                    .configuration
                    .cl_warnings()
                    .with_state(code, ClWarningState::Disabled);
                state.configuration = state.configuration.with_cl_pragma(warnings, Some(code));
            }
        }
        state
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
