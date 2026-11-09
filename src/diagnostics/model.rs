//! Diagnostic builders collect labelled source ranges, notes, and help in
//! arena storage. Phase errors implement the shared rendering interface.

#[cfg(test)]
use super::{
    OwnedDiagnostic,
    OwnedExplanation,
};
use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
        SourceVector,
        SourceVectors,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A diagnostic ready to be rendered. Its text and labels live in the arena
/// it was built in.
#[derive(Debug)]
pub(crate) struct Diagnostic<'d> {
    pub(crate) severity:      ErrorSeverity,
    /// Clang's `#pragma clang diagnostic fatal`, written as a `fatal error`.
    pub(crate) fatal:         bool,
    pub(crate) message:       &'d str,
    /// The message ends with the ` [-WNAME]` of the warning option that
    /// controls it, which bcc appended (see [`Explanation::option_suffix`]).
    pub(crate) option_suffix: bool,
    /// `cl`'s code for the diagnostic (`C2065`), which the MSVC formats
    /// write after the severity under the `cl` identity.
    pub(crate) code:          Option<&'static str>,
    pub(super) labels:        ArenaVec<'d, Label<'d>>,
    pub(super) notes:         ArenaVec<'d, &'d str>,
    pub(super) help:          ArenaVec<'d, &'d str>,
    /// The range of the token or syntax that the diagnostic was reported
    /// about, before it was moved to a place in a file, whose order says
    /// which reading of a file read twice it is in.
    pub(crate) reading:       Option<SourceVectors>,
}

impl<'d> Diagnostic<'d> {
    /// Adds the range the diagnostic is about. Its first location becomes the
    /// `-->` location.
    fn primary(mut self, source: SourceVectors, label: Option<&'d str>) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: label,
            primary: true,
        });
        self
    }

    /// Adds a related range, drawn with `-` instead of `^`.
    pub(crate) fn secondary(mut self, source: SourceVectors, label: &'d str) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: Some(label),
            primary: false,
        });
        self
    }

    /// Adds a range Clang's format underlines with `~` beside the caret,
    /// with no message; the other formats draw none.
    pub(crate) fn highlight(mut self, source: SourceVectors) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: None,
            primary: false,
        });
        self
    }

    /// Folds a follow-on diagnostic reported at the same place into this
    /// one, keeping only its related ranges: one mistake, one error.
    pub(crate) fn absorb(&mut self, other: Self, context: &Context<'_>) {
        for label in other.labels {
            let duplicate = self.labels.iter().any(|existing| {
                existing.message == label.message
                    && existing.source.vectors(context) == label.source.vectors(context)
            });
            if !label.primary && label.message.is_some() && !duplicate {
                self.labels.push(label);
            }
        }
    }

    /// The segments of the range the diagnostic is about, empty when it has
    /// no place.
    pub(crate) fn primary_segments<'a>(&self, context: &'a Context<'_>) -> &'a [SourceVector]
    where
        'd: 'a,
    {
        self.labels
            .iter()
            .find(|label| label.primary)
            .map_or(&[], |label| label.source.vectors(context))
    }

    /// Adds a related range given as explicit segments.
    pub(crate) fn secondary_segments(
        mut self,
        segments: &'d [SourceVector],
        label: &'d str,
    ) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Segments(segments),
            message: Some(label),
            primary: false,
        });
        self
    }
}

/// The location-independent part of a diagnostic: what went wrong, a short
/// label for the primary range, and any notes and help, all in one arena.
pub(crate) struct Explanation<'d> {
    arena:              &'d Bump,
    pub(crate) message: &'d str,
    pub(crate) label:   Option<&'d str>,
    pub(crate) notes:   ArenaVec<'d, &'d str>,
    pub(crate) help:    ArenaVec<'d, &'d str>,
    option_suffix:      bool,
    code:               Option<&'static str>,
}

impl<'d> Explanation<'d> {
    /// Marks the message as ending with the ` [-WNAME]` bcc appended for
    /// the warning option that controls it, as GCC and Clang print it,
    /// which the TCC format leaves out: TCC names no options. User text
    /// that merely ends that way (`#warning see [-Wfoo]`) keeps it.
    pub(crate) fn option_suffix(mut self) -> Self {
        self.option_suffix = true;
        self
    }

    pub(crate) fn label(mut self, label: &'d str) -> Self {
        self.label = Some(label);
        self
    }

    /// Gives the diagnostic `cl`'s code and drops bcc's notes and help:
    /// the message is then `cl`'s own, which stands alone.
    pub(crate) fn msvc_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self.notes.clear();
        self.help.clear();
        self
    }

    pub(crate) fn note(mut self, note: &'d str) -> Self {
        self.notes.push(note);
        self
    }

    pub(crate) fn help(mut self, help: &'d str) -> Self {
        self.help.push(help);
        self
    }

    /// Attaches the explanation to the source it is about.
    pub(crate) fn at(self, severity: ErrorSeverity, source: SourceVectors) -> Diagnostic<'d> {
        Diagnostic {
            severity,
            fatal: false,
            message: self.message,
            option_suffix: self.option_suffix,
            code: self.code,
            labels: ArenaVec::new_in(self.arena),
            notes: self.notes,
            help: self.help,
            reading: None,
        }
        .primary(source, self.label)
    }

    pub(crate) fn new(arena: &'d Bump, message: &'d str) -> Self {
        Self {
            arena,
            message,
            label: None,
            notes: ArenaVec::new_in(arena),
            help: ArenaVec::new_in(arena),
            option_suffix: false,
            code: None,
        }
    }

    /// The explanation with owned text, for tests to inspect.
    #[cfg(test)]
    #[expect(
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn to_owned_explanation(&self) -> OwnedExplanation {
        OwnedExplanation {
            message: self.message.to_owned(),
            label:   self.label.map(str::to_owned),
            notes:   self.notes.iter().map(|&note| note.to_owned()).collect(),
            help:    self.help.iter().map(|&help| help.to_owned()).collect(),
        }
    }
}

/// Conversion of a phase's error value into a user-facing diagnostic.
///
/// `source` is the error's primary range, already materialized by
/// [`GetSourceVectors`](crate::translation_phases::GetSourceVectors).
pub(crate) trait ToDiagnostic {
    /// Builds the diagnostic in `arena`.
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d>;

    /// The diagnostic with owned text, for tests to inspect.
    #[cfg(test)]
    fn to_diagnostic(&self, context: &Context<'_>, source: SourceVectors) -> OwnedDiagnostic {
        self.diagnostic_in(context, source, context.tu_arena())
            .to_owned_diagnostic()
    }
}

/// A source range the diagnostic points at, optionally with a short message
/// printed under it. A related range without a message is one Clang
/// underlines beside the caret ([`Diagnostic::highlight`]).
#[derive(Debug)]
pub(super) struct Label<'d> {
    pub(super) source:  LabelSource<'d>,
    pub(super) message: Option<&'d str>,
    pub(super) primary: bool,
}

/// Where a label points: a provenance range, or explicit segments when only
/// part of a range is relevant.
#[derive(Debug, Clone, Copy)]
pub(super) enum LabelSource<'d> {
    Range(SourceVectors),
    Segments(&'d [SourceVector]),
}

impl<'d> LabelSource<'d> {
    /// The provenance range, unless the label gives explicit segments.
    pub(super) const fn range(self) -> Option<SourceVectors> {
        match self {
            | Self::Range(source) => Some(source),
            | Self::Segments(_) => None,
        }
    }

    pub(super) fn vectors<'a>(self, context: &'a Context<'_>) -> &'a [SourceVector]
    where
        'd: 'a,
    {
        match self {
            | Self::Range(source) => context.get_source_vectors(source),
            | Self::Segments(segments) => segments,
        }
    }
}
