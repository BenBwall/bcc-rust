//! BCC C compiler

#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
use std::{
    env::var,
    fmt::Write as _,
    path::{
        Path,
        PathBuf,
    },
};

use clap::{
    Args,
    ColorChoice,
    Parser,
    error::{
        ErrorFormatter,
        RichFormatter,
    },
};
use owo_colors::OwoColorize;
use thiserror::Error;
#[cfg(test)]
use translation_phases::{
    TranslationPhase,
    parsing::ExternalDeclaration,
};
use translation_phases::{
    parsing::{
        InspectionOptions,
        Parser as LanguageParser,
        ParserError,
    },
    preprocessing::Token,
};

#[cfg(feature = "benchmarking-internals")]
use crate::translation_phases::box_path_from_str;
use crate::{
    translation_phases::{
        Context,
        GetSeverity,
        GetSourceFileIndex,
        GetSourceVectors,
        TranslationError,
        preprocessing::{
            CharacterTokenType,
            Preprocessor,
            StringTokenType,
            TokenType,
        },
    },
    util::{
        read_to_string_lossy,
        shared::{
            SharedString,
            SharedVec,
        },
    },
};

pub(crate) mod configuration;
pub(crate) mod float_parsing;
pub(crate) mod translation_phases;
pub(crate) mod util;

struct PreprocessorIterator {
    preprocessor:  Preprocessor,
    context:       Context,
    pending_token: Option<Token>,
}

impl PreprocessorIterator {
    fn new(
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
    fn new(
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

#[cfg(test)]
mod pipeline_iterator_tests {
    use super::*;
    use crate::{
        configuration::{
            CStandard,
            CompilerConfiguration,
            ExtensionPolicy,
        },
        translation_phases::preprocessing::{
            PreprocessorError,
            PreprocessorErrorType,
        },
    };

    #[test]
    fn diagnostic_is_yielded_before_the_token_produced_alongside_it() {
        let mut iterator = PreprocessorIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "#if (0, 2)\nCOMMA_RESULT_2\n#endif\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );
        iterator.context.configuration =
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny);

        let error = iterator.next().unwrap().unwrap_err();
        assert!(matches!(
            &error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                    ExtensionPolicy::Deny
                ),
                ..
            })
        ));
        let error_source_vectors = error.source_vectors(&mut iterator.context);
        assert!(
            !iterator
                .context
                .get_source_vectors(error_source_vectors)
                .is_empty()
        );

        let token = iterator.next().unwrap().unwrap();
        assert_eq!(token.kind, TokenType::Identifier);
        assert_eq!(
            iterator.context.string_cache.at(token.contents),
            "COMMA_RESULT_2"
        );
        assert!(iterator.next().is_none());
    }

    #[test]
    fn adjacent_string_lookahead_keeps_the_buffered_token_provenance() {
        let mut iterator = PreprocessorIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "\"a\" identifier\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        let string = iterator.next().unwrap().unwrap();
        assert!(matches!(string.kind, TokenType::String(_)));
        let identifier = iterator.next().unwrap().unwrap();
        assert_eq!(identifier.kind, TokenType::Identifier);
        assert_eq!(
            iterator.context.string_cache.at(identifier.contents),
            "identifier"
        );
        assert!(
            !iterator
                .context
                .get_source_vectors(identifier.source_vectors)
                .is_empty()
        );
        assert!(iterator.next().is_none());
    }

    #[test]
    fn adjacent_string_lookahead_defers_buffered_token_diagnostics() {
        let mut iterator = PreprocessorIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "\"a\" 0xg\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
                ..
            })
        ));
        assert!(iterator.next().is_some());
        assert!(iterator.next().is_none());
    }

    #[test]
    fn adjacent_string_lookahead_keeps_current_token_before_later_diagnostics() {
        let mut iterator = PreprocessorIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "\"\\q\" 0xg".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidEscapeSequence,
                ..
            })
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::InitialProcessing(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
                ..
            })
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::Integer(_)
        ));
        assert!(iterator.next().is_none());
    }

    #[test]
    fn adjacent_string_lookahead_keeps_deferred_eof_diagnostic_provenance() {
        let mut iterator = PreprocessorIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "\"a\"\n#error boom\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        let error = iterator.next().unwrap().unwrap_err();
        assert!(matches!(
            &error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ErrorDirective(message),
                ..
            }) if message.trim() == "boom"
        ));
        let source_vectors = error.source_vectors(&mut iterator.context);
        assert!(
            !iterator
                .context
                .get_source_vectors(source_vectors)
                .is_empty()
        );
        assert!(iterator.next().is_none());
    }

    #[test]
    fn parser_iterator_yields_declarations_and_exposes_the_syntax_store() {
        let mut iterator = ParserIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "int value;\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        assert!(matches!(
            iterator.next().unwrap().unwrap(),
            ExternalDeclaration::Declaration(_)
        ));
        assert!(iterator.next().is_none());
        assert!(
            format!("{:#?}", iterator.parser.syntax_debug()).contains("declarations:"),
            "the debug view should expose the arena referenced by parser output"
        );
    }

    #[test]
    fn parser_iterator_yields_a_parsed_initialized_declaration() {
        let mut iterator = ParserIterator::new(
            PathBuf::from("<test>").into_boxed_path(),
            "int value = 1;\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        assert!(matches!(
            iterator.next().unwrap().unwrap(),
            ExternalDeclaration::Declaration(_)
        ));
        assert!(iterator.next().is_none());
    }

    #[test]
    fn complete_translation_unit_owns_ordered_roots_and_typed_syntax() {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            PathBuf::from("<test>").into_boxed_path(),
            "int first; int second = 2;\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );

        let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);

        assert_eq!(unit.external_declarations().len(), 2);
        let ExternalDeclaration::Declaration(first) = unit.external_declarations()[0] else {
            panic!("expected the first root to be a declaration")
        };
        let ExternalDeclaration::Declaration(second) = unit.external_declarations()[1] else {
            panic!("expected the second root to be a declaration")
        };
        assert_eq!(unit.syntax().declaration(first).init_declarators().len(), 1);
        assert_eq!(
            unit.syntax().declaration(second).init_declarators().len(),
            1
        );
    }

    #[test]
    fn cli_parser_details_render_recovery_ranges_and_notes() {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            PathBuf::from("<test>").into_boxed_path(),
            "int first extra junk; int after;\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );
        let _unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
        let errors = context.take_pending_errors();
        let diagnostic = errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if error
                        .recovery
                        .is_some_and(|recovery| recovery.discarded_tokens > 0) =>
                    Some(error),
                | _ => None,
            })
            .expect("expected discarded-input recovery");

        let rendered = format_parser_diagnostic_details(diagnostic, &context);
        assert!(rendered.contains("code=Syntax"), "{rendered}");
        assert!(rendered.contains("discarded input:"), "{rendered}");
        assert!(rendered.contains("discarded-tokens=2"), "{rendered}");
        assert!(
            rendered.contains("note: parsing resumes here"),
            "{rendered}"
        );
    }
}

#[cfg(test)]
mod syntax_tree_consumer_tests {
    use super::*;
    use crate::translation_phases::parsing::{
        DirectDeclarator,
        TypeSpecifiers,
    };

    #[test]
    fn sibling_consumer_can_traverse_parameter_and_member_syntax() {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            PathBuf::from("<syntax-tree-consumer-test>").into_boxed_path(),
            "struct S { int member : 3; }; int f(int parameter);"
                .to_owned()
                .into(),
            SharedVec::default(),
            SharedVec::default(),
        );
        let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
        let tree = unit.syntax();

        let ExternalDeclaration::Declaration(struct_root) = unit.external_declarations()[0] else {
            panic!("expected struct declaration")
        };
        let TypeSpecifiers::StructOrUnion(struct_index) = tree
            .declaration(struct_root)
            .syntax()
            .declaration_specifiers
            .type_specifiers
        else {
            panic!("expected struct type specifier")
        };
        let members = tree.struct_declarations(
            tree.struct_or_union_specifier(struct_index)
                .struct_declaration_list
                .expect("struct definition has members"),
        );
        assert_eq!(members[0].type_specifiers, TypeSpecifiers::Int);
        let member_declarators = tree.struct_declarators(members[0].struct_declarator_list);
        assert!(member_declarators[0].declarator.is_some());
        assert!(member_declarators[0].bitfield_width.is_some());

        let ExternalDeclaration::Declaration(function_root) = unit.external_declarations()[1]
        else {
            panic!("expected function declaration")
        };
        let declarator = tree.declaration(function_root).init_declarators()[0].declarator;
        let parameter_list = tree
            .direct_declarators(declarator.kind)
            .iter()
            .find_map(|direct| match direct {
                | DirectDeclarator::Function { parameter_list, .. } => Some(*parameter_list),
                | _ => None,
            })
            .expect("function declarator has a parameter list");
        let parameters = tree.parameter_declarations(parameter_list);
        assert_eq!(
            parameters[0].declaration_specifiers.type_specifiers,
            TypeSpecifiers::Int
        );
        assert!(parameters[0].declarator.is_some());
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always)]
struct Cli {
    #[command(flatten)]
    input: CliInput,
    /// Add directory to include search path.
    #[clap(short = 'q', long = "iquote")]
    quote_include: Vec<PathBuf>,
    /// Add directory to system include search path.
    #[clap(short = 's', long = "isystem")]
    system_include: Vec<PathBuf>,
    #[command(flatten)]
    output: CliOutput,
    /// Suppress the `repeated-specifiers` quality warning group.
    #[clap(long)]
    no_repeated_specifier_warnings: bool,
}

#[derive(Args)]
struct CliOutput {
    /// Print preprocessor tokens instead of parser output.
    #[clap(long)]
    tokens: bool,
    #[command(flatten)]
    parser: ParserOutput,
}

#[derive(Args)]
struct ParserOutput {
    /// Print a deterministic, source-oriented C syntax tree.
    #[clap(long, conflicts_with = "tokens")]
    syntax_tree:      bool,
    /// Include line and column locations in `--syntax-tree` output.
    #[clap(long, requires = "syntax_tree")]
    syntax_locations: bool,
    /// Print raw parser arenas for storage debugging.
    #[clap(long, conflicts_with = "tokens")]
    raw_syntax:       bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct CliInput {
    /// Input string to be parsed.
    #[clap(short, long, conflicts_with = "input_file")]
    input:      Option<String>,
    /// Input file to be parsed.
    #[clap(conflicts_with = "input")]
    input_file: Option<PathBuf>,
}

#[doc(hidden)]
#[derive(Debug, Error)]
pub enum MainError {
    #[error("{}{}", "Failed to open input file: ".bright_red(), 0.bright_red())]
    OpenInputFileError(#[from] std::io::Error),
    #[error("{}{}", "Failed to parse command line arguments ".bright_red(), RichFormatter::format_error(.0).ansi())]
    ParseArgumentsError(#[from] clap::Error),
}

fn parse_include_env_var(env_var: &str, vec: &mut Vec<PathBuf>) {
    vec.extend(
        var(env_var)
            .unwrap_or_default()
            .split(':')
            .map(PathBuf::from),
    );
}

#[doc(hidden)]
pub fn run() -> Result<(), MainError> {
    let mut args = Cli::try_parse()?;
    let (input_string, source_filename) =
        match (args.input.input.take(), args.input.input_file.take()) {
            | (Some(input), None) => (
                SharedString::from(input),
                PathBuf::from("<input>").into_boxed_path(),
            ),
            | (None, Some(input_file)) => (
                SharedString::from(read_to_string_lossy(&input_file)?),
                input_file.into_boxed_path(),
            ),
            | _ => unreachable!("clap requires exactly one input source"),
        };
    parse_include_env_var("CPATH", &mut args.system_include);
    parse_include_env_var("C_INCLUDE_PATH", &mut args.system_include);

    if args.output.tokens {
        print_preprocessor_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
        );
    } else {
        print_parser_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
}

fn print_preprocessor_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
) {
    eprintln!("{}", "Printing all generated tokens:".bright_green());
    let mut iterator =
        PreprocessorIterator::new(source_filename, input_string, quote_include, system_include);
    while let Some(item) = iterator.next() {
        match item {
            | Ok(token) => eprintln!(
                "{}",
                match token.kind {
                    | TokenType::Identifier => format!(
                        "Identifier: {}",
                        iterator.context.string_cache.at(token.contents)
                    ),
                    | TokenType::Operator(ott) => format!("Operator: {ott:#?}"),
                    | TokenType::String(sltt) => format!(
                        "String-like token: {}",
                        match sltt {
                            | StringTokenType::WideString(s) | StringTokenType::String(s) => {
                                iterator.context.string_cache.at(s).to_string()
                            },
                        }
                    ),
                    | TokenType::Character(c) => format!(
                        "Character: {}",
                        match c {
                            | CharacterTokenType::WideChar(c) | CharacterTokenType::Char(c) => {
                                format!("{c:#?}")
                            },
                        }
                    ),
                    | TokenType::Keyword(k) => format!("Keyword: {k:#?}"),
                    | TokenType::Integer(i) => format!("Integer: {i:#?}"),
                    | TokenType::Float(f) => format!("Float: {f:#?}"),
                }
                .bright_magenta()
            ),
            | Err(error) => print_translation_error(
                &error,
                &mut iterator.context,
                iterator.preprocessor.source_file_index(),
            ),
        }
    }

    eprintln!(
        "{}",
        "Finished printing all generated tokens.".bright_green()
    );
    eprintln!(
        "{}{}",
        "String cache contents: ".bright_yellow(),
        iterator.context.string_cache.bright_yellow()
    );
    eprintln!(
        "{}{:?}",
        "Hash Hash stack: ".bright_red(),
        iterator.preprocessor.hash_hash_stack.bright_red()
    );
    eprintln!(
        "{}{:?}",
        "Tokenizer stack: ".bright_blue(),
        iterator.preprocessor.tokenizer_stack.bright_blue()
    );
    eprintln!(
        "{}{}",
        "Source vectors: ".bright_green(),
        iterator.context.source_vectors.bright_green()
    );
}

fn print_parser_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    let mut context = Context::new();
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let preprocessor = Preprocessor::new(
        &mut context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let source_file_index = preprocessor.source_file_index();
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    while let Some(error) = context.pop_pending_error() {
        print_translation_error(&error, &mut context, source_file_index);
    }

    if output.syntax_tree {
        eprint!(
            "{}",
            unit.syntax().inspect(
                unit.external_declarations(),
                &context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        eprintln!("{:#?}", unit.syntax().raw_debug());
    }
}

fn print_translation_error(
    error: &TranslationError,
    context: &mut Context,
    fallback_source_file_index: u32,
) {
    let source_vectors = error.source_vectors(context);
    let vectors = context.get_source_vectors(source_vectors);
    let source_file_index = vectors
        .first()
        .map_or(fallback_source_file_index, |vector| {
            vector.source_file_index
        });
    eprintln!(
        "{}: {error} at {:?}:{:?}",
        error.severity(),
        source_file_index,
        vectors.bright_blue(),
    );
    if let TranslationError::Parsing(error) = error {
        eprint!("{}", format_parser_diagnostic_details(error, context));
    }
}

fn format_parser_diagnostic_details(error: &ParserError, context: &Context) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "  context: code={:?} frame={:?} expected={:?} found={:?}",
        error.code, error.frame, error.expected, error.found,
    );
    for range in &error.ranges {
        let _ = writeln!(
            output,
            "  discarded input: {:?}",
            context.get_source_vectors(*range),
        );
    }
    if let Some(recovery) = error.recovery {
        let _ = writeln!(
            output,
            "  recovery: owner={:?} discarded-tokens={} stopped-at={:?}",
            recovery.owner, recovery.discarded_tokens, recovery.stopped_at,
        );
    }
    for related in &error.related {
        let _ = writeln!(
            output,
            "  note: {} at {:?}",
            related.message,
            context.get_source_vectors(related.source_vectors),
        );
    }
    output
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
pub fn preprocess_one_million() -> usize {
    let million_lines = one_million_lines();
    let iterator = PreprocessorIterator::new(
        box_path_from_str("<input>"),
        million_lines.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    iterator.count()
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
pub fn one_million_input_bytes() -> u64 {
    u64::try_from(one_million_lines().len()).expect("benchmark input length must fit in u64")
}

#[cfg(feature = "benchmarking-internals")]
fn one_million_lines() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/one-million-lines.c"))
}
