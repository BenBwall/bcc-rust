//! `_BitInt(N)` of up to 128 bits in the IR integer of its storage,
//! extended from N bits, and passed at calls as each target's convention
//! classifies it; wider ones in memory, computed by the runtime's routines.

use super::*;

/// An unsigned sum wraps at its width with a mask, a signed one extends
/// from its sign bit with a shift pair, and a bitwise operation of
/// extended operands needs neither.
#[test]
fn narrow_results_are_extended_from_their_width() {
    let text = functions(&lowered(
        "unsigned _BitInt(7) u(unsigned _BitInt(7) a, unsigned _BitInt(7) b) { return a + b; \
         }\n_BitInt(7) s(_BitInt(7) a, _BitInt(7) b) { return a * b; }\n_BitInt(7) x(_BitInt(7) \
         a, _BitInt(7) b) { return a ^ b; }",
    ));
    let body = |name: &str| {
        let start = text
            .find(&format!("@{name}("))
            .expect("the function is lowered");
        let end = text[start..]
            .find("\n}")
            .map_or(text.len(), |end| start + end);
        text[start..end].to_owned()
    };
    // Each parameter is extended on entry, then the result once more.
    let unsigned = body("u");
    assert_eq!(unsigned.matches("and.i8").count(), 3, "{unsigned}");
    let signed = body("s");
    assert_eq!(signed.matches("ashr.i8").count(), 3, "{signed}");
    let bitwise = body("x");
    assert_eq!(bitwise.matches("ashr.i8").count(), 2, "{bitwise}");
}

/// System V and Win64 describe a 65-128-bit value as the aggregate of its
/// two limbs, 8-aligned; WebAssembly passes the `i128` itself.
#[test]
fn wide_arguments_cross_calls_as_limbs_on_x86_64() {
    let source = "_BitInt(65) f(_BitInt(65) a) { return a; }";
    for target in [Target::LinuxGnu, Target::WindowsMsvc] {
        let text = lower_for(source, target).expect("lowers");
        assert!(
            text.contains("result(size 16, align 8, [0: i64, 8: i64])"),
            "{target:?}\n{text}"
        );
        assert!(
            text.contains("aggregate(size 16, align 8, [0: i64, 8: i64])"),
            "{target:?}\n{text}"
        );
    }
    let text = lower_for(source, Target::Wasm32Wasi).expect("lowers");
    assert!(!text.contains("aggregate("), "{text}");
    assert!(text.contains("@f(i128 noundef) -> i128"), "{text}");
}

/// A `_BitInt` of more than 128 bits is an object of its limbs: its
/// operations call the runtime's routines, which lowering declares as
/// internal functions for the middle end to define, and a call passes it as
/// an aggregate of its size, 8-aligned, which both x86-64 conventions pass
/// in memory.
#[test]
fn wide_bit_ints_are_objects_that_call_the_runtime() {
    let source = "_BitInt(200) f(_BitInt(200) a, _BitInt(200) b) { return a * b + 1; }";
    for target in [Target::LinuxGnu, Target::WindowsMsvc] {
        let text = lower_for(source, target).expect("lowers");
        for wanted in [
            "ptr result(size 32, align 8)",
            "ptr aggregate(size 32, align 8) noundef",
            "call @bcc.bitint.mul(",
            "call @bcc.bitint.add(",
            "function @bcc.bitint.mul(ptr, ptr, ptr, i32, i32) internal",
        ] {
            assert!(text.contains(wanted), "{target:?} {wanted}\n{text}");
        }
    }
    // Its size and alignment are constants.
    drop(lowered(
        "int f(void) { return sizeof(_BitInt(129)) + _Alignof(_BitInt(1000)); }",
    ));
}

/// A folded wide constant is an internal constant object of its limbs,
/// its padding bits extended, and a static object stores its limbs.
#[test]
fn wide_constants_are_objects_of_their_limbs() {
    let text = lowered("_BitInt(129) g = -2;\n_BitInt(129) f(void) { return (_BitInt(129))-3; }");
    assert!(
        text.contains(
            "global @g external size 24, align 8 = bytes \
             \"feffffffffffffffffffffffffffffffffffffffffffffff\""
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "global @.bitint internal constant size 24, align 8 = bytes \
             \"fdffffffffffffffffffffffffffffffffffffffffffffff\""
        ),
        "{text}"
    );
}

/// An unsigned 128-bit constant of 2^127 or more converts to a wide type
/// with zero limbs above it, not copies of its top bit (GCC's torture test
/// gcc.dg/torture/bitint-43.c passes `(unsigned _BitInt(128))-1` this way).
#[test]
fn unsigned_128_bit_constants_zero_extend_into_wide_types() {
    let text = lowered(
        "_BitInt(156) f(void) { return (unsigned __int128)-1; }\n_BitInt(156) g(void) { return \
         (__int128)-1; }",
    );
    assert!(
        text.contains(
            "global @.bitint internal constant size 24, align 8 = bytes \
             \"ffffffffffffffffffffffffffffffff0000000000000000\""
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "global @.bitint.1 internal constant size 24, align 8 = bytes \
             \"ffffffffffffffffffffffffffffffffffffffffffffffff\""
        ),
        "{text}"
    );
}

/// A switch on a wide value compares it with each case in turn.
#[test]
fn wide_switches_compare_with_each_case() {
    let text = lowered(
        "int f(_BitInt(300) x) { switch (x) { case 1: return 1; case 5: case 9: return 2; \
         default: return 0; } }",
    );
    assert_eq!(text.matches("call @bcc.bitint.cmp(").count(), 3, "{text}");
}

/// A conversion between a wide integer and a complex type is refused.
#[test]
fn wide_complex_conversions_are_refused() {
    let errors = lower("double _Complex f(_BitInt(300) x) { return x; }").expect_err("refused");
    assert!(
        errors.iter().all(|error| error
            == "not yet supported by lowering: conversions between complex types and _BitInt \
                types of more than 128 bits"),
        "{errors:?}"
    );
}

/// The middle end defines the routines a module calls, and those they
/// call, with the bodies bcc compiles from the runtime, as internal
/// functions; routines nothing calls stay out.
#[test]
fn the_middle_end_links_the_routines_a_module_calls() {
    let tu = Bump::new();
    // A program's function named as a routine of `runtime.c` keeps its name.
    let source = tu.alloc_str(
        "void __bcc_bitint_div(int x) { (void)x; }\n_BitInt(300) f(_BitInt(300) a, _BitInt(300) \
         b) { __bcc_bitint_div(1); return a / b; }\n",
    );
    let mut context =
        Context::with_configuration(&tu, crate::configuration::CompilerConfiguration::default());
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let arena = Bump::new();
    let module = crate::pipeline::lower_translation_unit(
        &context,
        &unit,
        &sema,
        &arena,
        None,
        Profile::PreAbi,
    )
    .expect("lowers");
    let defined = |name: &str| {
        module
            .functions()
            .any(|(_, function)| function.name == name && function.body.is_some())
    };
    assert!(defined("bcc.bitint.div"), "{module}");
    assert!(defined("__bcc_bitint_div"), "{module}");
    assert!(!defined("bcc.bitint.mul"), "{module}");
    assert!(
        module
            .functions()
            .filter(|(_, function)| function.name.starts_with("bcc.bitint."))
            .all(
                |(_, function)| function.linkage == crate::ir::Linkage::Internal
                    && function.body.is_some()
            ),
        "{module}"
    );
}

/// A wide value becomes a `__bf16` rounded once for GCC, which its own
/// routine does, and through `float` for Clang, as each compiler converts.
#[test]
fn wide_bfloat_conversions_round_as_each_compiler_does() {
    use crate::configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    };
    let source = "__bf16 f(_BitInt(300) x) { return x; }";
    for (identity, routine) in [
        (CompilerIdentity::Gcc, "call @bcc.bitint.to_bf16("),
        (CompilerIdentity::Clang, "call @bcc.bitint.to_f32("),
    ] {
        let configuration = CompilerConfiguration::default().with_compiler_identity(identity);
        let text = lower_with(source, configuration).expect("lowers");
        assert!(text.contains(routine), "{identity:?}\n{text}");
        assert!(text.contains("fptrunc.bf16"), "{identity:?}\n{text}");
    }
}

/// Wide results share their stack slots between statements, so a long run
/// of operations takes a few slots, not one per operation.
#[test]
fn wide_results_share_slots_between_statements() {
    let mut source = String::from("_BitInt(300) f(_BitInt(300) a) {\n");
    for _ in 0..50 {
        source.push_str("    a = a * 3 + 1;\n");
    }
    source.push_str("    return a;\n}");
    let text = lowered(&source);
    let slots = text.matches("stack_slot").count();
    assert!(slots <= 4, "{slots} slots\n{text}");
}

/// Within one expression a wide result frees its slot once read, so a long
/// chain takes a few slots and a balanced tree one per level.
#[test]
fn wide_results_share_slots_within_an_expression() {
    let chain = (1..200).fold(String::from("v[0]"), |chain, index| {
        format!("{chain} {} v[{}]", ["+", "-", "^"][index % 3], index % 8)
    });
    let text = lowered(&format!(
        "_BitInt(300) f(_BitInt(300) *v) {{ return {chain}; }}"
    ));
    let slots = text.matches("stack_slot").count();
    assert!(slots <= 3, "{slots} slots\n{text}");
}

/// A GNU `x ?: y` with a wide condition or result passes the tested
/// value's object to the merge.
#[test]
fn wide_omitted_conditionals_lower() {
    for source in [
        "_BitInt(200) f(_BitInt(200) g) { return g ?: 9; }",
        "_BitInt(200) f(int c, _BitInt(200) y) { return c ?: y; }",
        "int f(_BitInt(200) g) { return g ?: 4; }",
        "int f(_BitInt(200) g) { if (g ?: 0) return 1; return 0; }",
    ] {
        drop(lowered(source));
    }
}

/// A function whose wide results would need more stack than a default
/// thread has is refused, not compiled to overflow its stack.
#[test]
fn wide_results_beyond_the_stack_are_refused() {
    use crate::configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    };
    let source = "typedef _BitInt(8388608) H; H g(H, H, H, H, H, H, H);\nH f(H a) { return g(a * \
                  2, a * 3, a * 4, a * 5, a * 6, a * 7, a * 8); }";
    let configuration =
        CompilerConfiguration::default().with_compiler_identity(CompilerIdentity::Clang);
    let errors = lower_with(source, configuration).expect_err("refused");
    assert!(
        errors.iter().all(|error| error
            == "not yet supported by lowering: functions whose wide _BitInt results need more \
                than 6 MiB of stack"),
        "{errors:?}"
    );
}
