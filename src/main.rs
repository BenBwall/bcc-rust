//! BCC Compiler

use std::{
    env::var,
    hash::BuildHasherDefault,
    path::PathBuf,
    sync::Arc,
};

use clap::{
    Args,
    Parser,
};
use owo_colors::OwoColorize;
use rustc_hash::FxHasher;
use thiserror::Error;
use util::input::Input;

use crate::{
    translation_phases::{
        phase_4_preprocessing::{
            Preprocessor,
            StringLikeTokenType,
            TokenType,
        },
        GetPosition,
        GetSeverity,
    },
    util::{
        read_to_string_lossy,
        string_cache::StringCache,
    },
};

pub(crate) mod float_parsing;
pub(crate) mod translation_phases;
pub(crate) mod util;

pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub(crate) type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<FxHasher>>;
#[derive(Parser)]
#[command(author, version, about, long_about)]
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
    input_file: Option<String>,
}

enum ParsedInput {
    String(Input),
    File(Input),
}
#[derive(Debug, Error)]
enum MainError {
    #[error("Failed to open input file: {0}")]
    OpenInputFileError(#[from] std::io::Error),
    #[error("Failed to parse command line arguments: {0}")]
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

fn main() -> Result<(), MainError> {
    let mut args = Cli::try_parse()?;
    let mut string_cache = StringCache::new();
    eprintln!("{}", "Printing all generated tokens:".bright_green());
    let parsed_input = if args.input.input.is_some() {
        ParsedInput::String(args.input.input.unwrap().into())
    } else {
        ParsedInput::File(read_to_string_lossy(args.input.input_file.as_ref().unwrap())?.into())
    };
    let (input_string, source_filename) = match parsed_input {
        | ParsedInput::String(s) => (s, string_cache.intern("<input>")),
        | ParsedInput::File(s) => (
            s,
            string_cache.intern(args.input.input_file.as_ref().unwrap()),
        ),
    };
    parse_include_env_var("CPATH", &mut args.system_include);
    parse_include_env_var("C_INCLUDE_PATH", &mut args.system_include);

    let mut preprocessor = Preprocessor::new(
        input_string,
        source_filename,
        string_cache,
        Arc::new(args.quote_include),
        Arc::new(args.system_include),
    );
    while let Some(res) = preprocessor.next() {
        match res {
            | Ok(t) => eprintln!(
                "{}",
                match t.kind {
                    | TokenType::Identifier => format!(
                        "Identifier: {}",
                        preprocessor
                            .previous_phase
                            .string_cache
                            .get(t.contents)
                            .unwrap()
                    ),
                    | TokenType::Operator(ott) => format!("Operator: {ott:#?}"),
                    | TokenType::StringLike(sltt) => format!(
                        "String-like token: {}",
                        match sltt {
                            | StringLikeTokenType::WideString(s)
                            | StringLikeTokenType::String(s) => {
                                preprocessor
                                    .previous_phase
                                    .string_cache
                                    .get(s)
                                    .unwrap()
                                    .to_string()
                            },
                            | StringLikeTokenType::WideChar(c) | StringLikeTokenType::Char(c) =>
                                format!("{c:#?}"),
                        }
                    ),
                    | TokenType::Keyword(k) => format!("Keyword: {k:#?}"),
                    | TokenType::Integer(i) => format!("Integer: {i:#?}"),
                    | TokenType::Float(f) => format!("Float: {f:#?}"),
                }
                .bright_magenta()
            ),
            | Err(e) => {
                let position = e.position();
                eprintln!(
                    "{}: {e} at {}:{}:{}",
                    e.severity(),
                    preprocessor
                        .previous_phase
                        .string_cache
                        .get(position.source_file)
                        .expect("Invalid source file id."),
                    position.line.bright_blue(),
                    position.column.bright_blue()
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
        preprocessor.previous_phase.as_ref().bright_yellow()
    );
    eprintln!(
        "{}{:?}",
        "Hash Hash stack: ".bright_red(),
        preprocessor.hash_hash_stack.bright_red()
    );
    eprintln!(
        "{}{:?}",
        "Tokenizer stack: ".bright_blue(),
        preprocessor.tokenizer_stack.bright_blue()
    );
    Ok(())
}
