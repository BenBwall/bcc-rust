//! Emits objects in-process through LLVM-C, then invokes bundled clang/LLD
//! only to link. MinGW discovery matches the native build helper.

use std::{
    borrow::Cow,
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
        AtomicBool,
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
    compile_module_checked(module, out_path, options, 0, |_| true).map(|_| ())
}

/// Checks surviving annotated calls after LLVM's actual IR pass pipeline and
/// before object emission. Rejected diagnostics leave the output untouched.
pub(crate) fn compile_module_checked(
    module: &crate::ir::Module<'_>,
    out_path: &Path,
    options: LlvmOptions,
    diagnostic_count: usize,
    check: impl FnOnce(&[usize]) -> bool,
) -> io::Result<bool> {
    let arena = crate::util::bump::Bump::new();
    let buffer = super::api::Session::with_code(&arena, options.target, options.code, |api| {
        super::build_module(module, options.target, api, &arena)
            .map_err(|error| io::Error::other(error.to_string()))?;
        api.emit_checked(
            options.target,
            options.opt,
            options.disable_llvm_passes,
            false,
            |api| check(api.call_diagnostic_counts(diagnostic_count, &arena)),
        )
        .map_err(|error| io::Error::other(error.to_string()))
    })?;
    let Some(buffer) = buffer else {
        return Ok(false);
    };
    if options.kind == OutputKind::Object {
        return std::fs::write(out_path, buffer.bytes()).map(|()| true);
    }
    let (object, mut file) = TemporaryObject::create(out_path, "o", &arena)?;
    file.write_all(buffer.bytes())?;
    drop(file);
    let toolchain = Toolchain::bundled(options.target)?;
    if !needs_atomic_runtime(module) {
        return toolchain
            .link_object(&object.0, out_path, options)
            .map(|()| true);
    }
    let mut command = toolchain.command(&object.0, out_path, options);
    _ = command.arg("-latomic");
    let output = run(&mut command)?;
    if output.status.success() {
        Ok(true)
    } else {
        Err(io::Error::other(ClangError(output)))
    }
}

/// Whether the toolchain prints each command it runs to stderr first, as
/// Clang's `-v` does.
static PRINT_COMMANDS: AtomicBool = AtomicBool::new(false);

/// Makes the toolchain print each command it runs before running it, for
/// the driver's `-v`.
pub(crate) fn print_commands(enabled: bool) {
    PRINT_COMMANDS.store(enabled, Ordering::Relaxed);
}

/// Runs `command` to completion, first printing it when
/// [`print_commands`] asked for that.
fn run(command: &mut Command) -> io::Result<Output> {
    if PRINT_COMMANDS.load(Ordering::Relaxed) {
        drop(write_command(
            &mut io::stderr().lock(),
            command.get_program(),
            command.get_args(),
        ));
    }
    command.output()
}

/// Writes a command as Clang's `-v` and `-###` print one: a space before
/// each word, each word quoted, and `"`, `\` and `$` within it escaped
/// with a backslash; then a newline.
pub(crate) fn write_command<'a>(
    out: &mut impl io::Write,
    program: &'a OsStr,
    arguments: impl IntoIterator<Item = &'a OsStr>,
) -> io::Result<()> {
    for word in std::iter::once(program).chain(arguments) {
        out.write_all(b" \"")?;
        for byte in word.as_encoded_bytes() {
            if matches!(byte, b'"' | b'\\' | b'$') {
                out.write_all(b"\\")?;
            }
            out.write_all(std::slice::from_ref(byte))?;
        }
        out.write_all(b"\"")?;
    }
    out.write_all(b"\n")
}

/// Whether LLVM lowers some atomic operation of `module` to a libatomic
/// call: a generic byte-wise operation, a 128-bit one, or a misaligned one.
/// An executable that contains it links `-latomic`.
pub(crate) fn needs_atomic_runtime(module: &crate::ir::Module<'_>) -> bool {
    module.functions().any(|(_, f)| {
        f.body.as_ref().is_some_and(|body| {
            body.blocks().any(|block| {
                body.block_insts(block).iter().any(|&inst| {
                    matches!(body.inst(inst), crate::ir::InstData::Atomic { op, ty, align, .. }
                        if op.is_bytes() || *ty == crate::ir::Type::I128
                            || !op.is_fence() && align.bytes() < u64::from(ty.bits() / 8))
                })
            })
        })
    })
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
            let mut name = operand(out_path).into_owned().into_os_string();
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
    Os,
    Oz,
}

impl OptLevel {
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            | Self::O0 => "-O0",
            | Self::O1 => "-O1",
            | Self::O2 => "-O2",
            | Self::O3 => "-O3",
            | Self::Os => "-Os",
            | Self::Oz => "-Oz",
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

    /// This toolchain with `sysroot`, when given, as the system root it
    /// links and assembles against instead of its own, as `--sysroot`
    /// selects one for the headers.
    #[must_use]
    pub(crate) fn with_sysroot(mut self, sysroot: Option<&Path>) -> Self {
        if let Some(sysroot) = sysroot {
            self.sysroot = Some(sysroot.to_path_buf());
        }
        self
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
        let output = run(&mut self.command(object, out_path, options))?;
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
        self.assemble_linking(assembly, out_path, target, kind, &[])
    }

    /// [`Self::assemble`], linking an executable with the extra `libraries`
    /// (`-latomic`) as well.
    pub(crate) fn assemble_linking(
        &self,
        assembly: &Path,
        out_path: &Path,
        target: Target,
        kind: OutputKind,
        libraries: &[&str],
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
        _ = command
            .arg(operand(assembly).as_os_str())
            .arg("-o")
            .arg(out_path);
        if kind != OutputKind::Object && matches!(target, Target::LinuxGnu | Target::LinuxMusl) {
            _ = command.arg("-lm");
        }
        if kind != OutputKind::Object {
            _ = command.args(libraries);
        }
        let output = run(&mut command)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// Links `inputs` (objects, archives, libraries and linker options such
    /// as `-lm` or `-Wl,...`, in command-line order) into `out_path`, the
    /// way the driver links several translation units. `atomic_runtime`
    /// adds `-latomic` after them, as one object's link adds it.
    pub(crate) fn link_inputs<'a>(
        &self,
        inputs: impl IntoIterator<Item = &'a OsStr>,
        out_path: &Path,
        options: LlvmOptions,
        atomic_runtime: bool,
    ) -> io::Result<()> {
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
            .args(inputs)
            .arg("-o")
            .arg(out_path);
        // C99 §7.12 real math is provided by a separate library on Linux.
        if matches!(options.target, Target::LinuxGnu | Target::LinuxMusl) {
            _ = command.arg("-lm");
        }
        if atomic_runtime {
            _ = command.arg("-latomic");
        }
        let output = run(&mut command)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// Assembles or compiles one input the bundled clang understands, such as
    /// an assembly file, into the object `out_path`. `options` come before
    /// the input: its `-x` language and preprocessor options.
    pub(crate) fn compile_foreign<'a>(
        &self,
        input: &Path,
        out_path: &Path,
        target: Target,
        options: impl IntoIterator<Item = &'a OsStr>,
    ) -> io::Result<()> {
        let mut command = Command::new(&self.clang);
        _ = command.arg(concat_os("--target=", target.triple().as_ref()));
        if let Some(sysroot) = &self.sysroot {
            _ = command.arg(concat_os("--sysroot=", sysroot.as_os_str()));
        }
        _ = command
            .args(options)
            .arg("-c")
            .arg(operand(input).as_os_str())
            .arg("-o")
            .arg(out_path);
        let output = run(&mut command)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(ClangError(output)))
        }
    }

    /// The external link command. No IR parsing or compilation takes place.
    pub(crate) fn command(&self, object: &Path, out_path: &Path, options: LlvmOptions) -> Command {
        self.link_command(object, out_path, options, true)
    }

    /// A link whose caller supplies startup and runtime support. Clang's
    /// `-nostdlib` suppresses its default libraries, but not an explicit
    /// `-lm`, so this path also omits our automatic hosted libraries.
    #[cfg(test)]
    pub(crate) fn freestanding_command(
        &self,
        object: &Path,
        out_path: &Path,
        options: LlvmOptions,
    ) -> Command {
        self.link_command(object, out_path, options, false)
    }

    fn link_command(
        &self,
        object: &Path,
        out_path: &Path,
        options: LlvmOptions,
        hosted: bool,
    ) -> Command {
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
            .arg(operand(object).as_os_str())
            .arg("-o")
            .arg(out_path);
        // C99 §7.12 real math is provided by a separate library on Linux.
        // The CLI links the hosted C runtime, including its math functions.
        if options.kind != OutputKind::Object {
            if !hosted {
                _ = command.arg("-nostdlib");
            } else if matches!(options.target, Target::LinuxGnu | Target::LinuxMusl) {
                _ = command.arg("-lm");
            }
        }
        command
    }
}

/// `path` as a file operand of a command line: a relative path that starts
/// with `-`, such as an object named after `-o -app`, gets `./` before it,
/// so clang does not read it as an option.
pub(crate) fn operand(path: &Path) -> Cow<'_, Path> {
    if path.is_relative() && path.as_os_str().to_string_lossy().starts_with('-') {
        Cow::Owned(Path::new(".").join(path))
    } else {
        Cow::Borrowed(path)
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
