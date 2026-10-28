//! `_BitInt(N)` of up to 128 bits in the IR integer of its storage,
//! extended from N bits, and passed at calls as each target's convention
//! classifies it. Wider ones are refused.

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

#[test]
fn bit_ints_wider_than_128_bits_are_refused() {
    for source in [
        "int f(_BitInt(129) *p) { return *p; }",
        "int f(int i) { return (int)(_BitInt(129))i; }",
        "struct S { _BitInt(200) x; }; extern struct S s; int f(void) { return s.x; }",
        "_BitInt(129) g(_BitInt(129) a); int h(void) { g(0); return 0; }",
        "int f(void) { switch ((_BitInt(200))1) { case 1: return 1; default: return 0; } }",
    ] {
        let errors = lower(source).expect_err("refused");
        assert!(
            errors.iter().all(|error| error
                == "not yet supported by lowering: _BitInt types of more than 128 bits"),
            "{source}: {errors:?}"
        );
    }
    // Its size and alignment are constants.
    drop(lowered(
        "int f(void) { return sizeof(_BitInt(129)) + _Alignof(_BitInt(1000)); }",
    ));
}
