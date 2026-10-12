//! Codegen-only arms retain memory operations that LLVM IR passes remove.

use super::*;

#[test]
fn defined_register_return_packing_folds_away_at_o2() {
    let arena = Bump::new();
    let module = parse_module(
        &arena,
        "abi lowered
function @pair(ptr registers(i64, f64, align 16)) external {
block0(v0: ptr):
 v1 = iconst.i64 42
 store.i64 v1, v0, align 16
 v2 = iconst.i64 8
 v3 = ptr_add inbounds v0, v2
 v4 = fconst.f64 0x4045000000000000
 store.f64 v4, v3, align 8
 return
}
",
    )
    .unwrap();
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let scratch = Bump::new();
        Session::with(&scratch, target, |api| {
            build_module(&module, target, api, &scratch).unwrap();
            let before = api.print();
            assert!(before.text().contains("load <8 x i8>, ptr %v0, align 16"));
            assert!(
                before
                    .text()
                    .lines()
                    .any(|line| line.contains("load <8 x i8>") && line.ends_with("align 8"))
            );
            let _buffer = api.emit(target, Some(OptLevel::O2), false, true).unwrap();
            let optimized = api.print();
            assert!(
                optimized
                    .text()
                    .contains("ret { i64, double } { i64 42, double 4.200000e+01 }"),
                "{target:?}: {}",
                optimized.text()
            );
            for operation in ["alloca", "load", "freeze", "bitcast", "insertvalue"] {
                assert!(
                    !optimized.text().contains(operation),
                    "{}",
                    optimized.text()
                );
            }
        });
    }
}

#[test]
fn codegen_only_preserves_memory_that_o2_ir_passes_remove() {
    let arena = Bump::new();
    let module = parse_module(
        &arena,
        "abi lowered\nfunction @identity(i32) -> i32 external {\n slot0 = stack_slot 4, align \
         4\nblock0(v0: i32):\n v1 = stack_addr slot0\n store.i32 v0, v1, align 4\n v2 = load.i32 \
         v1, align 4\n return v2\n}\n",
    )
    .unwrap();
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for disable_llvm_passes in [true, false] {
            let scratch = Bump::new();
            Session::with(&scratch, target, |api| {
                build_module(&module, target, api, &scratch).unwrap();
                let _buffer = api
                    .emit(target, Some(OptLevel::O2), disable_llvm_passes, true)
                    .unwrap();
                let optimized = api.print();
                for pattern in ["alloca [4 x i8]", "store i32", "load i32"] {
                    assert_eq!(
                        optimized.text().contains(pattern),
                        disable_llvm_passes,
                        "{target:?}, disable_llvm_passes={disable_llvm_passes}: {}",
                        optimized.text()
                    );
                }
            });
        }
    }
}
