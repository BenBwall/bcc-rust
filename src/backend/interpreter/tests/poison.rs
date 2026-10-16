//! Poison: each fact whose violation produces it, how it propagates through
//! arithmetic, comparisons, conversions and `select`, `freeze`, and the uses
//! of it that are undefined behaviour.

use super::{
    arithmetic::binary,
    *,
};

const POISON: RuntimeValue = RuntimeValue::Poison;

#[test]
fn noundef_byval_pointer_does_not_require_defined_object_bytes() {
    let text = "abi lowered\nfunction @ignore(ptr byval(size 8, align 8) noundef) -> i32 internal \
                {\nblock0(v0: ptr):\n    v1 = iconst.i32 0\n    return v1\n}\nfunction @main() -> \
                i32 external {\n    slot0 = stack_slot 8, align 8\nblock0:\n    v0 = stack_addr \
                slot0\n    v1 = call @ignore(v0)\n    return v1\n}\n";
    assert_eq!(returned(text, "main"), int(0));
    let poison_pointer = text.replace("v0 = stack_addr slot0", "v0 = poison.ptr");
    assert_eq!(undefined(&poison_pointer, "main"), UbKind::Noundef);
}

#[test]
fn noundef_traps_unused_direct_indirect_and_variadic_arguments() {
    for (signature, call) in [
        ("i8 noundef", "call @ignore(v0)"),
        ("i8 noundef", "call_indirect v1(v0) : (i8 noundef) -> i32"),
        ("... noundef", "call @ignore(v0)"),
        ("... noundef", "call_indirect v1(v0) : (... noundef) -> i32"),
    ] {
        let entry = if signature.starts_with("...") {
            "block0:"
        } else {
            "block0(v0: i8):"
        };
        let zero = if signature.starts_with("...") {
            "v0"
        } else {
            "v1"
        };
        let text = format!(
            "function @ignore({signature}) -> i32 internal {{\n{entry}\n    {zero} = iconst.i32 \
             0\n    return {zero}\n}}\nfunction @main() -> i32 external {{\nblock0:\n    v0 = \
             poison.i8\n    v1 = func_addr @ignore\n    v2 = {call}\n    return v2\n}}\n"
        );
        assert_eq!(undefined(&text, "main"), UbKind::Noundef);
    }
}

#[test]
fn noundef_traps_entry_and_returns_including_partial_memory_poison() {
    let entry = "function @f(i32 noundef) -> i32 external {\nblock0(v0: i32):\n    v1 = \
                 iconst.i32 0\n    return v1\n}\n";
    let run = run_text_with(entry, "f", &[POISON], Limits::default());
    assert!(matches!(
        run.result,
        Err(Trap::UndefinedBehavior(UbKind::Noundef, _))
    ));
    let returned = "function @f() -> i32 noundef internal {\nblock0:\n    v0 = poison.i32\n    \
                    return v0\n}\nfunction @main() -> i32 external {\nblock0:\n    v0 = call \
                    @f()\n    v1 = iconst.i32 0\n    return v1\n}\n";
    assert_eq!(undefined(returned, "f"), UbKind::Noundef);
    assert_eq!(undefined(returned, "main"), UbKind::Noundef);
    let loaded = "function @f() -> i32 noundef external {\n    slot0 = stack_slot 4, align \
                  4\nblock0:\n    v0 = stack_addr slot0\n    v1 = iconst.i8 1\n    store.i8 v1, \
                  v0, align 1\n    v2 = load.i32 v0, align 4\n    return v2\n}\n";
    assert_eq!(undefined(loaded, "f"), UbKind::Noundef);
    let registers =
        "abi lowered\nfunction @f(ptr registers(i64, align 8)) -> noundef internal {\nblock0(v0: \
         ptr):\n    v1 = iconst.i32 1\n    store.i32 v1, v0, align 4\n    return\n}\nfunction \
         @main() -> i32 external {\n    slot0 = stack_slot 8, align 8\nblock0:\n    v0 = \
         stack_addr slot0\n    call @f(v0)\n    v1 = iconst.i32 0\n    return v1\n}\n";
    assert_eq!(undefined(registers, "main"), UbKind::Noundef);
}

#[test]
fn noundef_checks_poison_in_the_second_return_register_even_when_unused() {
    let text = "\
abi lowered
function @f(ptr registers(i64, i64, align 8)) -> noundef internal {
block0(v0: ptr):
    v1 = iconst.i64 7
    store.i64 v1, v0, align 8
    v2 = iconst.i64 8
    v3 = ptr_add inbounds v0, v2
    store.i64 v1, v3, align 8
    v4 = iconst.i64 15
    v5 = ptr_add inbounds v0, v4
    v6 = poison.i8
    store.i8 v6, v5, align 1
    return
}
function @main() -> i32 external {
    slot0 = stack_slot 16, align 8
block0:
    v0 = stack_addr slot0
    call @f(v0)
    v1 = iconst.i32 0
    return v1
}
";
    assert_eq!(undefined(text, "main"), UbKind::Noundef);
    let initialized = text.replace("poison.i8", "iconst.i8 9");
    assert_eq!(returned(&initialized, "main"), int(0));
    let unmarked = text.replace(" -> noundef", "");
    assert_eq!(returned(&unmarked, "main"), int(0));
}

#[test]
fn violated_wrap_flags_produce_poison() {
    let cases = [
        ("iadd.i8 nsw", 0x7F, 1, POISON),
        ("iadd.i8 nsw", 0xFF, 1, int(0)),
        ("iadd.i8 nuw", 0xFF, 1, POISON),
        ("iadd.i8 nuw", 0x7F, 1, int(0x80)),
        ("isub.i8 nsw", 0x80, 1, POISON),
        ("isub.i8 nuw", 0, 1, POISON),
        ("isub.i8 nsw", 0, 1, int(0xFF)),
        ("imul.i8 nsw", 0x40, 2, POISON),
        ("imul.i8 nsw", 0xC0, 2, int(0x80)),
        ("imul.i8 nuw", 0x80, 2, POISON),
        ("imul.i8 nuw", 0x7F, 2, int(0xFE)),
        ("shl.i8 nsw", 0x40, 1, POISON),
        ("shl.i8 nsw", 0xC0, 1, int(0x80)),
        ("shl.i8 nuw", 0x80, 1, POISON),
        ("shl.i8 nuw", 0x40, 1, int(0x80)),
        ("shl.i8", 1, 8, POISON),
        ("lshr.i8", 1, 9, POISON),
        ("ashr.i8", 1, 0xFF, POISON),
        ("udiv.i8 exact", 7, 2, POISON),
        ("udiv.i8 exact", 8, 2, int(4)),
        ("sdiv.i8 exact", 0xF9, 2, POISON),
        ("sdiv.i8 exact", 0xF8, 2, int(0xFC)),
        ("lshr.i8 exact", 3, 1, POISON),
        ("ashr.i8 exact", 0xF1, 4, POISON),
        ("ashr.i8 exact", 0xF0, 4, int(0xFF)),
    ];
    for (opcode, a, b, expected) in cases {
        assert_eq!(
            binary(opcode, "i8", a, b),
            expected,
            "{opcode} {a:#x} {b:#x}"
        );
    }
    let max = u128::MAX >> 1;
    assert_eq!(binary("iadd.i128 nsw", "i128", max, 1), POISON);
    assert_eq!(binary("iadd.i128 nuw", "i128", u128::MAX, 1), POISON);
    assert_eq!(binary("imul.i128 nuw", "i128", 1 << 64, 1 << 64), POISON);
    assert_eq!(binary("imul.i64 nsw", "i64", 1 << 32, 1 << 31), POISON);
    assert_eq!(
        binary("imul.i64 nsw", "i64", 1 << 31, 1 << 31),
        int(1 << 62)
    );
    assert_eq!(binary("iadd.i1 nuw", "i1", 1, 1), POISON);
    assert_eq!(binary("iadd.i1 nsw", "i1", 1, 1), POISON);
}

#[test]
fn poison_propagates_through_values() {
    let text = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    v1 = iconst.i32 1
    v2 = iadd.i32 v0, v1
    v3 = icmp.i32 eq v2, v1
    v4 = zext.i32 v3
    v5 = xor.i32 v4, v1
    v6 = udiv.i32 v0, v1
    v7 = or.i32 v5, v6
    return v7
}
";
    assert_eq!(returned(text, "f"), POISON);
    let text = "\
function @f() -> i64 external {
block0:
    v0 = poison.f64
    v1 = fconst.f64 0x3FF0000000000000
    v2 = fadd.f64 v0, v1
    v3 = fptosi.i64 v2
    return v3
}
";
    assert_eq!(returned(text, "f"), POISON);
    let text = "\
function @f() -> i1 external {
block0:
    v0 = poison.f32
    v1 = fcmp.f32 uno v0, v0
    return v1
}
";
    assert_eq!(returned(text, "f"), POISON);
}

#[test]
fn select_uses_only_the_chosen_operand() {
    let select = |cond: &str, a: &str, b: &str| {
        let text = format!(
            "function @f() -> i32 external {{\nblock0:\n    v0 = {cond}\n    v1 = {a}\n    v2 = \
             {b}\n    v3 = select.i32 v0, v1, v2\n    return v3\n}}\n"
        );
        returned(&text, "f")
    };
    let (yes, no) = ("iconst.i1 1", "iconst.i1 0");
    let (seven, poison) = ("iconst.i32 7", "poison.i32");
    assert_eq!(select(yes, seven, poison), int(7));
    assert_eq!(select(no, poison, seven), int(7));
    assert_eq!(select(yes, poison, seven), POISON);
    assert_eq!(select("poison.i1", seven, seven), POISON);
}

#[test]
fn freeze_turns_poison_into_a_fixed_value() {
    let text = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    v1 = freeze.i32 v0
    v2 = isub.i32 v1, v1
    v3 = iconst.i32 5
    v4 = freeze.i32 v3
    v5 = iadd.i32 v2, v4
    return v5
}
";
    assert_eq!(returned(text, "f"), int(5));
    let text = "\
function @f() -> ptr external {
block0:
    v0 = poison.ptr
    v1 = freeze.ptr v0
    return v1
}
";
    assert_eq!(returned(text, "f"), RuntimeValue::NULL);
    let text = "\
function @f() -> i1 external {
block0:
    v0 = poison.i32
    v1 = freeze.i32 v0
    v2 = icmp.i32 eq v1, v1
    brif v2, block1, block2
block1:
    return v2
block2:
    return v2
}
";
    assert_eq!(returned(text, "f"), int(1));
}

#[test]
fn out_of_range_float_conversions_are_poison() {
    let convert = |op: &str, bits: u64| {
        let text = format!(
            "function @f() -> i8 external {{\nblock0:\n    v0 = fconst.f64 0x{bits:X}\n    v1 = \
             {op}.i8 v0\n    return v1\n}}\n"
        );
        returned(&text, "f")
    };
    assert_eq!(convert("fptosi", 300.0_f64.to_bits()), POISON);
    assert_eq!(convert("fptosi", (-128.9_f64).to_bits()), int(0x80));
    assert_eq!(convert("fptosi", (-129.0_f64).to_bits()), POISON);
    assert_eq!(convert("fptoui", 255.5_f64.to_bits()), int(0xFF));
    assert_eq!(convert("fptoui", 256.0_f64.to_bits()), POISON);
    assert_eq!(convert("fptoui", (-1.0_f64).to_bits()), POISON);
    assert_eq!(convert("fptosi", 0x7FF8_0000_0000_0000), POISON);
}

#[test]
fn branching_on_poison_is_undefined() {
    let text = "\
function @f() -> i32 external {
block0:
    v0 = poison.i1
    v1 = iconst.i32 0
    brif v0, block1, block1
block1:
    return v1
}
";
    assert_eq!(undefined(text, "f"), UbKind::BranchOnPoison);
    let text = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    switch v0, block1, [1: block1]
block1:
    return v0
}
";
    assert_eq!(undefined(text, "f"), UbKind::SwitchOnPoison);
}

#[test]
fn division_by_zero_poison_and_overflow_is_undefined() {
    let divide = |op: &str, a: &str, b: &str| {
        let text = format!(
            "function @f() -> i8 external {{\nblock0:\n    v0 = {a}\n    v1 = {b}\n    v2 = \
             {op}.i8 v0, v1\n    return v2\n}}\n"
        );
        run_text(&text, "f")
    };
    let ub = |op: &str, a: &str, b: &str| match divide(op, a, b).result {
        | Err(Trap::UndefinedBehavior(kind, _)) => Some(kind),
        | _ => None,
    };
    let zero = "iconst.i8 0";
    let one = "iconst.i8 1";
    let minus_one = "iconst.i8 -1";
    let minimum = "iconst.i8 -128";
    let poison = "poison.i8";
    for op in ["udiv", "sdiv", "urem", "srem"] {
        assert_eq!(ub(op, one, zero), Some(UbKind::DivisionByZero), "{op}");
        assert_eq!(ub(op, one, poison), Some(UbKind::DivisionByPoison), "{op}");
        assert_eq!(ub(op, poison, one), None, "{op} of poison is poison");
    }
    for op in ["sdiv", "srem"] {
        assert_eq!(
            ub(op, minimum, minus_one),
            Some(UbKind::SignedDivisionOverflow),
            "{op}"
        );
        assert_eq!(
            ub(op, poison, minus_one),
            Some(UbKind::SignedDivisionOverflow),
            "{op}"
        );
    }
    assert_eq!(ub("udiv", minimum, minus_one), None);
    assert_eq!(binary("sdiv.i8", "i8", 0x81, 0xFF), int(0x7F));
}
