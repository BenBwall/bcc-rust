//! Runs the bundled clang on LLVM IR text: compiles it to an object, or
//! compiles and links an executable.
//!
//! The command is the one `docs/research/llvm-backend-integration.md`
//! verified on the pinned clang:
//!
//! ```text
//! clang --target=<triple> -O2 [-Xclang -disable-llvm-passes] -c -x ir x.ll -o x.o
//! clang --target=<triple> [--sysroot=<MinGW>] -fuse-ld=lld -O2 -x ir x.ll -o x.exe
//! ```
//!
//! Linking uses the bundled `lld`, and for Windows GNU the MinGW installation
//! the build already requires, found the same way: `MINGW_ROOT`, else the
//! directory above a `gcc.exe` on `PATH` that has the Windows headers.
//!
//! This is tooling around the compiler, not part of the arena-allocated
//! compile path: it reads the environment and builds paths once per clang
//! run.

use std::{
    ffi::OsStr,
    fmt,
    io,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Output,
    },
};

use crate::target::Target;

/// Compiles the LLVM IR file `ll_path` to `out_path` with the bundled clang,
/// as `options` say. A clang that fails returns a [`ClangError`] with its
/// diagnostics; one that succeeds may still have printed warnings.
pub(crate) fn compile_ll(ll_path: &Path, out_path: &Path, options: LlvmOptions) -> io::Result<()> {
    Toolchain::bundled(options.target)?.compile_ll(ll_path, out_path, options)
}

/// How to run clang on one `.ll` file. The comparison arms of
/// `middle-end.md` are settings of `opt` and `disable_llvm_passes`; the
/// module map of `llvm.rs` lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LlvmOptions {
    /// The `-O` level, or none for clang's default (`-O0`).
    pub(crate) opt:                 Option<OptLevel>,
    /// Runs no LLVM IR pass at all, not even the always-inliner, while the
    /// code generator keeps the `-O` level (`-Xclang -disable-llvm-passes`).
    pub(crate) disable_llvm_passes: bool,
    pub(crate) target:              Target,
    pub(crate) kind:                OutputKind,
}

/// A clang optimization level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OptLevel {
    O0,
    O1,
    O2,
    O3,
}

impl OptLevel {
    const fn flag(self) -> &'static str {
        match self {
            | Self::O0 => "-O0",
            | Self::O1 => "-O1",
            | Self::O2 => "-O2",
            | Self::O3 => "-O3",
        }
    }
}

/// What clang produces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OutputKind {
    /// An object file (`-c`).
    Object,
    /// A linked executable.
    Executable,
}

/// Where clang and the target's system root are.
#[derive(Clone, Debug)]
pub(crate) struct Toolchain {
    clang:   PathBuf,
    /// The MinGW installation that links Windows GNU executables.
    sysroot: Option<PathBuf>,
}

/// The pinned clang of the checkout this compiler was built from. The build
/// installs it under `target/llvm`, so the path is fixed at compile time;
/// a compiler copied away from its checkout must be given a [`Toolchain`].
#[cfg(windows)]
const BUNDLED_CLANG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/llvm/bin/clang.exe");
#[cfg(not(windows))]
const BUNDLED_CLANG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/llvm/bin/clang");

impl Toolchain {
    /// The checkout's bundled clang, with the MinGW system root when
    /// `target` is Windows GNU. Fails with [`io::ErrorKind::NotFound`] if
    /// either is missing.
    pub(crate) fn bundled(target: Target) -> io::Result<Self> {
        let clang = Path::new(BUNDLED_CLANG);
        if !clang.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                NotFound(
                    "the bundled clang; build the repository first",
                    BUNDLED_CLANG,
                ),
            ));
        }
        let sysroot = match target {
            | Target::WindowsGnu => Some(mingw_root().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    NotFound(
                        "MinGW; set MINGW_ROOT to an installation containing include/windows.h",
                        "",
                    ),
                )
            })?),
            | _ => None,
        };
        Ok(Self {
            clang: PathBuf::from(clang),
            sysroot,
        })
    }

    /// The clang executable.
    pub(crate) fn clang(&self) -> &Path {
        &self.clang
    }

    /// Compiles `ll_path` to `out_path` as [`compile_ll`] does.
    pub(crate) fn compile_ll(
        &self,
        ll_path: &Path,
        out_path: &Path,
        options: LlvmOptions,
    ) -> io::Result<()> {
        let output = self.command(ll_path, out_path, options).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// The clang command line for `options`.
    pub(crate) fn command(&self, ll_path: &Path, out_path: &Path, options: LlvmOptions) -> Command {
        let mut command = Command::new(&self.clang);
        _ = command
            .arg(concat_os("--target=", options.target.triple().as_ref()))
            .args(options.opt.map(OptLevel::flag));
        if options.disable_llvm_passes {
            _ = command.args(["-Xclang", "-disable-llvm-passes"]);
        }
        match options.kind {
            | OutputKind::Object => _ = command.arg("-c"),
            | OutputKind::Executable => {
                // Linking is the only step that reads the system root, so
                // an object build leaves both flags out rather than have
                // clang warn that they are unused.
                if let Some(sysroot) = &self.sysroot {
                    _ = command.arg(concat_os("--sysroot=", sysroot.as_os_str()));
                }
                _ = command.arg("-fuse-ld=lld");
            },
        }
        _ = command
            .args(["-x", "ir"])
            .arg(ll_path)
            .arg("-o")
            .arg(out_path);
        command
    }
}

/// `prefix` followed by `value`, as one command-line argument.
fn concat_os(prefix: &str, value: &OsStr) -> std::ffi::OsString {
    let mut argument = std::ffi::OsString::from(prefix);
    argument.push(value);
    argument
}

/// The MinGW installation: `MINGW_ROOT`, else the parent of a `PATH`
/// directory holding `gcc.exe` whose installation has `windows.h`. This is
/// the search `build_support/native.rs` makes for the C helper.
fn mingw_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("MINGW_ROOT") {
        return Some(PathBuf::from(root));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        let root = directory.parent()?;
        let has_headers = root.join("include/windows.h").is_file()
            || root.join("x86_64-w64-mingw32/include/windows.h").is_file();
        (directory.join("gcc.exe").is_file() && has_headers).then(|| root.to_path_buf())
    })
}

/// A clang run that failed, with what it printed.
#[derive(Debug)]
pub(crate) struct ClangError(pub(crate) Output);

impl fmt::Display for ClangError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "clang failed ({}):\n{}",
            self.0.status,
            String::from_utf8_lossy(&self.0.stderr)
        )
    }
}

impl std::error::Error for ClangError {}

/// A missing tool: what it is, and where it was looked for.
#[derive(Debug)]
struct NotFound(&'static str, &'static str);

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot find {}", self.0)?;
        if !self.1.is_empty() {
            write!(f, " at {}", self.1)?;
        }
        Ok(())
    }
}

impl std::error::Error for NotFound {}
