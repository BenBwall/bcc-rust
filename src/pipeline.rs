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
    parsing::{
        ExternalDeclaration,
        Parser as LanguageParser,
    },
};
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::shared::{
        SharedString,
        SharedVec,
    },
};

pub(crate) struct PreprocessorIterator {
    preprocessor:       Preprocessor,
    pub(crate) context: Context,
    pending_token:      Option<Token>,
}

impl PreprocessorIterator {
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
