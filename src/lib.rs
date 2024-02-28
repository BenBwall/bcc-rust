//! BCC C compiler

use std::{
    env::var,
    path::PathBuf,
};

use clap::{
    error::{
        ErrorFormatter,
        RichFormatter,
    },
    Args,
    ColorChoice,
    Parser,
};
use owo_colors::OwoColorize;
use thiserror::Error;

use crate::{
    translation_phases::{
        preprocessing::{
            CharacterTokenType,
            Preprocessor,
            StringTokenType,
            TokenType,
        },
        Context,
        GetPosition,
        GetSeverity,
    },
    util::{
        read_to_string_lossy,
        shared::{
            shared_path_from_str,
            SharedPath,
            SharedString,
            SharedVec,
        },
        string_cache::StringCache,
    },
};

pub(crate) mod float_parsing;
pub(crate) mod translation_phases;
pub(crate) mod util;

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
        | ParsedInput::String(s) => (s, shared_path_from_str("<input>")),
        | ParsedInput::File(s) => (s, SharedPath::from_path_buf(args.input.input_file.unwrap())),
    };
    parse_include_env_var("CPATH", &mut args.system_include);
    parse_include_env_var("C_INCLUDE_PATH", &mut args.system_include);

    let mut context = Context::new();

    let mut preprocessor = Preprocessor::new(
        &mut context,
        source_filename,
        input_string,
        args.quote_include.into(),
        args.system_include.into(),
    );
    while let Some(res) = preprocessor.next() {
        match res {
            | Ok(t) => eprintln!(
                "{}",
                match t.kind {
                    | TokenType::Identifier =>
                        format!("Identifier: {}", context.string_cache.at(t.contents)),
                    | TokenType::Operator(ott) => format!("Operator: {ott:#?}"),
                    | TokenType::String(sltt) => format!(
                        "String-like token: {}",
                        match sltt {
                            | StringTokenType::WideString(s) | StringTokenType::String(s) => {
                                context.string_cache.at(s).to_string()
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
            | Err(e) => {
                let position = e.position();
                eprintln!(
                    "{}: {e} at {}:{}:{}",
                    e.severity(),
                    context.string_cache.at(position.source_file),
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
        preprocessor.tokenizer.as_ref().bright_yellow()
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

#[doc(hidden)]
pub fn preprocess_hundred_thousand() {
    let million_lines = include_str!(concat!(env!("OUT_DIR"), "/hundred-thousand-lines.c"));
    let mut string_cache = StringCache::new();
    let preprocessor = Preprocessor::new(
        million_lines.to_owned().into(),
        string_cache.intern("<input>"),
        string_cache,
        SharedVec::default(),
        SharedVec::default(),
    );
    let count = preprocessor.count();
    eprintln!("{}{}", "Count: ".bright_yellow(), count);
}
