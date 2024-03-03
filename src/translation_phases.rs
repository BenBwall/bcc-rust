use std::{
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    num::NonZeroU32,
    path::PathBuf,
};

use crate::util::{
    shared::{
        SharedPath,
        SharedString,
    },
    string_cache::StringCache,
};

const ONE: NonZeroU32 = match NonZeroU32::new(1) {
    | Some(one) => one,
    | None => panic!("NonZeroU32::new(1) failed"),
};

#[allow(dead_code)]
#[derive(Error, Debug)]
pub(crate) enum TranslationError {
    #[error(transparent)]
    InitialProcessing(InitialProcessorError),
    #[error(transparent)]
    PreprocessorTokenizining(PreprocessorTokenizerError),
    #[error(transparent)]
    Preprocessing(PreprocessorError),
    #[error(transparent)]
    Parsing(ParserError),
}

impl GetSeverity for TranslationError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::InitialProcessing(error) => error.severity(),
            | Self::PreprocessorTokenizining(error) => error.severity(),
            | Self::Preprocessing(error) => error.severity(),
            | Self::Parsing(error) => error.severity(),
        }
    }
}

impl GetPosition for TranslationError {
    fn position(&self, context: &Context) -> SourcePosition {
        match self {
            | Self::InitialProcessing(error) => error.position(context),
            | Self::PreprocessorTokenizining(error) => error.position(context),
            | Self::Preprocessing(error) => error.position(context),
            | Self::Parsing(error) => error.position(context),
        }
    }
}

impl GetSourceVectors for TranslationError {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors {
        match self {
            | Self::InitialProcessing(error) => error.source_vectors(context),
            | Self::PreprocessorTokenizining(error) => error.source_vectors(context),
            | Self::Preprocessing(error) => error.source_vectors(context),
            | Self::Parsing(error) => error.source_vectors(context),
        }
    }
}

use owo_colors::OwoColorize;
use smallstr::SmallString;
use thiserror::Error;

use self::{
    initial_processing::InitialProcessorError,
    parsing::ParserError,
    preprocessing::PreprocessorError,
    preprocessor_tokenizer::PreprocessorTokenizerError,
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
    /// or `None` if the index is out of bounds or is in the middle of a
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
    pub(crate) index:  usize,
    pub(crate) line:   u32,
    pub(crate) column: NonZeroU32,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            index:  0,
            line:   1,
            column: ONE,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub(crate) struct SourceVector {
    pub(crate) index:       usize,
    pub(crate) column:      NonZeroU32,
    pub(crate) line:        u32,
    pub(crate) source_file: SharedPath,
    pub(crate) length:      usize,
}

impl Default for SourceVector {
    fn default() -> Self {
        Self {
            index:       0,
            column:      ONE,
            line:        1,
            source_file: SharedPath::from_path_buf(PathBuf::new()),
            length:      0,
        }
    }
}

impl GetPosition for SourceVector {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index,
            line:   self.line,
            column: self.column,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash, Copy, Default)]
pub(crate) struct SourceVectors {
    start_index: u32,
    length:      u32,
}

impl GetPosition for SourceVectors {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        let start = &context.source_vectors.0[self.start_index as usize];
        SourcePosition {
            index:  start.index,
            line:   start.line,
            column: start.column,
        }
    }
}

impl GetSourceVectors for SourceVectors {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
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

pub(crate) trait GetSourceFileName {
    fn source_file_name(&self) -> SharedPath;
}

pub(crate) trait GetPosition {
    fn position(&self, context: &Context) -> SourcePosition;
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn index(&self, context: &Context) -> usize {
        self.position(context).index
    }
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn column(&self, context: &Context) -> NonZeroU32 {
        self.position(context).column
    }
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn line(&self, context: &Context) -> u32 {
        self.position(context).line
    }
}

pub(crate) trait GetSourceVectors {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors;
}

pub(crate) trait SetPosition: GetPosition {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition);
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn set_index(&mut self, context: &mut Context, index: usize) {
        self.set_position(
            context,
            SourcePosition {
                index,
                line: self.line(context),
                column: self.column(context),
            },
        );
    }
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn set_column(&mut self, context: &mut Context, column: NonZeroU32) {
        self.set_position(
            context,
            SourcePosition {
                index: self.index(context),
                line: self.line(context),
                column,
            },
        );
    }
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn set_line(&mut self, context: &mut Context, line: u32) {
        self.set_position(
            context,
            SourcePosition {
                index: self.index(context),
                line,
                column: self.column(context),
            },
        );
    }
}

pub(crate) trait SetSourceFileName {
    fn set_source_file_name(&mut self, context: &mut Context, name: SharedPath);
}

impl GetPosition for SourcePosition {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        *self
    }
}

impl GetPosition for Infallible {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        match *self {}
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SourceFile {
    pub(crate) name:   SharedPath,
    pub(crate) source: SharedString,
    pub(crate) line:   u32,
    pub(crate) column: NonZeroU32,
    pub(crate) index:  usize,
}

impl Default for SourceFile {
    fn default() -> Self {
        Self {
            name:   SharedPath::from_path_buf(PathBuf::new()),
            source: SharedString::default(),
            line:   1,
            column: ONE,
            index:  0,
        }
    }
}

impl SourceFile {
    pub(crate) fn new(name: SharedPath, source: SharedString) -> Self {
        Self {
            name,
            source,
            line: 1,
            column: ONE,
            index: 0,
        }
    }
}

impl GetPosition for SourceFile {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index,
            line:   self.line,
            column: self.column,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SourceVectorStack(Vec<SourceVector>);

impl Display for SourceVectorStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (i, vector) in self.0.iter().enumerate() {
            writeln!(
                f,
                "SourceVector {}: index: {}, line: {}, column: {}, length: {}",
                i, vector.index, vector.line, vector.column, vector.length
            )?;
        }
        Ok(())
    }
}

pub(crate) struct Context {
    pub(crate) source_vectors:    SourceVectorStack,
    pub(crate) string_cache:      StringCache,
    is_tokenizing_include_string: bool,
    is_skipping_over_dead_code:   bool,
    pending_errors:               Vec<TranslationError>,
}

impl Context {
    pub(crate) fn new() -> Self {
        Self {
            source_vectors:               SourceVectorStack(Vec::new()),
            string_cache:                 StringCache::new(),
            is_tokenizing_include_string: false,
            is_skipping_over_dead_code:   false,
            pending_errors:               Vec::new(),
        }
    }

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        source_file: SharedPath,
        length: usize,
    ) -> u32 {
        let index = self.source_vectors.0.len().try_into().unwrap();
        self.source_vectors.0.push(SourceVector {
            index: start_position.index,
            column: start_position.column,
            line: start_position.line,
            source_file,
            length,
        });
        index
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn duplicate_source_vectors(
        self_source_vectors: &mut Vec<SourceVector>,
        source_vectors: SourceVectors,
    ) -> u32 {
        assert!(
            u32::try_from(source_vectors.length as usize + self_source_vectors.len()).is_ok(),
            "overflow in duplicate_source_vectors"
        );
        let start_index = self_source_vectors.len() as u32;
        for i in source_vectors.start_index..source_vectors.start_index + source_vectors.length {
            self_source_vectors.push(self_source_vectors[i as usize].clone());
        }
        start_index
    }

    pub(crate) fn create_source_vectors(
        &mut self,
        start_position: SourcePosition,
        source_file: SharedPath,
        length: usize,
    ) -> SourceVectors {
        let start_index = self.push_source_vector(start_position, source_file, length);
        let length = 1;
        SourceVectors {
            start_index,
            length,
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn merge_vectors(&mut self, v1: SourceVectors, v2: SourceVectors) -> SourceVectors {
        let start_index = self.source_vectors.0.len() as u32;
        for i in v1.start_index..v1.start_index + v1.length {
            self.source_vectors
                .0
                .push(self.source_vectors.0[i as usize].clone());
        }
        for i in v2.start_index..v2.start_index + v2.length {
            self.source_vectors
                .0
                .push(self.source_vectors.0[i as usize].clone());
        }
        assert!(
            u32::try_from(self.source_vectors.0.len()).is_ok(),
            "overflow in merge_vectors"
        );
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

    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(crate) fn missing_final_newline(&mut self, vector: SourceVector) {
        if !self.is_skipping_over_dead_code() {
            self.pending_errors
                .push(TranslationError::InitialProcessing(
                    InitialProcessorError::MissingFinalNewline(vector),
                ));
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_tokenizer_error(&mut self, error: PreprocessorTokenizerError) {
        if !self.is_skipping_over_dead_code() {
            self.pending_errors
                .push(TranslationError::PreprocessorTokenizining(error));
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_error(&mut self, error: PreprocessorError) {
        self.pending_errors
            .push(TranslationError::Preprocessing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn raw_preprocessor_error(
        self_pending_errors: &mut Vec<TranslationError>,
        error: PreprocessorError,
    ) {
        self_pending_errors.push(TranslationError::Preprocessing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_pending_error(&mut self) -> Option<TranslationError> {
        self.pending_errors.pop()
    }

    pub(crate) fn get_source_vectors(&self, source_vectors: SourceVectors) -> &[SourceVector] {
        let start_index = source_vectors.start_index as usize;
        let end_index = start_index + source_vectors.length as usize;
        &self.source_vectors.0[start_index..end_index]
    }
}

pub(crate) trait TranslationPhase:
    GetPosition + SetPosition + GetSourceFileName + SetSourceFileName
{
    type Item;
    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item>;
}
