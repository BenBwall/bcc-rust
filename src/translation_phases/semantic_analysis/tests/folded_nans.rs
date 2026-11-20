//! The NaN an invalid floating operation folds to ([`NanSource`]): the
//! positive quiet NaN GCC's and Clang's arithmetic give, except where GCC
//! leaves the operation for run time, in a function while math may trap,
//! and where Clang calls a complex routine; there it is the x86-64 default
//! NaN, sign bit set. C99: §F.7.4 paragraph 1, p. 449; PDF p. 461; §F.7.5
//! paragraph 1, p. 450; PDF p. 462.
//!
//! [`NanSource`]: super::super::constants::NanSource

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        CompilerIdentity,
        ExtensionPolicy,
        RelaxedFloatingPoint,
    },
    ir::Type,
};

const POSITIVE: u128 = 0x7FF8_0000_0000_0000;
const NEGATIVE: u128 = 0xFFF8_0000_0000_0000;

/// The identities and options the NaN depends on.
#[derive(Clone, Copy, Debug)]
enum Rules {
    Clang,
    Gcc,
    GccNoTrappingMath,
}

/// The `double` bits of the real part of every folded `operator`
/// expression in `source`, in order of analysis.
fn folded(source: &str, rules: Rules, operator: BinaryOperator) -> Vec<u128> {
    let identity = match rules {
        | Rules::Clang => CompilerIdentity::Clang,
        | Rules::Gcc | Rules::GccNoTrappingMath => CompilerIdentity::Gcc,
    };
    let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_gnu_extensions(true)
        .with_compiler_identity(identity)
        .with_relaxed_floating_point(
            RelaxedFloatingPoint::NoTrappingMath,
            matches!(rules, Rules::GccNoTrappingMath),
        );
    let mut bits = Vec::new();
    with_configuration(source, configuration, |context, unit| {
        assert_eq!(context.pending_error_count(), 0, "{rules:?}: {source}");
        bits.extend(
            unit.expressions
                .iter()
                .filter(|info| {
                    matches!(info.expression.kind, ExpressionType::Binary { operator: o, .. } if o == operator)
                })
                .map(|info| {
                    let value = info.floating().expect("a folded value");
                    crate::float_parsing::bits::constant_bits(&value.real, Type::F64).unwrap()
                }),
        );
    });
    bits
}

/// A static initializer folds as the compiler's arithmetic does, positive,
/// under every identity: at file scope and for a block-scope static.
#[test]
fn a_static_initializer_folds_the_positive_nan() {
    for rules in [Rules::Clang, Rules::Gcc, Rules::GccNoTrappingMath] {
        for source in [
            "double d = 0.0 / 0.0;",
            "static double d = (1.0 / 0.0 - 1.0 / 0.0) / 1.0;",
            "void f(void) { static double d = 0.0 / 0.0; (void)d; }",
            "struct S { double d; } s = { 0.0 / 0.0 };",
        ] {
            let bits = folded(source, rules, BinaryOperator::Division);
            assert_eq!(bits.last(), Some(&POSITIVE), "{rules:?}: {source}");
        }
    }
}

/// In a function Clang folds the operation, and GCC does only without
/// trapping math; otherwise it computes it at run time, where x86-64 gives
/// its negative default NaN.
#[test]
fn a_function_folds_the_nan_its_compiler_computes() {
    for (rules, expected) in [
        (Rules::Clang, POSITIVE),
        (Rules::Gcc, NEGATIVE),
        (Rules::GccNoTrappingMath, POSITIVE),
    ] {
        for source in [
            "void f(void) { double d = 0.0 / 0.0; (void)d; }",
            "double f(void) { return 0.0 / 0.0; }",
            "void g(double); void f(void) { g(0.0 / 0.0); }",
        ] {
            assert_eq!(
                folded(source, rules, BinaryOperator::Division),
                [expected],
                "{rules:?}: {source}"
            );
        }
    }
}

/// A NaN operand passes through with its sign, as GCC's `const_binop` and
/// LLVM's `APFloat` return it, and a negation flips the folded NaN.
#[test]
fn a_nan_operand_keeps_its_sign() {
    for rules in [Rules::Clang, Rules::Gcc] {
        assert_eq!(
            folded(
                "double a = -(0.0 / 0.0) + 1.0, b = (0.0 / 0.0) + 1.0, c = -(0.0 / 0.0) + (0.0 / \
                 0.0);",
                rules,
                BinaryOperator::Addition
            ),
            [NEGATIVE, POSITIVE, NEGATIVE],
            "{rules:?}"
        );
        assert_eq!(
            folded(
                "double a = 1.0 - (0.0 / 0.0), b = 1.0 - -(0.0 / 0.0);",
                rules,
                BinaryOperator::Subtraction
            ),
            [POSITIVE, NEGATIVE],
            "{rules:?}"
        );
    }
}

/// `float` and `long double` fold their own positive NaNs.
#[test]
fn every_standard_format_folds_the_positive_nan() {
    let source = "float f = 0.0f / 0.0f; long double l = 0.0L / 0.0L;";
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow),
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let bits: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| {
                    matches!(
                        info.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Division,
                            ..
                        }
                    )
                })
                .map(|info| info.floating().unwrap().real.x87_parts().unwrap())
                .collect();
            assert_eq!(
                bits,
                [(false, 0x7FFF, 0xC000_0000_0000_0000); 2],
                "the carrier's sign, exponent and significand"
            );
        },
    );
}

/// The quotient of two complex values is `__divdc3`'s at run time under
/// Clang in a function, and the compiler's in a static initializer.
#[test]
fn a_complex_quotient_in_a_function_is_the_routines() {
    let quotient = "(0.0 + 0.0i) / (0.0 + 0.0i)";
    assert_eq!(
        folded(
            &format!("_Complex double z = {quotient};"),
            Rules::Clang,
            BinaryOperator::Division
        ),
        [POSITIVE]
    );
    for rules in [Rules::Clang, Rules::Gcc] {
        assert_eq!(
            folded(
                &format!("void f(void) {{ _Complex double z = {quotient}; (void)z; }}"),
                rules,
                BinaryOperator::Division
            ),
            [NEGATIVE],
            "{rules:?}"
        );
    }
    // Clang's inline product folds when only one part is a NaN.
    assert_eq!(
        folded(
            "void f(void) { _Complex double z = (__builtin_inf() + 0.0i) * (0.0 + 1.0i); (void)z; \
             }",
            Rules::Clang,
            BinaryOperator::Multiplication
        )
        .last(),
        Some(&POSITIVE)
    );
}

/// Binary128 folds the same way: positive in a static initializer, and
/// libgcc's negative default NaN where GCC leaves the operation for run
/// time.
#[test]
fn binary128_folds_the_nan_its_compiler_computes() {
    for (source, rules, negative) in [
        ("__float128 q = 0.0Q / 0.0Q;", Rules::Gcc, false),
        ("__float128 q = 0.0Q / 0.0Q;", Rules::Clang, false),
        (
            "void f(void) { __float128 q = 0.0Q / 0.0Q; (void)q; }",
            Rules::Clang,
            false,
        ),
        (
            "void f(void) { __float128 q = 0.0Q / 0.0Q; (void)q; }",
            Rules::Gcc,
            true,
        ),
    ] {
        let identity = match rules {
            | Rules::Clang => CompilerIdentity::Clang,
            | _ => CompilerIdentity::Gcc,
        };
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_compiler_identity(identity),
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0, "{source}");
                let value = unit
                    .expressions
                    .iter()
                    .find(|info| {
                        matches!(
                            info.expression.kind,
                            ExpressionType::Binary {
                                operator: BinaryOperator::Division,
                                ..
                            }
                        )
                    })
                    .and_then(|info| info.binary128())
                    .expect("a folded binary128 value");
                assert!(value.real.is_nan(), "{source}");
                assert_eq!(value.real.is_negative(), negative, "{rules:?}: {source}");
            },
        );
    }
}
