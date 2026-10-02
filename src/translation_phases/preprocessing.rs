//! C99 preprocessing (translation phase 4) and the token-level work that
//! follows it: escape-sequence evaluation, adjacent string-literal
//! concatenation, and conversion of preprocessing tokens into parser tokens.

mod conditional;
mod directives;
mod driver;
mod errors;
mod expression;
mod macro_expansion;
#[cfg(test)]
mod tests;
mod token;
mod token_conversion;

use std::{
    fmt::Debug,
    mem::take,
    path::{
        Path,
        PathBuf,
    },
};

use driver::{
    TokenizerFrame,
    TokenizerFrameType,
    TranslationTimestamp,
};
pub(crate) use errors::{
    PreprocessorError,
    PreprocessorErrorType,
};
use expression::PreprocessorExpressionParser;
pub(crate) use macro_expansion::HashHash;
use macro_expansion::MacroDefinition;
pub(crate) use token::{
    CharacterTokenType,
    FloatTokenType,
    IntegerTokenType,
    KeywordTokenType,
    OperatorTokenType,
    StringTokenType,
    Token,
    TokenType,
};

use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        SourceVector,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::PreprocessorTokenizer,
    },
    util::{
        HashMap,
        HashSet,
        shared::{
            SharedString,
            SharedVec,
        },
        string_cache::StringCacheId,
    },
};

const PREDEFINED_MACRO_NAMES: [&str; 5] =
    ["__LINE__", "__FILE__", "__DATE__", "__TIME__", "_Pragma"];

#[derive(Debug)]
pub(crate) struct Preprocessor {
    pub(crate) tokenizer:       PreprocessorTokenizer,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    once_set:                   HashSet<u32>,
    macro_definitions:          HashMap<StringCacheId, MacroDefinition>,
    current_is_newline:         bool,
    last_was_newline:           bool,
    /// Provenance of the `if`, `ifdef`, or `ifndef` name of each conditional
    /// directive still waiting for its `#endif`, outermost first. It is owned
    /// rather than an arena range because token iteration compacts the
    /// preprocessor arena while a conditional remains open.
    open_conditionals:          Vec<Box<[SourceVector]>>,

    generate_placeholders:      bool,
    quote_include_directories:  SharedVec<PathBuf>,
    system_include_directories: SharedVec<PathBuf>,
    expression_parser:          PreprocessorExpressionParser,
    pending_parser_token:       Option<Token>,
    pending_parser_errors:      Vec<TranslationError>,
    /// Fixed on first use so every `__DATE__` and `__TIME__` in one
    /// translation unit agrees (C99 §6.10.8p1).
    translation_timestamp:      Option<TranslationTimestamp>,
}

impl GetPosition for Preprocessor {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.tokenizer.position(context)
    }
}

impl SetPosition for Preprocessor {
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.tokenizer.set_position(context, position);
    }
}

impl GetSourceFileIndex for Preprocessor {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.tokenizer.source_file_index()
    }
}

impl SetSourceFileIndex for Preprocessor {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.tokenizer
            .set_source_file_index(context, source_file_index);
    }
}

impl Preprocessor {
    pub(crate) fn new(
        context: &mut Context,
        source_name: Box<Path>,
        source: SharedString,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
    ) -> Self {
        let macro_definitions = PREDEFINED_MACRO_NAMES
            .into_iter()
            .map(|s| -> (StringCacheId, MacroDefinition) {
                (context.string_cache.intern(s), MacroDefinition::BuiltIn)
            })
            .collect();
        let source_file_index = context.intern_source_file(source_name);
        context.record_source_text(source_file_index, source.clone());
        let tokenizer = PreprocessorTokenizer::new(source_file_index, source);
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                tokenizer:  tokenizer.clone(),
            }],
            hash_hash_stack: Vec::new(),
            once_set: HashSet::default(),
            tokenizer,
            macro_definitions,
            last_was_newline: true,
            current_is_newline: true,
            open_conditionals: Vec::new(),
            generate_placeholders: false,
            quote_include_directories,
            system_include_directories,
            expression_parser: PreprocessorExpressionParser::new(),
            pending_parser_token: None,
            pending_parser_errors: Vec::new(),
            translation_timestamp: None,
        }
    }

    fn next_parser_token(&mut self, context: &mut Context) -> Option<Token> {
        context.append_pending_errors(take(&mut self.pending_parser_errors));
        if let Some(token) = self.pending_parser_token.take() {
            return Some(token);
        }
        loop {
            let Some(token) = self.next_preprocessor_token::<true>(context) else {
                for vectors in take(&mut self.open_conditionals) {
                    let source_vectors = context.push_source_vectors(&vectors);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(context, token) {
                return Some(result);
            }
        }
    }

    /// Produces the next iterator item while keeping buffered provenance alive.
    ///
    /// Adjacent-string concatenation may already have mapped a later token or
    /// EOF diagnostic. Source-vector compaction therefore belongs to the
    /// producer that owns that buffered work, not to each iterator consumer.
    pub(crate) fn next_iterator_item(&mut self, context: &mut Context) -> Option<Token> {
        if self.next_iterator_item_compacts() {
            context.source_vectors.0.clear();
        }
        self.next_item(context)
    }

    /// Whether the next [`Self::next_iterator_item`] call discards the
    /// preprocessor provenance arena. Consumers retaining provenance across
    /// calls, such as a deferred diagnostic, must resolve it first.
    pub(crate) fn next_iterator_item_compacts(&self) -> bool {
        self.pending_parser_token.is_none() && self.pending_parser_errors.is_empty()
    }
}

impl TranslationPhase for Preprocessor {
    type Item = Token;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        let token = self.next_parser_token(context)?;
        Some(self.concatenate_adjacent_strings(context, token))
    }
}
