//! The NaN an invalid floating operation folds to ([`NanSource`]): the
//! positive quiet NaN GCC's and Clang's arithmetic give in the initializer
//! of an object with static storage duration, which both compute at
//! translation time, and the x86-64 default NaN, sign bit set, in a function
//! body, where bcc computes every invalid operation as the hardware does.
//! C99: §6.7.8 paragraph 4, p. 125; PDF p. 137; §F.7.4 paragraph 1,
//! p. 449; PDF p. 461; §F.7.5 paragraph 1, p. 450; PDF p. 462.
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
    /// Clang's `-ftrapping-math`: strict exception behavior.
    ClangTrappingMath,
    Gcc,
    GccNoTrappingMath,
    /// GCC's `-fno-trapping-math -fcx-limited-range`.
    GccLimitedComplex,
}

impl Rules {
    fn configuration(self) -> CompilerConfiguration {
        let identity = match self {
            | Self::Clang | Self::ClangTrappingMath => CompilerIdentity::Clang,
            | Self::Gcc | Self::GccNoTrappingMath | Self::GccLimitedComplex =>
                CompilerIdentity::Gcc,
        };
        let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity);
        match self {
            | Self::Clang | Self::Gcc => configuration,
            | Self::ClangTrappingMath => configuration
                .with_relaxed_floating_point(RelaxedFloatingPoint::NoTrappingMath, false),
            | Self::GccNoTrappingMath => configuration
                .with_relaxed_floating_point(RelaxedFloatingPoint::NoTrappingMath, true),
            | Self::GccLimitedComplex => configuration
                .with_relaxed_floating_point(RelaxedFloatingPoint::NoTrappingMath, true)
                .with_relaxed_floating_point(RelaxedFloatingPoint::CxLimitedRange, true),
        }
    }
}

const ALL_RULES: [Rules; 5] = [
    Rules::Clang,
    Rules::ClangTrappingMath,
    Rules::Gcc,
    Rules::GccNoTrappingMath,
    Rules::GccLimitedComplex,
];

/// The `double` bits of the real part of every folded `operator`
/// expression in `source`, in order of analysis.
fn folded(source: &str, rules: Rules, operator: BinaryOperator) -> Vec<u128> {
    folded_in(source, rules.configuration(), operator)
}

/// [`folded`] under `configuration`.
fn folded_in(
    source: &str,
    configuration: CompilerConfiguration,
    operator: BinaryOperator,
) -> Vec<u128> {
    let mut bits = Vec::new();
    with_configuration(source, configuration, |context, unit| {
        assert_eq!(context.pending_error_count(), 0, "{source}");
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
/// under every option: at file scope, for a block-scope static or
/// thread-local object, and in an aggregate or a file-scope compound
/// literal.
#[test]
fn a_static_initializer_folds_the_positive_nan() {
    for rules in ALL_RULES {
        for source in [
            "double d = 0.0 / 0.0;",
            "static double d = (1.0 / 0.0 - 1.0 / 0.0) / 1.0;",
            "void f(void) { static double d = 0.0 / 0.0; (void)d; }",
            "void f(void) { static _Thread_local double d = 0.0 / 0.0; (void)d; }",
            "_Thread_local double d = 0.0 / 0.0;",
            "struct S { double d; } s = { 0.0 / 0.0 };",
            "double *p = &(double){ 0.0 / 0.0 };",
            "_Complex double z = (0.0 + 0.0i) / (0.0 + 0.0i);",
        ] {
            let bits = folded(source, rules, BinaryOperator::Division);
            assert_eq!(bits.last(), Some(&POSITIVE), "{rules:?}: {source}");
        }
    }
}

/// In a function body every invalid operation keeps the hardware's
/// negative default NaN, as if computed at run time, under every option:
/// GCC and Clang fold some of these too, but a NaN folded there would take
/// the wrong sign wherever it later meets one computed at run time.
#[test]
fn a_function_keeps_the_hardware_nan() {
    for rules in ALL_RULES {
        for source in [
            "void f(void) { double d = 0.0 / 0.0; (void)d; }",
            "double f(void) { return 0.0 / 0.0; }",
            "void g(double); void f(void) { g(0.0 / 0.0); }",
            "void f(void) { double a[2] = { 0.0 / 0.0, 1.0 }; (void)a; }",
            "double f(void) { return (double){ 0.0 / 0.0 }; }",
            "void f(void) { _Complex double z = (0.0 + 0.0i) / (0.0 + 0.0i); (void)z; }",
            "void f(void) { static double s = 1.0; double d = 0.0 / 0.0; (void)s; (void)d; }",
        ] {
            assert_eq!(
                folded(source, rules, BinaryOperator::Division),
                [NEGATIVE],
                "{rules:?}: {source}"
            );
        }
        // A complex product folds each part the same way.
        let product = "(__builtin_inf() + 0.0i) * (0.0 + 1.0i)";
        assert_eq!(
            folded(
                &format!("_Complex double z = {product};"),
                rules,
                BinaryOperator::Multiplication
            )
            .last(),
            Some(&POSITIVE),
            "{rules:?}"
        );
        assert_eq!(
            folded(
                &format!("void f(void) {{ _Complex double z = {product}; (void)z; }}"),
                rules,
                BinaryOperator::Multiplication
            )
            .last(),
            Some(&NEGATIVE),
            "{rules:?}"
        );
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

/// Binary128 folds the same way: positive in a static initializer, and
/// libgcc's negative default NaN in a function.
#[test]
fn binary128_folds_the_positive_nan_only_in_a_static_initializer() {
    for (source, negative) in [
        ("__float128 q = 0.0Q / 0.0Q;", false),
        (
            "void f(void) { static __float128 q = 0.0Q / 0.0Q; (void)q; }",
            false,
        ),
        (
            "void f(void) { __float128 q = 0.0Q / 0.0Q; (void)q; }",
            true,
        ),
        ("__float128 f(void) { return 0.0Q / 0.0Q; }", true),
    ] {
        for rules in ALL_RULES {
            with_configuration(source, rules.configuration(), |context, unit| {
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
                    .expect("a division")
                    .binary128()
                    .expect("a folded binary128 value");
                assert!(value.real.is_nan(), "{source}");
                assert_eq!(value.real.is_negative(), negative, "{rules:?}: {source}");
            });
        }
    }
}

/// The tcc and msvc identities accept GCC's `-fno-trapping-math` and
/// `-fcx-limited-range` but fold no NaN for them: TCC 0.9.27 computes every
/// invalid operation at run time and rejects one in a static initializer,
/// and there is no `cl` to compare with, so every NaN has the hardware's
/// sign, in a function and in a static initializer alike.
#[test]
fn tcc_and_msvc_keep_every_nan_at_run_time() {
    for identity in [CompilerIdentity::Tcc, CompilerIdentity::Msvc] {
        for rules in [
            Rules::Gcc,
            Rules::GccNoTrappingMath,
            Rules::GccLimitedComplex,
        ] {
            let configuration = rules.configuration().with_compiler_identity(identity);
            for source in [
                "double f(void) { return 0.0 / 0.0; }",
                "void g(double); void f(void) { g(0.0 / 0.0); }",
                "void f(void) { double d = 0.0 / 0.0; (void)d; }",
                "void f(void) { static double d = 0.0 / 0.0; (void)d; }",
                "double d = 0.0 / 0.0;",
                "void f(void) { _Complex double z = (0.0 + 0.0i) / (0.0 + 0.0i); (void)z; }",
            ] {
                assert_eq!(
                    folded_in(source, configuration, BinaryOperator::Division),
                    [NEGATIVE],
                    "{identity:?} {rules:?}: {source}"
                );
            }
        }
    }
}
