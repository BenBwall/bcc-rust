//! GNU assembly statements lowered to `asm` instructions: register outputs
//! through fresh slots assigned afterwards, `+` operands as an output and a
//! tied input, memory operands by address, immediates as constants, the
//! template in LLVM's syntax, and `asm goto` as a selector and a `switch`.

use super::*;

/// The lines of the lowered module that contain `needle`.
#[track_caller]
fn lines_with(source: &str, needle: &str) -> Vec<String> {
    let ir = lower_for(source, Target::LinuxGnu).unwrap_or_else(|errors| panic!("{errors:?}"));
    ir.lines()
        .filter(|line| line.contains(needle))
        .map(|line| line.trim().to_owned())
        .collect()
}

#[test]
fn update_operands_become_an_output_and_a_tied_input() {
    let ir = lower_for(
        "int f(int x) { __asm__(\"addl %1, %0\" : \"+r\"(x) : \"g\"(5)); return x; }",
        Target::LinuxGnu,
    )
    .unwrap();
    assert!(
        ir.contains(
            "asm \"addl $1, $0\" (out \"r\" i32 v1, in \"i\" i32 v2, in \"0\" i32 v0)\n    v3 = \
             load.i32 v1, align 4\n    return v3"
        ),
        "{ir}"
    );
}

#[test]
fn templates_escape_dollars_and_keep_modifiers() {
    assert_eq!(
        lines_with(
            "long f(long x) { __asm__ volatile(\"movl $1, %k0 # %%%=\" : \"=r\"(x)); return x; }",
            "asm"
        ),
        ["asm sideeffect \"movl $$1, ${0:k} # %${:uid}\" (out \"r\" i64 v1)"]
    );
    // Basic assembly substitutes nothing, but `$` is still LLVM's escape.
    assert_eq!(
        lines_with("void f(void) { __asm__(\"movl $1, %eax\"); }", "asm"),
        ["asm sideeffect \"movl $$1, %eax\" ()"]
    );
}

/// A memory operand keeps a scalar local in memory; registers named by
/// their constraint letters and the clobbers carry over.
#[test]
fn memory_operands_pass_addresses_and_clobbers_carry_over() {
    let ir = lower_for(
        "int f(void) { int m = 5; unsigned lo, hi; __asm__(\"rdtsc; addl $1, %2\" : \"=a\"(lo), \
         \"=d\"(hi), \"+m\"(m) :: \"memory\", \"cc\", \"rcx\", \"xmm1\"); return m + lo + hi; }",
        Target::LinuxGnu,
    )
    .unwrap();
    assert!(
        ir.contains(
            "(out \"{rax}\" i32 v3, out \"{rdx}\" i32 v4, out \"m\" i32 v2) clobber(memory, \
             flags, rcx, xmm1)"
        ),
        "{ir}"
    );
}

/// A float in a general register, and an integer in an SSE register, keep
/// their bits.
#[test]
fn register_classes_reinterpret_the_other_kind_of_value() {
    let ir = lower_for(
        "double f(double d, long i) { __asm__(\"\" : \"+r\"(d) : \"x\"(i)); return d; }",
        Target::LinuxGnu,
    )
    .unwrap();
    assert!(ir.contains("bitcast.i64"), "{ir}");
    assert!(ir.contains("bitcast.f64"), "{ir}");
    assert!(ir.contains("in \"x\" f64"), "{ir}");
}

/// A wider matching input keeps the low bits the narrower output reads,
/// as GCC's `ll_to_int` (`20030222-1.c`) and `stkalign.c` expect.
#[test]
fn matching_inputs_narrow_to_their_output() {
    for (source, expected) in [
        (
            "int f(long long x) { int i; __asm__(\"\" : \"=r\"(i) : \"0\"(x)); return i; }",
            "trunc.i32",
        ),
        (
            "unsigned f(void) { unsigned x; __asm__(\"\" : \"=g\"(x) : \"0\"(&x)); return x; }",
            "ptrtoint.i64",
        ),
    ] {
        let ir = lower_for(source, Target::LinuxGnu).unwrap();
        assert!(ir.contains(expected), "{source}\n{ir}");
        assert!(ir.contains("in \"0\" i32"), "{source}\n{ir}");
    }
}

#[test]
fn matching_inputs_widen_to_their_output() {
    let ir = lower_for(
        "long f(short s) { long r; __asm__(\"\" : \"=r\"(r) : \"0\"(s)); return r; }",
        Target::LinuxGnu,
    )
    .unwrap();
    assert!(ir.contains("sext.i64"), "{ir}");
    assert!(ir.contains("in \"0\" i64"), "{ir}");
}

/// `asm goto` sets a selector the IR switches on: 0 falls through and
/// label `N` is `N + 1`.
#[test]
fn asm_goto_dispatches_on_a_selector() {
    let ir = lower_for(
        "int f(int x) { __asm__ goto(\"testl %0, %0; jz %l1; js %l2\" :: \"r\"(x) :: zero, \
         negative); return 1; zero: return 0; negative: return -1; }",
        Target::LinuxGnu,
    )
    .unwrap();
    let asm = ir
        .lines()
        .find(|line| line.contains("asm "))
        .expect("an asm instruction");
    for expected in [
        "testl $1, $1; jz .Lbcc_asm_goto${:uid}_0; js .Lbcc_asm_goto${:uid}_1",
        "movl $$0, ${0:k}",
        ".Lbcc_asm_goto${:uid}_1:\\u{a}\\u{9}movl $$2, ${0:k}",
        "(out \"&r\" i32 v1, in \"r\" i32 v0)",
    ] {
        assert!(asm.contains(expected), "{expected}: {asm}");
    }
    assert!(
        ir.contains("switch v2, block1, [1: block2, 2: block4]"),
        "{ir}"
    );
}

#[test]
fn symbolic_immediates_are_symbol_addresses() {
    assert_eq!(
        lines_with(
            "int g; void h(void); void f(void) { __asm__(\"\" :: \"i\"(&g), \"i\"(h)); }",
            "asm"
        ),
        ["asm sideeffect \"\" (in \"i\" ptr v0, in \"i\" ptr v1)"]
    );
}
