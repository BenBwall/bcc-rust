//! GNU 128-bit arithmetic and target layout, with a shared Clang oracle.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
};

fn check_constant_diagnostics(statement: &str, expected: SemanticErrorKind) {
    use crate::diagnostics::ToDiagnostic as _;

    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for standard in [
            CStandard::C89,
            CStandard::C95,
            CStandard::C99,
            CStandard::C11,
            CStandard::C17,
            CStandard::C23,
            CStandard::C2y,
        ] {
            for gnu in [false, true] {
                let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_target(target)
                    .with_gnu_extensions(gnu);
                let arena = Bump::new();
                let mut reference = None;
                for ty in [
                    "long long",
                    "__int128_t",
                    "__int128",
                    "unsigned long long",
                    "__uint128_t",
                    "unsigned __int128",
                ] {
                    let source =
                        format!("int n; {} int following;", statement.replace("{type}", ty));
                    with_configuration(&source, configuration, |context, unit| {
                        let errors = context.take_pending_errors();
                        assert_eq!(errors.len(), 1, "{source}: {configuration:?}: {errors:?}");
                        let TranslationError::Semantic(error) = errors[0] else {
                            panic!("{source}: unexpected {:?}", errors[0]);
                        };
                        assert_eq!(error.kind, expected, "{source}: {configuration:?}");
                        let diagnostic = error.diagnostic_in(context, error.source_vectors, &arena);
                        if let Some(message) = &reference {
                            assert_eq!(diagnostic.message, *message, "{source}");
                        } else {
                            reference = Some(diagnostic.message);
                        }
                        let following = unit
                            .bindings
                            .iter()
                            .find(|b| context.string_cache.at(b.name.name) == "following")
                            .expect("following declaration survives the diagnostic");
                        assert_eq!(
                            unit.types.nodes[following.ty.index],
                            TypeKind::Scalar(Scalar::Int)
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn int128_false_static_assertion_matches_standard_integer_diagnostics() {
    check_constant_diagnostics(
        "_Static_assert(({type})0, \"false\");",
        SemanticErrorKind::FailedAssertion,
    );
}

#[test]
fn int128_nonconstant_static_assertion_matches_standard_integer_diagnostics() {
    check_constant_diagnostics(
        "_Static_assert(({type})n, \"runtime\");",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn int128_nonconstant_enumerator_matches_standard_integer_diagnostics() {
    check_constant_diagnostics(
        "enum { E = ({type})n };",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn int128_nonconstant_bit_field_matches_standard_integer_diagnostics() {
    check_constant_diagnostics(
        "struct S { int width : ({type})n; };",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn full_width_integer_operations_and_conversions() {
    use BinaryOperator as B;
    use UnaryOperator as U;
    let unsigned = |n: u128| Integer {
        value:  n as i128,
        bits:   128,
        signed: false,
    };
    let signed = |n| Integer {
        value:  n,
        bits:   128,
        signed: true,
    };
    let max = unsigned(u128::MAX);
    assert_eq!(max.to_i128(), None);
    assert_eq!(max.to_u64(), None);
    let arena = Bump::new();
    assert_eq!(
        crate::diagnostics::format_in!(&arena, "{max}"),
        "340282366920938463463374607431768211455"
    );
    assert_eq!(max.cast(128, true), signed(-1));
    assert_eq!(signed(-1).cast(128, false), max);
    assert_eq!(max.binary(B::Addition, unsigned(1)), Some(unsigned(0)));
    assert_eq!(unsigned(0).binary(B::Subtraction, unsigned(1)), Some(max));
    assert_eq!(max.binary(B::Multiplication, max), Some(unsigned(1)));
    assert_eq!(max.unary(U::Minus), Some(unsigned(1)));
    assert_eq!(max.binary(B::LessThan, unsigned(1)).unwrap().value, 0);
    assert_eq!(
        signed(-1)
            .binary(B::GreaterThan, unsigned(0))
            .unwrap()
            .value,
        1
    );
    for divisor in [1, 3, 7, 1_u128 << 64, (1_u128 << 127) + 1, u128::MAX] {
        assert_eq!(
            max.binary(B::Division, unsigned(divisor)),
            Some(unsigned(u128::MAX / divisor))
        );
        assert_eq!(
            max.binary(B::Modulo, unsigned(divisor)),
            Some(unsigned(u128::MAX % divisor))
        );
    }
    for shift in 64..128 {
        assert_eq!(
            unsigned(1).binary(B::LeftShift, Integer::int(shift)),
            Some(unsigned(1 << shift))
        );
        assert_eq!(
            max.binary(B::RightShift, Integer::int(shift)),
            Some(unsigned(u128::MAX >> shift))
        );
        assert_eq!(
            signed(i128::MIN).binary(B::RightShift, Integer::int(shift)),
            Some(signed(i128::MIN >> shift))
        );
    }
    assert_eq!(unsigned(1).binary(B::LeftShift, Integer::int(128)), None);
    assert_eq!(unsigned(1).binary(B::LeftShift, max), None);
    assert_eq!(signed(1).binary(B::LeftShift, Integer::int(127)), None);
    assert_eq!(
        signed(1).sign_bit_shift(B::LeftShift, Integer::int(127)),
        Some(signed(i128::MIN))
    );
    assert_eq!(
        signed(3).sign_bit_shift(B::LeftShift, Integer::int(127)),
        None
    );
    assert_eq!(signed(i128::MAX).binary(B::Addition, signed(1)), None);
    assert_eq!(signed(i128::MIN).binary(B::Subtraction, signed(1)), None);
    assert_eq!(signed(i128::MAX).binary(B::Multiplication, signed(2)), None);
    assert_eq!(signed(i128::MIN).unary(U::Minus), None);
    for op in [B::Division, B::Modulo] {
        assert_eq!(signed(i128::MIN).binary(op, signed(-1)), None);
        assert_eq!(max.binary(op, unsigned(0)), None);
    }
    assert_eq!(
        signed(i128::MIN).binary(B::Division, signed(3)),
        Some(signed(i128::MIN / 3))
    );
    assert_eq!(
        signed(i128::MIN).binary(B::Modulo, signed(3)),
        Some(signed(-2))
    );
    assert_eq!(
        signed(-(1_i128 << 100)).binary(B::Multiplication, signed(1 << 27)),
        Some(signed(i128::MIN))
    );
    for bits in [8, 16, 32, 64, 128] {
        let mask = u128::MAX >> (128 - bits);
        assert_eq!(max.cast(bits, false).value as u128, mask);
        assert_eq!(max.cast(bits, true).value, -1);
        let minimum = signed(i128::MIN >> (128 - bits)).cast(bits, true);
        if bits >= 32 {
            assert_eq!(
                minimum.binary(B::Division, signed(-1).cast(bits, true)),
                None
            );
            assert_eq!(minimum.binary(B::Modulo, signed(-1).cast(bits, true)), None);
        }
    }
}

#[test]
fn all_targets_type_and_evaluate_the_clang_probe() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        with_configuration(
            include_str!("../../../../tests/fixtures/targets/int128.c"),
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow).with_target(target),
            |context, unit| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{}: {:?}",
                    target.triple(),
                    context.take_pending_errors()
                );
                for (name, scalar) in [
                    ("signed_first", Scalar::Int128),
                    ("explicit_signed", Scalar::Int128),
                    ("signed_last", Scalar::Int128),
                    ("signed_alias", Scalar::Int128),
                    ("unsigned_first", Scalar::UnsignedInt128),
                    ("unsigned_last", Scalar::UnsignedInt128),
                    ("unsigned_alias", Scalar::UnsignedInt128),
                ] {
                    let binding = unit
                        .bindings
                        .iter()
                        .find(|b| context.string_cache.at(b.name.name) == name)
                        .unwrap();
                    assert_eq!(unit.types.nodes[binding.ty.index], TypeKind::Scalar(scalar));
                    assert_eq!(
                        unit.types.layout(binding.ty),
                        Some(Layout {
                            size:  16,
                            align: 16,
                        })
                    );
                }
                for (name, scalar) in [
                    ("signed_common", Scalar::Int128),
                    ("unsigned_common", Scalar::UnsignedInt128),
                ] {
                    let binding = unit
                        .bindings
                        .iter()
                        .find(|b| context.string_cache.at(b.name.name) == name)
                        .unwrap();
                    let initializer = unit
                        .expressions
                        .iter()
                        .find(|c| {
                            matches!(
                                c.expression.kind,
                                ExpressionType::Binary {
                                    operator: BinaryOperator::Addition,
                                    ..
                                }
                            ) && c.ty == binding.ty
                        })
                        .unwrap();
                    assert_eq!(
                        unit.types.nodes[initializer.ty.index],
                        TypeKind::Scalar(scalar)
                    );
                }
            },
        );
    }
}

#[test]
fn extended_bit_fields_use_clang_promotions() {
    with_source(
        "struct B {unsigned __int128 small:7; unsigned __int128 word:32; __int128 signed_word:32; \
         unsigned __int128 wide:100;}; void f(struct B b) { +b.small; +b.word; +b.signed_word; \
         +b.wide; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let mut unary = unit.expressions.iter().filter(|e| {
                matches!(
                    e.expression.kind,
                    ExpressionType::Unary {
                        operator: UnaryOperator::Plus,
                        ..
                    }
                )
            });
            for scalar in [
                Scalar::Int,
                Scalar::UnsignedInt,
                Scalar::Int,
                Scalar::UnsignedInt128,
            ] {
                assert_eq!(
                    unit.types.nodes[unary.next().unwrap().ty.index],
                    TypeKind::Scalar(scalar)
                );
            }
        },
    );
}

#[test]
fn full_width_unsigned_switch_intervals_keep_their_order() {
    with_source(
        "void f(__uint128_t u) { switch(u) { case 0: break; case ((__uint128_t)1 << 127) ... \
         (__uint128_t)-1: break; } }",
        |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
        },
    );
    with_source(
        "void f(__uint128_t u) { switch(u) { case ((__uint128_t)1 << 127) ... (__uint128_t)-1: \
         break; case (__uint128_t)-1: break; } }",
        |context, _| {
            assert_eq!(context.pending_error_count(), 1);
            assert!(
                matches!(context.take_pending_errors()[0], TranslationError::Semantic(e) if e.kind == SemanticErrorKind::DuplicateCase)
            );
        },
    );
}

#[test]
fn unsigned_high_half_floating_and_pointer_constants_use_the_value() {
    with_source(
        "static long double f = (__uint128_t)-1; static void *p = (void *)((__uint128_t)1 << 64); \
         static _Bool b = (__uint128_t)-1; void test(void) { (void)(long double)((__uint128_t)1 \
         << 127); }",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let pointer = unit
                .expressions
                .iter()
                .find(|e| {
                    matches!(e.expression.kind, ExpressionType::Cast { .. })
                        && matches!(unit.types.nodes[e.ty.index], TypeKind::Pointer(_))
                })
                .unwrap();
            assert_eq!(pointer.integer.unwrap().to_u64(), Some(0));
            let floating = unit
                .expressions
                .iter()
                .find(|e| {
                    matches!(e.expression.kind, ExpressionType::Cast { .. })
                        && unit.types.nodes[e.ty.index] == TypeKind::Scalar(Scalar::LongDouble)
                })
                .unwrap()
                .floating
                .unwrap();
            assert!(!floating.real.is_zero());
            assert_eq!(
                floating
                    .real
                    .compare(crate::float_parsing::LongDouble::ZERO),
                1
            );
        },
    );
}
