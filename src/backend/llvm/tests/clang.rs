//! Tests that run the bundled clang: it accepts every golden module, its
//! data layouts match ours, and small programs compile, link and run under
//! both the `-O2` and the `-O2 -Xclang -disable-llvm-passes` arm.
//!
//! Each clang run costs about 0.4 s, so each test runs its clang processes
//! side by side, and the test harness runs the tests side by side. Without
//! the bundled clang the tests print why and pass.

use std::{
    path::Path,
    process::Command,
    thread,
};

use super::{
    golden::GOLDENS,
    *,
};
use crate::test_support::TempDir;

/// The bundled toolchain for `target`, or `None` after explaining why the
/// test cannot run.
fn toolchain(target: Target) -> Option<Toolchain> {
    match Toolchain::bundled(target) {
        | Ok(toolchain) => Some(toolchain),
        | Err(error) => {
            eprintln!("skipping: {error}");
            None
        },
    }
}

#[test]
fn clang_accepts_every_golden_module() {
    let target = Target::LinuxGnu;
    if toolchain(target).is_none() {
        return;
    }
    let directory = TempDir::new("llvm-golden");
    let options = LlvmOptions {
        opt: Some(OptLevel::O0),
        disable_llvm_passes: false,
        target,
        kind: OutputKind::Object,
    };
    thread::scope(|scope| {
        let runs: Vec<_> = GOLDENS
            .iter()
            .map(|golden| {
                let ll = directory.join(format!("{}.ll", golden.name));
                std::fs::write(&ll, golden.emit()).unwrap();
                let object = directory.join(format!("{}.o", golden.name));
                scope.spawn(move || (golden.name, compile_ll(&ll, &object, options)))
            })
            .collect();
        for run in runs {
            let (name, result) = run.join().unwrap();
            if let Err(error) = result {
                panic!("{name}: {error}");
            }
        }
    });
}

#[test]
fn command_lines_follow_the_options() {
    let Some(toolchain) = toolchain(Target::LinuxGnu) else {
        return;
    };
    let arguments = |opt, disable_llvm_passes, kind| {
        let options = LlvmOptions {
            opt,
            disable_llvm_passes,
            target: Target::LinuxGnu,
            kind,
        };
        let command = toolchain.command(Path::new("in.ll"), Path::new("out"), options);
        command
            .get_args()
            .map(|argument| argument.to_str().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let target = "--target=x86_64-unknown-linux-gnu";
    assert_eq!(
        arguments(None, false, OutputKind::Object),
        format!("{target} -c -x ir in.ll -o out")
    );
    for (level, flag) in [
        (OptLevel::O0, "-O0"),
        (OptLevel::O1, "-O1"),
        (OptLevel::O2, "-O2"),
        (OptLevel::O3, "-O3"),
    ] {
        assert_eq!(
            arguments(Some(level), true, OutputKind::Executable),
            format!("{target} {flag} -Xclang -disable-llvm-passes -fuse-ld=lld -x ir in.ll -o out")
        );
    }
}

#[test]
fn data_layouts_match_clang() {
    let targets = [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ];
    let Some(toolchain) = toolchain(Target::LinuxGnu) else {
        return;
    };
    let directory = TempDir::new("llvm-layout");
    directory.write("empty.c", "");
    let source = directory.join("empty.c");
    let outputs: Vec<_> = targets
        .map(|target| {
            Command::new(toolchain.clang())
                .arg(format!("--target={}", target.triple()))
                .args(["-S", "-emit-llvm", "-o", "-"])
                .arg(&source)
                .spawn_output()
        })
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    for (target, output) in targets.into_iter().zip(outputs) {
        assert!(output.status.success(), "{}", ClangError(output));
        let text = String::from_utf8(output.stdout).unwrap();
        let line = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap_or_else(|| panic!("no {key} in\n{text}"))
                .to_string()
        };
        assert_eq!(
            line("target datalayout = "),
            format!("\"{}\"", data_layout(target)),
            "{target:?}"
        );
        // Clang appends the installed MSVC version to the MSVC triple.
        assert!(
            line("target triple = ").starts_with(&format!("\"{}", target.triple())),
            "{target:?}: {text}"
        );
    }
}

/// Starts a command with its output captured.
trait SpawnOutput {
    fn spawn_output(&mut self) -> std::process::Child;
}

impl SpawnOutput for Command {
    fn spawn_output(&mut self) -> std::process::Child {
        self.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
}

/// The target the tests run on, if the back end supports it.
fn host() -> Option<Target> {
    if cfg!(all(windows, target_env = "gnu")) {
        Some(Target::WindowsGnu)
    } else if cfg!(all(windows, target_env = "msvc")) {
        Some(Target::WindowsMsvc)
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        Some(Target::LinuxGnu)
    } else if cfg!(all(target_os = "linux", target_env = "musl")) {
        Some(Target::LinuxMusl)
    } else {
        None
    }
}

/// Compiles `program` for the host under the A2 (`-O2`) and A1 (`-O2
/// -Xclang -disable-llvm-passes`) arms, runs both executables, and checks
/// the exit status and the standard output (with Windows line ends made
/// plain).
fn assert_runs(name: &str, program: &str, status: i32, stdout: &str) {
    let Some(target) = host() else {
        eprintln!("skipping: the host is not a supported target");
        return;
    };
    let Some(toolchain) = toolchain(target) else {
        return;
    };
    let directory = TempDir::new("llvm-run");
    let ll = directory.join(format!("{name}.ll"));
    std::fs::write(&ll, emit(program, Profile::PostAbi, target)).unwrap();
    thread::scope(|scope| {
        let runs: Vec<_> = [false, true]
            .into_iter()
            .map(|disable_llvm_passes| {
                let options = LlvmOptions {
                    opt: Some(OptLevel::O2),
                    disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                };
                let suffix = if disable_llvm_passes { "a1" } else { "a2" };
                let executable = directory.join(format!("{name}-{suffix}.exe"));
                let (toolchain, ll) = (&toolchain, &ll);
                scope.spawn(move || {
                    toolchain
                        .compile_ll(ll, &executable, options)
                        .unwrap_or_else(|error| panic!("{name} {suffix}: {error}"));
                    (suffix, run(&executable))
                })
            })
            .collect();
        for run in runs {
            let (suffix, (actual_status, actual_stdout)) = run.join().unwrap();
            assert_eq!(
                (actual_status, actual_stdout.as_str()),
                (status, stdout),
                "{name} {suffix}"
            );
        }
    });
}

/// Runs an executable, returning its exit status and standard output.
fn run(executable: &Path) -> (i32, String) {
    let output = Command::new(executable).output().unwrap();
    let stdout = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    (output.status.code().unwrap(), stdout)
}

#[test]
fn sum_loop_runs() {
    assert_runs(
        "sum",
        "\
function @sum(i32) -> i32 internal {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    jump block1(v2, v1)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 sle v3, v0
    brif v5, block2, block3
block2:
    v6 = iadd.i32 nsw v4, v3
    v7 = iadd.i32 nsw v3, v2
    jump block1(v7, v6)
block3:
    return v4
}

function @main() -> i32 external {
block0:
    v0 = iconst.i32 10
    v1 = call @sum(v0)
    return v1
}
",
        55,
        "",
    );
}

#[test]
fn recursive_factorial_runs() {
    assert_runs(
        "factorial",
        "\
function @fact(i64) -> i64 internal {
block0(v0: i64):
    v1 = iconst.i64 1
    v2 = icmp.i64 sle v0, v1
    brif v2, block1, block2
block1:
    return v1
block2:
    v3 = isub.i64 nsw v0, v1
    v4 = call @fact(v3)
    v5 = imul.i64 nsw v0, v4
    return v5
}

function @main() -> i32 external {
block0:
    v0 = iconst.i64 5
    v1 = call @fact(v0)
    v2 = trunc.i32 v1
    return v2
}
",
        120,
        "",
    );
}

#[test]
fn printf_call_runs() {
    assert_runs(
        "hello",
        "\
global @format internal constant size 10, align 1 = bytes \"68656c6c6f2025640a00\"

function @printf(ptr, ...) -> i32 external

function @main() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 42
    store.i32 v1, v0, align 4
    v2 = load.i32 v0, align 4
    v3 = global_addr @format
    v4 = call @printf(v3, v2)
    v5 = iconst.i32 0
    return v5
}
",
        0,
        "hello 42\n",
    );
}

/// Reads a byte and a function pointer through relocations at offsets 8
/// and 16 of a table, and calls the function, which switches on the byte:
/// 30 + 5 + 7.
#[test]
fn relocated_pointers_and_switch_run() {
    assert_runs(
        "relocations",
        "\
global @text internal constant size 4, align 1 = bytes \"0a141e28\"
global @table internal constant size 24, align 8 = bytes \
         \"070000000000000000000000000000000000000000000000\" relocs [8: @text + 2, 16: @plus]

function @plus(i32) -> i32 internal {
block0(v0: i32):
    v1 = iconst.i32 5
    switch v0, block2, [10: block1(v1), 30: block1(v1)]
block1(v2: i32):
    v3 = iadd.i32 nsw v0, v2
    return v3
block2:
    v4 = iconst.i32 0
    return v4
}

function @main() -> i32 external {
block0:
    v0 = global_addr @table
    v1 = load.i32 v0, align 8
    v2 = iconst.i64 8
    v3 = ptr_add inbounds v0, v2
    v4 = load.ptr v3, align 8
    v5 = load.i8 v4, align 1
    v6 = zext.i32 v5
    v7 = iconst.i64 16
    v8 = ptr_add inbounds v0, v7
    v9 = load.ptr v8, align 8
    v10 = call_indirect v9(v6) : (i32) -> i32
    v11 = iadd.i32 nsw v10, v1
    return v11
}
",
        42,
        "",
    );
}
