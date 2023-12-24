//! BCC Compiler

use std::{
    fs::read_to_string,
    hash::BuildHasherDefault,
};

use clap::{
    Args,
    Parser,
};
use owo_colors::OwoColorize;
use rustc_hash::FxHasher;
use thiserror::Error;

use crate::{
    translation_phases::{
        phase_4_preprocessing::Preprocessor,
        GetPosition,
        GetSeverity,
    },
    util::string_cache::StringCache,
};

pub(crate) mod float_parsing;
pub(crate) mod translation_phases;
pub(crate) mod util;

pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Parser)]
#[command(author, version, about, long_about)]
struct Cli {
    #[command(flatten)]
    input: Input,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Input {
    #[clap(short, long, conflicts_with = "input_file")]
    input:      Option<String>,
    #[clap(conflicts_with = "input")]
    input_file: Option<String>,
}

enum ParsedInput {
    String(String),
    File(String),
}
#[derive(Debug, Error)]
enum MainError {
    #[error("Failed to open input file: {0}")]
    OpenInputFileError(#[from] std::io::Error),
}

fn main() -> Result<(), MainError> {
    let args = Cli::parse();
    let mut string_cache = StringCache::new();
    println!("{}", "Printing all generated tokens:".bright_green());
    let parsed_input = if args.input.input.is_some() {
        ParsedInput::String(args.input.input.unwrap())
    } else {
        ParsedInput::File(read_to_string(args.input.input_file.as_ref().unwrap())?)
    };
    let (input_string, source_file) = match parsed_input {
        | ParsedInput::String(ref s) => (s.as_str(), string_cache.intern("<input>")),
        | ParsedInput::File(ref s) => (
            s.as_str(),
            string_cache.intern(args.input.input_file.as_ref().unwrap()),
        ),
    };
    let mut preprocessor = Preprocessor::new(input_string, source_file, string_cache);
    while let Some(res) = preprocessor.next() {
        match res {
            | Ok(t) => println!("{t:?}"),
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
    println!(
        "{}",
        "Finished printing all generated tokens.".bright_green()
    );
    Ok(())
}
