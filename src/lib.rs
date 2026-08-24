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
use translation_phases::{
    TranslationPhase,
    parsing::{
        ExternalDeclaration,
        Parser as LanguageParser,
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

        self.context.source_vectors.0.clear();
        let token = self.preprocessor.next_item(&mut self.context);
        if let Some(error) = self.context.pop_pending_error() {
            self.pending_token = token;
            Some(Err(error))
        } else {
            token.map(Ok)
        }
    }
}

struct ParserIterator {
    parser:       LanguageParser,
    context:      Context,
    pending_item: Option<ExternalDeclaration>,
}

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
}

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always)]
struct Cli {
    #[command(flatten)]
    input:          CliInput,
    /// Add directory to include search path.
    #[clap(short = 'q', long = "iquote")]
    quote_include:  Vec<PathBuf>,
    /// Add directory to system include search path.
    #[clap(short = 's', long = "isystem")]
    system_include: Vec<PathBuf>,
    /// Print preprocessor tokens instead of parser output.
    #[clap(long)]
    tokens:         bool,
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

    if args.tokens {
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
) {
    eprintln!("{}", "Printing all parser output:".bright_green());
    let mut iterator =
        ParserIterator::new(source_filename, input_string, quote_include, system_include);
    while let Some(item) = iterator.next() {
        match item {
            | Ok(item) => eprintln!(
                "{}",
                format!("External declaration: {item:#?}").bright_magenta()
            ),
            | Err(error) => print_translation_error(
                &error,
                &mut iterator.context,
                iterator.parser.source_file_index(),
            ),
        }
    }

    eprintln!("{}", "Finished printing all parser output.".bright_green());
    eprintln!(
        "{}{}",
        "Parser syntax store: ".bright_cyan(),
        format!("{:#?}", iterator.parser.syntax_debug()).bright_cyan()
    );
    eprintln!(
        "{}{}",
        "String cache contents: ".bright_yellow(),
        iterator.context.string_cache.bright_yellow()
    );
    eprintln!(
        "{}{}",
        "Source vectors: ".bright_green(),
        iterator.context.source_vectors.bright_green()
    );
}

fn print_translation_error(
    error: &TranslationError,
    context: &mut Context,
    source_file_index: u32,
) {
    let source_vectors = error.source_vectors(context);
    let vectors = context.get_source_vectors(source_vectors);
    eprintln!(
        "{}: {error} at {:?}:{:?}",
        error.severity(),
        source_file_index,
        vectors.bright_blue(),
    );
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
