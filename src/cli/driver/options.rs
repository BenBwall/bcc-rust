//! The driver's own options, beside the compiler's.

#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the dependency file path."
)]
use std::path::PathBuf;

use clap::{
    Args,
    ValueEnum,
};

/// The driver's options: the `-E` stage, the `-x` language of the inputs
/// after it, and the linker's inputs and options.
#[derive(Args)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the repeated values as `Vec<String>`."
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Clap owns independent command-line switches."
)]
pub(in crate::cli) struct DriverOptions {
    /// Preprocess only: write the preprocessed source to stdout, or to -o
    /// PATH.
    #[arg(short = 'E', conflicts_with_all = ["emit", "interpret", "tokens", "semantic_types",
        "syntax_tree", "raw_syntax"])]
    pub(in crate::cli) preprocess:             bool,
    /// With -E, write no line markers.
    #[arg(short = 'P', requires = "preprocess")]
    pub(in crate::cli) no_line_markers:        bool,
    /// With -E, write the macro definitions in effect at the end of the
    /// input instead of the text (also accepts GCC -dM).
    #[arg(long, requires = "preprocess")]
    pub(in crate::cli) dump_macros:            bool,
    /// Check the input for errors without generating code or printing a
    /// view; like every mode, exit with status 1 after an error (also
    /// accepts GCC -fsyntax-only).
    #[arg(long, conflicts_with_all = ["generation", "preprocess", "tokens"])]
    pub(in crate::cli) syntax_only:            bool,
    /// Treat the input files after it as LANGUAGE: c, assembler,
    /// assembler-with-cpp, or none to decide by extension (also accepts GCC
    /// -x LANGUAGE).
    #[arg(long, value_name = "LANGUAGE")]
    pub(in crate::cli) language:               Vec<String>,
    /// A linker input or option, kept in order with the input files:
    /// -lLIB, -LDIR, -Wl,ARGS, -Xlinker ARG, -static, -nostdlib,
    /// -nostartfiles, -nodefaultlibs, -rdynamic and -pthread.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    pub(in crate::cli) link_option:            Vec<String>,
    /// An assembler option, as clang reads it, for the assembly inputs:
    /// GCC -Wa,ARGS, or -Xassembler ARG.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    pub(in crate::cli) assembler_option:       Vec<String>,
    /// Write make rules for the files the source depends on instead of its
    /// preprocessed text: every file, or only the user's (also accepts GCC
    /// -M and -MM).
    #[arg(long, value_enum, value_name = "FILES", conflicts_with_all = ["emit", "interpret",
        "tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(in crate::cli) dependencies:           Option<DependencyFiles>,
    /// Also write those make rules while compiling, to the -MF file or the
    /// output's name with `.d` (also accepts GCC -MD and -MMD).
    #[arg(long, value_enum, value_name = "FILES")]
    pub(in crate::cli) write_dependencies:     Option<DependencyFiles>,
    /// Write the make rules to PATH (also accepts GCC -MF PATH).
    #[arg(long, value_name = "PATH")]
    pub(in crate::cli) dependency_file:        Option<PathBuf>,
    /// Name TARGET as the rule's target instead of the object (also accepts
    /// GCC -MT TARGET, and -MQ TARGET quoted for make).
    #[arg(long, value_name = "TARGET", allow_hyphen_values = true)]
    pub(in crate::cli) dependency_target:      Vec<String>,
    /// Print `Note: including file: PATH` to stderr for each header an
    /// `#include` enters, with a space more for each level of nesting, as
    /// `cl /showIncludes` does for Ninja's dependency scanning.
    #[arg(long)]
    pub(in crate::cli) show_includes:          bool,
    /// Add an empty rule for each header, so make does not fail when one is
    /// deleted (also accepts GCC -MP).
    #[arg(long)]
    pub(in crate::cli) phony_dependencies:     bool,
    /// Print the header each `#include` enters to stderr, after a dot per
    /// level of nesting (`. a.h`, `.. b.h`), as GCC's and Clang's -H do.
    #[arg(short = 'H', long)]
    pub(in crate::cli) header_tree:            bool,
    /// With --dependencies, take a header that cannot be found to be one a
    /// later step generates: no error, and the rules list it as written
    /// (also accepts GCC -MG).
    #[arg(long, requires = "dependencies")]
    pub(in crate::cli) generated_dependencies: bool,
    /// Print each command the driver runs (the bundled clang, which
    /// assembles and links) and the directories it searches for headers
    /// to stderr, as GCC and Clang print theirs.
    #[arg(short = 'v', long)]
    pub(in crate::cli) verbose:                bool,
    /// Name the driver NAME in its own warnings and errors (`NAME: warning:
    /// ...`), as the driver personalities do for the compiler they mimic.
    #[arg(long, hide = true, value_name = "NAME")]
    pub(in crate::cli) driver_name:            Option<String>,
}

/// Which dependencies a make rule lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(in crate::cli) enum DependencyFiles {
    /// Every file read, including system headers (-M, -MD).
    All,
    /// Only files outside system headers (-MM, -MMD).
    User,
}
