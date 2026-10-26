//! Driver personalities make bcc's command line mimic another compiler's:
//! `--gcc-mode`, `--clang-mode`, `--msvc-mode` (`cl.exe`),
//! `--clang-cl-mode` and `--tcc-mode`, or an executable name such as `gcc`,
//! `cc`, `clang`, `cl` or `tcc` (also with a prefix, `bcc-gcc` or
//! `x86_64-w64-mingw32-gcc`, and a version suffix, `gcc-14`). A personality
//! reads the rest of the command line with that compiler's conventions and
//! translates it into bcc's own options, which the ordinary driver then
//! runs, so every personality shares one implementation of each option.
//!
//! The translation also selects what makes bcc behave as that compiler: its
//! default target, language mode and system header directories, the
//! compiler identity its predefined macros claim, its diagnostic format, its
//! default output names, and its version banners. The compiler's options
//! that bcc does not implement are accepted and ignored where they cannot
//! change the program's meaning, and rejected where they would; unknown
//! options are diagnosed as that compiler diagnoses them.
//!
//! For `gcc -O2 -Wshadow a.c b.c`, the GCC personality keeps `-O2`, drops
//! `-Wshadow` (a warning GCC knows and bcc lacks), and adds `-o a.out`,
//! GCC's identity and diagnostic format, and the host's system headers.
//!
//! Read [`select`] and [`run`], then [`gnu::plan`], [`msvc::plan`] and
//! [`tcc::plan`]. `docs/design/driver-modes.md` records the design and
//! every deviation from the compilers mimicked.
//!
//! Files by role:
//! - Personalities: `personality/gnu.rs` (GCC and Clang), `personality/msvc.rs`
//!   (`cl.exe` and clang-cl), `personality/tcc.rs`.
//! - Version banners and queries: `personality/version.rs`.
//! - The option names each compiler knows: `personality/known.rs`, over the
//!   lists in `personality/options/`; which of those bcc ignores, warns about
//!   or refuses: `personality/unimplemented.rs`.
//!
//! Driver conventions are host CLI policy; the translated options select
//! the language modes of C99 §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Personalities rewrite argv before any compilation starts: the OS hands it over as \
              owned strings, and bcc's parser takes it back as owned strings."
)]

// Personalities
mod gnu;
mod msvc;
mod tcc;

// Shared tables and text
mod known;
mod unimplemented;
mod version;

use std::{
    ffi::{
        OsStr,
        OsString,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

use super::MainError;
use crate::target::Target;

/// Runs the command line as `personality` reads it and returns the exit
/// status: the status of the first step that fails, or of the last.
pub(super) fn run(
    personality: Personality,
    arguments: Vec<OsString>,
    run_bcc: impl Fn(Vec<OsString>) -> Result<i32, MainError>,
) -> Result<i32, MainError> {
    let invocation = Invocation::new(personality, arguments);
    let plan = match personality {
        | Personality::Gcc | Personality::Clang => gnu::plan(&invocation)?,
        | Personality::Msvc | Personality::ClangCl => msvc::plan(&invocation)?,
        | Personality::Tcc => tcc::plan(&invocation)?,
    };
    let mut status = 0;
    for step in plan.steps {
        status = match step {
            | Step::Bcc(arguments) => run_bcc(arguments)?,
            | Step::Print { text, stderr } => {
                print(&text, stderr);
                0
            },
            | Step::Run { program, arguments } =>
                match Command::new(&program).args(arguments).status() {
                    | Ok(status) => exit_status(status),
                    | Err(error) => {
                        invocation
                            .error(format_args!("cannot run '{}': {error}", program.display()));
                        1
                    },
                },
        };
        if status != 0 {
            break;
        }
    }
    drop(plan.temporary);
    Ok(status)
}

/// The personality the command line selects: the first `--NAME-mode` or
/// `--driver-mode=NAME` argument, which is removed, or else the name the
/// executable was run as.
pub(super) fn select(arguments: &mut Vec<OsString>) -> Option<Personality> {
    let flag = arguments.iter().skip(1).position(|argument| {
        argument
            .to_str()
            .is_some_and(|text| Personality::from_flag(text).is_some())
    });
    if let Some(index) = flag {
        let flag = arguments.remove(index + 1);
        return flag.to_str().and_then(Personality::from_flag);
    }
    let program = arguments.first()?;
    Personality::from_program(Path::new(program)).map(|(personality, _)| personality)
}

/// A compiler whose command line bcc can mimic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Personality {
    /// GCC 14.2.
    Gcc,
    /// The pinned Clang (23.1).
    Clang,
    /// MSVC 19.33's `cl.exe`.
    Msvc,
    /// clang-cl, Clang's `cl.exe`-compatible driver.
    ClangCl,
    /// TCC 0.9.27.
    Tcc,
}

impl Personality {
    fn from_flag(text: &str) -> Option<Self> {
        let name = text
            .strip_prefix("--driver-mode=")
            .or_else(|| text.strip_prefix("--")?.strip_suffix("-mode"))?;
        Some(match name {
            | "gcc" | "gnu" => Self::Gcc,
            | "clang" => Self::Clang,
            | "msvc" | "cl" => Self::Msvc,
            | "clang-cl" => Self::ClangCl,
            | "tcc" => Self::Tcc,
            | _ => return None,
        })
    }

    /// The personality an executable named like `program` has, and the
    /// target its prefix names, if it names one: `gcc`, `cc`, `c89` and
    /// `c99` are GCC, `clang` Clang, `cl` MSVC, `clang-cl` clang-cl and
    /// `tcc` TCC, after an optional `PREFIX-` (`bcc-` or a target triple)
    /// and before an optional version (`-14`, `-18.1`) and `.exe`.
    fn from_program(program: &Path) -> Option<(Self, Option<Target>)> {
        let name = program.file_stem()?.to_str()?;
        let name = name.to_ascii_lowercase();
        let mut name = name.as_str();
        // `gcc-14`, `clang-18.1`.
        if let Some((stem, version)) = name.rsplit_once('-')
            && !version.is_empty()
            && version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            name = stem;
        }
        let (prefix, tool) = if let Some(prefix) = name.strip_suffix("clang-cl") {
            (prefix, "clang-cl")
        } else {
            name.rsplit_once('-')
                .map_or(("", name), |(prefix, tool)| (prefix, tool))
        };
        let personality = match tool {
            | "gcc" | "cc" | "c89" | "c99" => Self::Gcc,
            | "clang" => Self::Clang,
            | "cl" => Self::Msvc,
            | "clang-cl" => Self::ClangCl,
            | "tcc" => Self::Tcc,
            | _ => return None,
        };
        let target = target_named(prefix.trim_end_matches('-'));
        Some((personality, target))
    }

    /// What diagnostics call the driver.
    const fn name(self) -> &'static str {
        match self {
            | Self::Gcc => "gcc",
            | Self::Clang => "clang",
            | Self::Msvc => "cl",
            | Self::ClangCl => "clang-cl",
            | Self::Tcc => "tcc",
        }
    }
}

/// The bcc target a triple prefix names, in bcc's spelling or in the GCC
/// and Debian spellings of the same targets.
fn target_named(triple: &str) -> Option<Target> {
    Target::parse(triple).or(match triple {
        | "x86_64-linux-gnu" | "x86_64-pc-linux-gnu" => Some(Target::LinuxGnu),
        | "x86_64-linux-musl" | "x86_64-pc-linux-musl" => Some(Target::LinuxMusl),
        | _ => None,
    })
}

/// A command line a personality reads.
pub(super) struct Invocation {
    personality: Personality,
    /// `argv[0]`, which bcc's own options follow in each step.
    program:     OsString,
    /// What diagnostics call the driver: the name it was run as, or the
    /// personality's.
    name:        String,
    /// The target the executable's prefix names.
    target:      Option<Target>,
    /// The arguments after `argv[0]`.
    arguments:   Vec<OsString>,
}

impl Invocation {
    fn new(personality: Personality, mut arguments: Vec<OsString>) -> Self {
        let program = if arguments.is_empty() {
            OsString::from("bcc")
        } else {
            arguments.remove(0)
        };
        let from_program = Personality::from_program(Path::new(&program));
        let target = from_program
            .filter(|(named, _)| *named == personality)
            .and_then(|(_, target)| target);
        let name = match from_program {
            | Some((named, _)) if named == personality =>
                Path::new(&program).file_stem().map_or_else(
                    || personality.name().to_owned(),
                    |stem| stem.to_string_lossy().into_owned(),
                ),
            | _ => personality.name().to_owned(),
        };
        Self {
            personality,
            program,
            name,
            target,
            arguments,
        }
    }

    /// Prints `NAME: error: MESSAGE` to stderr, as GCC and Clang word driver
    /// errors.
    fn error(&self, message: std::fmt::Arguments<'_>) {
        print(
            &format_args!("{}: error: {message}\n", self.name).to_string(),
            true,
        );
    }

    /// Prints `NAME: warning: MESSAGE` to stderr.
    fn warning(&self, message: std::fmt::Arguments<'_>) {
        print(
            &format_args!("{}: warning: {message}\n", self.name).to_string(),
            true,
        );
    }
}

/// What a personality runs.
#[derive(Default)]
pub(super) struct Plan {
    steps:     Vec<Step>,
    /// A directory the steps write temporary files to, removed afterwards.
    temporary: Option<TemporaryDirectory>,
}

impl Plan {
    /// A plan that prints `text` to stdout.
    fn print(text: String) -> Self {
        Self {
            steps:     vec![Step::Print {
                text,
                stderr: false,
            }],
            temporary: None,
        }
    }
}

/// One step of a [`Plan`].
enum Step {
    /// Run bcc's driver on these arguments, `argv[0]` first.
    Bcc(Vec<OsString>),
    /// Print text.
    Print { text: String, stderr: bool },
    /// Run a program bcc built and exit with its status.
    Run {
        program:   PathBuf,
        arguments: Vec<OsString>,
    },
}

/// A directory removed with its contents when dropped.
struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    /// Creates a fresh directory under the system's temporary directory.
    fn create() -> std::io::Result<Self> {
        use std::sync::atomic::{
            AtomicU32,
            Ordering,
        };
        static NEXT: AtomicU32 = AtomicU32::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "bcc-run-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                | Ok(()) => return Ok(Self(path)),
                | Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                | Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// The target a personality compiles for by default: the host's, with GCC
/// and TCC preferring MinGW on Windows and Clang following the host's ABI.
fn host_target(personality: Personality) -> Target {
    if cfg!(windows) {
        match personality {
            | Personality::Msvc | Personality::ClangCl => Target::WindowsMsvc,
            | Personality::Clang if cfg!(target_env = "msvc") => Target::WindowsMsvc,
            | _ => Target::WindowsGnu,
        }
    } else if matches!(personality, Personality::Msvc | Personality::ClangCl) {
        Target::WindowsMsvc
    } else if cfg!(target_env = "musl") {
        Target::LinuxMusl
    } else {
        Target::LinuxGnu
    }
}

/// The C library a compiler for `target` uses on this host, when the host
/// runs that target or `sysroot` names one: the header directories
/// `/usr/local/include`, the multiarch directory and `/usr/include`, in
/// GCC's order on Debian-style systems, unless `nostdinc`. A `sysroot` is
/// passed on as bcc's `--sysroot`, even with `nostdinc`: bcc searches its
/// `usr/local/include` and `usr/include` (the multiarch directory follows
/// them) and links and assembles against it, so headers and libraries come
/// from one root. bcc's own mode searches none of them unless asked.
fn system_headers(
    target: Target,
    sysroot: Option<&OsStr>,
    nostdinc: bool,
    arguments: &mut Vec<OsString>,
) {
    if let Some(sysroot) = sysroot {
        let mut sysroot_option = OsString::from("--sysroot=");
        sysroot_option.push(sysroot);
        arguments.push(sysroot_option);
    }
    let linux = matches!(target, Target::LinuxGnu | Target::LinuxMusl);
    if nostdinc || !linux || (sysroot.is_none() && !cfg!(target_os = "linux")) {
        return;
    }
    let root = Path::new(sysroot.unwrap_or(OsStr::new("/")));
    let multiarch = match target {
        | Target::LinuxMusl => "x86_64-linux-musl",
        | _ => "x86_64-linux-gnu",
    };
    let include = root.join("usr").join("include");
    let directories = if sysroot.is_some() {
        vec![include.join(multiarch)]
    } else {
        vec![
            root.join("usr").join("local").join("include"),
            include.join(multiarch),
            include,
        ]
    };
    for directory in directories
        .into_iter()
        .filter(|directory| directory.is_dir())
    {
        arguments.push("--idirafter".into());
        arguments.push(directory.into_os_string());
    }
}

/// The exit status a program's `status` gives the shell.
fn exit_status(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    status.code().unwrap_or(1)
}

/// Writes `text` to stdout or stderr, ignoring a closed stream.
fn print(text: &str, stderr: bool) {
    use std::io::Write as _;
    if stderr {
        drop(std::io::stderr().lock().write_all(text.as_bytes()));
    } else {
        let mut stdout = std::io::stdout().lock();
        drop(
            stdout
                .write_all(text.as_bytes())
                .and_then(|()| stdout.flush()),
        );
    }
}

// Tests
#[cfg(test)]
mod tests;
