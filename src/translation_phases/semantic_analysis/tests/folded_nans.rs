//! The NaN an invalid floating operation folds to ([`NanSource`]): the
//! positive quiet NaN GCC's and Clang's arithmetic give, except where the
//! operation is computed at run time: by GCC in a function while math may
//! trap, by Clang in a function under strict exception behavior, and where
//! either calls a complex routine; there it is the x86-64 default NaN, sign
//! bit set. C99: §F.7.4 paragraph 1, p. 449; PDF p. 461; §F.7.5
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
/// under every identity: at file scope and for a block-scope static.
#[test]
fn a_static_initializer_folds_the_positive_nan() {
    for rules in [
        Rules::Clang,
        Rules::ClangTrappingMath,
        Rules::Gcc,
        Rules::GccNoTrappingMath,
    ] {
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

/// Without trapping math GCC folds real arithmetic in a function, but the
/// product or quotient of two complex values still calls `__muldc3` or
/// `__divdc3` at run time, unless `-fcx-limited-range` has GCC compute it
/// inline, where it folds. A complex sum folds componentwise.
#[test]
fn gcc_without_trapping_math_calls_the_complex_routines() {
    let in_function =
        |expression: &str| format!("void f(void) {{ _Complex double z = {expression}; (void)z; }}");
    let quotient = in_function("(0.0 + 0.0i) / (0.0 + 0.0i)");
    let product = in_function("(__builtin_inf() + 0.0i) * (0.0 + 1.0i)");
    let sum = in_function("(__builtin_inf() + 1.0i) + (-__builtin_inf() + 1.0i)");
    for (rules, expected) in [
        (Rules::GccNoTrappingMath, NEGATIVE),
        (Rules::GccLimitedComplex, POSITIVE),
    ] {
        assert_eq!(
            folded(&quotient, rules, BinaryOperator::Division),
            [expected],
            "{rules:?}"
        );
        assert_eq!(
            folded(&product, rules, BinaryOperator::Multiplication).last(),
            Some(&expected),
            "{rules:?}"
        );
        assert_eq!(
            folded(&sum, rules, BinaryOperator::Addition).last(),
            Some(&POSITIVE),
            "{rules:?}"
        );
    }
}

/// Under Clang's strict exception behavior, from `-ftrapping-math` or
/// `#pragma STDC FENV_ACCESS ON`, an invalid operation in a function is
/// computed at run time, but Clang's evaluator still folds the initializer
/// of a constant automatic array or structure, unless a complex quotient
/// by a real divisor raises an exception.
#[test]
fn clang_strict_exceptions_compute_the_nan_at_run_time() {
    let cases = [
        ("void f(void) { double d = 0.0 / 0.0; (void)d; }", NEGATIVE),
        (
            "void f(void) { double d = (double){0.0 / 0.0}; (void)d; }",
            NEGATIVE,
        ),
        ("double f(void) { return 0.0 / 0.0; }", NEGATIVE),
        (
            "void f(void) { double a[1] = {0.0 / 0.0}; (void)a; }",
            POSITIVE,
        ),
        (
            "void f(void) { struct { int i; double d; } s = {1, 0.0 / 0.0}; (void)s; }",
            POSITIVE,
        ),
        (
            "void f(void) { _Complex double z[1] = {(1.0i) / 0.0}; (void)z; }",
            NEGATIVE,
        ),
        (
            "void f(void) { static double d = 0.0 / 0.0; (void)d; }",
            POSITIVE,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            folded(source, Rules::ClangTrappingMath, BinaryOperator::Division).last(),
            Some(&expected),
            "{source}"
        );
        let pragma = format!("#pragma STDC FENV_ACCESS ON\n{source}");
        assert_eq!(
            folded(&pragma, Rules::Clang, BinaryOperator::Division).last(),
            Some(&expected),
            "{pragma}"
        );
    }
}

/// Under Clang's strict exception behavior an automatic array or
/// structure takes the compiler's NaN only when Clang's evaluator folds its
/// whole initializer, and the hardware's otherwise: a run-time member, the
/// address of an automatic object, a builtin it does not fold, an unordered
/// comparison, or an operation or conversion that rounds, overflows or
/// divides by zero has Clang compute every member at run time.
#[test]
fn clang_strict_exceptions_fold_only_a_whole_constant_aggregate() {
    let folds = [
        "double a[2] = {0.5, 0.0 / 0.0};",
        "double a[2] = {1.0 / 4.0 + 0.5, 0.0 / 0.0};",
        "double a[2] = {[1] = 0.0 / 0.0};",
        "double a[2] = {-(0.0 / 0.0), (float)(0.0 / 0.0) * 2.0};",
        "float a[2] = {0.5, 0.0f / 0.0f};",
        "float a[2] = {16777216, 0.0 / 0.0};",
        "long double a[2] = {1.0L / 4.0L, 0.0L / 0.0L};",
        "struct { double *p; double v; } s = {&g, 0.0 / 0.0};",
        "struct { struct { int i; double d; } s; double w[2]; } n = {{1, 0.0 / 0.0}, {1, 2}};",
        "_Complex double z[2] = {(1.0 + 1.0i) / 2.0, 0.0 / 0.0};",
        "_Complex double z[2] = {(0.1 + 0.2i) * (3.0 + 1.0i), 0.0 / 0.0};",
        "double a[2] = {__builtin_inf() / 0.0, 0.0 / 0.0};",
        "double a[2] = {(double)(0.5 < 1.0) + __builtin_inf(), 0.0 / 0.0};",
    ];
    let computes = [
        "double a[2] = {0.0 / 0.0, x};",
        "struct { double *p; double v; } s = {&x, 0.0 / 0.0};",
        "double a[2] = {0.0 / 0.0, (double)(long)&x};",
        "double a[2] = {0.0 / 0.0, x = 1};",
        "double a[2] = {0.0 / 0.0, (int)(0.0 / 0.0 == 0.0 / 0.0)};",
        "double a[2] = {1.0 / 3.0, 0.0 / 0.0};",
        "float a[2] = {0.1, 0.0f / 0.0f};",
        "double a[2] = {(float)16777217, 0.0 / 0.0};",
        "double a[2] = {(float)0.1, 0.0 / 0.0};",
        "double a[2] = {1.0 / 0.0, 0.0 / 0.0};",
        "double a[2] = {1e308 * 10.0, 0.0 / 0.0};",
        "double a[2] = {0x1p-1022 / 3.0, 0.0 / 0.0};",
        "long double a[2] = {1.0L / 3.0L, 0.0L / 0.0L};",
        "_Complex double z[2] = {(1.0 + 1.0i) / 3.0, 0.0 / 0.0};",
        "_Complex double z[2] = {3.0 * (0.1 + 0.1i), 0.0 / 0.0};",
        "_Complex double z[2] = {(1.0i) / 0.0, 0.0 / 0.0};",
        "struct { struct { int i; double d; } s; double w[2]; } n = {{1, 0.0 / 0.0}, {x, 1}};",
        "double a[2][1] = {{0.0 / 0.0}, {x}};",
        "struct { int i; double d; } s[2] = {{1, 0.0 / 0.0}, {2, 1.0 / 3.0}};",
        "struct { int *p; double v; } s = {(int[]){1, 2}, 0.0 / 0.0};",
        "double a[2] = {__builtin_sqrt(4.0), 0.0 / 0.0};",
        "double a[2] = {(1.0 / 3.0, 0.5), 0.0 / 0.0};",
    ];
    let nans = |source: &str, rules| {
        let mut bits = folded(source, rules, BinaryOperator::Division);
        bits.retain(|&bits| bits >> 52 & 0x7FF == 0x7FF && bits & 0xF_FFFF_FFFF_FFFF != 0);
        bits
    };
    for (sources, expected) in [(&folds[..], POSITIVE), (&computes[..], NEGATIVE)] {
        for declaration in sources {
            let source = format!(
                "static double g;
void f(double x) {{ {declaration} (void)x; }}"
            );
            let pragma = format!(
                "#pragma STDC FENV_ACCESS ON
{source}"
            );
            for (source, rules) in [(&source, Rules::ClangTrappingMath), (&pragma, Rules::Clang)] {
                let bits = nans(source, rules);
                assert!(!bits.is_empty(), "{source}");
                assert!(
                    bits.iter().all(|&bits| bits == expected),
                    "{source}: {bits:x?}"
                );
            }
        }
    }
}

/// `#pragma STDC FENV_ACCESS` applies from where it appears, at file scope
/// or in a function's body, until another turns it off; GCC ignores it.
#[test]
fn fenv_access_applies_where_it_is_on() {
    let source = "void before(void) { double d = 0.0 / 0.0; (void)d; }\n#pragma STDC FENV_ACCESS \
                  ON\nvoid on(void) { double d = 0.0 / 0.0; (void)d; }\n#pragma STDC FENV_ACCESS \
                  OFF\nvoid off(void) { double d = 0.0 / 0.0; (void)d; }\nvoid inner(void) \
                  {\n#pragma STDC FENV_ACCESS ON\n double d = 0.0 / 0.0; (void)d;\n}\nvoid \
                  after(void) { double d = 0.0 / 0.0; (void)d; }";
    assert_eq!(
        folded(source, Rules::Clang, BinaryOperator::Division),
        [POSITIVE, NEGATIVE, POSITIVE, NEGATIVE, POSITIVE]
    );
    assert_eq!(
        folded(source, Rules::GccNoTrappingMath, BinaryOperator::Division),
        [POSITIVE; 5]
    );
}

/// A function whose `0.0 / 0.0` the tests below fold.
fn function(name: &str) -> String {
    format!("void {name}(void) {{ double d = 0.0 / 0.0; (void)d; }}\n")
}

/// Clang's other pragmas that make exceptions strict apply as
/// `FENV_ACCESS` does: `clang fp exceptions(strict|maytrap)` and
/// `float_control(except, on)` until `ignore` or `off`, and
/// `float_control(push)` and `(pop)` save and restore the whole state,
/// `FENV_ACCESS` included. A pragma at file scope overrides
/// `-ftrapping-math`; `FENV_ACCESS ON` stays strict under
/// `exceptions(ignore)`. Clang 23 at `-O0` gives these signs.
#[test]
fn clang_floating_pragmas_apply_where_they_are_in_effect() {
    let source = [
        function("plain"),
        "#pragma clang fp exceptions(strict)\n".into(),
        function("fp_strict"),
        "#pragma clang fp exceptions(ignore)\n".into(),
        function("fp_ignore"),
        "#pragma clang fp contract(fast) exceptions(maytrap)\n".into(),
        function("fp_maytrap"),
        "#pragma float_control(except, off)\n".into(),
        function("control_off"),
        "#pragma float_control(except, on, push)\n".into(),
        function("control_pushed"),
        "#pragma float_control(pop)\n".into(),
        function("control_popped"),
        "#pragma float_control(push)\n#pragma STDC FENV_ACCESS ON\n".into(),
        function("fenv_pushed"),
        "#pragma float_control(pop)\n".into(),
        function("fenv_popped"),
        "void in_body(void) {\n#pragma float_control(except, on)\n double d = 0.0 / 0.0; \
         (void)d;\n}\n"
            .into(),
        function("after_body"),
        "#pragma STDC FENV_ACCESS ON\n#pragma clang fp exceptions(ignore)\n".into(),
        function("fenv_ignore"),
        "#pragma STDC FENV_ACCESS OFF\n".into(),
        function("fenv_off"),
    ]
    .concat();
    let (n, p) = (NEGATIVE, POSITIVE);
    assert_eq!(
        folded(&source, Rules::Clang, BinaryOperator::Division),
        [p, n, p, n, p, n, p, n, p, n, p, n, p]
    );
    assert_eq!(
        folded(&source, Rules::ClangTrappingMath, BinaryOperator::Division),
        [n, n, p, n, p, n, p, n, p, n, p, n, p]
    );
    // GCC reads none of them, and traps by default.
    assert_eq!(
        folded(&source, Rules::GccNoTrappingMath, BinaryOperator::Division),
        [p; 13]
    );
}

/// Microsoft's `#pragma fenv_access(on)` is `FENV_ACCESS ON` where its
/// pragmas are read (`-fms-extensions`), and ignored elsewhere.
#[test]
fn microsoft_fenv_access_applies_with_its_pragmas() {
    let source = [
        "#pragma fenv_access(on)\n".into(),
        function("on"),
        "#pragma fenv_access (off)\n".into(),
        function("off"),
    ]
    .concat();
    let microsoft = Rules::Clang.configuration().with_msvc_extensions(true);
    assert_eq!(
        folded_in(&source, microsoft, BinaryOperator::Division),
        [NEGATIVE, POSITIVE]
    );
    assert_eq!(
        folded(&source, Rules::Clang, BinaryOperator::Division),
        [POSITIVE; 2]
    );
}

/// `STDC FENV_ROUND` with a constant direction other than to nearest makes
/// Clang compute the operation at run time at `-O0`; `FE_DYNAMIC` and
/// `FE_TONEAREST` do not. Clang and bcc warn that the pragma is not
/// supported, but Clang honors it.
#[test]
fn a_directed_rounding_pragma_computes_the_nan_at_run_time() {
    let source = [
        "#pragma STDC FENV_ROUND FE_UPWARD\n".into(),
        function("upward"),
        "#pragma STDC FENV_ROUND FE_DYNAMIC\n".into(),
        function("dynamic"),
        "#pragma STDC FENV_ROUND FE_TOWARDZERO\n".into(),
        function("toward_zero"),
        "#pragma STDC FENV_ROUND FE_TONEAREST\n".into(),
        function("nearest"),
    ]
    .concat();
    let quiet = Rules::Clang.configuration().with_suppress_warnings(true);
    assert_eq!(
        folded_in(&source, quiet, BinaryOperator::Division),
        [NEGATIVE, POSITIVE, NEGATIVE, POSITIVE]
    );
}

/// Clang replaces macros in these pragmas' operands, which bcc does not
/// read, so every function after such a pragma keeps the run-time NaN.
#[test]
fn a_floating_pragma_naming_a_macro_keeps_the_nan_at_run_time() {
    let source = [
        "#define off on\n".into(),
        function("before"),
        "#pragma float_control(except, off)\n".into(),
        function("replaced"),
        "#pragma clang fp exceptions(ignore)\n".into(),
        function("later"),
    ]
    .concat();
    assert_eq!(
        folded(&source, Rules::Clang, BinaryOperator::Division),
        [POSITIVE, NEGATIVE, NEGATIVE]
    );
}

/// Under `-fno-trapping-math`, GCC's `optimize` attribute or
/// `#pragma GCC optimize` can turn trapping math back on for a function
/// (`trapping-math`, `no-fast-math`, or an `-O` level after `-Ofast`), so a
/// function any of its declarations has one on keeps the run-time NaN.
/// `pop_options` and `reset_options` end the pragma. Clang ignores both.
#[test]
fn gcc_optimize_options_keep_the_nan_at_run_time() {
    let source = [
        function("plain"),
        "__attribute__((optimize(\"trapping-math\"))) ".into(),
        function("attribute"),
        "void declared(void) __attribute__((optimize(\"O2\")));\n".into(),
        function("declared"),
        "#pragma GCC push_options\n#pragma GCC optimize(\"no-fast-math\")\n".into(),
        function("pragma"),
        "void pragma_declared(void);\n#pragma GCC pop_options\n".into(),
        function("pragma_declared"),
        function("popped"),
        "#pragma GCC optimize(\"trapping-math\")\n#pragma GCC reset_options\n".into(),
        function("reset"),
    ]
    .concat();
    let (n, p) = (NEGATIVE, POSITIVE);
    assert_eq!(
        folded(&source, Rules::GccNoTrappingMath, BinaryOperator::Division),
        [p, n, n, n, n, p, p]
    );
    assert_eq!(
        folded(&source, Rules::Clang, BinaryOperator::Division),
        [p; 7]
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
                    .expect("a division")
                    .binary128()
                    .expect("a folded binary128 value");
                assert!(value.real.is_nan(), "{source}");
                assert_eq!(value.real.is_negative(), negative, "{rules:?}: {source}");
            },
        );
    }
}
