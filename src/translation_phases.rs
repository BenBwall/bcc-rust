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

pub(crate) trait TranslationPhase {
    type Error: std::error::Error + GetPosition + GetSeverity;
    type Yield;
    fn next(&mut self) -> Result<Option<Self::Yield>, Self::Error>;
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
    pub(crate) index:       usize,
    pub(crate) line:        u32,
    pub(crate) column:      u32,
    pub(crate) source_file: StringCacheId,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            index:       0,
            line:        1,
            column:      1,
            source_file: StringCacheId::from_usize(0),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourceVector {
    pub(crate) position: SourcePosition,
    pub(crate) length:   usize,
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
    start_index: usize,
    length:      usize,
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
    pub(crate) source:            SourceFile,
    source_stack:                 Vec<SourceFile>,
    source_vectors:               Vec<SourceVector>,
    string_cache:                 Interner,
    is_tokenizing_include_string: bool,
}

impl Context {
    fn new(source: Box<str>, name: PathBuf) -> Self {
        Self {
            source: SourceFile {
                name,
                source,
                line_number: 1,
                column_number: 1,
                index: 0,
            },
            source_stack: Vec::new(),
            source_vectors: Vec::new(),
            string_cache: Interner::new(),
            is_tokenizing_include_string: false,
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

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        length: usize,
    ) -> usize {
        let index = self.source_vectors.len();
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

    pub(crate) fn current_position(&self) -> SourcePosition {
        self.source.position()
    }
}

pub(crate) struct TestArgs {
    pub(crate) from_phase_0: Option<&mut Vec<char>>,
    pub(crate) from_phase_1: Option<&mut Vec<char>>,
    pub(crate) from_phase_2: Option<&mut Vec<Result<char, RemoveEscapedNewlinesError>>>,
    pub(crate) from_phase_3:
        Option<&mut Vec<Result<PreprocessorToken, PreprocessorTokenizerError>>>,
    pub(crate) from_phase_4: Option<&mut Vec<Result<Token, PreprocessingError>>>,
    pub(crate) from_phase_5: Option<&mut Vec<Result<TopLevelStatement, ParsingError>>>,
    pub(crate) input:        Box<str>,
    pub(crate) name:         PathBuf,
}

impl Default for TestArgs {
    fn default() -> Self {
        Self {
            from_phase_0: None,
            from_phase_1: None,
            from_phase_2: None,
            from_phase_3: None,
            from_phase_4: None,
            from_phase_5: None,
            input:        Box::default(),
            name:         PathBuf::default(),
        }
    }
}

#[cfg(test)]
pub(crate) fn test_run(args: TestArgs) {
    let TestArgs {
        from_phase_0,
        from_phase_1,
        from_phase_2,
        from_phase_3,
        from_phase_4,
        from_phase_5,
        input,
        name,
    } = args;
    let mut phase0 = NewlineTracking::new();
    let mut phase1 = MapCharacterSets::new();
    let mut phase2 = RemoveEscapedNewlines::new();
    let mut phase3 = PreprocessorTokenizer::new();
    let mut phase4 = Preprocessing::new();
    let mut phase5 = Parsing::new();
    let mut context = Context::new(input, name);
    let mut pending_chars = StackQueue::<u8, 4>::new();
    loop {
        context.set_current_char_position(context.current_position());
        let next = context.next_char();
        let mut previous_returned_some = false;
        macro_rules! handle_next {
            ($phase:ident, $res_vec:ident) => {
                let pending = get_next(&mut $phase, &mut context, next);
                if let Some(v) = $res_vec {
                    v.push(pending.clone());
                }
                let next = match pending {
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
                        continue;
                    },
                };
            };
        }
        handle_next!(phase0, from_phase_0);
        handle_next!(phase1, from_phase_1);
        handle_next!(phase2, from_phase_2);
        handle_next!(phase3, from_phase_3);
        handle_next!(phase4, from_phase_4);
        handle_next!(phase5, from_phase_5);
        if !previous_returned_some {
            break;
        }
    }
}
