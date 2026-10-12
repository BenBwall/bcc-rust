//! Verifier boundaries for the scalar intrinsic instruction formats.
use super::*;

#[test]
fn gcc_scalar_ir_rejects_invalid_controlling_and_operand_types() {
    for (operation, parameter, invalid) in [
        ("bswap.i8", "i8", "invalid"),
        ("clz.f32", "f32", "invalid"),
        ("dyn_alloca.i32", "i32", "invalid"),
        ("popcount.i32", "i64", "expected i32"),
    ] {
        let text = format!(
            "function @f({parameter}) external {{\nblock0(v0: {parameter}):\n    v1 = {operation} \
             v0\n    return\n}}\n"
        );
        let errors = verify_text(&text);
        assert!(!errors.is_empty(), "{operation} unexpectedly verified");
        if invalid != "invalid" {
            assert!(errors.iter().any(|e| e.contains(invalid)), "{errors:?}");
        }
    }
    let text = "function @f(i32, i64) external {\nblock0(v0: i32, v1: i64):\n    v2 = \
                uadd_overflow.i32 v0, v1\n    return\n}\n";
    assert!(
        verify_text(text)
            .iter()
            .any(|e| e.contains("operand 1 is i64, expected i32"))
    );
}
