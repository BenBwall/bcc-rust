#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only constructor arguments, compiled only under `cfg(test)`."
)]
use std::path::PathBuf;

#[cfg(test)]
use super::super::SharedVec;
use super::super::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    Bump,
    Context,
    FileFrame,
    FxBuildHasher,
    GetPosition,
    HeaderSearch,
    LexedFiles,
    LiteralScratch,
    MacroDefinition,
    OutputPurpose,
    Path,
    Preprocessor,
    PreprocessorExpressionParser,
    PreprocessorState,
    Resting,
    SourcePosition,
    command_line,
    language_features,
};

impl<'tu, 'pp> Preprocessor<'tu, 'pp> {
    pub(crate) fn new_with_arena_source(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: &Path,
        source: &'tu str,
        search: HeaderSearch<'_>,
    ) -> Self {
        Self::new_inner(pp, context, source_name, source, Some(source), search)
    }

    fn new_inner(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: &Path,
        source: &str,
        arena_source: Option<&'tu str>,
        search: HeaderSearch<'_>,
    ) -> Self {
        context.set_header_search(search);
        let mut macro_definitions = ArenaMap::with_hasher_in(FxBuildHasher, pp);
        for name in PREDEFINED_MACRO_NAMES {
            if (name == "__STDC_VERSION__"
                && context.configuration.standard().version_macro().is_none())
                || (name == "__STRICT_ANSI__" && !context.configuration.strict_ansi())
            {
                continue;
            }
            _ = macro_definitions
                .insert(context.string_cache.intern(name), MacroDefinition::BuiltIn);
        }
        for &(name, feature) in language_features::LANGUAGE_BUILTINS {
            if context.configuration.accepts(feature) {
                _ = macro_definitions
                    .insert(context.string_cache.intern(name), MacroDefinition::BuiltIn);
            }
        }
        let source_file_index = context.intern_source_file(source_name);
        let mut lexed_files = LexedFiles::new_in(pp);
        let tokenizer = lexed_files.open(context, source_file_index, source);
        let mut file_frames = ArenaVec::new_in(pp);
        file_frames.push(FileFrame {
            conditional_base:           0,
            physical_source_file_index: source_file_index,
            include_search_index:       None,
            tokenizer:                  tokenizer.clone(),
        });
        if let Some(source) = arena_source {
            context.record_arena_source_text(source_file_index, source);
        } else {
            context.record_source_text(source_file_index, source);
        }
        let end = (tokenizer.position(context), source_file_index);
        // The stack is read from the top: builtins, command line, main file.
        let command_line = command_line::source(context);
        let command_line_file = (!command_line.is_empty()).then(|| {
            let index =
                context.add_synthetic_source_file(Path::new("<command line>"), command_line);
            file_frames.push(FileFrame {
                conditional_base:           0,
                physical_source_file_index: index,
                include_search_index:       None,
                tokenizer:                  lexed_files.open_command_line(
                    context,
                    index,
                    command_line,
                ),
            });
            index
        });
        let definitions = language_features::with_identity_macros(
            context.tu_arena(),
            context
                .configuration
                .target()
                .predefined_macros(context.tu_arena(), context.configuration.gnu_extensions()),
            context.configuration,
        );
        let builtin_index =
            context.add_synthetic_source_file(Path::new("<built-in>/predefined.h"), definitions);
        let tokenizer = lexed_files.open(context, builtin_index, definitions);
        file_frames.push(FileFrame {
            conditional_base:           0,
            physical_source_file_index: builtin_index,
            include_search_index:       None,
            tokenizer:                  tokenizer.clone(),
        });
        Self {
            resting: Some(Resting {
                state: PreprocessorState {
                    counter: 0,
                    va_opt_name: context.string_cache.intern("__VA_OPT__"),
                    query_depth: 0,
                    conditional_queries: false,
                    in_directive: false,
                    retain_placeholders: false,
                    arena: pp,
                    once_set: ArenaSet::with_hasher_in(FxBuildHasher, pp),
                    macro_definitions,
                    deprecated_macros: ArenaMap::with_hasher_in(FxBuildHasher, pp),
                    lexed_files,
                    file_frames,
                    command_line_file,
                    open_conditionals: ArenaVec::new_in(pp),
                    translation_timestamp: None,
                    literal_scratch: LiteralScratch::new(pp),
                },
                tokenizer,
                last_was_newline: true,
                current_is_newline: true,
                output_purpose: OutputPurpose::Preprocessing,
                expression_parser: PreprocessorExpressionParser::new(pp),
                pending_parser_token: None,
                pending_parser_errors: ArenaVec::new_in(pp),
                source_segment_limit: usize::MAX,
            }),
            expansion: Bump::new(),
            end,
        }
    }

    #[cfg(test)]
    #[expect(
        clippy::disallowed_types,
        reason = "Test-only constructor arguments, compiled only under `cfg(test)`."
    )]
    pub(crate) fn new(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: Box<Path>,
        source: &str,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
    ) -> Self {
        let quote: Vec<&Path> = quote_include_directories
            .iter()
            .map(PathBuf::as_path)
            .collect();
        let system: Vec<&Path> = system_include_directories
            .iter()
            .map(PathBuf::as_path)
            .collect();
        let preprocessor = Self::new_inner(
            pp,
            context,
            &source_name,
            source,
            None,
            HeaderSearch {
                quote: &quote,
                angled: &system,
                ..HeaderSearch::default()
            },
        );
        drop((quote, system));
        drop((
            source_name,
            quote_include_directories,
            system_include_directories,
        ));
        preprocessor
    }

    pub(crate) fn prepare_for_parsing(&mut self) {
        self.resting_mut().output_purpose = OutputPurpose::Parsing;
    }

    pub(in crate::translation_phases::preprocessing) fn resting_mut(
        &mut self,
    ) -> &mut Resting<'tu, 'pp> {
        self.resting
            .as_mut()
            .expect("preprocessing does not resume after stopping inside an expansion")
    }

    /// The most bytes the expansion arena has held at once.
    #[cfg(feature = "benchmarking-internals")]
    pub(crate) fn expansion_high_water(&self) -> usize {
        self.expansion.high_water()
    }

    /// Where reading stopped, for end-of-input diagnostics.
    pub(crate) fn end_position(&self) -> SourcePosition {
        self.end.0
    }

    /// The presumed source file where reading stopped.
    pub(crate) fn end_source_file_index(&self) -> u32 {
        self.end.1
    }
}

/// The names defined before the first line is read.
///
/// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172. None of the conditionally
/// defined names of §6.10.8 paragraph 2, p. 161; PDF p. 173, is defined.
/// `_Pragma` is an operator (§6.10.9, p. 161; PDF p. 173), not a macro; it
/// is registered here so that rescanning recognizes it.
pub(in crate::translation_phases::preprocessing) const PREDEFINED_MACRO_NAMES: [&str; 10] = [
    "__LINE__",
    "__FILE__",
    "__DATE__",
    "__TIME__",
    "_Pragma",
    "__STDC__",
    "__STDC_VERSION__",
    "__STRICT_ANSI__",
    "__STDC_HOSTED__",
    "__STDC_MB_MIGHT_NEQ_WC__",
];
