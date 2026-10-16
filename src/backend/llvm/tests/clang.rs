//! Tests that run the bundled clang: it accepts every golden module, its
//! data layouts match ours, and small programs compile, link and run under
//! both the `-O2` and the O2 code generation without IR passes arm.
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
fn llvm_emits_every_golden_module() {
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
        code: CodeOptions::default(),
    };
    thread::scope(|scope| {
        let runs: Vec<_> = GOLDENS
            .iter()
            .map(|golden| {
                let object = directory.join(format!("{}.o", golden.name));
                scope.spawn(move || {
                    let arena = Bump::new();
                    let module = parse_module(&arena, golden.input).unwrap();
                    (golden.name, compile_module(&module, &object, options))
                })
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
fn extended_poison_arguments_agree_with_the_interpreter() {
    use crate::backend::interpreter::{
        RuntimeValue,
        Termination,
        run_text,
    };

    for (ty, extension) in [("i8", "zeroext"), ("i16", "signext")] {
        let program = format!(
            "abi lowered
function @ignore({ty} {extension}) -> i32 internal {{
block0(v0: {ty}):
    v1 = iconst.i32 0
    return v1
}}
function @unused_result() -> {ty} {extension} internal {{
block0:
    v0 = poison.{ty}
    return v0
}}
function @main() -> i32 external {{
block0:
    v0 = poison.{ty}
    v1 = call @ignore(v0)
    v2 = func_addr @ignore
    v3 = call_indirect v2(v0) : ({ty} {extension}) -> i32
    v4 = call @unused_result()
    v5 = iadd.i32 v1, v3
    return v5
}}
"
        );
        let arena = Bump::new();
        let module = parse_module(&arena, &program).unwrap();
        let printed = module.to_string();
        let reparsed = parse_module(&arena, &printed).unwrap();
        assert_eq!(reparsed.to_string(), printed);
        let result = run_text(&printed, "main").result.unwrap();
        assert_eq!(
            result.termination,
            Termination::Returned(Some(RuntimeValue::Int(0)))
        );
        let llvm = emit(&printed, Profile::PostAbi, Target::LinuxGnu);
        assert!(!llvm.contains("noundef"), "{llvm}");
        assert_runs("extended-poison", &printed, 0, "");
        if let Some(target) = host()
            && toolchain(target).is_some()
        {
            let directory = TempDir::new("llvm-poison-o0");
            let executable = directory.join("poison.exe");
            compile_module(
                &reparsed,
                &executable,
                LlvmOptions {
                    opt: Some(OptLevel::O0),
                    disable_llvm_passes: false,
                    target,
                    kind: OutputKind::Executable,
                    code: CodeOptions::default(),
                },
            )
            .unwrap();
            assert_eq!(run(&executable), (0, String::new()));
        }
    }
}

#[test]
fn partially_poison_register_returns_preserve_defined_bytes() {
    use crate::backend::interpreter::{
        RuntimeValue,
        Termination,
        run_text,
    };

    let reproducer = GOLDENS
        .iter()
        .find(|golden| golden.name == "poison-register-return")
        .unwrap();
    assert_eq!(
        run_text(reproducer.input, "main")
            .result
            .unwrap()
            .termination,
        Termination::Returned(Some(RuntimeValue::Int(42)))
    );
    assert_runs("round-16-reproducer", reproducer.input, 42, "");

    // Exercise each return representation, both register parts, each end of
    // a part, and direct/indirect unpacking into a caller-owned return area.
    for (registers, bytes, start) in [
        ("i16", 2, 0),
        ("i32", 4, 0),
        ("i64", 8, 0),
        ("f32", 4, 0),
        ("f64", 8, 0),
        ("f128", 16, 0),
        ("i128", 16, 0),
        ("i32, i64", 8, 8),
        ("i64, f64", 8, 8),
    ] {
        for defined in [start, start + bytes - 1] {
            let poisoned = if defined == start {
                defined + 1
            } else {
                defined - 1
            };
            for indirect in [false, true] {
                let call = if indirect {
                    format!("call_indirect v1(v0) : (ptr registers({registers}, align 1))")
                } else {
                    "call @f(v0)".to_string()
                };
                let program = format!(
                    "abi lowered
function @f(ptr registers({registers}, align 1)) internal {{
block0(v0: ptr):
 v1 = iconst.i64 {defined}
 v2 = ptr_add inbounds v0, v1
 v3 = iconst.i8 42
 store.i8 v3, v2, align 1
 v4 = iconst.i64 {poisoned}
 v5 = ptr_add inbounds v0, v4
 v6 = poison.i8
 store.i8 v6, v5, align 1
 return
}}
function @main() -> i32 external {{
 slot0 = stack_slot 16, align 1
block0:
 v0 = stack_addr slot0
 v1 = func_addr @f
 {call}
 v2 = iconst.i64 {defined}
 v3 = ptr_add inbounds v0, v2
 v4 = load.i8 v3, align 1
 v5 = zext.i32 v4
 return v5
}}
"
                );
                let result = run_text(&program, "main").result.unwrap();
                assert_eq!(
                    result.termination,
                    Termination::Returned(Some(RuntimeValue::Int(42))),
                    "{program}"
                );
                assert_runs("partially-poison-return", &program, 42, "");
                // Exercise O2 for both ABI profiles even when this host has
                // no runner for the other target's native executables.
                for target in [Target::LinuxGnu, Target::WindowsGnu] {
                    let arena = Bump::new();
                    let module = parse_module(&arena, &program).unwrap();
                    Session::with(&arena, target, |api| {
                        build_module(&module, target, api, &arena).unwrap();
                        let _buffer = api.emit(target, Some(OptLevel::O2), false, true).unwrap();
                        let optimized = api.print();
                        let main = optimized.text().split("@main(").nth(1).unwrap();
                        assert!(
                            main.contains("ret i32 42"),
                            "{target:?}: {}",
                            optimized.text()
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn partially_poison_x87_return_preserves_defined_mantissa_bytes() {
    let program = "abi lowered
function @f(ptr registers(f80, align 16)) internal {
block0(v0: ptr):
 v1 = iconst.i64 -9223372036854775808
 store.i64 v1, v0, align 16
 v2 = iconst.i64 8
 v3 = ptr_add inbounds v0, v2
 v4 = iconst.i16 16383
 store.i16 v4, v3, align 8
 v5 = iconst.i8 42
 store.i8 v5, v0, align 1
 v6 = iconst.i64 1
 v7 = ptr_add inbounds v0, v6
 v8 = poison.i8
 store.i8 v8, v7, align 1
 return
}
function @main() -> i32 external {
 slot0 = stack_slot 16, align 16
block0:
 v0 = stack_addr slot0
 call @f(v0)
 v1 = load.i8 v0, align 1
 v2 = zext.i32 v1
 return v2
}
";
    let result = crate::backend::interpreter::run_text(program, "main")
        .result
        .unwrap();
    assert_eq!(
        result.termination,
        crate::backend::interpreter::Termination::Returned(Some(
            crate::backend::interpreter::RuntimeValue::Int(42)
        ))
    );
    assert_runs("poison-x87-return", program, 42, "");
}

#[test]
fn register_pair_return_areas_agree_with_the_interpreter() {
    let program = "abi lowered
function @pair(ptr registers(i32, i64, align 4)) internal {
block0(v0: ptr):
 v1 = iconst.i32 11
 store.i32 v1, v0, align 4
 v2 = iconst.i64 4
 v3 = ptr_add v0, v2
 v4 = iconst.i32 22
 store.i32 v4, v3, align 4
 v5 = iconst.i64 8
 v6 = ptr_add v0, v5
 v7 = iconst.i64 33
 store.i64 v7, v6, align 4
 return
}
function @main() -> i32 external {
 slot0 = stack_slot 16, align 8
block0:
 v0 = stack_addr slot0
 v1 = iconst.i64 4
 v2 = ptr_add v0, v1
 v3 = iconst.i32 99
 store.i32 v3, v2, align 4
 call @pair(v0)
 v4 = load.i32 v0, align 4
 v5 = load.i32 v2, align 4
 v6 = iconst.i64 8
 v7 = ptr_add v0, v6
 v8 = load.i64 v7, align 4
 v9 = iconst.i32 11
 v10 = icmp.i32 eq v4, v9
 v11 = iconst.i64 33
 v12 = icmp.i64 eq v8, v11
 v13 = and.i1 v10, v12
 v14 = iconst.i32 1
 v15 = select.i32 v13, v5, v14
 return v15
}
";
    let arena = Bump::new();
    let module = parse_module(&arena, program).unwrap();
    let mut host = crate::backend::interpreter::DefaultHost::new_in(&arena);
    let result = crate::backend::interpreter::run_main(&module, &mut host).unwrap();
    assert_eq!(
        result.termination,
        crate::backend::interpreter::Termination::Returned(Some(
            crate::backend::interpreter::RuntimeValue::Int(99)
        ))
    );
    assert_runs("register-pair-layout", program, 99, "");
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
            code: CodeOptions::default(),
        };
        let command = toolchain.command(Path::new("in.o"), Path::new("out"), options);
        command
            .get_args()
            .map(|argument| argument.to_str().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let target = "--target=x86_64-unknown-linux-gnu";
    for level in [OptLevel::O0, OptLevel::O1, OptLevel::O2, OptLevel::O3] {
        assert_eq!(
            arguments(Some(level), true, OutputKind::Executable),
            format!("{target} {} -fuse-ld=lld in.o -o out -lm", level.flag())
        );
    }
    assert_eq!(
        arguments(None, false, OutputKind::SharedLibrary),
        format!("{target} -fuse-ld=lld -shared in.o -o out")
    );
    assert_eq!(
        arguments(None, false, OutputKind::PositionDependentExecutable),
        format!("{target} -fuse-ld=lld -no-pie in.o -o out")
    );
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

/// Compiles `program` for the host under the A2 (`-O2`) and A1 (O2 code
/// generation without IR passes) arms, runs both executables, and checks
/// the exit status and the standard output (with Windows line ends made
/// plain).
fn assert_runs(name: &str, program: &str, status: i32, stdout: &str) {
    let Some(target) = host() else {
        eprintln!("skipping: the host is not a supported target");
        return;
    };
    let Some(_toolchain) = toolchain(target) else {
        return;
    };
    let directory = TempDir::new("llvm-run");

    thread::scope(|scope| {
        let runs: Vec<_> = [false, true]
            .into_iter()
            .map(|disable_llvm_passes| {
                let options = LlvmOptions {
                    opt: Some(OptLevel::O2),
                    disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                    code: CodeOptions::default(),
                };
                let suffix = if disable_llvm_passes { "a1" } else { "a2" };
                let executable = directory.join(format!("{name}-{suffix}.exe"));

                scope.spawn(move || {
                    let arena = Bump::new();
                    let module = parse_module(&arena, program).unwrap();
                    compile_module(&module, &executable, options)
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
    let output = crate::test_support::native::run_native(&mut Command::new(executable))
        .unwrap_or_else(|error| panic!("{error}"));
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

/// Jumps through a static table of label addresses and through label
/// differences added back to a label, as threaded interpreters do:
/// 100 + 200 + 300 + 10 + 20 + 30 - 600. The interpreter agrees.
#[test]
fn label_tables_and_differences_run() {
    let program = "global @table internal constant size 24, align 8 = bytes          \
                   \"000000000000000000000000000000000000000000000000\" relocs [0: \
                   blockaddress(@select,          label0), 8: blockaddress(@select, label1), 16: \
                   blockaddress(@select, label2)]
global @offsets internal constant size 12, align 4 = bytes \"000000000000000000000000\"          \
                   relocs [0: labeldiff.i32(@offset, label0, label0), 4: labeldiff.i32(@offset, \
                   label1,          label0), 8: labeldiff.i32(@offset, label2, label0)]

function @select(i64) -> i32 internal {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i64):
    v1 = global_addr @table
    v2 = iconst.i64 8
    v3 = imul.i64 v0, v2
    v4 = ptr_add v1, v3
    v5 = load.ptr v4, align 8
    v6 = iconst.i32 100
    indirect_br v5, [block1(v6), block2, block3]
block1(v7: i32):
    return v7
block2:
    v8 = iconst.i32 200
    return v8
block3:
    v9 = block_addr label0
    v10 = iconst.i32 300
    indirect_br v9, [block1(v10), block2, block3]
}

function @offset(i64) -> i32 internal {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i64):
    v1 = global_addr @offsets
    v2 = iconst.i64 4
    v3 = imul.i64 v0, v2
    v4 = ptr_add v1, v3
    v5 = load.i32 v4, align 4
    v6 = sext.i64 v5
    v7 = block_addr label0
    v8 = ptr_add inbounds v7, v6
    indirect_br v8, [block1, block2, block3]
block1:
    v9 = iconst.i32 10
    return v9
block2:
    v10 = iconst.i32 20
    return v10
block3:
    v11 = iconst.i32 30
    return v11
}

function @main() -> i32 external {
block0:
    v0 = iconst.i64 0
    v1 = iconst.i64 1
    v2 = iconst.i64 2
    v3 = call @select(v0)
    v4 = call @select(v1)
    v5 = call @select(v2)
    v6 = call @offset(v0)
    v7 = call @offset(v1)
    v8 = call @offset(v2)
    v9 = iadd.i32 v3, v4
    v10 = iadd.i32 v9, v5
    v11 = iadd.i32 v10, v6
    v12 = iadd.i32 v11, v7
    v13 = iadd.i32 v12, v8
    v14 = iconst.i32 600
    v15 = isub.i32 v13, v14
    return v15
}
";
    let run = crate::backend::interpreter::run_text(program, "main");
    assert!(
        matches!(
            run.result,
            Ok(crate::backend::interpreter::Outcome {
                termination: crate::backend::interpreter::Termination::Returned(Some(
                    crate::backend::interpreter::RuntimeValue::Int(60)
                )),
                ..
            })
        ),
        "{:?}",
        run.result
    );
    assert_runs("labels", program, 60, "");
}

#[test]
fn objects_and_assembly_are_emitted_in_memory() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let arena = Bump::new();
        let module = parse_module(
            &arena,
            "function @answer() -> i32 external {\nblock0:\n v0 = iconst.i32 42\n return v0\n}\n",
        )
        .unwrap();
        for assembly in [false, true] {
            let scratch = Bump::new();
            Session::with(&scratch, target, |api| {
                build_module(&module, target, api, &scratch).unwrap();
                let buffer = api
                    .emit(target, Some(OptLevel::O2), true, assembly)
                    .unwrap();
                if assembly {
                    assert!(
                        std::str::from_utf8(buffer.bytes())
                            .unwrap()
                            .contains("answer")
                    );
                } else if target == Target::LinuxGnu {
                    assert!(buffer.bytes().starts_with(b"\x7fELF"));
                } else {
                    assert!(buffer.bytes().starts_with(&[0x64, 0x86]));
                }
            });
        }
    }
}

#[test]
fn constant_arithmetic_retains_poison_flags_without_ir_passes() {
    let text = emit(
        "function @overflow() -> i8 external {\nblock0:\n v0 = iconst.i8 127\n v1 = iconst.i8 1\n \
         v2 = iadd.i8 nsw nuw v0, v1\n return v2\n}\n",
        Profile::PostAbi,
        Target::LinuxGnu,
    );
    assert!(text.contains("add nuw nsw i8 127, 1"), "{text}");
    assert!(!text.contains("phi"), "{text}");
}
