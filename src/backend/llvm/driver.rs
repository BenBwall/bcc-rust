//! Emits objects in-process through LLVM-C, then invokes bundled clang/LLD
//! only to link. MinGW discovery matches the native build helper.

use std::{
    ffi::OsStr,
    fmt,
    io::{
        self,
        Write as _,
    },
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Output,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use crate::target::{
    CodeOptions,
    Target,
};

/// Builds a verified post-ABI module with LLVM-C and links only when requested.
pub(crate) fn compile_module(
    module: &crate::ir::Module<'_>,
    out_path: &Path,
    options: LlvmOptions,
) -> io::Result<()> {
    let arena = crate::util::bump::Bump::new();
    let buffer = super::api::Session::with_code(&arena, options.target, options.code, |api| {
        super::build_module(module, options.target, api, &arena)
            .map_err(|error| io::Error::other(error.to_string()))?;
        api.emit(
            options.target,
            options.opt,
            options.disable_llvm_passes,
            false,
        )
        .map_err(|error| io::Error::other(error.to_string()))
    })?;
    if options.kind == OutputKind::Object {
        return std::fs::write(out_path, buffer.bytes());
    }
    let (object, mut file) = TemporaryObject::create(out_path, "o", &arena)?;
    file.write_all(buffer.bytes())?;
    drop(file);
    let toolchain = Toolchain::bundled(options.target)?;
    let atomic_runtime = module.functions().any(|(_, f)| {
        f.body.as_ref().is_some_and(|body| {
            body.blocks().any(|block| {
                body.block_insts(block).iter().any(|&inst| {
                    matches!(body.inst(inst), crate::ir::InstData::Atomic { op, ty, align, .. }
                        if op.is_bytes() || *ty == crate::ir::Type::I128
                            || !op.is_fence() && align.bytes() < u64::from(ty.bits() / 8))
                })
            })
        })
    });
    if !atomic_runtime {
        return toolchain.link_object(&object.0, out_path, options);
    }
    let mut command = toolchain.command(&object.0, out_path, options);
    _ = command.arg("-latomic");
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(ClangError(output)))
    }
}

/// A temporary file beside the output (an object, or the native back
/// end's assembly), deleted on success and on failure.
pub(crate) struct TemporaryObject(PathBuf);

impl TemporaryObject {
    /// Exclusively creates a file with `extension` beside the output. Only
    /// a file this invocation created acquires a cleanup guard; collisions
    /// are retried.
    pub(crate) fn create(
        out_path: &Path,
        extension: &str,
        arena: &crate::util::bump::Bump,
    ) -> io::Result<(Self, std::fs::File)> {
        use std::fmt::Write as _;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut suffix = crate::util::bump::ArenaString::new_in(arena);
        loop {
            suffix.clear();
            write!(
                suffix,
                ".bcc-{}-{}.{extension}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )
            .map_err(io::Error::other)?;
            let mut name = out_path.as_os_str().to_os_string();
            name.push(suffix.as_str());
            let path = PathBuf::from(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                | Ok(file) => return Ok((Self(path), file)),
                | Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                | Err(error) => return Err(error),
            }
        }
    }
}

impl TemporaryObject {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryObject {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.0));
    }
}

/// LLVM IR passes and target-machine code generation options. The comparison
/// arms of `middle-end.md` are settings of `opt` and `disable_llvm_passes`; the
/// module map of `llvm.rs` lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LlvmOptions {
    /// IR pipeline and machine level; none is equivalent to O0.
    pub(crate) opt:                 Option<OptLevel>,
    /// Runs no LLVM IR pass at all, not even the always-inliner, while the
    /// code generator keeps the selected optimization level.
    pub(crate) disable_llvm_passes: bool,
    pub(crate) target:              Target,
    pub(crate) kind:                OutputKind,
    /// The relocation and thread-local storage models.
    pub(crate) code:                CodeOptions,
}

/// An LLVM optimization level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OptLevel {
    O0,
    O1,
    O2,
    O3,
}

impl OptLevel {
    pub(crate) const fn flag(self) -> &'static str {
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
    /// A linked executable, position-independent on Linux as clang links
    /// it by default.
    Executable,
    /// An executable linked at a fixed address (`-no-pie`), which code
    /// compiled with `-fno-pic` needs.
    PositionDependentExecutable,
    /// A shared library (`-shared`), which code compiled with `-fpic` or
    /// `-fPIC` can go into.
    SharedLibrary,
}

impl OutputKind {
    /// The clang driver flag that links this kind, if it needs one.
    pub(crate) const fn link_flag(self) -> Option<&'static str> {
        match self {
            | Self::Object => Some("-c"),
            | Self::Executable => None,
            | Self::PositionDependentExecutable => Some("-no-pie"),
            | Self::SharedLibrary => Some("-shared"),
        }
    }
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
    #[cfg(test)]
    pub(crate) fn clang(&self) -> &Path {
        &self.clang
    }

    /// Links an object with the bundled toolchain.
    pub(crate) fn link_object(
        &self,
        object: &Path,
        out_path: &Path,
        options: LlvmOptions,
    ) -> io::Result<()> {
        let output = self.command(object, out_path, options).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// Assembles an assembly file into an object (`OutputKind::Object`) or
    /// an executable linked with LLD, for the native back end.
    pub(crate) fn assemble(
        &self,
        assembly: &Path,
        out_path: &Path,
        target: Target,
        kind: OutputKind,
    ) -> io::Result<()> {
        let mut command = Command::new(&self.clang);
        _ = command.arg(concat_os("--target=", target.triple().as_ref()));
        if let Some(sysroot) = &self.sysroot {
            _ = command.arg(concat_os("--sysroot=", sysroot.as_os_str()));
        }
        if kind != OutputKind::Object {
            _ = command.arg("-fuse-ld=lld");
        }
        _ = command.args(kind.link_flag());
        _ = command.arg(assembly).arg("-o").arg(out_path);
        if kind != OutputKind::Object && matches!(target, Target::LinuxGnu | Target::LinuxMusl)
        {
            _ = command.arg("-lm");
        }
        let output = command.output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// The external link command. No IR parsing or compilation takes place.
    pub(crate) fn command(&self, object: &Path, out_path: &Path, options: LlvmOptions) -> Command {
        let mut command = Command::new(&self.clang);
        _ = command
            .arg(concat_os("--target=", options.target.triple().as_ref()))
            .args(options.opt.map(OptLevel::flag));
        if let Some(sysroot) = &self.sysroot {
            _ = command.arg(concat_os("--sysroot=", sysroot.as_os_str()));
        }
        _ = command
            .arg("-fuse-ld=lld")
            .args(options.kind.link_flag())
            .arg(object)
            .arg("-o")
            .arg(out_path);
        // C99 §7.12 real math is provided by a separate library on Linux.
        // The CLI links the hosted C runtime, including its math functions.
        if matches!(options.target, Target::LinuxGnu | Target::LinuxMusl) {
            _ = command.arg("-lm");
        }
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
