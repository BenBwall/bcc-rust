//! Inline assembly: operand registers, the substituted template, saved
//! registers, flag outputs, spilled values around a statement, and the
//! refusals. Listings with inline assembly are assembled, never encoded
//! directly, so these tests read the text without the encoder check.

use super::{
    function_text,
    refusal,
    spilled,
};
use crate::{
    backend::native::emit_assembly,
    ir::parse_module,
    target::Target,
    util::bump::Bump,
};

/// The listing of a textual post-ABI module for `target`.
fn listing(ir: &str, target: Target) -> String {
    let arena = Bump::new();
    let module = parse_module(&arena, ir).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    let mut text = String::new();
    emit_assembly(&module, target, &mut text).unwrap_or_else(|error| panic!("{error}\n{ir}"));
    text
}

/// The text between `#APP` and `#NO_APP` of each statement, in order.
fn statements(text: &str) -> Vec<&str> {
    text.split("#APP\n")
        .skip(1)
        .map(|rest| rest.split("\n#NO_APP").next().unwrap())
        .collect()
}

/// `cpuid` with fixed registers, an output sharing its input's register,
/// and `%rbx`, which the prologue saves.
const CPUID: &str = "abi lowered
function @cpuid(i32, ptr, ptr) external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32, v1: ptr, v2: ptr):
    v3 = stack_addr slot0
    v4 = stack_addr slot1
    v5 = iconst.i32 0
    asm sideeffect \"cpuid # $0 $1 ${2:k}\" (out \"{rax}\" i32 v3, out \"{rbx}\" i32 v4, in \
                     \"{rax}\" i32 v0, in \"{rcx}\" i32 v5)
    v6 = load.i32 v3, align 4
    store.i32 v6, v1, align 4
    v7 = load.i32 v4, align 4
    store.i32 v7, v2, align 4
    return
}
";

#[test]
fn fixed_registers_are_set_around_the_statement() {
    let text = listing(CPUID, Target::LinuxGnu);
    let cpuid = function_text(&text, "cpuid");
    assert_eq!(statements(cpuid), ["\tcpuid # %eax %ebx %eax"]);
    assert!(cpuid.contains("pushq\t%rbx"), "{cpuid}");
    assert!(cpuid.contains("popq\t%rbx"), "{cpuid}");
    let before = cpuid.split("#APP").next().unwrap();
    assert!(before.contains("%rcx"), "the input is set before:\n{cpuid}");
}

/// Free registers, immediates, symbols, memory, modifiers, `%=` and the
/// AT&T dialect alternative.
#[test]
fn templates_substitute_every_location() {
    let text = listing(
        "abi lowered
global @g external size 4, align 4 = zero
function @f(i64, i32, ptr) -> i64 external {
    slot0 = stack_slot 8, align 8
block0(v0: i64, v1: i32, v2: ptr):
    v3 = stack_addr slot0
    v4 = iconst.i32 7
    v5 = global_addr @g
    asm \"op $0 ${0:k} ${0:b} ${0:w} ${0:q} ${0:z} $1 ${1:c} ${1:n} $2 ${2:P} $3 ${3:H} ${:uid} \
         $(att$|intel$) $$5\" (out \"&r\" i64 v3, in \"i\" i32 v4, in \"i\" ptr v5, in \"m\" i32 \
         v2)
    v6 = load.i64 v3, align 8
    return v6
}
",
        Target::LinuxGnu,
    );
    // The early-clobber output takes the first free register, the memory
    // operand's address the next.
    assert_eq!(
        statements(&text),
        ["\top %rax %eax %al %ax %rax q $7 7 -7 $g g (%rcx) 8(%rcx) 0 att $5"]
    );
}

/// A flag output is read with `setcc` right after the statement.
#[test]
fn flag_outputs_are_set_from_the_condition() {
    let text = listing(
        "abi lowered
function @eq(i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    asm \"cmpl $2, $1\" (out \"@ccz\" i32 v2, in \"r\" i32 v0, in \"r\" i32 v1)
    v3 = load.i32 v2, align 4
    return v3
}
",
        Target::LinuxGnu,
    );
    let after = text.split("#NO_APP\n").nth(1).unwrap();
    assert!(after.starts_with("\tsete\t"), "{text}");
}

/// Clobbered callee-saved registers are saved, `%xmm6` too on Win64.
#[test]
fn clobbered_callee_saved_registers_are_saved() {
    let ir = "abi lowered
function @f() external {
block0:
    asm sideeffect \"\" () clobber(memory, rbx, r12, xmm6)
    return
}
";
    let linux = listing(ir, Target::LinuxGnu);
    assert!(
        linux.contains("pushq\t%rbx") && linux.contains("pushq\t%r12"),
        "{linux}"
    );
    let windows = listing(ir, Target::WindowsGnu);
    assert!(windows.contains("%xmm6"), "{windows}");
}

/// With every value spilled, the moves into the statement's registers
/// read their slots directly, and the allocation checker accepts it.
#[test]
fn spilled_values_reach_the_statement() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let text = spilled(CPUID, target);
        assert_eq!(statements(&text).len(), 1, "{text}");
    }
}

#[test]
fn statements_the_back_end_cannot_place_are_refused() {
    let wrap = |inst: &str| {
        format!(
            "abi lowered\nfunction @f(i32) external {{\n    slot0 = stack_slot 4, align \
             4\nblock0(v0: i32):\n    v1 = stack_addr slot0\n    {inst}\n    return\n}}\n"
        )
    };
    for (inst, reason) in [
        (
            "asm \"\" () clobber(rbp)",
            "inline assembly with a clobbered %rsp or %rbp",
        ),
        (
            "asm \"$0\" (out \"@ccz\" i32 v1)",
            "inline assembly with an operand modifier the native listing does not write",
        ),
        (
            "asm \"${0:h}\" (in \"{rsi}\" i32 v0)",
            "inline assembly with an operand modifier the native listing does not write",
        ),
        (
            "asm \"\" (in \"{rax}\" i32 v0, in \"{rax}\" i32 v0)",
            "inline assembly with operands that need the same register",
        ),
        (
            "asm \"\" (in \"r\" i32 v0) clobber(rax, rcx, rdx, rbx, rsi, rdi, r8, r9, r12, r13, \
             r14, r15)",
            "inline assembly with more register operands than free registers",
        ),
    ] {
        let message = refusal(&wrap(inst), Target::LinuxGnu);
        assert!(
            message.ends_with(&format!("{reason} (in @f)")),
            "{inst}: {message}"
        );
    }
}

/// A frame object is addressed directly, so a memory output (as an
/// `asm goto` selector) still has a location when the statement clobbers
/// every general register.
#[test]
fn frame_memory_needs_no_register() {
    let text = listing(
        "abi lowered
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    asm sideeffect \"movl $$1, $0\" (out \"m\" i32 v0) clobber(rax, rcx, rdx, rbx, rsi, rdi, r8, \
         r9, r10, r11, r12, r13, r14, r15)
    v1 = load.i32 v0, align 4
    return v1
}
",
        Target::LinuxGnu,
    );
    let [statement] = statements(&text)[..] else {
        panic!("{text}");
    };
    assert!(
        statement.starts_with("\tmovl $1, ") && statement.ends_with("(%rsp)"),
        "{statement}"
    );
}

/// A `+m` operand's output and read share one address register.
#[test]
fn update_memory_operands_share_their_address() {
    let text = listing(
        "abi lowered
function @f(ptr) external {
block0(v0: ptr):
    asm \"addl $$1, $0 # $1\" (out \"m\" i32 v0, in \"m\" i32 v0)
    return
}
",
        Target::LinuxGnu,
    );
    let [statement] = statements(&text)[..] else {
        panic!("{text}");
    };
    let (first, second) = statement
        .strip_prefix("\taddl $1, ")
        .and_then(|rest| rest.split_once(" # "))
        .unwrap_or_else(|| panic!("{statement}"));
    assert_eq!(first, second, "{statement}");
    let before = text.split("#APP").next().unwrap();
    assert_eq!(before.matches("movq").count(), 1, "{text}");
}

/// `%V` is the naked 64-bit register, whatever the operand's width, as GCC
/// prints it: an `int` in `%eax` is `rax`, so `movq %%%V0` is `movq %rax`.
/// GNU extension: GCC manual, "x86 Operand Modifiers".
#[test]
fn naked_registers_are_64_bits_wide() {
    let text = listing(
        "abi lowered
function @f(i32, i8) external {
block0(v0: i32, v1: i8):
    asm sideeffect \"movq %${0:V}, %r11 # ${1:V} $0 $1\" (in \"{rax}\" i32 v0, in \"{rbx}\" i8 v1) \
         clobber(r11)
    return
}
",
        Target::LinuxGnu,
    );
    assert_eq!(statements(&text), ["\tmovq %rax, %r11 # rbx %eax %bl"]);
}

/// The most constrained operand chooses first: with `%rcx`-`%rbx`
/// clobbered, the `Q` input takes `%rax` and the unrestricted early-clobber
/// output another register, as GCC assigns
/// `asm ("movl %1,%0" : "=&r"(out) : "Q"(x) : "rcx", "rdx", "rbx")`.
#[test]
fn constrained_operands_choose_registers_first() {
    let text = listing(
        "abi lowered
function @f(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    asm \"movl $1, $0\" (out \"&r\" i32 v1, in \"{rax,rcx,rdx,rbx}\" i32 v0) clobber(rcx, rdx, rbx)
    v2 = load.i32 v1, align 4
    return v2
}
",
        Target::LinuxGnu,
    );
    let [statement] = statements(&text)[..] else {
        panic!("{text}");
    };
    let (input, output) = statement
        .strip_prefix("\tmovl %")
        .and_then(|rest| rest.split_once(", %"))
        .unwrap_or_else(|| panic!("{statement}"));
    assert_eq!(input, "eax", "{statement}");
    assert!(
        !["eax", "ecx", "edx", "ebx"].contains(&output),
        "{statement}"
    );
}
