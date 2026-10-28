//! Imaginary interchange constants (GNU): a complex type of the real
//! component's, whose imaginary part has the value a real constant of the
//! suffix has, excess precision included. C23 §H.5.2p2.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        CompilerIdentity,
        ExtensionPolicy,
    },
    float_parsing::LongDouble,
};

/// The spelled type and imaginary part of the initializer of `x` in
/// `source`, under `identity`.
fn imaginary(source: &str, identity: CompilerIdentity) -> (String, LongDouble) {
    let mut result = (String::new(), LongDouble::ZERO);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(identity),
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0, "{source}");
            let info = unit
                .expressions
                .iter()
                .find(|info| {
                    matches!(
                        info.expression.kind,
                        ExpressionType::Constant(Constant::Float(_))
                    )
                })
                .expect("a floating constant");
            result.0 = format!("{:?}", unit.types.kind(info.ty));
            let value = info.floating().expect("a folded value");
            assert!(value.real.is_zero(), "{source}");
            result.1 = value.imag;
        },
    );
    result
}

#[test]
fn half_constants_keep_each_identitys_value() {
    // GCC evaluates an `f16` constant in `float`; Clang rounds it.
    for (identity, expected) in [
        (CompilerIdentity::Gcc, f64::from(0.1_f32)),
        (CompilerIdentity::Clang, 0.099_975_585_937_5),
    ] {
        for source in ["int x = sizeof(0.1f16i);", "int x = sizeof(0.1if16);"] {
            let (ty, value) = imaginary(source, identity);
            assert!(ty.contains("Complex") && ty.contains("Float16"), "{ty}");
            assert!(
                value == LongDouble::from_double(expected),
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn gcc_interchange_constants_have_their_complex_types() {
    for (source, ty, value) in [
        ("int x = sizeof(1.5f32i);", "ComplexFloat32", 1.5),
        ("int x = sizeof(2.5if64);", "ComplexFloat64", 2.5),
        ("int x = sizeof(3.5f32xi);", "ComplexFloat32x", 3.5),
        ("int x = sizeof(4.5if64x);", "ComplexFloat64x", 4.5),
    ] {
        let (spelled, imaginary) = imaginary(source, CompilerIdentity::Gcc);
        assert!(spelled.contains(ty), "{source}: {spelled}");
        assert!(imaginary == LongDouble::from_double(value), "{source}");
    }
}
