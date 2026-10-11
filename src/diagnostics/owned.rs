//! Owned diagnostic views let tests inspect and compare arena-backed
//! messages after their compilation context has gone away.

use super::{
    Diagnostic,
    model::{
        Label,
        LabelSource,
    },
};
use crate::{
    translation_phases::{
        ErrorSeverity,
        SourceVector,
        SourceVectors,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// An [`Explanation`] with owned text, for tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct OwnedExplanation {
    pub(crate) message: String,
    pub(crate) label:   Option<String>,
    pub(crate) notes:   Vec<String>,
    pub(crate) help:    Vec<String>,
}

/// A [`Diagnostic`] with owned text, for tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedDiagnostic {
    pub(crate) severity: ErrorSeverity,
    pub(crate) message:  String,
    pub(super) labels:   Vec<(OwnedLabelSource, Option<String>, bool)>,
    pub(super) notes:    Vec<String>,
    pub(super) help:     Vec<String>,
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum OwnedLabelSource {
    Range(SourceVectors),
    Segments(Vec<SourceVector>),
}

#[cfg(test)]
impl Diagnostic<'_> {
    #[expect(
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn to_owned_diagnostic(&self) -> OwnedDiagnostic {
        OwnedDiagnostic {
            severity: self.severity,
            message:  self.message.to_owned(),
            labels:   self
                .labels
                .iter()
                .map(|label| {
                    let source = match label.source {
                        | LabelSource::Range(source) => OwnedLabelSource::Range(source),
                        | LabelSource::Segments(segments) =>
                            OwnedLabelSource::Segments(segments.to_vec()),
                    };
                    (source, label.message.map(str::to_owned), label.primary)
                })
                .collect(),
            notes:    self.notes.iter().map(|&note| note.to_owned()).collect(),
            help:     self.help.iter().map(|&help| help.to_owned()).collect(),
        }
    }
}

#[cfg(test)]
impl OwnedDiagnostic {
    /// The diagnostic, borrowing this one's text, with its lists in `arena`.
    #[expect(
        clippy::disallowed_types,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(super) fn borrowed<'d>(&'d self, arena: &'d Bump) -> Diagnostic<'d> {
        let mut labels = ArenaVec::new_in(arena);
        labels.extend(self.labels.iter().map(|(source, message, primary)| Label {
            source:  match source {
                | OwnedLabelSource::Range(source) => LabelSource::Range(*source),
                | OwnedLabelSource::Segments(segments) => LabelSource::Segments(segments),
            },
            message: message.as_deref(),
            primary: *primary,
        }));
        let mut notes = ArenaVec::new_in(arena);
        notes.extend(self.notes.iter().map(String::as_str));
        let mut help = ArenaVec::new_in(arena);
        help.extend(self.help.iter().map(String::as_str));
        Diagnostic {
            severity: self.severity,
            message: &self.message,
            labels,
            notes,
            help,
        }
    }
}
