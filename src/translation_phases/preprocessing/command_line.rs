//! GCC command-line preprocessing inputs to phase 4. Definitions and
//! undefinitions use the ordinary directive parser and builtin protections.
//! C99: extension to §6.10.3p9, p. 152; PDF p. 164; §6.10.3.5, p. 155;
//! PDF p. 167; include lookup §6.10.2p3, pp. 149-150; PDF pp. 161-162.

use std::fmt::Write as _;

use crate::{
    configuration::PreprocessingOption,
    translation_phases::Context,
    util::bump::ArenaString,
};

/// Produce a deterministic synthetic source without reimplementing macro
/// lexing, parameter lists, redefinition diagnostics or reserved-name checks.
/// GCC truncates definitions at the first newline. Forced includes are read
/// in their command-line order, after every `-D`/`-U`, starting in the working
/// directory rather than beside the main source (C99 §6.10.2p3).
pub(super) fn source<'tu>(context: &Context<'tu>) -> &'tu str {
    let mut source = ArenaString::new_in(context.tu_arena());
    for option in context.preprocessing_options {
        match *option {
            | PreprocessingOption::Define(definition) => {
                let definition = definition.split('\n').next().unwrap_or("");
                let (name, value) = definition.split_once('=').unwrap_or((definition, "1"));
                _ = writeln!(source, "#define {name} {value}");
            },
            | PreprocessingOption::Undefine(name) => {
                _ = writeln!(source, "#undef {}", name.split('\n').next().unwrap_or(""));
            },
            | PreprocessingOption::Include(_) => {},
        }
    }
    for option in context.preprocessing_options {
        if let PreprocessingOption::Include(file) = option {
            _ = write!(source, "#include \"");
            // Header-name backslashes are path separators on Windows, not C
            // escapes. Normalize them without allocating an owned string.
            for character in file.chars() {
                _ = source.write_char(if character == '\\' { '/' } else { character });
            }
            _ = writeln!(source, "\"");
        }
    }
    source.into_str()
}
