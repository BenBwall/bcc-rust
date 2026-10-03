//! Iterator adapters that drive the preprocessor and parser while
//! interleaving their diagnostics with the items they produce.

#[cfg(test)]
mod tests;

use std::path::{
    Path,
    PathBuf,
};

#[cfg(test)]
use crate::translation_phases::{
    TranslationPhase,
    parsing::ExternalDeclaration,
};
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        parsing::Parser as LanguageParser,
        preprocessing::{
            Preprocessor,
            Token,
        },
        preprocessor_tokenizer::LexingStrategy,
    },
    util::shared::{
        SharedString,
        SharedVec,
    },
};

/// How the front end schedules translation phases 1 through 7.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum PreprocessingStrategy {
    /// Pull each token through every phase when the parser asks for it.
    #[default]
    Streaming,
    /// Lex each source file completely when it is opened (phases 1-3), then
    /// pull tokens through phases 4-7 on demand.
    BatchLexing,
    /// Lex each source file completely and preprocess the whole translation
    /// unit (phases 1-6) before parsing, as a standalone preprocessor would.
    Batch,
}

impl PreprocessingStrategy {
    pub(crate) fn lexing(self) -> LexingStrategy {
        match self {
            | Self::Streaming => LexingStrategy::Streaming,
            | Self::BatchLexing | Self::Batch => LexingStrategy::Batch,
        }
    }

    /// Opens the main source file under this strategy.
    pub(crate) fn preprocessor(
        self,
        context: &mut Context,
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Preprocessor {
        context.set_lexing_strategy(self.lexing());
        Preprocessor::new(
            context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        )
    }

    /// Creates the language parser over `preprocessor`. The batch strategy
    /// preprocesses the whole translation unit first.
    pub(crate) fn parser(
        self,
        preprocessor: Preprocessor,
        context: &mut Context,
    ) -> LanguageParser {
        match self {
            | Self::Streaming | Self::BatchLexing => LanguageParser::new(preprocessor),
            | Self::Batch => LanguageParser::after_preprocessing(preprocessor, context),
        }
    }
}

pub(crate) struct PreprocessorIterator {
    preprocessor:       Preprocessor,
    pub(crate) context: Context,
    pending_token:      Option<Token>,
}

impl PreprocessorIterator {
    #[cfg(any(test, feature = "benchmarking-internals"))]
    pub(crate) fn new(
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Self {
        Self::with_lexing(
            LexingStrategy::Streaming,
            source_filename,
            input_string,
            quote_include,
            system_include,
        )
    }

    pub(crate) fn with_lexing(
        lexing: LexingStrategy,
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Self {
        let mut context = Context::new();
        context.set_lexing_strategy(lexing);
        let preprocessor = Preprocessor::new(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        );
        Self {
            preprocessor,
            context,
            pending_token: None,
        }
    }

    /// Whether the next [`Iterator::next`] call can discard preprocessor
    /// provenance that earlier items still reference.
    pub(crate) fn compacts_on_next(&self) -> bool {
        !self.context.has_pending_errors()
            && self.pending_token.is_none()
            && self.preprocessor.next_iterator_item_compacts()
    }
}

impl Iterator for PreprocessorIterator {
    type Item = Result<Token, TranslationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.context.pop_pending_error() {
            return Some(Err(error));
        }
        if let Some(token) = self.pending_token.take() {
            return Some(Ok(token));
        }

        let token = self.preprocessor.next_iterator_item(&mut self.context);
        if let Some(error) = self.context.pop_pending_error() {
            self.pending_token = token;
            Some(Err(error))
        } else {
            token.map(Ok)
        }
    }
}

#[cfg(test)]
struct ParserIterator {
    parser:       LanguageParser,
    context:      Context,
    pending_item: Option<ExternalDeclaration>,
}

#[cfg(test)]
impl ParserIterator {
    pub(crate) fn new(
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Self {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        );
        Self {
            parser: LanguageParser::new(preprocessor),
            context,
            pending_item: None,
        }
    }
}

#[cfg(test)]
impl Iterator for ParserIterator {
    type Item = Result<ExternalDeclaration, TranslationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.context.pop_pending_error() {
            return Some(Err(error));
        }
        if let Some(item) = self.pending_item.take() {
            return Some(Ok(item));
        }

        let item = self.parser.next_item(&mut self.context);
        if let Some(error) = self.context.pop_pending_error() {
            self.pending_item = item;
            Some(Err(error))
        } else {
            item.map(Ok)
        }
    }
}
