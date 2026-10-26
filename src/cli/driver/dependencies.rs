//! Writes make dependency rules for a translation unit (`-M`, `-MM`,
//! `-MD`, `-MMD`), as GCC writes them: the target, a colon, the source and
//! each header it read, wrapped with backslash-newlines, and with `-MP` an
//! empty rule for each header.
//!
//! The rules describe the source files a translation unit was read from,
//! C99: §5.1.1.1 paragraph 1, p. 9; PDF p. 21; source file inclusion,
//! §6.10.2, pp. 149-150; PDF pp. 161-162. Their form is a GCC convention.

#[expect(
    clippy::disallowed_types,
    reason = "Dependency files and targets are startup paths beside clap's."
)]
use std::path::PathBuf;
use std::{
    fmt::Write as _,
    io::{
        self,
        Write as _,
    },
    path::Path,
};

use super::{
    super::{
        Cli,
        MainError,
    },
    Source,
    options::DependencyFiles,
    output_or_stem,
};
use crate::{
    translation_phases::Context,
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// Where and how one translation unit's rules are written.
#[expect(
    clippy::disallowed_types,
    reason = "Dependency files and targets are startup paths beside clap's."
)]
pub(super) struct DependencyRequest {
    files:          DependencyFiles,
    /// The file, or `None` for stdout.
    destination:    Option<PathBuf>,
    /// The target without `-MT` or `-MQ`.
    default_target: PathBuf,
}

/// The rules `source` writes: for `-M` and `-MM` (`only`), to the `-MF`
/// file, the `-o` path or stdout; for `-MD` and `-MMD`, to the `-MF` file or
/// beside `object`, the object the job writes, with `.d`. The target is
/// `object`, or the source's name with `.o`.
pub(super) fn request(
    args: &Cli,
    source: Source<'_>,
    only: bool,
    object: Option<&Path>,
) -> Result<Option<DependencyRequest>, MainError> {
    let driver = &args.driver;
    let files = if only {
        driver.dependencies
    } else {
        driver.write_dependencies
    };
    let Some(files) = files else {
        return Ok(None);
    };
    let default_target = match object {
        | Some(object) => object.into(),
        | None if matches!(source, Source::Text(_)) => "input.o".into(),
        | None => output_or_stem(None, source.path(), "o")?,
    };
    let destination = match (&driver.dependency_file, only) {
        | (Some(file), _) => Some(file.clone()),
        | (None, true) => args.code_generation.output_path.clone(),
        | (None, false) => Some(default_target.with_extension("d")),
    };
    Ok(Some(DependencyRequest {
        files,
        destination,
        default_target,
    }))
}

/// Writes the rules of the translation unit `context` read.
pub(super) fn write(
    context: &Context<'_>,
    args: &Cli,
    request: &DependencyRequest,
) -> Result<(), MainError> {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    let mut column = 0;
    let mut targets = args
        .driver
        .dependency_target
        .iter()
        .map(AsRef::as_ref)
        .peekable();
    let default = request.default_target.to_string_lossy();
    if targets.peek().is_none() {
        add_word(&mut text, &mut column, escape(&arena, &default));
    }
    for target in targets {
        add_word(&mut text, &mut column, target);
    }
    text.push(':');
    column += 1;
    let headers = context
        .opened_source_files()
        .filter(|&(index, _)| {
            request.files == DependencyFiles::All || !context.has_system_header_part(index)
        })
        .map(|(_, path)| escape(&arena, &path.to_string_lossy()));
    let mut first = true;
    let mut phony = ArenaString::new_in(&arena);
    for header in headers {
        add_word(&mut text, &mut column, header);
        if !first && args.driver.phony_dependencies {
            _ = write!(phony, "\n{header}:\n");
        }
        first = false;
    }
    text.push('\n');
    text.push_str(&phony);
    match &request.destination {
        | Some(path) => std::fs::write(path, text.as_bytes()).map_err(|source| {
            MainError::WriteOutputFileError {
                path: path.clone(),
                source,
            }
        }),
        | None => {
            let mut stdout = io::stdout().lock();
            drop(
                stdout
                    .write_all(text.as_bytes())
                    .and_then(|()| stdout.flush()),
            );
            Ok(())
        },
    }
}

/// Prints `Note: including file: PATH` to stderr for each header the
/// translation unit read, as `cl /showIncludes` does. `cl` indents a header
/// by its nesting depth, which bcc does not keep; Ninja reads the path
/// after the prefix and any spaces.
pub(super) fn show_includes(context: &Context<'_>) {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    for (_, path) in context.opened_source_files().skip(1) {
        _ = writeln!(text, "Note: including file: {}", path.display());
    }
    drop(io::stderr().lock().write_all(text.as_bytes()));
}

/// GCC wraps rule lines before this column.
const WRAP_COLUMN: usize = 76;

/// Appends ` word`, first breaking the line with a backslash when it would
/// pass [`WRAP_COLUMN`].
fn add_word(text: &mut ArenaString<'_>, column: &mut usize, word: &str) {
    if *column != 0 {
        if *column + 1 + word.len() > WRAP_COLUMN {
            text.push_str(" \\\n ");
            *column = 1;
        } else {
            text.push(' ');
            *column += 1;
        }
    }
    text.push_str(word);
    *column += word.len();
}

/// A file name as make reads it: blanks and `#` backslashed and `$`
/// doubled, as GCC writes dependencies.
fn escape<'a>(arena: &'a Bump, name: &str) -> &'a str {
    let mut escaped = ArenaString::new_in(arena);
    for character in name.chars() {
        match character {
            | ' ' | '\t' | '#' => escaped.push('\\'),
            | '$' => escaped.push('$'),
            | _ => {},
        }
        escaped.push(character);
    }
    escaped.into_str()
}
