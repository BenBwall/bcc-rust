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
    super::output_file,
    Cli,
    MainError,
};

/// One input, in command-line order.
#[derive(Clone, Copy, Debug)]
pub(super) enum Input<'a> {
    /// A C translation unit's source.
    C(Source<'a>),
    /// Assembly, which the bundled clang assembles.
    Assembly(Assembly<'a>),
    /// An object, archive or library file, or a linker option.
    Link(&'a OsStr),
}

impl<'a> Input<'a> {
    pub(super) const fn c_source(self) -> Option<Source<'a>> {
        match self {
            | Self::C(source) => Some(source),
            | _ => None,
        }
    }

    /// The file the input names, if it is one: not `--input`, stdin or a
    /// linker option.
    pub(super) fn file(self) -> Option<&'a Path> {
        match self {
            | Self::C(Source::File(path) | Source::Preprocessed(path)) if path != Path::new("-") =>
                Some(path),
            | Self::Assembly(assembly) => Some(assembly.path),
            | Self::Link(name) if !name.to_string_lossy().starts_with('-') => Some(Path::new(name)),
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
    /// Already preprocessed text (`.i`, or `-x cpp-output`), which is not
    /// preprocessed again, as GCC's `-fpreprocessed` reads it.
    Preprocessed(&'a Path),
}

impl<'a> Source<'a> {
    pub(super) fn path(self) -> &'a Path {
        match self {
            | Self::Text(_) => Path::new("<input>"),
            | Self::File(path) | Self::Preprocessed(path) => path,
        }
    }
}

/// An assembly input and the `-x` language it was given under.
#[derive(Clone, Copy, Debug)]
pub(super) struct Assembly<'a> {
    pub(super) path:     &'a Path,
    /// `-x assembler` (false) or `-x assembler-with-cpp` (true), or `None`
    /// to decide by extension.
    pub(super) language: Option<bool>,
}

impl Assembly<'_> {
    /// Whether the C preprocessor runs first: `-x assembler-with-cpp`, or
    /// a `.S` or `.sx` file without `-x`.
    pub(super) fn preprocessed(self) -> bool {
        self.language.unwrap_or_else(|| {
            matches!(
                self.path.extension().and_then(OsStr::to_str),
                Some("S" | "sx")
            )
        })
    }
}

/// The inputs and linker options in command-line order, each file
/// classified by the `-x` language in effect, otherwise by its extension:
/// `.c` and `.h` are C, `.i` preprocessed C, `.s`, `.S` and `.sx` assembly,
/// and anything else goes to the linker, as GCC decides.
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
        Preprocessed,
        Assembly(bool),
    }
    enum Item<'a> {
        File(&'a Path),
        Link(&'a OsStr),
        Language(&'a str),
    }
    if let Some(text) = &args.input.input {
        let mut inputs = Vec::from([Input::C(Source::Text(text))]);
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
                    | "c" | "c-header" => Some(Language::C),
                    | "cpp-output" => Some(Language::Preprocessed),
                    | "assembler" => Some(Language::Assembly(false)),
                    | "assembler-with-cpp" => Some(Language::Assembly(true)),
                    | _ => return Err(MainError::LanguageNotRecognized(name.into())),
                };
            },
            | Item::Link(option) => inputs.push(Input::Link(option)),
            | Item::File(path) => inputs.push(match language {
                | Some(Language::C) => Input::C(Source::File(path)),
                | Some(Language::Preprocessed) => Input::C(Source::Preprocessed(path)),
                | Some(Language::Assembly(preprocessed)) => Input::Assembly(Assembly {
                    path,
                    language: Some(preprocessed),
                }),
                | None => by_extension(path),
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

/// The input `path` is without `-x`: `-` (stdin), `.c` and `.h` are C, `.i`
/// preprocessed C, `.s`, `.S` and `.sx` assembly, and anything else goes to
/// the linker.
fn by_extension(path: &Path) -> Input<'_> {
    match path.extension().and_then(OsStr::to_str) {
        | _ if path == Path::new("-") => Input::C(Source::File(path)),
        | Some("c" | "h") => Input::C(Source::File(path)),
        | Some("i") => Input::C(Source::Preprocessed(path)),
        | Some("s" | "S" | "sx") => Input::Assembly(Assembly {
            path,
            language: None,
        }),
        | _ => Input::Link(path.as_os_str()),
    }
}

/// Whether the driver compiles `path` without `-x`, as C or assembly,
/// rather than passing it to the linker.
pub(in crate::cli) fn compiles(path: &Path) -> bool {
    !matches!(by_extension(path), Input::Link(_))
}

/// Whether the driver compiles `path` without `-x` as C.
pub(in crate::cli) fn is_c(path: &Path) -> bool {
    matches!(by_extension(path), Input::C(_))
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

/// Refuses a final output that is one of the input files, before any job
/// runs: compilation writes temporary objects, so only this check stops the
/// link from replacing a source, as GCC refuses `gcc a.c b.c -o a.c`. The
/// comparison is by file identity (device and inode on Unix), so a symbolic
/// or hard link to an input is refused too. An input that cannot be read is
/// left for its job to report.
pub(super) fn check_output(inputs: &[Input<'_>], output: &Path) -> Result<(), MainError> {
    if output == Path::new("-") {
        return Ok(());
    }
    for path in inputs.iter().filter_map(|input| input.file()) {
        if std::fs::metadata(path).is_err() {
            continue;
        }
        let same_file = output_file::is_input(path, output).map_err(|source| {
            MainError::InspectOutputFileError {
                path: output.into(),
                source,
            }
        })?;
        if same_file {
            return Err(MainError::OutputIsInput {
                input:  path.into(),
                output: output.into(),
            });
        }
    }
    Ok(())
}
