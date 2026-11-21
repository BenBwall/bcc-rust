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
/// computed at run time. Under `-ftrapping-math` alone, Clang's evaluator
/// still folds the initializer of a constant automatic array or structure,
/// unless a complex quotient by a real divisor raises an exception; under
/// the pragma bcc keeps that at run time too. A static initializer folds.
#[test]
fn clang_strict_exceptions_compute_the_nan_at_run_time() {
    let (n, p) = (NEGATIVE, POSITIVE);
    let cases = [
        ("void f(void) { double d = 0.0 / 0.0; (void)d; }", n, n),
        (
            "void f(void) { double d = (double){0.0 / 0.0}; (void)d; }",
            n,
            n,
        ),
        ("double f(void) { return 0.0 / 0.0; }", n, n),
        ("void f(void) { double a[1] = {0.0 / 0.0}; (void)a; }", p, n),
        (
            "void f(void) { struct { int i; double d; } s = {1, 0.0 / 0.0}; (void)s; }",
            p,
            n,
        ),
        (
            "void f(void) { _Complex double z[1] = {(1.0i) / 0.0}; (void)z; }",
            n,
            n,
        ),
        (
            "void f(void) { static double d = 0.0 / 0.0; (void)d; }",
            p,
            p,
        ),
    ];
    for (source, trapping, pragma_expected) in cases {
        assert_eq!(
            folded(source, Rules::ClangTrappingMath, BinaryOperator::Division).last(),
            Some(&trapping),
            "{source}"
        );
        let pragma = format!("#pragma STDC FENV_ACCESS ON\n{source}");
        assert_eq!(
            folded(&pragma, Rules::Clang, BinaryOperator::Division).last(),
            Some(&pragma_expected),
            "{pragma}"
        );
    }
}

/// Under Clang's strict exception behavior an automatic array or
/// structure takes the compiler's NaN only when Clang's evaluator folds its
/// whole initializer, and the hardware's otherwise: a run-time member, the
/// address of an automatic object, a builtin it does not fold, an unordered
/// comparison, or an operation or conversion that rounds, overflows or
/// divides by zero has Clang compute every member at run time. Under a
/// floating-point pragma, which bcc does not follow, every member stays at
/// run time.
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
            for (source, rules, expected) in [
                (&source, Rules::ClangTrappingMath, expected),
                (&pragma, Rules::Clang, NEGATIVE),
            ] {
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

/// A function whose `0.0 / 0.0` the tests below fold.
fn function(name: &str) -> String {
    format!("void {name}(void) {{ double d = 0.0 / 0.0; (void)d; }}\n")
}

/// A floating-point environment pragma that Clang or `cl` reads, anywhere
/// in the translation unit and with any operand, keeps every function's
/// invalid operations at run time under their rules: bcc does not follow
/// where one is in effect. A static initializer still folds, and GCC
/// ignores the pragmas.
#[test]
fn a_floating_pragma_anywhere_keeps_every_nan_at_run_time() {
    let pragmas = [
        "#pragma STDC FENV_ACCESS ON\n",
        "#pragma STDC FENV_ACCESS OFF\n",
        "#pragma STDC FENV_ROUND FE_TONEAREST\n",
        "#pragma STDC FENV_ROUND FE_UPWARD\n",
        "#pragma clang fp exceptions(ignore)\n",
        "#pragma clang fp contract(fast) exceptions(strict)\n",
        "#define X exceptions(strict)\n#pragma clang fp X\n",
        "#pragma float_control(precise, on)\n",
        "#pragma float_control(except, on, push)\n",
        "#pragma float_control(pop)\n",
        "#pragma fenv_access(on)\n",
        "_Pragma(\"STDC FENV_ACCESS ON\")\n",
    ];
    let quiet = |rules: Rules| rules.configuration().with_suppress_warnings(true);
    let (n, p) = (NEGATIVE, POSITIVE);
    for pragma in pragmas {
        // Before the functions, inside one, after a declaration's
        // attributes, and at the end.
        let placements = [
            format!("{pragma}{}{}", function("f"), function("g")),
            format!(
                "void f(void) {{\n{pragma} double d = 0.0 / 0.0; (void)d;\n}}\n{}",
                function("g")
            ),
            format!(
                "__attribute__((noinline))\n{pragma}{}{}",
                function("f"),
                function("g")
            ),
            format!(
                "{}{}static double s = 0.0 / 0.0;\n{pragma}",
                function("f"),
                function("g")
            ),
        ];
        for source in placements {
            let static_nan: &[u128] = if source.contains("static double") {
                &[p]
            } else {
                &[]
            };
            let runtime = [&[n, n][..], static_nan].concat();
            let folds = [&[p, p][..], static_nan].concat();
            for configuration in [
                quiet(Rules::Clang),
                quiet(Rules::Clang).with_compiler_identity(CompilerIdentity::ClangCl),
            ] {
                assert_eq!(
                    folded_in(&source, configuration, BinaryOperator::Division),
                    runtime,
                    "{configuration:?}: {source}"
                );
            }
            // The msvc identity keeps every NaN at run time regardless.
            let msvc =
                quiet(Rules::GccNoTrappingMath).with_compiler_identity(CompilerIdentity::Msvc);
            assert!(
                folded_in(&source, msvc, BinaryOperator::Division)
                    .iter()
                    .all(|&bits| bits == n),
                "{source}"
            );
            assert_eq!(
                folded_in(
                    &source,
                    quiet(Rules::GccNoTrappingMath),
                    BinaryOperator::Division
                ),
                folds,
                "{source}"
            );
        }
    }
    // Other `clang fp` clauses leave the environment alone.
    let source = format!(
        "#pragma clang fp contract(fast) reassociate(off)\n{}",
        function("f")
    );
    assert_eq!(
        folded_in(&source, quiet(Rules::Clang), BinaryOperator::Division),
        [p]
    );
}

/// Under `-fno-trapping-math`, GCC's `optimize` attribute or
/// `#pragma GCC optimize` can turn trapping math back on for a function
/// (`trapping-math`, `no-fast-math`, or an `-O` level after `-Ofast`).
/// `#pragma GCC optimize` or `push_options` anywhere keeps every function's
/// NaN at run time, and an `optimize` attribute every function's after it;
/// `pop_options` and `reset_options` alone do not. Clang ignores them.
#[test]
fn gcc_optimize_options_keep_the_nan_at_run_time() {
    let (n, p) = (NEGATIVE, POSITIVE);
    let attribute = [
        function("plain"),
        "__attribute__((optimize(\"trapping-math\"))) ".into(),
        function("attribute"),
        function("later"),
    ]
    .concat();
    assert_eq!(
        folded(
            &attribute,
            Rules::GccNoTrappingMath,
            BinaryOperator::Division
        ),
        [p, n, n]
    );
    assert_eq!(
        folded(&attribute, Rules::Clang, BinaryOperator::Division),
        [p; 3]
    );
    for (pragma, expected) in [
        ("#pragma GCC optimize(\"no-fast-math\")\n", n),
        ("#pragma GCC push_options\n", n),
        ("#pragma GCC pop_options\n#pragma GCC reset_options\n", p),
    ] {
        let source = [function("before"), pragma.into(), function("after")].concat();
        assert_eq!(
            folded(&source, Rules::GccNoTrappingMath, BinaryOperator::Division),
            [expected; 2],
            "{source}"
        );
        assert_eq!(
            folded(&source, Rules::Clang, BinaryOperator::Division),
            [p; 2],
            "{source}"
        );
    }
}

/// Binary128 folds positive in a static initializer, and has libgcc's
/// negative default NaN in a function, where bcc does not fold it again
/// ([`Analyzer::settle_function_nans`]).
#[test]
fn binary128_folds_the_nan_its_compiler_computes() {
    for (source, rules, negative) in [
        ("__float128 q = 0.0Q / 0.0Q;", Rules::Gcc, false),
        ("__float128 q = 0.0Q / 0.0Q;", Rules::Clang, false),
        (
            "void f(void) { __float128 q = 0.0Q / 0.0Q; (void)q; }",
            Rules::Clang,
            true,
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

/// In a function a folded NaN takes its compiler's sign only where it is
/// the value the code produces: returned, passed to a function that is no
/// builtin, stored, or initializing an object every read of which is such a
/// value too, or copied into one. Where it meets a NaN computed at run time
/// in one operation, whose result is its first operand's NaN on x86 and
/// GCC, Clang and bcc order the operands differently, it keeps the
/// hardware's NaN: a constant operand of such an operation, an object one
/// reads, a copy of that object, an object read through a pointer or by a
/// nested function, a builtin's or library function's argument, and an
/// argument for a parameter that a function defined before reads in such an
/// operation, which inlining the call would meet.
#[test]
fn a_nan_that_meets_a_run_time_operation_stays_at_run_time() {
    let (n, p) = (NEGATIVE, POSITIVE);
    let cases = [
        ("double f(void) { return 0.0 / 0.0; }", p),
        ("void g(double); void f(void) { g(-(0.0 / 0.0)); }", p),
        (
            "void g(double *); void f(void) { double r = 0.0 / 0.0; g(&r); }",
            p,
        ),
        ("double f(void) { double r = 0.0 / 0.0; return r; }", p),
        ("double f(void) { double r; r = 0.0 / 0.0; return r; }", p),
        ("double h; void f(void) { double r = 0.0 / 0.0; h = r; }", p),
        (
            "double f(void) { double r = 0.0 / 0.0, y = r; return y; }",
            p,
        ),
        (
            "void g(double); void f(void) { struct { int i; double d[2]; } s = {1, {0.0 / 0.0, \
             1}}; g(s.d[0]); }",
            p,
        ),
        ("double f(double x) { return 0.0 / 0.0 + x; }", n),
        ("double f(double x) { return x - (0.0 / 0.0 + 1.0); }", n),
        (
            "double f(double x) { double r = 0.0 / 0.0; return r + x; }",
            n,
        ),
        (
            "double f(double x) { double r = 0.0 / 0.0; return x * r; }",
            n,
        ),
        (
            "double f(double x) { double r = 0.0 / 0.0; x += r; return x; }",
            n,
        ),
        (
            "double f(double x) { double r = 0.0 / 0.0, y = r; return y + x; }",
            n,
        ),
        (
            "double f(double x) { double r = 0.0 / 0.0, *p = &r; return *p + x; }",
            n,
        ),
        (
            "double f(double x) { return __builtin_fmax(0.0 / 0.0, x); }",
            n,
        ),
        (
            "double fmax(double, double); double f(double x) { return fmax(0.0 / 0.0, x); }",
            n,
        ),
        (
            "static void put(double d) { (void)d; } void f(void) { put(0.0 / 0.0); }",
            p,
        ),
        (
            "static double add(const double *p, double y) { return *p + y; } double f(double x) { \
             double r = 0.0 / 0.0; return add(&r, x); }",
            n,
        ),
        (
            "static double add(double a, double y) { return a + y; } double f(double x) { return \
             add(0.0 / 0.0, x); }",
            n,
        ),
        ("int f(void) { return __builtin_signbit(0.0 / 0.0); }", n),
    ];
    for rules in [Rules::Clang, Rules::GccNoTrappingMath] {
        for (source, expected) in cases {
            assert_eq!(
                folded(source, rules, BinaryOperator::Division),
                [expected],
                "{rules:?}: {source}"
            );
        }
    }
    // A GNU nested function's reads count for the object it reads.
    assert_eq!(
        folded(
            "double f(double x) { double r = 0.0 / 0.0; double n(void) { return r * x; } return \
             n(); }",
            Rules::GccNoTrappingMath,
            BinaryOperator::Division
        ),
        [n]
    );
    // Clang folds a scalar compound literal's read; its stored value is
    // final where the literal's value is.
    for (source, expected) in [
        ("double f(void) { return (double){0.0 / 0.0}; }", p),
        ("double f(double x) { return (double){0.0 / 0.0} + x; }", n),
    ] {
        assert_eq!(
            folded(source, Rules::Clang, BinaryOperator::Division),
            [expected],
            "{source}"
        );
    }
}
