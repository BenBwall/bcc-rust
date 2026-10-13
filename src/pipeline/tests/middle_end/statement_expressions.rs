//! GNU statement-expression values, header jump targets and signed overflow.
use super::*;

const VALUES: &str =
    include_str!("../../../../tests/fixtures/semantic/statement-expression-values.c");
const HEADERS: &str =
    include_str!("../../../../tests/fixtures/semantic/statement-expression-header-targets.c");
const SIGNED_OVERFLOW: &str =
    include_str!("../../../../tests/fixtures/semantic/signed-overflow-boundaries.c");
const LABELED: &str =
    include_str!("../../../../tests/fixtures/semantic/statement-expression-labeled-tails.c");

#[test]
fn statement_expression_labeled_tails_execute() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(LABELED, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0);
            });
        }
    }
    compare_native(LABELED);
}

#[test]
fn statement_expression_values_interpret_correctly() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(VALUES, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0);
            });
        }
    }
}

#[test]
fn statement_expression_values_match_native_clang() {
    compare_native(VALUES);
}

#[test]
fn statement_expression_header_jumps_and_signed_overflow_interpret_correctly() {
    for source in [HEADERS, SIGNED_OVERFLOW] {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

#[test]
fn statement_expression_header_jumps_and_signed_overflow_match_native_clang() {
    for source in [HEADERS, SIGNED_OVERFLOW] {
        compare_native(source);
    }
}

fn compare_native(source: &str) {
    let Some(target) = host() else {
        return;
    };
    let directory = TempDir::new("statement-expressions");
    let toolchain = Toolchain::bundled(target).unwrap();
    let input = directory.join("case.c");
    std::fs::write(&input, source).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let options = LlvmOptions {
            opt: Some(opt),
            disable_llvm_passes: false,
            target,
            kind: OutputKind::Executable,
        };
        let clang = directory.join(format!("clang-{opt:?}.exe"));
        let mut command = toolchain.command(&input, &clang, options);
        _ = command.arg("-std=gnu99");
        if source == HEADERS {
            // Clang 23.1.1 crashes or miscompiles 15 header exits.
            // bcc still executes all 21 cases against their literal results.
            _ = command.arg("-DCLANG_HEADER_REFERENCE");
        }
        let output = crate::test_support::native::run_native(&mut command).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = run_native(&clang);
        if source == HEADERS {
            let mut gcc = Command::new("gcc");
            _ = gcc.arg("--version");
            if crate::test_support::native::run_native(&mut gcc).is_ok_and(|o| o.status.success()) {
                let executable = directory.join("gcc.exe");
                let mut command = Command::new("gcc");
                _ = command
                    .args([
                        "-std=gnu99",
                        if opt == OptLevel::O0 { "-O0" } else { "-O2" },
                    ])
                    .arg(&input)
                    .arg("-o")
                    .arg(&executable);
                let output = crate::test_support::native::run_native(&mut command).unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(run_native(&executable).0, 0, "GCC: {source}");
            } else {
                eprintln!(
                    "GCC header reference unavailable; retaining literal and Clang subset checks"
                );
            }
        }
        assert_eq!(expected.0, 0, "Clang: {source}");
        for optimize in [false, true] {
            let executable = directory.join(format!("bcc-{opt:?}-{optimize}.exe"));
            with_module(source, target, optimize, Profile::PostAbi, |module, _| {
                compile_module(module, &executable, options).unwrap();
            });
            assert_eq!(run_native(&executable), expected, "bcc: {source}");
        }
    }
}
