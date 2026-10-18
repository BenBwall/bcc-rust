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
fn thread_locals_in_named_sections_are_refused() {
    let message = refusal(
        "abi lowered
global @t external thread_local size 4, align 4 section \"tls_here\" = zero
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: a thread-local global in a named section (in \
         @t)"
    );
}

#[test]
fn frame_and_return_addresses_read_the_frame_a_frame_pointer_would_build() {
    let text = assembly(
        "abi lowered
function @f() -> ptr external {
block0:
    v0 = return_address
    v1 = frame_address
    v2 = ptr_add v1, v0
    return v2
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    // `%rbp` is pushed first, at CFA - 16, the frame address; the return
    // address is at CFA - 8, right above it.
    let push = f.find("\tpushq\t%rbp").expect(f);
    assert!(!f[..push].contains("pushq"), "{f}");
    assert!(f.contains("movq\t8(%rsp), %"), "{f}");
    assert!(
        f.contains("leaq\t(%rsp), %") || f.contains("leaq\t0(%rsp), %"),
        "{f}"
    );
}

#[test]
fn mingw_reaches_undefined_data_through_refptr_stubs() {
    // #69: data the module does not define may be in a DLL, which MinGW's
    // auto-import relocates through a 64-bit stub, as Clang reaches it; a
    // 32-bit RIP-relative operand would fail beyond 2 GiB. Defined data,
    // functions (import thunks) and unused declarations stay direct, and
    // MSVC, which does not auto-import, addresses undefined data directly
    // as Clang does.
    let ir = "abi lowered
global @imported external size 16, align 4
global @unused external size 4, align 4
global @local external size 4, align 4 = zero
function @read() -> i32 external {
block0:
    v0 = global_addr @imported
    v1 = iconst.i64 12
    v2 = ptr_add v0, v1
    v3 = load.i32 v2, align 4
    v4 = global_addr @local
    v5 = load.i32 v4, align 4
    v6 = iadd.i32 v3, v5
    v7 = call @thunk()
    v8 = iadd.i32 v6, v7
    return v8
}
function @thunk() -> i32 external
";
    let coff = assembly(ir, Target::WindowsGnu);
    for line in [
        "movq\t.refptr.imported(%rip), %",
        "\t.section\t.rdata$.refptr.imported,\"dr\",discard,.refptr.imported\n",
        ".refptr.imported:\n\t.quad\timported\n",
        "local(%rip)",
        "callq\tthunk\n",
    ] {
        assert!(coff.contains(line), "{line}\n{coff}");
    }
    assert!(
        coff.lines()
            .filter(|line| line.contains("imported") && line.contains("(%rip)"))
            .all(|line| line.contains(".refptr.imported(%rip)")),
        "{coff}"
    );
    assert!(!coff.contains("refptr.unused"), "{coff}");
    assert!(!coff.contains("refptr.local"), "{coff}");
    assert!(!coff.contains(".weak"), "{coff}");
    let msvc = assembly(ir, Target::WindowsMsvc);
    assert!(msvc.contains("imported+12(%rip)"), "{msvc}");
    assert!(!msvc.contains("refptr"), "{msvc}");
    let elf = assembly(ir, Target::LinuxGnu);
    assert!(elf.contains("imported@GOTPCREL(%rip)"), "{elf}");
}

#[test]
fn symbol_attributes_become_directives() {
    let ir = "abi lowered
global @value external size 4, align 4 hidden section \"mine\" = bytes \"05000000\"
global @other external size 4, align 4 alias @value
global @maybe weak size 4, align 4
constructor @init priority 101
destructor @init priority 65535
function @init() internal align 64 {
block0:
    return
}
function @fini() weak section \"text_fini\" {
block0:
    v0 = global_addr @maybe
    v1 = load.i32 v0, align 4
    v2 = call @missing()
    return
}
function @copy() weak alias @init
function @missing() -> i32 weak
";
    let elf = assembly(ir, Target::LinuxGnu);
    for line in [
        "\t.section\tmine,\"aw\",@progbits\n\t.globl\tvalue\n\t.hidden\tvalue\n",
        "\t.globl\tother\nother = value\n",
        "\t.weak\tcopy\n\t.type\tcopy,@function\ncopy = init\n",
        "\t.weak\tmaybe\n",
        "\t.weak\tmissing\n",
        "\t.p2align\t6, 0x90\n\t.type\tinit,@function\n",
        "\t.section\ttext_fini,\"ax\",@progbits\n\t.weak\tfini\n",
        "\t.section\t.init_array.101,\"aw\",@init_array\n\t.p2align\t3\n\t.quad\tinit\n",
        "\t.section\t.fini_array,\"aw\",@fini_array\n",
        "maybe@GOTPCREL(%rip)",
        "callq\tmissing@PLT",
    ] {
        assert!(elf.contains(line), "{line}\n{elf}");
    }
    let coff = assembly(ir, Target::WindowsGnu);
    for line in [
        "\t.section\t.ctors.65434,\"dw\"\n",
        "\t.section\t.dtors,\"dw\"\n",
        "\t.section\t.rdata$.refptr.maybe,\"dr\",discard,.refptr.maybe\n",
        "movq\t.refptr.maybe(%rip), %",
        "callq\t*.refptr.missing(%rip)",
        "\t.section\ttext_fini,\"xr\"\n",
    ] {
        assert!(coff.contains(line), "{line}\n{coff}");
    }
    assert!(!coff.contains(".hidden"), "{coff}");
    let msvc = assembly(ir, Target::WindowsMsvc);
    assert!(
        msvc.contains("\t.section\t.CRT$XCA00101,\"dr\"\n"),
        "{msvc}"
    );
    assert!(msvc.contains("\t.section\t.CRT$XTX,\"dr\"\n"), "{msvc}");
}

#[test]
fn returns_twice_calls_and_addresses_are_refused() {
    for body in [
        "v1 = call @setjmp(v0)",
        "v2 = func_addr @setjmp\n    v1 = iconst.i32 0",
    ] {
        let message = refusal(
            &format!(
                "abi lowered\nfunction @setjmp(ptr) -> i32 external\nfunction @f(ptr) -> i32 \
                 external {{\nblock0(v0: ptr):\n    {body}\n    return v1\n}}\n"
            ),
            Target::LinuxGnu,
        );
        assert_eq!(
            message,
            "not yet supported by the native back end: a call to, or the address of, a function \
             that may return twice, such as `setjmp` (in @f)"
        );
    }
    // An alias of a defined attributed function inherits its contract, and
    // an address stored in a global escapes to indirect calls.
    let alias = refusal(
        "abi lowered\nfunction @real(ptr) -> i32 external returns_twice {\nblock0(v0: ptr):\n    \
         v1 = iconst.i32 0\n    return v1\n}\nfunction @save(ptr) -> i32 external alias \
         @real\nfunction @f(ptr) -> i32 external {\nblock0(v0: ptr):\n    v1 = call @save(v0)\n    \
         return v1\n}\n",
        Target::LinuxGnu,
    );
    assert!(alias.ends_with("such as `setjmp` (in @f)"), "{alias}");
    let escaped = refusal(
        "abi lowered\nglobal @save internal size 8, align 8 = bytes \"0000000000000000\" relocs \
         [0: @_setjmp]\nfunction @_setjmp(ptr) -> i32 external\nfunction @f(ptr) -> i32 external \
         {\nblock0(v0: ptr):\n    v1 = global_addr @save\n    v2 = load.ptr v1, align 8\n    v3 = \
         call_indirect v2(v0) : (ptr) -> i32\n    return v3\n}\n",
        Target::LinuxGnu,
    );
    assert!(
        escaped.ends_with("such as `setjmp` (in @save)"),
        "{escaped}"
    );
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
fn oversized_byval_arguments_are_refused_before_selection() {
    // A 3 GiB structure copied to the stack for a call, and one received,
    // would need argument offsets beyond 32-bit displacements (#69); the
    // back end refuses them instead of panicking in the copy.
    let message = refusal(
        "abi lowered
function @take(ptr byval(size 3221225472, align 8)) -> i32 external

function @give(ptr) -> i32 external {
block0(v0: ptr):
    v1 = call @take(v0)
    return v1
}
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: 3221225480 bytes of stack arguments (in @give)"
    );
    let message = refusal(
        "abi lowered
function @take(ptr byval(size 3221225472, align 8)) -> i32 external {
block0(v0: ptr):
    v1 = load.i32 v0, align 4
    return v1
}
",
        Target::LinuxGnu,
    );
    assert_eq!(
        message,
        "not yet supported by the native back end: 3221225480 bytes of stack arguments (in @take)"
    );
    // A large structure that still fits is compiled.
    assert!(
        assembly(
            "abi lowered
function @take(ptr byval(size 65536, align 8)) -> i32 external

function @give(ptr) -> i32 external {
block0(v0: ptr):
    v1 = call @take(v0)
    return v1
}
",
            Target::LinuxGnu,
        )
        .contains("rep movsb")
    );
}

#[test]
fn frames_beyond_32_bit_displacements_are_refused() {
    // The local slots and the stack arguments of each call and of the
    // function itself stay within their separate 1 GiB bounds, but the
    // final frame would need offsets beyond signed 32-bit displacements:
    // the outgoing area below 1 GiB of locals (`f`), or a stack parameter
    // after 1 GiB of incoming arguments, above 1 GiB of outgoing ones (`h`).
    for (ir, function) in [
        (
            "abi lowered
function @g(ptr byval(size 1073741816, align 8)) external
function @f() -> i32 external {
    slot0 = stack_slot 1073741824, align 8
block0:
    v0 = stack_addr slot0
    call @g(v0)
    v1 = load.i32 v0, align 4
    return v1
}
",
            "f",
        ),
        (
            "abi lowered
function @g(ptr byval(size 1073741816, align 8)) external
function @h(ptr byval(size 1073741760, align 8), i64, i64, i64, i64, i64, i64, i64) -> i64 \
             external {
    slot0 = stack_slot 4096, align 8
block0(v0: ptr, v1: i64, v2: i64, v3: i64, v4: i64, v5: i64, v6: i64, v7: i64):
    v8 = stack_addr slot0
    call @g(v8)
    return v7
}
",
            "h",
        ),
    ] {
        let message = refusal(ir, Target::LinuxGnu);
        assert!(
            message.starts_with("not yet supported by the native back end: a ")
                && message.ends_with(&format!("-byte stack frame (in @{function})")),
            "{message}"
        );
    }
    // The same frames with half the locals or incoming arguments compile.
    let fitting = "abi lowered
function @g(ptr byval(size 1073741816, align 8)) external
function @f() -> i32 external {
    slot0 = stack_slot 536870912, align 8
block0:
    v0 = stack_addr slot0
    call @g(v0)
    v1 = load.i32 v0, align 4
    return v1
}
";
    assert!(assembly(fitting, Target::LinuxGnu).contains("rep movsb"));
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

/// A defined, a zeroed and a declared thread-local global.
const THREAD_LOCALS: &str = "abi lowered
global @seed external thread_local size 4, align 4 = bytes \"07000000\"
global @zeroed internal thread_local size 8, align 8 = zero
global @shared external thread_local size 4, align 4
function @f() -> i32 external {
block0:
    v0 = global_addr @seed
    v1 = load.i32 v0, align 4
    v2 = global_addr @zeroed
    v3 = load.i64 v2, align 8
    v4 = trunc.i32 v3
    v5 = global_addr @shared
    v6 = load.i32 v5, align 4
    v7 = iadd.i32 v1, v4
    v8 = iadd.i32 v7, v6
    return v8
}
";

#[test]
fn elf_thread_locals_use_local_exec_for_definitions_and_initial_exec_otherwise() {
    let text = assembly(THREAD_LOCALS, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert_eq!(f.matches("movq\t%fs:0, ").count(), 3, "{f}");
    assert!(f.contains("leaq\tseed@TPOFF(%"), "{f}");
    assert!(f.contains("leaq\tzeroed@TPOFF(%"), "{f}");
    assert!(f.contains("addq\tshared@GOTTPOFF(%rip), %"), "{f}");
    assert!(
        text.contains(".section\t.tdata,\"awT\",@progbits"),
        "{text}"
    );
    assert!(text.contains(".section\t.tbss,\"awT\",@nobits"), "{text}");
}

#[test]
fn windows_thread_locals_index_the_tls_array() {
    let text = assembly(THREAD_LOCALS, Target::WindowsGnu);
    let f = function_text(&text, "f");
    assert_eq!(f.matches("movl\t_tls_index(%rip), ").count(), 3, "{f}");
    assert_eq!(f.matches("movq\t%gs:88, ").count(), 3, "{f}");
    assert!(f.contains("leaq\tseed@SECREL32(%"), "{f}");
    assert!(f.contains("leaq\tshared@SECREL32(%"), "{f}");
    assert_eq!(text.matches(".section\t.tls$,\"dw\"").count(), 2, "{text}");
}

#[test]
fn block_labels_name_landing_blocks_and_fill_label_tables() {
    let text = assembly(
        "abi lowered
global @table internal constant size 12, align 8 = bytes \"000000000000000000000000\" relocs [0: \
         blockaddress(@f, label0) + 2, 8: labeldiff.i32(@f, label1, label0)]
function @f(ptr, i32) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: ptr, v1: i32):
    indirect_br v0, [block1(v1), block2]
block1(v2: i32):
    return v2
block2:
    v3 = block_addr label1
    v4 = iconst.i32 4
    indirect_br v3, [block1(v4), block2]
block3:
    unreachable
}
",
        Target::LinuxGnu,
    );
    let f = function_text(&text, "f");
    assert_eq!(f.matches("\tjmpq\t*%").count(), 2, "{f}");
    assert!(f.contains("leaq\t.LBA0_1(%rip), %"), "{f}");
    for label in [".LBA0_0:", ".LBA0_1:", ".LBA0_2:"] {
        assert_eq!(f.matches(label).count(), 1, "{label}\n{f}");
    }
    // The parameter of block1 arrives through a landing block, so label 0
    // is not at the block a direct branch would enter.
    let landing = f.find(".LBA0_0:").unwrap();
    assert!(f[landing..].contains("\tjmp\t"), "{f}");
    assert!(text.contains("\t.quad\t.LBA0_0+2\n"), "{text}");
    assert!(text.contains("\t.long\t.LBA0_1-.LBA0_0\n"), "{text}");
}
