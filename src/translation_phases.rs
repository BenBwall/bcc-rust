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
    path::PathBuf,
    sync::Arc,
};

#[derive(Error)]
enum TranslationError {
    #[error(transparent)]
    Phase0(Infallible),
    #[error(transparent)]
    Phase1(Infallible),
    #[error(transparent)]
    Phase2(RemoveEscapedNewlinesError),
    #[error(transparent)]
    Phase3(PreprocessorTokenizerError),
    #[error(transparent)]
    Phase4(PreprocessingError),
    #[error(transparent)]
    Phase5(ParsingError),
}

pub(crate) fn get_next<T: TranslationPhase>(
    phase: &mut T,
    context: &mut Context,
    input: Option<T::Input>,
) -> Result<Option<T::Yield>, T::Error> {
    match input {
        | Some(input) => phase.next_item(input, context),
        | None => phase.eoi(context),
    }
}

#[cfg(test)]
macro_rules! run {
    ($input: expr, $name: expr, $($source_sensitive_phases: ident),+ ---- $($translation_phase:ident),+) => {
        {
            use crate::translation_phases::{
                Context,
                TranslationError,
                get_next,
            };
            use crate::util::string_cache::StringCache;
            $(let mut $translation_phase = $translation_phase;)*
            let mut string_cache = StringCache::new();
            let name = string_cache.intern($name);
            let mut context = Context::new($input.into(), name, string_cache);
            let mut result = Vec::new();
            loop {
                let next = context.next_char();
                let mut previous_returned_some = false;
                $(
                    let next = match get_next(&mut $translation_phase, &mut context, next) {
                        | Ok(Some(v)) => {
                            previous_returned_some = true;
                            Some(v)
                        },
                        | Ok(None) => {
                            if previous_returned_some {
                                continue;
                            }
                            None
                        },
                        | Err(e) => {
                            let e: TranslationError = e.into();
                            result.push(Err(Box::new(e)));
                            continue;
                        }
                    };
                )*
                if let Some(next) = next {
                    result.push(Ok(next));
                } else {
                    if !previous_returned_some {
                        break;
                    }
                }
            }
            result
        }
    }
}

use owo_colors::OwoColorize;
use smallstr::SmallString;
use smallvec::SmallVec;
use thiserror::Error;

use self::{
    phase_0_newline_tracking::NewlineTracking,
    phase_1_map_character_sets::MapCharacterSets,
    phase_2_remove_escaped_newlines::{
        RemoveEscapedNewlines,
        RemoveEscapedNewlinesError,
    },
    phase_3_preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenizer,
        PreprocessorTokenizerError,
    },
    phase_4_preprocessing::{
        PreprocessingError,
        Token,
    },
    phase_5_parsing::{
        ParsingError,
        TopLevelStatement,
    },
};
use crate::util::string_cache::{
    Id as StringCacheId,
    Interner,
};

pub(crate) mod phase_0_newline_tracking;
pub(crate) mod phase_1_map_character_sets;
pub(crate) mod phase_2_remove_escaped_newlines;
pub(crate) mod phase_3_preprocessor_tokenizer;
pub(crate) mod phase_4_preprocessing;
pub(crate) mod phase_5_parsing;

pub(crate) type TokenString = SmallString<[u8; 1024]>;

trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or index is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;

    /// Returns the character at the given index but in lowercase,
    /// See [`StrExt::char_at`] for more information.
    fn char_at_case_insensitive(&self, index: usize) -> Option<char> {
        self.char_at(index).map(|c| c.to_ascii_lowercase())
    }

    /// Returns the string cloned into a [`TokenString`].
    fn to_token_string(&self) -> TokenString
    where
        for<'a> &'a Self: Into<TokenString>,
    {
        self.into()
    }
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

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

pub(crate) struct SourceFile {
    pub(crate) name:          PathBuf,
    pub(crate) source:        Box<str>,
    pub(crate) line_number:   usize,
    pub(crate) column_number: usize,
    pub(crate) index:         usize,
}

impl SourceFile {
    pub(crate) fn position(&self) -> SourcePosition {
        SourcePosition {
            index:       self.index,
            line:        self.line_number,
            column:      self.column_number,
            source_file: self.name,
        }
    }
}

pub(crate) struct Context {
    pub(crate) source: SourceFile,
    source_stack:      Vec<SourceFile>,
    phase_0_stack:     Vec<NewlineTracking>,
    phase_1_stack:     Vec<MapCharacterSets>,
    phase_2_stack:     Vec<RemoveEscapedNewlines>,
    phase_3_stack:     Vec<PreprocessorTokenizer>,
    current_phase_0:   NewlineTracking,
    current_phase_1:   MapCharacterSets,
    current_phase_2:   RemoveEscapedNewlines,
    current_phase_3:   PreprocessorTokenizer,
    string_cache:      Interner,
}

impl Context {
    fn new(source: Box<str>, name: PathBuf) -> Self {
        Self {
            source:          SourceFile {
                name,
                source,
                line_number: 1,
                column_number: 1,
                index: 0,
            },
            source_stack:    Vec::new(),
            phase_0_stack:   Vec::new(),
            phase_1_stack:   Vec::new(),
            phase_2_stack:   Vec::new(),
            phase_3_stack:   Vec::new(),
            current_phase_0: NewlineTracking::new(),
            current_phase_1: MapCharacterSets::new(),
            current_phase_2: RemoveEscapedNewlines::new(),
            current_phase_3: PreprocessorTokenizer::new(),
            string_cache:    Interner::new(),
        }
    }

    #[cold]
    pub(crate) fn push_frame(
        &mut self,
        phase_0: NewlineTracking,
        phase_1: MapCharacterSets,
        phase_2: RemoveEscapedNewlines,
        phase_3: PreprocessorTokenizer,
    ) {
        self.phase_0_stack.push(phase_0);
        self.phase_1_stack.push(phase_1);
        self.phase_2_stack.push(phase_2);
        self.phase_3_stack.push(phase_3);
    }

    #[cold]
    pub(crate) fn pop_frame(&mut self) {
        self.current_phase_0 = self.phase_0_stack.pop().unwrap();
        self.current_phase_1 = self.phase_1_stack.pop().unwrap();
        self.current_phase_2 = self.phase_2_stack.pop().unwrap();
        self.current_phase_3 = self.phase_3_stack.pop().unwrap();
    }

    #[cold]
    pub(crate) fn pop_source(&mut self) -> bool {
        let Some(source) = self.source_stack.pop() else {
            return false;
        };
        self.source = source;
    }

    #[cold]
    pub(crate) fn push_source(&mut self, source: Box<str>, name: StringCacheId) {
        let source_file = SourceFile {
            name,
            source,
            line_number: 1,
            column_number: 1,
            index: 0,
        };
        self.source_stack
            .push(std::mem::replace(&mut self.source, source_file));
    }

    fn next_char(&mut self) -> Option<char> {
        loop {
            if let Some(c) = self.source.source.char_at(self.source.index) {
                self.source.index += c.len_utf8();
                return Some(c);
            }
            if !self.pop_source() {
                return None;
            }
        }
    }
}

pub(crate) struct TestArgs {
    pub(crate) expected_from_phase_0: Vec<Result<char, Infallible>>,
    pub(crate) expected_from_phase_1: Vec<Result<char, Infallible>>,
    pub(crate) expected_from_phase_2: Vec<Result<char, RemoveEscapedNewlinesError>>,
    pub(crate) expected_from_phase_3: Vec<Result<PreprocessorToken, PreprocessorTokenizerError>>,
    pub(crate) expected_from_phase_4: Vec<Result<Token, PreprocessingError>>,
    pub(crate) expected_from_phase_5: Vec<Result<TopLevelStatement, ParsingError>>,
    pub(crate) input:                 Box<str>,
    pub(crate) name:                  PathBuf,
}

#[cfg(test)]
pub(crate) fn test(args: TestArgs) {
    let TestArgs {
        expected_from_phase_0,
        expected_from_phase_1,
        expected_from_phase_2,
        expected_from_phase_3,
        expected_from_phase_4,
        expected_from_phase_5,
        input,
        name,
    } = args;
    let mut phase4 = Preprocessing::new();
    let mut phase5 = Parsing::new();
    let mut from_phase_0 = Vec::new();
    let mut from_phase_1 = Vec::new();
    let mut from_phase_2 = Vec::new();
    let mut from_phase_3 = Vec::new();
    let mut from_phase_4 = Vec::new();
    let mut from_phase_5 = Vec::new();
    let mut context = Context::new(input, name);
    let mut result = Vec::new();
    loop {
        let next = context.next_char();
        let mut previous_returned_some = false;
        macro_rules! handle_next {
            ($pending:expr, $res_vec:ident) => {
                $res_vec.push($pending.clone());
                let next = match $pending {
                    | Ok(Some(v)) => {
                        previous_returned_some = true;
                        Some(v)
                    },
                    | Ok(None) => {
                        if previous_returned_some {
                            continue;
                        }
                        None
                    },
                    | Err(e) => {
                        let e: TranslationError = e.into();
                        result.push(Err(Box::new(e)));
                        continue;
                    },
                };
            };
        }
        let mut phase_0 = context.current_phase_0;
        let pending = phase_0.next_item(&mut context);
        context.current_phase_0 = phase_0;
        handle_next!(pending, from_phase_0);
        let mut phase_1 = context.current_phase_1;
        let pending = phase_1.next_item(next, &mut context);
        context.current_phase_1 = phase_1;
        handle_next!(pending, from_phase_1);
        let mut phase_2 = context.current_phase_2;
        let pending = phase_2.next_item(next, &mut context);
        context.current_phase_2 = phase_2;
        handle_next!(pending, from_phase_2);
        let mut phase_3 = context.current_phase_3;
        let pending = phase_3.next_item(next, &mut context);
        context.current_phase_3 = phase_3;
        handle_next!(pending, from_phase_3);
        let pending = phase4.next_item(next, &mut context);
        handle_next!(pending, from_phase_4);
        let pending = phase5.next_item(next, &mut context);
        handle_next!(pending, from_phase_5);
        if let Some(next) = next {
            result.push(Ok(next));
        } else if !previous_returned_some {
            break;
        }
    }
    result
}
