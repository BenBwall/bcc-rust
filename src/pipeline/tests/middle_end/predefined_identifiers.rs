//! `__func__`, `__FUNCTION__` and `__PRETTY_FUNCTION__` run end to end. The
//! program prints all three in functions of every parameter shape, in nested
//! blocks and a statement expression, and at file scope. The expected text is
//! the bundled clang's own output, which also spells `__PRETTY_FUNCTION__` as
//! a full signature in C.
use super::*;

const PROGRAM: &str =
    include_str!("../../../../tests/fixtures/semantic/predefined-function-names.c");
const OUTPUT: &str =
    include_str!("../../../../tests/fixtures/semantic/predefined-function-names.stdout");

#[test]
fn function_names_interpret_to_clangs_text() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            let (status, output) = with_module(PROGRAM, target, optimize, profile, interpret);
            assert_eq!(
                (status, output.as_str()),
                (0, OUTPUT),
                "{target:?} {optimize} {profile:?}"
            );
        }
    }
}

#[test]
fn function_names_match_native_clang_in_every_arm() {
    let Some(target) = host() else { return };
    let Ok(toolchain) = Toolchain::bundled(target) else {
        return;
    };
    let directory = TempDir::new("predefined-identifiers");
    let input = directory.join("case.c");
    std::fs::write(&input, PROGRAM).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let clang = directory.join(format!("clang-{opt:?}.exe"));
        let mut command = toolchain.command(
            &input,
            &clang,
            LlvmOptions {
                opt: Some(opt),
                disable_llvm_passes: false,
                target,
                kind: OutputKind::Executable,
            },
        );
        _ = command.arg("-std=gnu99");
        let output = crate::test_support::native::run_native(&mut command).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(run_native(&clang), (0, OUTPUT.to_owned()), "clang {opt:?}");
    }
    for arm in ARMS {
        let executable = directory.join(format!("bcc-{}.exe", arm.name));
        with_module(PROGRAM, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &executable,
                LlvmOptions {
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                },
            )
            .unwrap();
        });
        assert_eq!(
            run_native(&executable),
            (0, OUTPUT.to_owned()),
            "{}",
            arm.name
        );
    }
}
