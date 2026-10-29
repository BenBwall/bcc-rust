//! Finds Visual Studio or its Build Tools with `vswhere`, takes the
//! environment `vcvars64.bat` sets, and reports the host's Microsoft
//! toolchain in the ignored `probe`.

use std::{
    ffi::OsString,
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Output,
    },
    sync::OnceLock,
};

use crate::native::run_native;

/// Reports what the host has: `vswhere` and the installations it lists, the
/// environment `vcvars64.bat` sets, `cl.exe` and `link.exe`, the Windows SDK
/// from the registry, `lld-link`, and whether a C program builds and runs
/// with `cl`, with `bcc --msvc-mode`, and with the bundled clang's link
/// alone. It only prints, so it passes on any host.
#[test]
#[ignore = "reports the Windows host's Microsoft toolchain; run with --ignored --nocapture"]
fn probe() {
    if !cfg!(windows) {
        println!("host: not Windows; nothing to probe");
        return;
    }
    section("process environment");
    for name in ENVIRONMENT {
        println!("{name}={}", variable(std::env::var_os(name)));
    }
    section("vswhere");
    let vswhere = vswhere();
    println!(
        "vswhere: {} (exists: {})",
        vswhere.display(),
        vswhere.is_file()
    );
    if vswhere.is_file() {
        show(Command::new(&vswhere).args([
            "-all",
            "-prerelease",
            "-products",
            "*",
            "-format",
            "text",
        ]));
    }
    let Some(msvc) = msvc() else {
        println!("no installation with the x64 C compiler and vcvars64.bat; probing the rest");
        probe_sdk();
        probe_bundled(None);
        return;
    };
    section("installation");
    println!("installation: {}", msvc.install.display());
    let tools = msvc.install.join(r"VC\Tools\MSVC");
    println!("{}: {:?}", tools.display(), entries(&tools));
    println!(
        "Microsoft.VCToolsVersion.default.txt: {:?}",
        fs::read_to_string(
            msvc.install
                .join(r"VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt")
        )
        .map(|text| text.trim().to_owned())
    );
    section("vcvars64.bat environment");
    for name in ENVIRONMENT {
        println!(
            "{name}={}",
            variable(msvc.variable(name).map(OsString::from))
        );
    }
    section("cl.exe and link.exe");
    show(
        msvc.command("where")
            .args(["cl", "link", "lld-link", "clang-cl"]),
    );
    show(&mut msvc.cl());
    show(&mut msvc.command("link"));
    probe_sdk();
    probe_bundled(Some(msvc));
}

/// The variables `vcvars64.bat` sets that the tools read.
const ENVIRONMENT: [&str; 14] = [
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "VSINSTALLDIR",
    "VCINSTALLDIR",
    "VCToolsInstallDir",
    "VCToolsVersion",
    "VisualStudioVersion",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "WindowsSDKLibVersion",
    "UniversalCRTSdkDir",
    "UCRTVersion",
    "PATH",
];

/// The Windows 10/11 SDK the registry names, its versions, and the import
/// libraries a C program links: the UCRT's and `kernel32.lib`.
fn probe_sdk() {
    section("Windows SDK (registry)");
    let mut roots = Vec::new();
    for key in [
        r"HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots",
    ] {
        let output = report(Command::new("reg").args(["query", key, "/v", "KitsRoot10"]));
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        roots.extend(text.lines().filter_map(|line| {
            let (_, value) = line.split_once("REG_SZ")?;
            Some(PathBuf::from(value.trim()))
        }));
    }
    roots.dedup();
    for root in roots {
        for kind in ["Include", "Lib"] {
            let directory = root.join(kind);
            println!("{}: {:?}", directory.display(), entries(&directory));
        }
        for version in entries(&root.join("Lib")) {
            let lib = root.join("Lib").join(&version);
            for library in [r"ucrt\x64\ucrt.lib", r"um\x64\kernel32.lib"] {
                println!("{version}\\{library}: {}", lib.join(library).is_file());
            }
        }
    }
}

/// The bundled LLVM's linkers, and a C program built every way the host
/// allows: by `cl`, by `bcc --msvc-mode` with and without the `vcvars64.bat`
/// environment, and by the bundled clang linking `cl`'s object without it.
fn probe_bundled(msvc: Option<&Msvc>) {
    section("bundled LLVM");
    for tool in ["clang", "clang-cl", "lld", "lld-link"] {
        let path = bundled(tool);
        println!("{}: {}", path.display(), path.is_file());
    }
    let lld_link = bundled("lld-link");
    if lld_link.is_file() {
        show(Command::new(lld_link).arg("--version"));
    }
    show(Command::new("where").args(["lld-link", "clang-cl", "cl"]));
    let root = scratch("probe");
    let source = root.join("hello.c");
    fs::write(&source, HELLO).unwrap();
    if let Some(msvc) = msvc {
        section("cl /nologo hello.c");
        show(msvc.cl().current_dir(&root).args(["/nologo", "hello.c"]));
        drop(run_program(&root.join("hello.exe")));
        section("cl /EP: predefined macros");
        fs::write(root.join("macros.c"), MACROS).unwrap();
        show(
            msvc.cl()
                .current_dir(&root)
                .args(["/nologo", "/EP", "macros.c"]),
        );
        section("bcc --msvc-mode /nologo /Febcc.exe hello.c (vcvars64.bat environment)");
        show(
            msvc.bcc()
                .current_dir(&root)
                .args(["/nologo", "/Febcc.exe", "hello.c"]),
        );
        drop(run_program(&root.join("bcc.exe")));
        section("bundled clang links cl's hello.obj without the environment");
        show(Command::new(bundled("clang")).current_dir(&root).args([
            "--target=x86_64-pc-windows-msvc",
            "-fuse-ld=lld",
            "-v",
            "hello.obj",
            "-o",
            "clang.exe",
        ]));
        drop(run_program(&root.join("clang.exe")));
    }
    section("bcc --msvc-mode /nologo hello.c (process environment)");
    show(
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .current_dir(&root)
            .args(["--msvc-mode", "/nologo", "/Feplain.exe", "hello.c"]),
    );
    drop(run_program(&root.join("plain.exe")));
    drop(fs::remove_dir_all(root));
}

const HELLO: &str =
    "#include <stdio.h>\nint main(void) { printf(\"hello %d\\n\", 42); return 3; }\n";

const MACROS: &str = r"_MSC_VER=_MSC_VER
_MSC_FULL_VER=_MSC_FULL_VER
_MSC_BUILD=_MSC_BUILD
_MSC_EXTENSIONS=_MSC_EXTENSIONS
_MSVC_TRADITIONAL=_MSVC_TRADITIONAL
_WIN32=_WIN32
_WIN64=_WIN64
_M_X64=_M_X64
_M_AMD64=_M_AMD64
_MT=_MT
_DLL=_DLL
__STDC__=__STDC__
__STDC_VERSION__=__STDC_VERSION__
__STDC_HOSTED__=__STDC_HOSTED__
";

/// A Visual Studio or Build Tools installation with the x64 C compiler, and
/// the environment its `vcvars64.bat` sets.
pub(super) struct Msvc {
    pub(super) install: PathBuf,
    environment:        Vec<(OsString, OsString)>,
}

/// The newest installation `vswhere` finds with the x64 C compiler, found
/// once per test run; `None` off Windows or without one.
pub(super) fn msvc() -> Option<&'static Msvc> {
    static MSVC: OnceLock<Option<Msvc>> = OnceLock::new();
    MSVC.get_or_init(|| {
        if !cfg!(windows) {
            return None;
        }
        let vswhere = vswhere();
        if !vswhere.is_file() {
            return None;
        }
        let output = run_native(Command::new(&vswhere).args([
            "-latest",
            "-prerelease",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ]))
        .ok()?;
        let install = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if install.as_os_str().is_empty() {
            return None;
        }
        let environment = vcvars(&install)?;
        Some(Msvc {
            install,
            environment,
        })
    })
    .as_ref()
}

impl Msvc {
    /// `program`, found and run in the `vcvars64.bat` environment.
    pub(super) fn command(&self, program: &str) -> Command {
        let path = self
            .variable("PATH")
            .and_then(|path| {
                std::env::split_paths(path)
                    .map(|directory| directory.join(format!("{program}.exe")))
                    .find(|candidate| candidate.is_file())
            })
            .unwrap_or_else(|| PathBuf::from(program));
        let mut command = Command::new(path);
        _ = command
            .env_clear()
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .env("NO_COLOR", "1");
        command
    }

    /// The real `cl.exe`.
    pub(super) fn cl(&self) -> Command {
        self.command("cl")
    }

    /// `bcc --msvc-mode` in the same environment.
    pub(super) fn bcc(&self) -> Command {
        self.bcc_in(&["--msvc-mode"])
    }

    /// `bcc --clang-cl-mode --backend=native`: the cl personalities with
    /// bcc's native back end, which only clang-cl's pass-through of `-`
    /// options can select.
    pub(super) fn bcc_native(&self) -> Command {
        self.bcc_in(&["--clang-cl-mode", "--backend=native"])
    }

    fn bcc_in(&self, mode: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command
            .env_clear()
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .env("NO_COLOR", "1")
            .args(mode);
        command
    }

    /// `compiler`'s command in this environment.
    pub(super) fn compiler(&self, compiler: Compiler) -> Command {
        match compiler {
            | Compiler::Cl => self.cl(),
            | Compiler::Bcc => self.bcc(),
            | Compiler::BccNative => self.bcc_native(),
        }
    }

    /// A variable of the `vcvars64.bat` environment, matched as Windows
    /// matches names, without case.
    pub(super) fn variable(&self, name: &str) -> Option<&std::ffi::OsStr> {
        self.environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_os_str())
    }
}

/// A compiler that builds the programs these tests compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Compiler {
    /// The real `cl.exe`, the reference.
    Cl,
    /// `bcc --msvc-mode`, with the LLVM back end.
    Bcc,
    /// bcc's native back end, through the clang-cl personality.
    BccNative,
}

impl Compiler {
    pub(super) const ALL: [Self; 3] = [Self::Cl, Self::Bcc, Self::BccNative];

    /// A short name for directories and reports.
    pub(super) const fn label(self) -> &'static str {
        match self {
            | Self::Cl => "cl",
            | Self::Bcc => "bcc",
            | Self::BccNative => "bcc-native",
        }
    }
}

/// What a built program did: its exit status and standard output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Outcome {
    pub(super) status: Option<i32>,
    pub(super) stdout: String,
}

/// Runs `command` (a compiler or linker) in `directory`, printing it and
/// its output; `Err` holds the output of a failing one.
pub(super) fn build(command: &mut Command, directory: &Path) -> Result<(), String> {
    let output = report(command.current_dir(directory));
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{command:?} failed with {}:\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Runs the built `program` and returns what it did, `Err` if it did not
/// start.
pub(super) fn outcome(program: &Path) -> Result<Outcome, String> {
    let output = run_program(program).ok_or_else(|| format!("no {}", program.display()))?;
    Ok(Outcome {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
    })
}

/// Collects the differences a comparison finds, so one run reports all of
/// them before failing.
#[derive(Default)]
pub(super) struct Differences(Vec<String>);

impl Differences {
    /// Records `label`'s difference when `actual` is not `expected`.
    pub(super) fn compare<T: PartialEq + std::fmt::Debug>(
        &mut self,
        label: &str,
        expected: &T,
        actual: &T,
    ) {
        if expected != actual {
            self.0
                .push(format!("{label}:\n  cl:  {expected:?}\n  bcc: {actual:?}"));
        }
    }

    pub(super) fn push(&mut self, difference: String) {
        self.0.push(difference);
    }

    /// Prints every difference and fails if there is one.
    pub(super) fn assert_none(self, subject: &str) {
        for difference in &self.0 {
            println!("DIFFERENCE {difference}");
        }
        assert!(
            self.0.is_empty(),
            "{} {subject} differences:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}

/// The environment after `vcvars64.bat`: `cmd /c` runs it and `set` prints
/// every variable.
#[cfg(windows)]
fn vcvars(install: &Path) -> Option<Vec<(OsString, OsString)>> {
    use std::os::windows::process::CommandExt as _;
    let script = install.join(r"VC\Auxiliary\Build\vcvars64.bat");
    if !script.is_file() {
        return None;
    }
    let mut command = Command::new("cmd.exe");
    // `/s` strips the outer quotes, leaving the quoted script path.
    _ = command.raw_arg(format!("/d /s /c \"\"{}\" >nul && set\"", script.display()));
    let output = run_native(&mut command).ok()?;
    if !output.status.success() {
        return None;
    }
    let environment: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            (!name.is_empty()).then(|| (name.into(), value.into()))
        })
        .collect();
    environment
        .iter()
        .any(|(name, _): &(OsString, OsString)| name.eq_ignore_ascii_case("INCLUDE"))
        .then_some(environment)
}

#[cfg(not(windows))]
fn vcvars(_: &Path) -> Option<Vec<(OsString, OsString)>> {
    None
}

/// `vswhere.exe`, where the Visual Studio installer always puts it.
fn vswhere() -> PathBuf {
    let programs = std::env::var_os("ProgramFiles(x86)")
        .unwrap_or_else(|| OsString::from(r"C:\Program Files (x86)"));
    Path::new(&programs).join(r"Microsoft Visual Studio\Installer\vswhere.exe")
}

/// A tool of the bundled LLVM installation.
pub(super) fn bundled(tool: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/llvm/bin")
        .join(format!("{tool}.exe"))
}

/// A fresh directory for one test's files.
pub(super) fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("bcc-msvc-{}-{label}", std::process::id()));
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    root
}

/// Runs a built program through the crash-dialog wrapper, as AGENTS.md
/// requires on Windows.
pub(super) fn run_program(program: &Path) -> Option<Output> {
    if !program.is_file() {
        println!("(no {})", program.display());
        return None;
    }
    let mut command = Command::new("python");
    _ = command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/cargo_runner.py"))
        .arg(program);
    Some(report(&mut command))
}

/// Runs `command` and prints its status and output.
pub(super) fn show(command: &mut Command) {
    drop(report(command));
}

/// [`show`], returning the output.
pub(super) fn report(command: &mut Command) -> Output {
    println!("$ {command:?}");
    match run_native(command) {
        | Ok(output) => {
            println!("status: {}", output.status);
            for (name, stream) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
                let text = String::from_utf8_lossy(stream);
                if !text.trim().is_empty() {
                    println!("{name}:\n{}", text.trim_end());
                }
            }
            output
        },
        | Err(error) => {
            println!("error: {error}");
            Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        },
    }
}

fn section(title: &str) {
    println!("\n==== {title}");
}

fn variable(value: Option<OsString>) -> String {
    value.map_or_else(
        || "(unset)".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    )
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
