//! The prologues and epilogues of [`EntryConvention`]s other than C. A real
//! handler's round trip is in `tests/integration/x86_interrupt.rs`.
//!
//! [`EntryConvention`]: crate::ir::EntryConvention

use super::{
    assembly,
    function_text,
    refusal,
};
use crate::{
    backend::native::emit_assembly,
    ir::parse_module,
    target::Target,
    util::bump::Bump,
};

const LINUX: Target = Target::LinuxGnu;

/// A handler that changes `%rcx` through its parameter's use: its frame
/// address arrives in `%rax`, the error code below it.
const EXCEPTION: &str = "abi lowered\nfunction @h(ptr, i64) external noinline interrupt \
                         {\nblock0(v0: ptr, v1: i64):\n    store.i64 v1, v0, align 8\n    \
                         return\n}\n";

#[test]
fn an_exception_handler_returns_past_its_error_code_through_iretq() {
    let text = assembly(EXCEPTION, LINUX);
    for line in [
        "\tcld\n",
        "\tpushq\t%rbp\n",
        "\tpushq\t%rax\n",
        "\tandq\t$-16, %rsp\n",
        "\tmovq\t%rsp, %rbp\n",
        "\tleaq\t8(%rsp), %rsp\n\tiretq\n",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(!text.contains("retq\n\t.cfi"), "{text}");
    assert!(!text.contains(".cfi_startproc"), "{text}");
    // The frame's address and error code come from memory, not registers.
    assert!(!text.contains("%rdi"), "{text}");
    assert!(!text.contains("%rsi"), "{text}");
}

#[test]
fn an_interrupt_handler_has_no_error_code_to_pop() {
    let text = assembly(
        "abi lowered\nfunction @h(ptr) external interrupt {\nblock0(v0: ptr):\n    return\n}\n",
        LINUX,
    );
    assert!(
        text.contains("%rsp\n\tpopq\t%rax\n\tpopq\t%rbp\n\tiretq"),
        "{text}"
    );
}

#[test]
fn a_calling_handler_saves_what_the_call_may_change() {
    let ir = "abi lowered\nfunction @g() external\nfunction @h(ptr) external interrupt \
              {\nblock0(v0: ptr):\n    call @g()\n    return\n}\n";
    let text = assembly(ir, LINUX);
    for register in ["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"] {
        assert!(
            text.contains(&format!("\tpushq\t%{register}\n")),
            "{register}\n{text}"
        );
        assert!(
            text.contains(&format!("\tpopq\t%{register}\n")),
            "{register}\n{text}"
        );
    }
    // With SSE in use, its registers too; none under -mgeneral-regs-only.
    for register in 0..16 {
        assert!(
            text.contains(&format!("\tmovaps\t%xmm{register}, ")),
            "{register}\n{text}"
        );
    }
    let arena = Bump::new();
    let mut module = parse_module(&arena, ir).unwrap();
    module.set_general_regs_only(true);
    let mut text = String::new();
    emit_assembly(&module, LINUX, &mut text).unwrap();
    assert!(text.contains("\tpushq\t%r11\n"), "{text}");
    assert!(!text.contains("xmm"), "{text}");
}

#[test]
fn user_interrupts_return_through_uiret() {
    let arena = Bump::new();
    let mut module = parse_module(&arena, EXCEPTION).unwrap();
    module.set_user_interrupts(true);
    let mut text = String::new();
    emit_assembly(&module, LINUX, &mut text).unwrap();
    assert!(text.contains("\tleaq\t8(%rsp), %rsp\n\tuiret\n"), "{text}");
    assert!(!text.contains("iretq"), "{text}");
    super::object::equivalent(&text).unwrap();
}

#[test]
fn a_handler_reads_no_return_or_frame_address() {
    for opcode in ["return_address", "frame_address"] {
        let ir = format!(
            "abi lowered\nfunction @h(ptr) external interrupt {{\nblock0(v0: ptr):\n    v1 = \
             {opcode}\n    return\n}}\n"
        );
        assert!(
            refusal(&ir, LINUX).contains("a return or frame address in an interrupt handler"),
            "{opcode}"
        );
    }
}

#[test]
fn a_saving_function_keeps_all_but_its_result() {
    let ir = "abi lowered\nfunction @g() external\nfunction @f(i64) -> i64 external \
              preserve_registers {\nblock0(v0: i64):\n    call @g()\n    return v0\n}\n";
    let text = assembly(ir, LINUX);
    let function = function_text(&text, "f");
    for register in ["rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"] {
        assert!(
            function.contains(&format!("\tpushq\t%{register}\n")),
            "{register}\n{function}"
        );
    }
    assert!(!function.contains("pushq\t%rax"), "{function}");
    assert!(!function.contains("popq\t%rax"), "{function}");
    assert!(function.contains("\tretq\n"), "{function}");
}

#[test]
fn a_naked_function_has_no_code_of_its_own() {
    let text = assembly(
        "abi lowered\nfunction @n() external noinline naked {\nblock0:\n    return\n}\n",
        LINUX,
    );
    let function = function_text(&text, "n");
    assert!(function.contains("\tud2\n"), "{function}");
    for instruction in ["pushq", "popq", "subq", "retq"] {
        assert!(!function.contains(instruction), "{instruction}\n{function}");
    }
}

/// Its frame goes below the entry `%rsp`, which a register variable reads.
#[test]
fn a_naked_function_keeps_its_frame_below_the_entry_stack() {
    let text = assembly(
        "abi lowered\nfunction @n() -> ptr external naked {\n    slot0 = stack_slot 8, align \
         8\nblock0:\n    v0 = stack_save\n    v1 = iconst.i64 1\n    v2 = stack_addr slot0\n    \
         store.i64 v1, v2, align 8\n    return v0\n}\n",
        LINUX,
    );
    let function = function_text(&text, "n");
    assert!(function.contains("\tsubq\t$16, %rsp\n"), "{function}");
    assert!(function.contains("\tleaq\t16(%rsp), %"), "{function}");
}

#[test]
fn a_naked_function_cannot_move_the_stack() {
    let ir = "abi lowered\nfunction @n(i64) external naked {\nblock0(v0: i64):\n    v1 = \
              dyn_alloca.i64 v0, align 16\n    return\n}\n";
    assert!(
        refusal(ir, LINUX).contains("a naked function body that needs a stack frame set up"),
        "{}",
        refusal(ir, LINUX)
    );
}

#[test]
fn general_register_code_refuses_floating_registers() {
    let arena = Bump::new();
    let ir = "abi lowered\nfunction @f(ptr, ptr) external {\nblock0(v0: ptr, v1: ptr):\n    v2 = \
              load.f64 v0, align 8\n    store.f64 v2, v1, align 8\n    return\n}\n";
    let mut module = parse_module(&arena, ir).unwrap();
    module.set_general_regs_only(true);
    let mut text = String::new();
    let error = emit_assembly(&module, LINUX, &mut text)
        .unwrap_err()
        .to_string();
    assert!(error.contains("SSE or x87 registers"), "{error}");
}
