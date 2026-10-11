//! Codegen-only arms retain memory operations that LLVM IR passes remove.

use super::*;

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
            let api = Session::new(&scratch, target);
            build_module(&module, target, &api, &scratch).unwrap();
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
        }
    }
}
