//! Which assembler assembles the assembly inputs: the bundled clang's
//! integrated assembler, or the system's `as`, which GCC's driver runs for
//! every `.s` and `.S` input (GCC manual, "Overall Options"). bcc's own
//! objects never pass through either: its back ends write them.
//!
//! The system assembler is found as GCC's driver finds the programs it runs
//! (GCC manual, "Directory Options", `-B`; "Environment Variables",
//! `COMPILER_PATH`), and the bundled clang runs it with
//! `-fno-integrated-as`, passing the `-Wa,` and `-Xassembler` options on as
//! GCC does (GCC manual, "Assembler Options").

#[expect(
    clippy::disallowed_types,
    reason = "The assembler's prefix is a startup string beside clap's arguments."
)]
use std::{
    ffi::OsString,
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
};

use clap::ValueEnum;

use super::{
    Cli,
    Severity,
    report,
};
use crate::target::Target;

/// Which assembler the driver runs for the assembly inputs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, ValueEnum)]
pub(in crate::cli) enum Assembler {
    /// The bundled clang's integrated assembler (also accepts Clang
    /// -fintegrated-as).
    #[default]
    Integrated,
    /// The system's `as`, which must exist (also accepts Clang
    /// -fno-integrated-as).
    System,
    /// The system's `as` when there is one, as GCC runs it; otherwise the
    /// integrated assembler, with a note.
    Auto,
}

/// What the bundled clang needs to assemble an input with the selected
/// assembler, or `Err` after reporting that `System` found none.
/// `Ok(None)` is the integrated assembler; `Ok(Some(prefix))` the system
/// one, which clang finds as `prefix` followed by `as` (its `-B`).
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "The assembler's prefix and the search path are startup strings beside clap's \
              arguments."
)]
pub(super) fn select(args: &Cli, input: &Path) -> Result<Option<OsString>, ()> {
    let choice = args.driver.assembler;
    if choice == Assembler::Integrated {
        return Ok(None);
    }
    let found = if writes_host_objects(args.target) {
        find(
            &args.driver.program_prefix,
            std::env::var_os("COMPILER_PATH").as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
    } else {
        None
    };
    match (found, choice) {
        | (Some(prefix), _) => Ok(Some(prefix)),
        | (None, Assembler::System) => {
            report(
                args,
                Severity::Error,
                format_args!(
                    "cannot assemble `{}`: no system assembler `as` for {} in the -B prefixes, \
                     COMPILER_PATH or PATH",
                    input.display(),
                    args.target.triple()
                ),
            );
            Err(())
        },
        | (None, _) => {
            // Once per command, not once per input.
            static NOTED: AtomicBool = AtomicBool::new(false);
            if !NOTED.swap(true, Ordering::Relaxed) {
                report(
                    args,
                    Severity::Note,
                    format_args!(
                        "no system assembler `as` for {} found; assembling with the bundled \
                         clang's integrated assembler, which rejects GNU as options such as -Wa,-v",
                        args.target.triple()
                    ),
                );
            }
            Ok(None)
        },
    }
}

/// Whether the host's `as` writes `target`'s objects: ELF for x86-64 Linux,
/// with glibc or musl, on an x86-64 Linux host. Elsewhere, Windows and the
/// WebAssembly site among them, the integrated assembler runs.
fn writes_host_objects(target: Target) -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && matches!(target, Target::LinuxGnu | Target::LinuxMusl)
}

/// The prefix that names the first `as` found, searched as GCC's driver
/// searches for a program it runs: each `-B` prefix in order, then each
/// `COMPILER_PATH` directory, then each `PATH` directory. GCC manual,
/// "Directory Options": a `-B` prefix is joined to the program's name, and
/// one that names a directory gets the separator it lacks; GCC's own
/// installation directories, which follow the `-B` prefixes there, are no
/// part of bcc.
#[expect(
    clippy::disallowed_types,
    reason = "The assembler's prefix is a startup string beside clap's arguments."
)]
fn find(
    prefixes: &[PathBuf],
    compiler_path: Option<&std::ffi::OsStr>,
    path: Option<&std::ffi::OsStr>,
) -> Option<OsString> {
    let directories = [compiler_path, path]
        .into_iter()
        .flatten()
        .flat_map(std::env::split_paths)
        .filter(|directory| !directory.as_os_str().is_empty());
    prefixes
        .iter()
        .cloned()
        .chain(directories)
        .map(|prefix| {
            let mut prefix = prefix.into_os_string();
            if Path::new(&prefix).is_dir() && !ends_with_separator(&prefix) {
                prefix.push(std::path::MAIN_SEPARATOR_STR);
            }
            prefix
        })
        .find(|prefix| {
            let mut program = prefix.clone();
            program.push("as");
            is_executable(Path::new(&program))
        })
}

/// Whether `prefix` ends with a directory separator.
fn ends_with_separator(prefix: &std::ffi::OsStr) -> bool {
    prefix
        .as_encoded_bytes()
        .last()
        .is_some_and(|byte| std::path::is_separator(char::from(*byte)))
}

/// Whether `program` is a file the driver may run.
fn is_executable(program: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(program) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    use super::find;

    #[test]
    #[cfg(unix)]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        reason = "The test builds a scratch directory of programs."
    )]
    fn searches_the_prefixes_then_compiler_path_then_path() {
        let root = std::env::temp_dir().join(format!("bcc-as-search-{}", std::process::id()));
        let program = |directory: &str, name: &str, mode: u32| {
            let directory = root.join(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(name);
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        program("prefix", "x-as", 0o755);
        program("compiler", "as", 0o755);
        program("path", "as", 0o755);
        program("plain", "as", 0o644);
        let os = |text: std::path::PathBuf| text.into_os_string();
        let compiler = os(root.join("compiler"));
        let path = std::env::join_paths([root.join("plain"), root.join("path")]).unwrap();
        // A prefix that is no directory is joined to the name.
        let mut x = os(root.join("prefix"));
        x.push("/x-");
        assert_eq!(
            find(&[root.join("prefix/x-")], Some(&compiler), Some(&path)),
            Some(x)
        );
        // A directory prefix gets its separator; one without `as` is skipped.
        let mut joined = os(root.join("compiler"));
        joined.push("/");
        assert_eq!(
            find(&[root.join("prefix")], Some(&compiler), Some(&path)),
            Some(joined)
        );
        // A file that is not executable is not the program.
        let mut on_path = os(root.join("path"));
        on_path.push("/");
        assert_eq!(find(&[], None, Some(&path)), Some(on_path));
        assert_eq!(find(&[], None, None), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
