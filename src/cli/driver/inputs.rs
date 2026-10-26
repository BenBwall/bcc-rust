//! Orders the inputs and linker options as the command line gave them,
//! classifies each input file, and names the outputs `-c` and `-S` write.

#[expect(
    clippy::disallowed_types,
    reason = "Clap owns argv; output names are startup paths outside compilation arenas."
)]
use std::path::PathBuf;
use std::{
    ffi::OsStr,
    path::Path,
};

use clap::ArgMatches;

use super::{
    Cli,
    MainError,
};

/// One input, in command-line order.
#[derive(Clone, Copy, Debug)]
pub(super) enum Input<'a> {
    /// `--input TEXT`.
    Text(&'a str),
    /// A C source file, or `-` for stdin.
    Source(&'a Path),
    /// Assembly, which the bundled clang assembles.
    Assembly(&'a Path),
    /// An object, archive or library file, or a linker option.
    Link(&'a OsStr),
}

impl<'a> Input<'a> {
    pub(super) fn c_source(self) -> Option<Source<'a>> {
        match self {
            | Self::Text(text) => Some(Source::Text(text)),
            | Self::Source(path) => Some(Source::File(path)),
            | _ => None,
        }
    }
}

/// A C translation unit's source.
#[derive(Clone, Copy, Debug)]
pub(super) enum Source<'a> {
    /// `--input TEXT`, named `<input>`.
    Text(&'a str),
    /// A file, or `-` for stdin.
    File(&'a Path),
}

impl<'a> Source<'a> {
    pub(super) fn path(self) -> &'a Path {
        match self {
            | Self::Text(_) => Path::new("<input>"),
            | Self::File(path) => path,
        }
    }
}

/// The inputs and linker options in command-line order, each file
/// classified by the `-x` language in effect, otherwise by its extension:
/// `.c`, `.i` and `.h` are C, `.s`, `.S` and `.sx` assembly, and anything
/// else goes to the linker, as GCC decides.
#[expect(
    clippy::disallowed_types,
    reason = "Startup orders clap's argument values in a `Vec` beside clap's own."
)]
pub(super) fn ordered_inputs<'a>(
    args: &'a Cli,
    matches: &ArgMatches,
) -> Result<Vec<Input<'a>>, MainError> {
    enum Language {
        C,
        Assembly,
    }
    enum Item<'a> {
        File(&'a Path),
        Link(&'a OsStr),
        Language(&'a str),
    }
    if let Some(text) = &args.input.input {
        let mut inputs = Vec::from([Input::Text(text)]);
        inputs.extend(
            args.driver
                .link_option
                .iter()
                .map(|option| Input::Link(option.as_ref())),
        );
        return Ok(inputs);
    }
    let indices = |id: &str| matches.indices_of(id).into_iter().flatten();
    let mut items: Vec<(usize, Item<'a>)> = indices("input_files")
        .zip(&args.input.input_files)
        .map(|(index, path)| (index, Item::File(path)))
        .chain(
            indices("link_option")
                .zip(&args.driver.link_option)
                .map(|(index, option)| (index, Item::Link(option.as_ref()))),
        )
        .chain(
            indices("language")
                .zip(&args.driver.language)
                .map(|(index, language)| (index, Item::Language(language))),
        )
        .collect();
    items.sort_by_key(|(index, _)| *index);
    let mut language = None;
    let mut inputs = Vec::with_capacity(items.len());
    for (_, item) in items {
        match item {
            | Item::Language(name) => {
                language = match name {
                    | "none" => None,
                    | "c" | "c-header" | "cpp-output" => Some(Language::C),
                    | "assembler" | "assembler-with-cpp" => Some(Language::Assembly),
                    | _ => return Err(MainError::LanguageNotRecognized(name.into())),
                };
            },
            | Item::Link(option) => inputs.push(Input::Link(option)),
            | Item::File(path) => inputs.push(match language {
                | Some(Language::C) => Input::Source(path),
                | Some(Language::Assembly) => Input::Assembly(path),
                | None => match path.extension().and_then(OsStr::to_str) {
                    | _ if path == Path::new("-") => Input::Source(path),
                    | Some("c" | "i" | "h") => Input::Source(path),
                    | Some("s" | "S" | "sx") => Input::Assembly(path),
                    | _ => Input::Link(path.as_os_str()),
                },
            }),
        }
    }
    if inputs.iter().all(|input| {
        matches!(input, Input::Link(option)
        if option.to_string_lossy().starts_with('-'))
    }) {
        return Err(MainError::Driver("no input files"));
    }
    Ok(inputs)
}

/// The `-o` path, or the input's file name with `extension` in the working
/// directory, as GCC names a `-c` or `-S` output.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Output paths are startup paths beside clap's."
)]
pub(super) fn output_or_stem(
    output: Option<&Path>,
    input: &Path,
    extension: &str,
) -> Result<PathBuf, MainError> {
    if let Some(output) = output {
        return Ok(output.to_path_buf());
    }
    if input == Path::new("<input>") || input == Path::new("-") {
        return Err(MainError::Driver(
            "-c and -S name their output after the input file; give -o for --input or stdin",
        ));
    }
    let name = input.file_name().map_or(input.as_os_str(), |name| name);
    Ok(Path::new(name).with_extension(extension))
}
