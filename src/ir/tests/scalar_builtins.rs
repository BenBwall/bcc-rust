//! Verifier boundaries for the scalar intrinsic instruction formats.
use super::*;

#[test]
fn narrow_overflow_predicates_execute_before_and_after_fold_and_in_llvm() {
    use std::{
        fmt::Write as _,
        process::Command,
    };

    use crate::{
        backend::{
            interpreter::{
                self,
                DefaultHost,
                RuntimeValue,
                Termination,
            },
            llvm::{
                self,
                LlvmOptions,
                OptLevel,
                OutputKind,
                Toolchain,
            },
        },
        optimizer::{
            self,
            OptimizerOptions,
            Pass,
            PassList,
        },
        target::Target,
        test_support::TempDir,
    };

    let mut functions = String::new();
    let mut main =
        String::from("function @main() -> i32 external {\nblock0:\n    v0 = iconst.i32 0\n");
    let mut reference = String::from("int main(void){int bad=0;\n");
    let mut id = 1;
    let mut accumulator = 0;
    for (bits, ctype) in [
        (8, "unsigned char"),
        (16, "unsigned short"),
        (32, "unsigned int"),
        (64, "unsigned long long"),
        (128, "unsigned __int128"),
    ] {
        let maximum = u128::MAX >> (128 - bits);
        let root = 1_u128 << (bits / 2);
        for (opcode, builtin, cases) in [
            (
                "uadd_overflow",
                "add",
                [(maximum, 1, 1), (maximum - 1, 1, 0), (0, maximum, 0)],
            ),
            (
                "umul_overflow",
                "mul",
                [(root, root, 1), (root - 1, root, 0), (0, maximum, 0)],
            ),
        ] {
            writeln!(
                functions,
                "function @{opcode}_{bits}(i{bits}, i{bits}) -> i1 internal {{\nblock0(v0: \
                 i{bits}, v1: i{bits}):\n    v2 = {opcode}.i{bits} v0, v1\n    return v2\n}}\n"
            )
            .unwrap();
            for (a, b, expected) in cases {
                // The literals are independent boundary expectations. Check
                // both parameterized instructions and constant-foldable ones.
                writeln!(
                    main,
                    "    v{id} = iconst.i{bits} {a}\n    v{} = iconst.i{bits} {b}\n    v{} = call \
                     @{opcode}_{bits}(v{id}, v{})\n    v{} = {opcode}.i{bits} v{id}, v{}\n    v{} \
                     = iconst.i1 {expected}",
                    id + 1,
                    id + 2,
                    id + 1,
                    id + 3,
                    id + 1,
                    id + 4
                )
                .unwrap();
                let expected_id = id + 4;
                let mut next = id + 5;
                for result in [id + 2, id + 3] {
                    writeln!(
                        main,
                        "    v{} = xor.i1 v{result}, v{}\n    v{} = zext.i32 v{}\n    v{} = \
                         or.i32 v{accumulator}, v{}",
                        next,
                        expected_id,
                        next + 1,
                        next,
                        next + 2,
                        next + 1
                    )
                    .unwrap();
                    accumulator = next + 2;
                    next += 3;
                }
                id = next;
                // Build 128-bit C constants from two 64-bit pieces, keeping
                // the Clang oracle valid even at the full-width boundary.
                writeln!(
                    reference,
                    "{{{ctype} a=({ctype})(((unsigned \
                     __int128){}ULL<<64)|{}ULL),b=({ctype})(((unsigned \
                     __int128){}ULL<<64)|{}ULL),r;bad|=__builtin_{builtin}_overflow(a,b,&r)!\
                     ={expected};}}",
                    a >> 64,
                    a & u128::from(u64::MAX),
                    b >> 64,
                    b & u128::from(u64::MAX)
                )
                .unwrap();
            }
        }
    }
    writeln!(main, "    return v{accumulator}\n}}").unwrap();
    functions.push_str(&main);
    reference.push_str("return bad;}\n");
    let host_target = if cfg!(windows) {
        Target::WindowsGnu
    } else {
        Target::LinuxGnu
    };
    let directory = TempDir::new("narrow-overflow");
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for fold in [false, true] {
            let arena = Bump::new();
            let scratch = Bump::new();
            let mut module = parse(&arena, &functions);
            assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
            if fold {
                let mut options = OptimizerOptions::default();
                options.passes = Some(PassList::new(&[Pass::Fold]).unwrap());
                _ = optimizer::optimize(&mut module, &options, &scratch);
            }
            let mut host = DefaultHost::for_target(&scratch, target);
            assert_eq!(
                interpreter::run(&module, "main", &[], &mut host)
                    .unwrap()
                    .termination,
                Termination::Returned(Some(RuntimeValue::Int(0)))
            );
            let mut llvm_text = String::new();
            llvm::emit_module(&module, target, &mut llvm_text).unwrap();
            for bits in [8, 16, 32, 64, 128] {
                for operation in ["uadd", "umul"] {
                    assert!(
                        llvm_text.contains(&format!("llvm.{operation}.with.overflow.i{bits}")),
                        "{llvm_text}"
                    );
                }
            }
            if target != host_target {
                continue;
            }
            let toolchain = Toolchain::bundled(target).unwrap();
            let input = directory.join("reference.c");
            std::fs::write(&input, &reference).unwrap();
            for opt in [OptLevel::O0, OptLevel::O2] {
                let options = LlvmOptions {
                    opt: Some(opt),
                    disable_llvm_passes: false,
                    target,
                    kind: OutputKind::Executable,
                    code: crate::target::CodeOptions::default(),
                };
                let clang = directory.join("clang.exe");
                let output = crate::test_support::native::run_native(
                    &mut toolchain.command(&input, &clang, options),
                )
                .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let expected =
                    crate::test_support::native::run_native(&mut Command::new(&clang)).unwrap();
                assert_eq!(expected.status.code(), Some(0));
                let executable = directory.join("bcc.exe");
                llvm::compile_module(&module, &executable, options).unwrap();
                let actual =
                    crate::test_support::native::run_native(&mut Command::new(executable)).unwrap();
                assert_eq!(
                    actual.status.code(),
                    expected.status.code(),
                    "fold={fold}, {opt:?}"
                );
            }
        }
    }
}

#[test]
fn gcc_scalar_ir_rejects_invalid_controlling_and_operand_types() {
    for (operation, parameter, invalid) in [
        ("bswap.i8", "i8", "invalid"),
        ("clz.f32", "f32", "invalid"),
        ("dyn_alloca.i32", "i32", "invalid"),
        ("popcount.i32", "i64", "expected i32"),
    ] {
        let text = format!(
            "function @f({parameter}) external {{\nblock0(v0: {parameter}):\n    v1 = {operation} \
             v0\n    return\n}}\n"
        );
        let errors = verify_text(&text);
        assert!(!errors.is_empty(), "{operation} unexpectedly verified");
        if invalid != "invalid" {
            assert!(errors.iter().any(|e| e.contains(invalid)), "{errors:?}");
        }
    }
    let text = "function @f(i32, i64) external {\nblock0(v0: i32, v1: i64):\n    v2 = \
                uadd_overflow.i32 v0, v1\n    return\n}\n";
    assert!(
        verify_text(text)
            .iter()
            .any(|e| e.contains("operand 1 is i64, expected i32"))
    );
}
