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
    num::NonZeroU32,
    ops::{
        Add,
        Deref,
    },
    path::Path,
    sync::Arc,
};

use crate::util::string_cache::{
    StringCache,
    StringCacheId,
};

const ONE: NonZeroU32 = NonZeroU32::new(1).unwrap();

#[derive(Error)]
enum TranslationError {
    #[error(transparent)]
    InitialProcessing(InitialProcessorError),
    #[error(transparent)]
    PreprocessorTokenizining(PreprocessorTokenizerError),
    #[error(transparent)]
    Preprocessing(PreprocessorError),
    #[error(transparent)]
    Parsing(ParserError),
}

use owo_colors::OwoColorize;
use smallstr::SmallString;
use smallvec::SmallVec;
use thiserror::Error;

use self::{
    initial_processing::InitialProcessorError,
    parsing::{
        ParserError,
        TopLevelStatement,
    },
    phase_0_newline_tracking::NewlineTracking,
    phase_1_map_character_sets::MapCharacterSets,
    phase_2_remove_escaped_newlines::{
        RemoveEscapedNewlines,
        RemoveEscapedNewlinesError,
    },
    preprocessing::{
        PreprocessorError,
        Token,
    },
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenizer,
        PreprocessorTokenizerError,
    },
};
use crate::util::string_cache::{
    Interner,
    StringCacheId,
};

pub(crate) mod initial_processing;
pub(crate) mod parsing;
pub(crate) mod preprocessing;
pub(crate) mod preprocessor_tokenizer;

pub(crate) type TokenString = SmallString<[u8; 1024]>;

trait NonZeroExt {
    fn saturating_add_assign(&mut self, num: u32);
}

impl NonZeroExt for NonZeroU32 {
    fn saturating_add_assign(&mut self, num: u32) {
        *self = self.saturating_add(num);
    }
}

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
    pub(crate) index:       u32,
    pub(crate) line:        u32,
    pub(crate) column:      NonZeroU32,
    pub(crate) source_file: StringCacheId,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            index:       0,
            line:        1,
            column:      ONE,
            source_file: StringCacheId::from_u32(0),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourceVector {
    pub(crate) position: SourcePosition,
    pub(crate) length:   u32,
}

impl Default for SourceVector {
    fn default() -> Self {
        Self {
            position: SourcePosition::default(),
            length:   0,
        }
    }
}

pub(crate) struct SourceVectors {
    start_index: u32,
    length:      u32,
}

impl Default for SourceVectors {
    fn default() -> Self {
        Self {
            start_index: 0,
            length:      0,
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
    pub(crate) name:   Box<Path>,
    pub(crate) source: Box<str>,
    pub(crate) line:   u32,
    pub(crate) column: NonZeroU32,
    pub(crate) index:  u32,
}

impl GetPosition for SourceFile {
    fn position(&self) -> SourcePosition {
        SourcePosition {
            index:       self.index,
            line:        self.line,
            column:      self.column,
            source_file: self.name,
        }
    }
}

pub(crate) struct Context {
    source: SourceFile,
    source_stack: Vec<SourceFile>,
    source_vectors: Vec<SourceVector>,
    string_cache: StringCache,
    is_tokenizing_include_string: bool,
    is_skipping_over_dead_code: bool,
    pending_errors: Vec<TranslationError>,
}

impl GetPosition for Context {
    fn position(&self) -> SourcePosition {
        self.source.position()
    }
}

impl Context {
    fn new(source: Box<str>, name: Box<Path>) -> Self {
        Self {
            source: SourceFile {
                name,
                source,
                line: 1,
                column: ONE,
                index: 0,
            },
            source_stack: Vec::new(),
            source_vectors: Vec::new(),
            string_cache: Interner::new(),
            is_tokenizing_include_string: false,
            is_skipping_over_dead_code: false,
            pending_errors: Vec::new(),
        }
    }

    #[cold]
    pub(crate) fn pop_source(&mut self) -> bool {
        let Some(source) = self.source_stack.pop() else {
            return false;
        };
        self.source = source;
    }

    #[cold]
    pub(crate) fn push_source(&mut self, source: Box<str>, name: Box<Path>) {
        let source_file = SourceFile {
            name,
            source,
            line: 1,
            column: ONE,
            index: 0,
        };
        self.source_stack
            .push(std::mem::replace(&mut self.source, source_file));
    }

    fn next_char(&mut self) -> Option<char> {
        self.source.source.char_at(self.source.index as usize)
    }

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        length: u32,
    ) -> u32 {
        let index = self.source_vectors.len().try_into().unwrap();
        self.source_vectors.push(SourceVector {
            position: start_position,
            length,
        });
        index
    }

    pub(crate) fn merge_vectors(&mut self, v1: SourceVectors, v2: SourceVectors) -> SourceVectors {
        let start_index = self.source_vectors.len();
        for i in v1.start_index..v1.start_index + v1.length {
            self.source_vectors.push(self.source_vectors[i]);
        }
        for i in v2.start_index..v2.start_index + v2.length {
            self.source_vectors.push(self.source_vectors[i]);
        }
        let length = v1.length + v2.length;
        SourceVectors {
            start_index,
            length,
        }
    }

    pub(crate) fn is_tokenizing_include_string(&self) -> bool {
        self.is_tokenizing_include_string
    }

    pub(crate) fn set_is_tokenizing_include_string(&mut self, value: bool) {
        self.is_tokenizing_include_string = value;
    }

    pub(crate) fn is_skipping_over_dead_code(&self) -> bool {
        self.is_skipping_over_dead_code
    }

    pub(crate) fn set_is_skipping_over_dead_code(&mut self, value: bool) {
        self.is_skipping_over_dead_code = value;
    }

    pub(crate) fn missing_final_newline(&mut self) {
        if !self.is_skipping_over_dead_code {
            self.pending_errors
                .push(TranslationError::InitialProcessing(
                    InitialProcessorError::MissingFinalNewLine(self.source.position()),
                ));
        }
    }

    pub(crate) fn preprocessor_tokenizer_error(&mut self, error: PreprocessorTokenizerError) {
        if !self.is_skipping_over_dead_code {
            self.pending_errors
                .push(TranslationError::PreprocessorTokenizining(error));
        }
    }

    pub(crate) fn preprocessor_error(&mut self, error: PreprocessorError) {
        self.pending_errors
            .push(TranslationError::Preprocessing(error));
    }
}

pub(crate) trait TranslationPhase {
    type Item;
    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item>;
}
