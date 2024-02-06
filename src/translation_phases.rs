use std::{
    borrow::Borrow,
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    ops::{
        Add,
        Deref,
    },
    sync::Arc,
};

use owo_colors::OwoColorize;
use smallvec::SmallVec;

use crate::util::string_cache::Id as StringCacheId;

pub(crate) mod phase_0_newline_tracking;
pub(crate) mod phase_1_map_character_sets;
pub(crate) mod phase_2_remove_escaped_newlines;
pub(crate) mod phase_3_preprocessor_tokenizer;
pub(crate) mod phase_4_preprocessing;

#[doc(hidden)]
#[macro_export]
macro_rules! bail {
    ($e:expr $(,)?) => {
        match $e {
            | Ok(v) => v,
            | Err(e) => return Some(Err(e.into())),
        }
    };
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
    Note,
}

impl Display for ErrorSeverity {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Warning => write!(f, "{}", "Warning".bright_yellow()),
            | Self::Error => write!(f, "{}", "Error".bright_red()),
            | Self::Note => write!(f, "{}", "Note".bright_blue()),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourcePosition {
    pub(crate) index:       usize,
    pub(crate) line:        usize,
    pub(crate) column:      usize,
    pub(crate) source_file: StringCacheId,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourceVector {
    pub(crate) position: SourcePosition,
    pub(crate) length:   usize,
}

pub(crate) struct SourceVectors {
    inner: SourceVectorsInner,
}

impl PartialEq for SourceVectors {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for SourceVectors {}

impl Clone for SourceVectors {
    fn clone(&self) -> Self {
        Self {
            inner: match &self.inner {
                | SourceVectorsInner::Empty => SourceVectorsInner::Empty,
                | SourceVectorsInner::Inline(sv) => SourceVectorsInner::Inline(*sv),
                | SourceVectorsInner::Multiple(svs) => SourceVectorsInner::Multiple(svs.clone()),
            },
        }
    }
}

impl Hash for SourceVectors {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state);
    }
}

fn add(lhs: &[SourceVector], rhs: &[SourceVector]) -> SourceVectors {
    let mut v = SmallVec::<[SourceVector; 1024]>::new();
    v.reserve(lhs.len() + rhs.len());
    v.extend_from_slice(lhs);
    v.extend_from_slice(rhs);
    v.as_ref().into()
}

impl Add<&SourceVectors> for &[SourceVector] {
    type Output = SourceVectors;

    fn add(self, rhs: &SourceVectors) -> Self::Output {
        add(self, rhs)
    }
}

impl Add<&[SourceVector]> for &SourceVectors {
    type Output = SourceVectors;

    fn add(self, rhs: &[SourceVector]) -> Self::Output {
        add(self, rhs)
    }
}

impl Add<&SourceVectors> for &SourceVectors {
    type Output = SourceVectors;

    fn add(self, rhs: &SourceVectors) -> Self::Output {
        add(self, rhs)
    }
}

enum SourceVectorsInner {
    Empty,
    Inline(SourceVector),
    Multiple(Arc<[SourceVector]>),
}

impl Debug for SourceVectors {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.deref().fmt(f)
    }
}

impl Deref for SourceVectors {
    type Target = [SourceVector];

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            | SourceVectorsInner::Inline(sv) => std::slice::from_ref(sv),
            | SourceVectorsInner::Multiple(svs) => svs,
            | SourceVectorsInner::Empty => &[],
        }
    }
}

impl AsRef<[SourceVector]> for SourceVectors {
    fn as_ref(&self) -> &[SourceVector] {
        self
    }
}

impl Borrow<[SourceVector]> for SourceVectors {
    fn borrow(&self) -> &[SourceVector] {
        self
    }
}

impl Default for SourceVectors {
    fn default() -> Self {
        Self {
            inner: SourceVectorsInner::Empty,
        }
    }
}

impl SourceVectors {
    #[allow(dead_code)]
    fn new() -> Self {
        Self::default()
    }
}

impl From<Arc<[SourceVector]>> for SourceVectors {
    fn from(v: Arc<[SourceVector]>) -> Self {
        Self {
            inner: match v.len() {
                | 0 => SourceVectorsInner::Empty,
                | 1 => SourceVectorsInner::Inline(v[0]),
                | _ => SourceVectorsInner::Multiple(v),
            },
        }
    }
}

impl From<SourceVector> for SourceVectors {
    fn from(v: SourceVector) -> Self {
        Self {
            inner: SourceVectorsInner::Inline(v),
        }
    }
}

impl<'a> From<&'a [SourceVector]> for SourceVectors {
    fn from(v: &'a [SourceVector]) -> Self {
        Self {
            inner: match v.len() {
                | 0 => SourceVectorsInner::Empty,
                | 1 => SourceVectorsInner::Inline(v[0]),
                | _ => SourceVectorsInner::Multiple(v.into()),
            },
        }
    }
}

pub(crate) trait SavePoint: Clone + Debug {
    fn current_position(&self) -> SourcePosition;
}

impl SavePoint for SourcePosition {
    fn current_position(&self) -> SourcePosition {
        *self
    }
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

impl GetSeverity for ErrorSeverity {
    fn severity(&self) -> ErrorSeverity {
        *self
    }
}

impl GetSeverity for Infallible {
    fn severity(&self) -> ErrorSeverity {
        match *self {}
    }
}

pub(crate) trait GetPosition {
    fn position(&self) -> SourcePosition;
}

impl GetPosition for SourcePosition {
    fn position(&self) -> SourcePosition {
        *self
    }
}

impl GetPosition for Infallible {
    fn position(&self) -> SourcePosition {
        match *self {}
    }
}

pub(crate) trait TranslationPhase:
    Iterator<Item = Result<Self::Yield, Self::Error>>
{
    type Yield;
    type SavePoint: SavePoint;
    type Error: std::error::Error + GetSeverity + GetPosition + PartialEq;
    fn save(&self) -> Self::SavePoint;
    fn restore(&mut self, save_point: Self::SavePoint);
    fn current_position(&self) -> SourcePosition;
    fn set_line_number(&mut self, line: usize);
}
