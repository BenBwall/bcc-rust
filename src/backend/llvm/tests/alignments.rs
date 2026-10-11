//! The IR alignment limit is representable by every LLVM-C memory operation.

use super::*;

#[test]
fn maximum_ir_alignment_emits_for_globals_allocas_and_memory_operations() {
    let text = "\
global @g internal size 4, align 2147483648 = zero

function @f(ptr, i64, i8) external {
    slot0 = stack_slot 4, align 2147483648
block0(v0: ptr, v1: i64, v2: i8):
    v3 = stack_addr slot0
    v4 = load.i32 v0, align 2147483648
    store.i32 v4, v3, align 2147483648
    copy v3, v0, v1, align 2147483648
    fill v3, v2, v1, align 2147483648
    return
}
";
    let output = emit(text, Profile::PostAbi, Target::LinuxGnu);
    // Global, alloca, load, store, both copy operands and the fill destination.
    assert_eq!(output.matches("align 2147483648").count(), 7, "{output}");
}
