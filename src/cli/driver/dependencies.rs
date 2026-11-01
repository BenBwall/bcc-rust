//! Writes make dependency rules for a translation unit (`-M`, `-MM`,
//! `-MD`, `-MMD`), as GCC writes them: the target, a colon, the source and
//! each header it read, wrapped with backslash-newlines, and with `-MP` an
//! empty rule for each header. Under TCC's identity they take TCC's form
//! (`gen_makedeps` in `tcctools.c`): every file on a line of its own.
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
    configuration::{
        CompilerIdentity,
        RuleSet,
    },
    headers::HeaderSearch,
    translation_phases::Context,
    util::bump::{
        ArenaString,
        ArenaVec,
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

/// The outputs a job's rules are named after, as GCC derives them from
/// the stage and `-o`.
#[derive(Clone, Copy, Default)]
pub(super) struct DependencyNames<'a> {
    /// The path whose name, with `.d`, names the `-MD` file; `None` names
    /// it after the source in the working directory.
    pub(super) file:   Option<&'a Path>,
    /// The rule's target without `-MT` or `-MQ`; `None` is the source's
    /// name with `.o`.
    pub(super) target: Option<&'a Path>,
}

/// The rules `source` writes: for `-M` and `-MM` (`only`), to the `-MF`
/// file, the `-o` path or stdout; for `-MD` and `-MMD`, to the `-MF` file or
/// the `.d` file `names` gives. `-MF -` and `-o -` write to stdout, and a
/// `-` in `names` is not a name, so `-S -o -` names the file after the
/// source (GCC would write a file named `-.d`).
pub(super) fn request(
    args: &Cli,
    source: Source<'_>,
    only: bool,
    names: DependencyNames<'_>,
) -> Result<Option<DependencyRequest>, MainError> {
    let path = (!matches!(source, Source::Text(_))).then(|| source.path());
    request_at(args, path, only, names)
}

/// [`request`] for the input file `path`, or for text from stdin (`None`):
/// also for an assembly input, whose rules the bundled clang writes from
/// [`DependencyRequest::clang_arguments`].
pub(super) fn request_at(
    args: &Cli,
    path: Option<&Path>,
    only: bool,
    names: DependencyNames<'_>,
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
    let default_target = match (named(names.target), path) {
        | (Some(target), _) => target.into(),
        | (None, None) => "input.o".into(),
        | (None, Some(path)) => output_or_stem(None, path, "o")?,
    };
    let destination = match (&driver.dependency_file, only) {
        | (Some(file), _) => named(Some(file.as_path())).map(Into::into),
        | (None, true) => named(args.code_generation.output_path.as_deref()).map(Into::into),
        | (None, false) => Some(match (named(names.file), path) {
            | (Some(file), _) => file.with_extension("d"),
            | (None, None) => "input.d".into(),
            | (None, Some(path)) => output_or_stem(None, path, "d")?,
        }),
    };
    Ok(Some(DependencyRequest {
        files,
        destination,
        default_target,
    }))
}

impl DependencyRequest {
    /// The clang options that write these rules for an input the bundled
    /// clang reads: `-M`/`-MM` for a dependency job (`only`), `-MD`/`-MMD`
    /// beside its output, to the same file and for the same targets as a C
    /// source's rules; the default target is quoted with `-MQ`, as GCC
    /// quotes it. Clang writes them in GCC's form.
    #[expect(
        clippy::disallowed_types,
        reason = "Startup builds the clang command line beside clap's arguments."
    )]
    pub(super) fn clang_arguments(&self, args: &Cli, only: bool) -> Vec<std::ffi::OsString> {
        let mut options: Vec<std::ffi::OsString> = Vec::new();
        options.push(
            match (only, self.files) {
                | (true, DependencyFiles::All) => "-M",
                | (true, DependencyFiles::User) => "-MM",
                | (false, DependencyFiles::All) => "-MD",
                | (false, DependencyFiles::User) => "-MMD",
            }
            .into(),
        );
        if let Some(file) = &self.destination {
            options.push("-MF".into());
            options.push(file.into());
        }
        if args.driver.dependency_target.is_empty() {
            options.push("-MQ".into());
            options.push(self.default_target.as_os_str().into());
        }
        for target in &args.driver.dependency_target {
            options.push("-MT".into());
            options.push(target.into());
        }
        if args.driver.phony_dependencies {
            options.push("-MP".into());
        }
        options
    }
}

/// `path`, unless it is `-`, which names stdout rather than a file.
fn named(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| *path != Path::new("-"))
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
    // A header `-MG` took to be generated is listed as written, in the
    // order it was included; GCC's `-MM` leaves out a `<...>` one, which
    // Clang's lists.
    let gcc = context.configuration.compiler_identity().rules() == RuleSet::Gcc;
    let mut files = ArenaVec::new_in(&arena);
    files.extend(context.opened_source_files().filter(|&(index, _)| {
        request.files == DependencyFiles::All || !context.has_system_header_part(index)
    }));
    files.extend(
        context
            .generated_headers
            .iter()
            .filter(|&&(_, angled)| request.files == DependencyFiles::All || !angled || !gcc)
            .map(|&(index, _)| (index, context.get_source_file(index))),
    );
    files.sort_by_key(|&(index, _)| index);
    let headers = files
        .iter()
        .map(|(_, path)| escape(&arena, &path.to_string_lossy()));
    if context.configuration.compiler_identity() == CompilerIdentity::Tcc {
        // `target: \`, then `  file \` per file, the last without the
        // backslash; `-MP` adds `file:` for each file after the source.
        text.clear();
        let mut targets = args.driver.dependency_target.iter();
        match targets.next() {
            | Some(first) => {
                text.push_str(first);
                for target in targets {
                    text.push(' ');
                    text.push_str(target);
                }
            },
            | None => text.push_str(&default),
        }
        text.push(':');
        let mut phony = ArenaString::new_in(&arena);
        for (index, header) in headers.enumerate() {
            _ = write!(text, " \\\n  {header}");
            if index != 0 && args.driver.phony_dependencies {
                _ = writeln!(phony, "{header}:");
            }
        }
        text.push('\n');
        text.push_str(&phony);
        return emit(request, &text);
    }
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
    emit(request, &text)
}

/// Writes the rules `text` where `request` sends them.
fn emit(request: &DependencyRequest, text: &str) -> Result<(), MainError> {
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

/// How Clang begins the name of a header it reached from a main file in
/// the working directory: that directory, `.`, appended to with the host's
/// separator (`llvm::sys::path::append`), so `.\sub/s.h` on Windows.
const HERE: &str = if cfg!(windows) { ".\\" } else { "./" };

/// Prints `Note: including file:` and the header to stderr for each
/// `#include` that entered one, as `cl /showIncludes` does, with a space
/// more before the path for each level of nesting; Ninja reads the path
/// after the prefix and the spaces. clang-cl begins a path as `-H` does.
pub(super) fn show_includes(context: &Context<'_>) {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    let clang = context.configuration.compiler_identity().rules() == RuleSet::Clang;
    for entry in context.include_entries.iter().flatten() {
        _ = writeln!(
            text,
            "Note: including file:{:width$}{}{}",
            "",
            if clang && entry.here { HERE } else { "" },
            context.get_source_file(entry.file).display(),
            width = entry.depth as usize,
        );
    }
    drop(io::stderr().lock().write_all(text.as_bytes()));
}

/// Prints the header each `#include` entered to stderr for `-H`, after a
/// dot per level of nesting, as GCC and Clang do. Clang names a header
/// reached from a main file in the working directory through its
/// includers' directories `./a.h` or `./sub/../b.h` ([`HERE`]), GCC `a.h`.
/// Clang escapes the name as a string literal's contents
/// (`Lexer::Stringify`), so `.\\sub\\t.h` on Windows; GCC prints it as is.
pub(super) fn header_tree(context: &Context<'_>) {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    let clang = context.configuration.compiler_identity().rules() == RuleSet::Clang;
    for entry in context.include_entries.iter().flatten() {
        for _ in 0..entry.depth {
            text.push('.');
        }
        text.push(' ');
        let mut name = ArenaString::new_in(&arena);
        if clang && entry.here {
            name.push_str(HERE);
        }
        _ = write!(name, "{}", context.get_source_file(entry.file).display());
        if clang {
            for character in name.chars() {
                if matches!(character, '\\' | '"') {
                    text.push('\\');
                }
                text.push(character);
            }
        } else {
            text.push_str(&name);
        }
        text.push('\n');
    }
    drop(io::stderr().lock().write_all(text.as_bytes()));
}

/// Prints the header search directories to stderr for `-v`, as GCC and
/// Clang list them: the `-iquote` directories after `#include "..." search
/// starts here:`, the others, in search order, after `#include <...>
/// search starts here:`, and `End of search list.` Build systems such as
/// `cmake` read the list for the implicit include directories. bcc's embedded
/// resource headers are no directory on disk and are not listed.
pub(super) fn print_search(search: &HeaderSearch<'_>) {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    text.push_str("#include \"...\" search starts here:\n");
    let on_disk = |directory: &&&Path| !directory.to_string_lossy().starts_with('<');
    for directory in search.quote.iter().filter(on_disk) {
        _ = writeln!(text, " {}", directory.display());
    }
    text.push_str("#include <...> search starts here:\n");
    let angled = search
        .angled
        .iter()
        .chain(search.system)
        .chain(search.after);
    for directory in angled.filter(on_disk) {
        _ = writeln!(text, " {}", directory.display());
    }
    text.push_str("End of search list.\n");
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
