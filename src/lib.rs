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

#[cfg(test)]
mod preprocessor_iterator_tests {
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
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct CliInput {
    /// Input string to be preprocessed.
    #[clap(short, long, conflicts_with = "input_file")]
    input:      Option<String>,
    /// Input file to be preprocessed.
    #[clap(conflicts_with = "input")]
    input_file: Option<PathBuf>,
}

enum ParsedInput {
    String(SharedString),
    File(SharedString),
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
    eprintln!("{}", "Printing all generated tokens:".bright_green());
    let parsed_input = if args.input.input.is_some() {
        ParsedInput::String(args.input.input.unwrap().into())
    } else {
        ParsedInput::File(read_to_string_lossy(args.input.input_file.as_ref().unwrap())?.into())
    };
    let (input_string, source_filename) = match parsed_input {
        | ParsedInput::String(s) => (s, PathBuf::from("<input>").into_boxed_path()),
        | ParsedInput::File(s) => (s, args.input.input_file.unwrap().into_boxed_path()),
    };
    parse_include_env_var("CPATH", &mut args.system_include);
    parse_include_env_var("C_INCLUDE_PATH", &mut args.system_include);

    let mut iterator = PreprocessorIterator::new(
        source_filename,
        input_string,
        args.quote_include.into(),
        args.system_include.into(),
    );
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
            | Err(error) => {
                let source_vectors = error.source_vectors(&mut iterator.context);
                let file = iterator.preprocessor.source_file_index();
                let vec = iterator.context.get_source_vectors(source_vectors);
                eprintln!(
                    "{}: {error} at {:?}:{:?}",
                    error.severity(),
                    file,
                    vec.bright_blue(),
                );
            },
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
    Ok(())
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
