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
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact of how the diagnostic is written."
)]
pub(crate) struct Diagnostic<'d> {
    pub(crate) severity:         ErrorSeverity,
    /// Clang's `#pragma clang diagnostic fatal`, written as a `fatal error`.
    pub(crate) fatal:            bool,
    /// A diagnostic pragma classified it, which keeps an error from Clang's
    /// `-Wfatal-errors` (`NoErrorAsFatal`, which
    /// `DiagnosticsEngine::makeUserMapping` sets on a pragma's mapping).
    pub(crate) pragma_mapped:    bool,
    pub(crate) message:          &'d str,
    /// The message ends with the ` [-WNAME]` of the warning option that
    /// controls it, which bcc appended (see [`Explanation::option_suffix`]).
    pub(crate) option_suffix:    bool,
    /// `cl`'s code for the diagnostic (`C2065`), which the MSVC formats
    /// write after the severity under the `cl` identity.
    pub(crate) code:             Option<&'static str>,
    /// `cl`'s `/WX` made this warning an error, so it is written as a
    /// warning, after `error C2220: the following warning is treated as an
    /// error` when it is the first: [`TreatedAsError`].
    pub(crate) treated_as_error: TreatedAsError,
    /// Clang names a place rather than a range for it, as for its
    /// "expected" errors and some warnings: Clang's snippet marks only where
    /// the primary range starts, with no `~` over the token there, and its
    /// macro notes' carets are points too.
    pub(crate) point:            bool,
    /// Clang names a place rather than a range for each related note,
    /// whose carets are then points.
    pub(crate) related_points:   bool,
    pub(super) labels:           ArenaVec<'d, Label<'d>>,
    pub(super) notes:            ArenaVec<'d, &'d str>,
    pub(super) help:             ArenaVec<'d, &'d str>,
    /// The edits GCC and Clang suggest with it, which they print under the
    /// source they change.
    pub(super) fix_its:          ArenaVec<'d, FixIt<'d>>,
    /// The range of the token or syntax that the diagnostic was reported
    /// about, before it was moved to a place in a file, whose order says
    /// which reading of a file read twice it is in.
    pub(crate) reading:          Option<SourceVectors>,
    /// `cl`'s code generator reported it, which `cl` leads with the file's
    /// full path and the line alone: `C:\src\a.c(3) : warning C4715: ...`.
    pub(crate) back_end:         bool,
    /// `cl`'s notes, which the MSVC formats write after the message, each
    /// on a line of its own led by its place: `file(line): note: text`.
    pub(crate) msvc_notes:       ArenaVec<'d, MsvcNote<'d>>,
}

/// One of `cl`'s notes: its words, at the diagnostic's own place or at
/// another (`see declaration of 'S'`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MsvcNote<'d> {
    pub(crate) source: Option<SourceVectors>,
    pub(crate) text:   &'d str,
}

/// Whether `cl`'s `/WX` made a warning an error. `cl` then writes `error
/// C2220: the following warning is treated as an error` before the first
/// such warning of a translation unit, and every one as a warning.
/// Microsoft Learn: "Compiler Error C2220".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TreatedAsError {
    No,
    /// The first in the translation unit, which C2220 precedes.
    First,
    Later,
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

    /// Adds one of `cl`'s notes, at `source` or else at the diagnostic's
    /// place.
    pub(crate) fn msvc_note(mut self, source: Option<SourceVectors>, text: &'d str) -> Self {
        self.msvc_notes.push(MsvcNote { source, text });
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

    /// Suggests `text` at `place`, which GCC and Clang print under the
    /// diagnostic's source.
    pub(crate) fn fix_it(mut self, place: FixItPlace, text: &'d str) -> Self {
        self.fix_its.push(FixIt {
            place,
            text,
            note: None,
        });
        self
    }

    /// Suggests `text` at `place` with the related note added last, under
    /// whose source GCC and Clang print it.
    pub(crate) fn note_fix_it(mut self, place: FixItPlace, text: &'d str) -> Self {
        let note = self.labels.len().checked_sub(1);
        self.fix_its.push(FixIt { place, text, note });
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
            pragma_mapped: false,
            message: self.message,
            option_suffix: self.option_suffix,
            code: self.code,
            treated_as_error: TreatedAsError::No,
            point: false,
            related_points: false,
            labels: ArenaVec::new_in(self.arena),
            notes: self.notes,
            help: self.help,
            fix_its: ArenaVec::new_in(self.arena),
            reading: None,
            back_end: false,
            msvc_notes: ArenaVec::new_in(self.arena),
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

impl Label<'_> {
    /// Whether this is a range Clang underlines beside the caret.
    pub(super) const fn is_highlight(&self) -> bool {
        !self.primary && self.message.is_none()
    }
}

/// An edit GCC and Clang suggest with a diagnostic, its fix-it hint: `text`
/// put at `place`, which a removal leaves empty. Each prints it under the
/// source line it changes, and the formats of bcc, MSVC and TCC drop it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixIt<'d> {
    pub(crate) place: FixItPlace,
    pub(crate) text:  &'d str,
    /// The label of the related note it is printed with, by its index, or
    /// `None` for the diagnostic's own snippet.
    pub(super) note:  Option<usize>,
}

/// Where a fix-it hint changes the source, by the tokens of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixItPlace {
    /// Before the first token of the range: an insertion.
    Before(SourceVectors),
    /// After the last token of the range: an insertion.
    After(SourceVectors),
    /// The range's tokens, from the first to the end of the last, which the
    /// text replaces.
    Replace(SourceVectors),
    /// The bytes from an edge of one range's tokens to an edge of
    /// another's, which the text replaces: each the start of the range's
    /// first token, or with `true` the end of its last, as Clang's character
    /// ranges run from the end of one statement to the start of the next.
    Span {
        from: (SourceVectors, bool),
        to:   (SourceVectors, bool),
    },
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
