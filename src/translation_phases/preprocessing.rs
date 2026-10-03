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

use conditional::ConditionalGroup;
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
    LiteralId,
    LiteralUnit,
    OperatorTokenType,
    StringTokenType,
    Token,
    TokenType,
};

use self::macro_expansion::FunctionLikeMacroArgument;
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::TokenSource,
    },
    util::{
        HashMap,
        HashSet,
        chunked_queue::ChunkedQueue,
        shared::{
            SharedString,
            SharedVec,
        },
        string_cache::StringCacheId,
    },
};

const PREDEFINED_MACRO_NAMES: [&str; 9] = [
    "__LINE__",
    "__FILE__",
    "__DATE__",
    "__TIME__",
    "_Pragma",
    "__STDC__",
    "__STDC_VERSION__",
    "__STDC_HOSTED__",
    "__STDC_MB_MIGHT_NEQ_WC__",
];

/// Parser-only diagnostics require invocation metadata; standalone token
/// production does not retain that side information.
#[derive(Debug)]
enum OutputPurpose {
    Preprocessing,
    Parsing,
}

#[derive(Debug)]
pub(crate) struct Preprocessor {
    pub(crate) tokenizer:       TokenSource,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    once_set:                   HashSet<u32>,
    macro_definitions:          HashMap<StringCacheId, MacroDefinition>,
    current_is_newline:         bool,
    /// Collect use-site hint metadata only when a language parser will consume
    /// the output.
    output_purpose:             OutputPurpose,
    last_was_newline:           bool,
    /// Provenance of the `if`, `ifdef`, or `ifndef` name of each conditional
    /// directive still waiting for its `#endif`, outermost first. It is owned
    /// rather than an arena range because token iteration compacts the
    /// preprocessor arena while a conditional remains open.
    open_conditionals:          Vec<ConditionalGroup>,

    generate_placeholders:      bool,
    /// The tokenizer-stack depth of the `#` or `##` operand being replaced,
    /// at which reading stops when that operand ends, or 0 outside such
    /// replacement.
    operand_fence:              usize,
    /// Argument prescan stops here without suppressing expansion within it.
    expansion_fence:            usize,
    empty_arguments:            std::rc::Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
    empty_disabled_macros:      std::rc::Rc<[StringCacheId]>,
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
    pub(crate) fn prepare_for_parsing(&mut self) {
        self.output_purpose = OutputPurpose::Parsing;
    }

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
        let tokenizer = TokenSource::new(context, source_file_index, source);
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile {
                    conditional_base:           0,
                    physical_source_file_index: source_file_index,
                },
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
            operand_fence: 0,
            expansion_fence: 0,
            output_purpose: OutputPurpose::Preprocessing,
            empty_arguments: std::rc::Rc::default(),
            empty_disabled_macros: std::rc::Rc::from([]),
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
                    let source_vectors = context.push_source_vectors(&vectors.source);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(context, token) {
                if matches!(self.output_purpose, OutputPurpose::Parsing)
                    && let Some(site) = self.expansion_end()
                {
                    context.record_expansion_end(result.source_vectors, site);
                }
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
            context.compact_preprocessor_vectors();
        }
        self.next_item(context)
    }

    /// Runs translation phases 4 through 6 over the whole translation unit
    /// before any token is parsed. Diagnostics stay pending in `context`.
    ///
    /// Each token's provenance is copied to the token arena as it is
    /// produced, so the preprocessor arena is compacted between tokens
    /// instead of holding every whitespace and intermediate vector.
    pub(crate) fn preprocess_all(&mut self, context: &mut Context) -> ChunkedQueue<Token> {
        let mut tokens = ChunkedQueue::default();
        while let Some(mut token) = self.next_iterator_item(context) {
            token.source_vectors = context.retain_token_source(token.source_vectors);
            tokens.push_back(token);
        }
        tokens
    }

    /// Whether the next [`Self::next_iterator_item`] call discards the
    /// preprocessor provenance arena. Consumers retaining provenance across
    /// calls, such as a deferred diagnostic, must resolve it first.
    ///
    /// A `##` operand held while the other operand's argument expands still
    /// refers to the arena, so compaction waits until every paste completes.
    pub(crate) fn next_iterator_item_compacts(&self) -> bool {
        self.pending_parser_token.is_none()
            && self.pending_parser_errors.is_empty()
            && self
                .hash_hash_stack
                .iter()
                .all(|operand| matches!(operand, HashHash::Empty))
    }
}

impl TranslationPhase for Preprocessor {
    type Item = Token;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        let token = self.next_parser_token(context)?;
        Some(self.concatenate_adjacent_strings(context, token))
    }
}
