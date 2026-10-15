//! Instruction selection, frames and refusals, checked on the assembly of
//! small textual modules.

use super::{
    assembly,
    function_text,
    refusal,
};
use crate::target::Target;

const ADD: &str = "abi lowered
function @add(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    return v2
}
";

#[test]
fn hints_turn_a_two_address_add_into_one_instruction() {
    let text = assembly(ADD, Target::LinuxGnu);
    let add = function_text(&text, "add");
    assert!(add.contains("addl\t%esi, %edi"), "{add}");
    assert!(add.contains("movq\t%rdi, %rax"), "{add}");
    assert!(
        !add.contains("subq"),
        "a leaf without slots needs no frame:\n{add}"
    );
    let text = assembly(ADD, Target::WindowsGnu);
    let add = function_text(&text, "add");
    assert!(add.contains("addl\t%edx, %ecx"), "{add}");
}

#[test]
fn constants_become_immediates_and_compares_fuse_into_branches() {
    let text = assembly(
        "abi lowered
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 5
    v2 = iadd.i32 v0, v1
    v3 = iconst.i32 100
    v4 = icmp.i32 slt v2, v3
    brif v4, block1, block2
block1:
    return v2
block2:
    v5 = icmp.i32 ult v0, v3
    v6 = zext.i32 v5
    return v6
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    assert!(f.contains("addl\t$5, "), "{f}");
    assert!(f.contains("cmpl\t$100, "), "{f}");
    assert!(f.contains("\tjl\t") || f.contains("\tjge\t"), "{f}");
    assert!(!f.contains("setl"), "the branch compare is fused:\n{f}");
    assert!(f.contains("setb"), "the unfused compare materializes:\n{f}");
    assert!(f.contains("movzbl"), "{f}");
}

#[test]
fn division_and_variable_shifts_use_their_fixed_registers() {
    let text = assembly(
        "abi lowered
function @f(i32, i32, i64, i64) -> i64 external {
block0(v0: i32, v1: i32, v2: i64, v3: i64):
    v4 = sdiv.i32 v0, v1
    v5 = urem.i64 v2, v3
    v6 = sext.i64 v4
    v7 = shl.i64 v5, v6
    v8 = lshr.i64 v7, v3
    return v8
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    assert!(f.contains("cltd\n\tidivl\t"), "{f}");
    assert!(f.contains("movl\t$0, %edx\n\tdivq\t"), "{f}");
    assert!(f.contains("shlq\t%cl, "), "{f}");
    assert!(f.contains("shrq\t%cl, "), "{f}");
    assert!(
        !f.contains("idivl\t%eax") && !f.contains("idivl\t%edx"),
        "{f}"
    );
}

#[test]
fn narrow_right_shifts_and_compares_read_exactly_their_bits() {
    let text = assembly(
        "abi lowered
function @f(i8, i8, i16) -> i32 external {
block0(v0: i8, v1: i8, v2: i16):
    v3 = ashr.i8 v0, v1
    v4 = iconst.i16 3
    v5 = lshr.i16 v2, v4
    v6 = icmp.i8 slt v3, v0
    v7 = icmp.i16 eq v5, v2
    v8 = and.i1 v6, v7
    v9 = zext.i32 v8
    return v9
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    assert!(
        f.contains("movsbl"),
        "an arithmetic shift sign-extends:\n{f}"
    );
    assert!(f.contains("movzwl"), "a logical shift zero-extends:\n{f}");
    assert!(f.contains("cmpb\t"), "{f}");
    assert!(f.contains("cmpw\t"), "{f}");
}

const MEMORY: &str = "abi lowered
global @counter external size 8, align 8 = zero
global @other external size 8, align 8
function @f(i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i64 8
    v3 = ptr_add v1, v2
    store.i64 v0, v3, align 8
    v4 = load.i64 v3, align 8
    v5 = global_addr @counter
    store.i64 v4, v5, align 8
    v6 = global_addr @other
    v7 = load.i64 v6, align 8
    return v7
}
";

#[test]
fn addresses_fold_into_memory_operands() {
    let text = assembly(MEMORY, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains("movq\t%rdi, 8(%rsp)"), "{f}");
    assert!(
        f.contains("counter(%rip)"),
        "a defined global is RIP-relative:\n{f}"
    );
    assert!(
        f.contains("other@GOTPCREL(%rip)"),
        "an undefined global goes through the GOT on ELF:\n{f}"
    );
    assert!(!f.contains("leaq"), "no address needs a register:\n{f}");
    assert!(text.contains("\t.bss\n\t.globl\tcounter"), "{text}");
    let text = assembly(MEMORY, Target::WindowsGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("other(%rip)"),
        "COFF references any symbol directly:\n{f}"
    );
}

#[test]
fn read_only_addresses_go_where_the_loader_can_relocate_them() {
    let ir = "abi lowered
global @text internal constant size 3, align 1 = bytes \"686900\"
global @table external constant size 16, align 8 = bytes \"00000000000000000000000000000000\" \
              relocs [0: @text, 8: @text + 1]
";
    let text = assembly(ir, Target::LinuxGnu);
    assert!(
        text.contains(".section\t.data.rel.ro,\"aw\",@progbits\n\t.globl\ttable\n"),
        "a PIE relocates the table at load time:\n{text}"
    );
    assert!(
        text.contains(".section\t.rodata\n\t.p2align\t0\n"),
        "{text}"
    );
    let text = assembly(ir, Target::WindowsGnu);
    assert!(!text.contains(".data.rel.ro"), "{text}");
    assert!(
        text.contains(".section\t.rdata,\"dr\"\n\t.globl\ttable\n"),
        "{text}"
    );
}

const CALLS: &str = "abi lowered
global @fmt internal constant size 4, align 1 = bytes \"25640a00\"
function @printf(ptr, ...) -> i32 external
function @seven(i32, i32, i32, i32, i32, i32, i32) -> i32 external
function @narrow(i8 signext) -> i8 signext external
function @main() -> i32 external {
block0:
    v0 = iconst.i32 1
    v1 = call @seven(v0, v0, v0, v0, v0, v0, v0)
    v2 = global_addr @fmt
    v3 = call @printf(v2, v1)
    v4 = iconst.i8 200
    v5 = call @narrow(v4)
    v6 = sext.i32 v5
    return v6
}
";

#[test]
fn calls_place_arguments_by_convention() {
    let text = assembly(CALLS, Target::LinuxGnu);
    let main = function_text(&text, "main");
    assert!(
        main.contains(", 0(%rsp)"),
        "the seventh argument is on the stack:\n{main}"
    );
    assert!(main.contains("callq\tseven@PLT"), "{main}");
    assert!(
        main.contains("movl\t$0, %eax\n\tcallq\tprintf@PLT"),
        "{main}"
    );
    assert!(!main.contains("__main"), "{main}");
    let text = assembly(CALLS, Target::WindowsGnu);
    let main = function_text(&text, "main");
    assert!(
        main.contains("callq\t__main"),
        "MinGW's main runs constructors:\n{main}"
    );
    assert!(
        main.contains(", 32(%rsp)"),
        "the fifth argument follows the home area:\n{main}"
    );
    assert!(main.contains(", 48(%rsp)"), "{main}");
    assert!(!main.contains("movl\t$0, %eax\n\tcallq\tprintf"), "{main}");
    assert!(main.contains("callq\tprintf\n"), "{main}");
}

#[test]
fn frames_keep_the_stack_aligned_and_probe_large_windows_frames() {
    let ir = "abi lowered
function @g(ptr) external
function @f() external {
    slot0 = stack_slot 10000, align 16
block0:
    v0 = stack_addr slot0
    call @g(v0)
    return
}
";
    let text = assembly(ir, Target::WindowsGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("movl\t$10040, %eax\n\tcallq\t___chkstk_ms\n\tsubq\t%rax, %rsp"),
        "{f}"
    );
    assert!(f.contains(".seh_stackalloc 10040"), "{f}");
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains("subq\t$10008, %rsp"), "{f}");
    assert!(f.contains(".cfi_def_cfa_offset 10016"), "{f}");
}

#[test]
fn switches_become_compare_chains_with_edge_blocks() {
    let text = assembly(
        "abi lowered
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 10
    v2 = iconst.i32 20
    switch v0, block1(v1), [1: block1(v2), 2: block2, 7: block1(v0)]
block1(v3: i32):
    return v3
block2:
    return v0
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    for case in ["$1", "$2", "$7"] {
        assert!(f.contains(&format!("cmpl\t{case}, ")), "{f}");
    }
    assert!(f.matches("\tje\t").count() >= 2, "{f}");
}

#[test]
fn unsupported_constructs_are_refused_with_their_function() {
    let message = refusal(
        "abi lowered
function @f(f128) -> f128 external {
block0(v0: f128):
    return v0
}
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: f128 values (in @f)"
    );
    let message = refusal(
        "abi lowered
function @g(i128) -> i128 external {
block0(v0: i128):
    v1 = clz.i128 v0
    return v1
}
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: the `clz` instruction (in @g)"
    );
    let message = refusal(
        "abi lowered
function @w(i128) -> i128 external {
block0(v0: i128):
    return v0
}
",
        Target::WindowsGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: an i128 passed by value on Win64 (in @w)"
    );
    let message = refusal(
        "function @h(i32) -> i32 external {
block0(v0: i32):
    return v0
}
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: a module not lowered for the ABI"
    );
}

#[test]
fn bit_operations_use_baseline_instructions() {
    let text = assembly(
        "abi lowered
function @f(i32, i64, i16) -> i64 external {
block0(v0: i32, v1: i64, v2: i16):
    v3 = clz_defined.i32 v0
    v4 = ctz.i64 v1
    v5 = popcount.i64 v1
    v6 = bswap.i16 v2
    v7 = zext.i64 v3
    v8 = iadd.i64 v7, v4
    v9 = iadd.i64 v8, v5
    v10 = zext.i64 v6
    v11 = iadd.i64 v9, v10
    return v11
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    assert!(f.contains("bsrl") && f.contains("cmovel"), "{f}");
    assert!(f.contains("bsfq"), "{f}");
    assert!(!f.contains("popcnt") && !f.contains("lzcnt"), "{f}");
    assert!(f.contains("rolw\t$8, "), "{f}");
}

#[test]
fn floats_use_sse_registers_and_read_only_constants() {
    let ir = "abi lowered
function @f(f64, f32, i64) -> f64 external {
block0(v0: f64, v1: f32, v2: i64):
    v3 = fconst.f64 0x400c000000000000
    v4 = fadd.f64 v0, v3
    v5 = fpext.f64 v1
    v6 = fmul.f64 v4, v5
    v7 = uitofp.f64 v2
    v8 = fdiv.f64 v6, v7
    v9 = fcmp.f64 oeq v8, v0
    v10 = fneg.f64 v8
    v11 = select.f64 v9, v8, v10
    return v11
}
";
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("addsd\t.LCPI0_"),
        "a constant operand is read from memory:\n{f}"
    );
    assert!(f.contains("cvtss2sd"), "{f}");
    assert!(f.contains("cvtsi2sdq"), "{f}");
    assert!(f.contains("ucomisd"), "{f}");
    assert!(f.contains("setnp"), "oeq tests parity too:\n{f}");
    assert!(
        f.contains("xorps\t.LCPI0_"),
        "fneg flips the sign bit:\n{f}"
    );
    assert!(
        f.contains("cmov"),
        "a float select picks in general registers:\n{f}"
    );
    assert!(
        text.contains(".LCPI0_0:\n\t.quad\t0x400c000000000000"),
        "{text}"
    );
}

#[test]
fn win64_saves_the_sse_registers_it_keeps_across_calls() {
    let ir = "abi lowered
function @g() external
function @f(f64) -> f64 external {
block0(v0: f64):
    call @g()
    v1 = fadd.f64 v0, v0
    return v1
}
";
    let text = assembly(ir, Target::WindowsGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("movaps\t%xmm6, ") && f.contains(".seh_savexmm %xmm6"),
        "{f}"
    );
    assert!(f.contains("), %xmm6"), "the epilogue restores it:\n{f}");
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(
        !f.contains("movaps\t%xmm6, "),
        "System V has no callee-saved SSE register:\n{f}"
    );
}

#[test]
fn dynamic_allocation_keeps_a_frame_pointer() {
    let ir = "abi lowered
function @g(ptr) external
function @f(i64) external {
    slot0 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_save
    v2 = dyn_alloca.i64 v0, align 32
    call @g(v2)
    v3 = stack_addr slot0
    store.i64 v0, v3, align 8
    stack_restore v1
    return
}
";
    let text = assembly(ir, Target::WindowsGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("movq\t%rsp, %rbp\n\t.seh_setframe %rbp, 0"),
        "{f}"
    );
    assert!(
        f.contains("callq\t___chkstk_ms\n\tsubq\t"),
        "dynamic pages are probed:\n{f}"
    );
    // The block starts 16-byte aligned above the outgoing arguments; the
    // address rounds up into its 16 spare bytes, never down below it.
    let round_up = f.find("addq\t$16, %").expect(f);
    assert!(f[round_up..].contains("andq\t$-32, "), "{f}");
    assert!(
        f.contains("(%rbp)"),
        "the fixed frame is addressed from %rbp:\n{f}"
    );
    assert!(f.contains("leaq\t") && f.contains("(%rbp), %rsp\n"), "{f}");
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains(".cfi_def_cfa_register %rbp"), "{f}");
    assert!(!f.contains("chkstk"), "{f}");
}

#[test]
fn dynamic_allocation_in_a_leaf_keeps_the_stack_aligned() {
    // No call, no 16-byte slot: only the dynamic block needs `%rsp`
    // aligned, which with `%rbp` pushed takes 8 bytes of padding.
    let ir = "abi lowered
function @f(i64) -> i64 external {
    slot0 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    store.i64 v0, v1, align 8
    v2 = dyn_alloca.i64 v0, align 16
    v3 = ptrtoint.i64 v2
    return v3
}
";
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains("pushq\t%rbp\n"), "{f}");
    assert!(f.contains("subq\t$16, %rsp\n"), "{f}");
}

#[test]
fn int128_values_use_register_pairs() {
    let ir = "abi lowered
function @f(i128, i128) -> i128 external {
block0(v0: i128, v1: i128):
    v2 = iadd.i128 v0, v1
    v3 = imul.i128 v2, v0
    v4 = icmp.i128 slt v3, v1
    v5 = select.i128 v4, v3, v1
    v6 = udiv.i128 v5, v0
    return v6
}
";
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains("adcq"), "{f}");
    assert!(f.contains("mulq"), "{f}");
    assert!(f.contains("sbbq"), "{f}");
    assert!(f.contains("callq\t__udivti3@PLT"), "{f}");
}
