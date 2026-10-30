//! GCC vector extensions checked through the translation-unit seam.

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
};

#[test]
fn aligned_vector_derived_types_keep_their_pointee_and_layout() {
    with_source(
        "typedef int V __attribute__((vector_size(16))); typedef V *P \
         __attribute__((aligned(1))); typedef V VA[2] __attribute__((aligned(1))); typedef int *Q \
         __attribute__((aligned(1))); _Static_assert(_Alignof(*(P)0)==16,\"pointee\"); \
         _Static_assert(_Alignof(V)==16,\"vector\");",
        |context, sema| {
            assert_eq!(context.pending_error_count(), 0);
            for name in ["P", "VA", "Q"] {
                let binding = sema
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                assert!(!sema.types.unanalyzed(binding.ty), "{name}");
                assert_eq!(sema.types.layout(binding.ty).unwrap().align, 1, "{name}");
            }
            let vector = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "V")
                .unwrap();
            assert_eq!(
                sema.types.layout(vector.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
            for kind in sema.types.nodes {
                if let TypeKind::Vector { align, .. } = kind {
                    assert_eq!(*align, 16, "alignment must not descend into the vector");
                }
            }
        },
    );
}

#[test]
fn vector_cast_to_void() {
    with_source(
        "typedef int V __attribute__((vector_size(16))); void f(V v) {(void)v;}",
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let cast = sema
                .expressions
                .iter()
                .find(|info| matches!(info.expression.kind, ExpressionType::Cast { .. }))
                .unwrap();
            assert_eq!(
                sema.types.nodes[cast.ty.index],
                TypeKind::Scalar(Scalar::Void)
            );
        },
    );
}

#[test]
fn intrinsic_headers_preserve_vector_alignment() {
    // As in the resource allocation test, exclude hosted allocation helpers.
    let source = format!(
        "#define __BCC_MM_MALLOC_H\n{}",
        include_str!("../../../../tests/fixtures/targets/x86-intrinsics.c")
    );
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true)
            .with_target(target);
        with_configuration(&source, configuration, |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            for (name, align) in [("__m128", 16), ("__m128_u", 1), ("__m128i_u", 1)] {
                let binding = sema
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                assert_eq!(
                    sema.types.layout(binding.ty),
                    Some(Layout { size: 16, align })
                );
            }
        });
    }
}

#[test]
fn vector_layout_matches_clang() {
    with_source(
        include_str!("../../../../tests/fixtures/targets/vector.c"),
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let vector = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "v4i")
                .unwrap();
            assert_eq!(
                sema.types.layout(vector.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
        },
    );
}

#[test]
fn vector_operations_keep_known_types() {
    with_source(
        include_str!("../../../../tests/fixtures/targets/vector.c"),
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            for expression in sema.expressions {
                assert_ne!(
                    sema.types.nodes[expression.ty.index],
                    TypeKind::Unknown,
                    "{:?}",
                    expression.expression.kind
                );
            }
        },
    );
}

#[test]
fn vector_operand_conversions_use_input_elements() {
    with_source(
        "typedef int I __attribute__((vector_size(16))); typedef float F \
         __attribute__((vector_size(16))); void f(I a,F b) { a+b; b<1.0f; }",
        |context, sema| {
            assert_eq!(context.pending_error_count(), 0);
            let int = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "I")
                .unwrap()
                .ty;
            let float = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "F")
                .unwrap()
                .ty;
            let mut conversions = sema
                .conversions
                .iter()
                .filter(|c| c.kind == ConversionKind::Arithmetic);
            assert_eq!(conversions.next().unwrap().ty, int);
            assert_eq!(conversions.next().unwrap().ty, int);
            assert_eq!(conversions.next().unwrap().ty, float);
            assert_eq!(conversions.next().unwrap().ty, float);
            assert!(conversions.next().is_none());
        },
    );
}

#[test]
fn invalid_vector_constraints_do_not_taint_following_input() {
    with_source(
        "typedef int V __attribute__((vector_size(16))); typedef float F \
         __attribute__((vector_size(16))); void f(V v,F a,int i,long l) { v=v+l; a=a%a; v=!v; \
         v=__builtin_shufflevector(v,v,i,1,2,3); v=__builtin_convertvector(v,int); \
         __builtin_ia32_pshufd(v,i); } int following;\n",
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                6,
                "{:?}",
                context.take_pending_errors()
            );
            let following = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "following")
                .unwrap();
            assert_eq!(
                sema.types.nodes[following.ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
        },
    );
}

#[test]
fn overloaded_intrinsics_reject_invalid_element_types() {
    with_source(
        "typedef unsigned U __attribute__((vector_size(16))); struct S {int n;}; void f(U u,void \
         *p,struct S s,double _Complex c) {__builtin_elementwise_abs(u); \
         __builtin_nontemporal_load(p); __builtin_nondeterministic_value(s); \
         __builtin_elementwise_abs(c);}",
        |context, _| assert_eq!(context.pending_error_count(), 4),
    );
}

#[test]
fn vector_comparison_masks_have_natural_alignment() {
    with_source(
        "typedef int A __attribute__((vector_size(16),aligned(1))); void f(A a) {a<a;}",
        |context, sema| {
            assert_eq!(context.pending_error_count(), 0);
            let mask = sema
                .expressions
                .iter()
                .find(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::LessThan,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(
                sema.types.layout(mask.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
        },
    );
}

// Load the behavior fixtures through the translation-unit seam.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "Fixture discovery and loading are outside compiler allocation measurements."
)]
fn vector_fixture_regressions() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic/vectors");
    let mut paths = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "vector fixture corpus is empty");
    for path in paths {
        let source = std::fs::read_to_string(&path).unwrap();
        let expected: usize = source
            .lines()
            .next()
            .unwrap()
            .strip_prefix("// errors: ")
            .unwrap()
            .parse()
            .unwrap();
        for target in [
            Target::LinuxGnu,
            Target::LinuxMusl,
            Target::WindowsGnu,
            Target::WindowsMsvc,
        ] {
            // MSVC rejects __float128 itself; its target diagnostic has
            // separate float128 coverage. Exercise vector rules on
            // supporting targets.
            if target == Target::WindowsMsvc && source.contains("__float128") {
                continue;
            }
            for (standard, gnu) in [
                (CStandard::C99, false),
                (CStandard::C17, true),
                (CStandard::C23, false),
            ] {
                let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_gnu_extensions(gnu)
                    .with_target(target);
                with_configuration(&source, configuration, |context, sema| {
                    assert_eq!(
                        context.pending_error_count(),
                        expected,
                        "{}: {:?}",
                        path.display(),
                        context.take_pending_errors()
                    );
                    let following = sema
                        .bindings
                        .iter()
                        .find(|b| context.string_cache.at(b.name.name) == "following")
                        .unwrap();
                    assert_eq!(
                        sema.types.nodes[following.ty.index],
                        TypeKind::Scalar(Scalar::Int)
                    );
                    if expected == 0 {
                        for info in sema.expressions.iter().filter(|i| {
                            matches!(
                                i.expression.kind,
                                ExpressionType::Call { .. }
                                    | ExpressionType::Binary { .. }
                                    | ExpressionType::Conditional(_)
                                    | ExpressionType::Cast { .. }
                            )
                        }) {
                            assert_ne!(
                                sema.types.nodes[info.ty.index],
                                TypeKind::Unknown,
                                "{}: {:?}",
                                path.display(),
                                info.expression.kind
                            );
                        }
                    }
                });
            }
        }
    }
}

/// Each identity accepts and refuses the vector operations its compiler
/// does: the scalar splats GCC's `scalar_to_vector` and Clang's
/// `tryGCCVectorConvertAndSplat` accept, GCC's strict and Clang's lax
/// conversions between vectors, GCC's `++` and `&v[i]`, Clang's lax `?:`,
/// GCC's power-of-two lane counts and its `__builtin_shuffle`. Each case was
/// checked against GCC 15.2 and the pinned Clang.
#[test]
fn vector_rules_follow_each_identity() {
    use crate::configuration::CompilerIdentity;
    const PRELUDE: &str =
        "typedef int v4si __attribute__((vector_size(16))); typedef unsigned v4su \
         __attribute__((vector_size(16))); typedef float v4sf __attribute__((vector_size(16))); \
         typedef signed char v16qi __attribute__((vector_size(16))); typedef long long v2di \
         __attribute__((vector_size(16))); typedef int v2si __attribute__((vector_size(8))); enum \
         E { E0 }; extern v4si a, b; extern v4su u; extern v4sf f; extern v16qi q; extern v2di l; \
         extern v2si s2; extern int i; extern unsigned ui; extern long long ll; extern float fl; \
         extern double db; extern long double ld; extern _Bool bo; extern enum E en; extern short \
         sh; extern _Atomic v4si av; extern _Atomic v4sf af;\n";
    // (statement, accepted by GCC, accepted by Clang)
    for (body, gcc, clang) in [
        ("f = f * 2;", true, Some(true)),
        ("f = f * 16777217;", false, Some(false)),
        ("f = f * i;", false, Some(false)),
        ("f = f * sh;", true, Some(true)),
        ("f = f * 0.1;", false, Some(false)),
        ("f = f * 1.5;", true, Some(true)),
        ("a = a + ui;", true, Some(false)),
        ("a = a + fl;", false, Some(true)),
        ("a = a + 1.0f;", false, Some(true)),
        ("a = a + ld;", false, Some(true)),
        ("a = a + bo;", false, Some(true)),
        ("l = l + en;", false, Some(true)),
        ("q = q + 200;", false, Some(true)),
        ("q = q + 4294967295u;", true, Some(false)),
        ("s2 = s2 + ll;", false, Some(true)),
        ("s2 += ll;", false, Some(false)),
        ("a = a << ll;", true, Some(true)),
        ("a = ll << a;", false, Some(true)),
        ("a = a + u;", true, Some(true)),
        ("a = u;", false, Some(true)),
        ("a = f;", false, Some(true)),
        ("a = a + f;", false, Some(true)),
        ("u = a < u;", true, Some(true)),
        ("f = f == f;", true, Some(true)),
        ("a = i ? a : u;", false, Some(true)),
        ("a++; --a;", true, Some(false)),
        ("int *p = &a[1];", true, Some(false)),
        ("a = (v4si)l; ll = (long long)s2;", true, Some(true)),
        ("db = (double)s2;", false, Some(false)),
        (
            "typedef int odd __attribute__((vector_size(12))); odd o;",
            false,
            Some(true),
        ),
        (
            "typedef int V __attribute__((vector_size(sizeof(int) * 4))); V v = a;",
            true,
            Some(true),
        ),
        // Clang has no `__builtin_shuffle`.
        ("a = __builtin_shuffle(a, b);", true, Some(false)),
        ("a = __builtin_shuffle(a, f);", false, Some(false)),
        // Atomic operands: GCC reads them first; Clang splats no scalar into
        // one and refuses two in arithmetic, not in a comparison.
        ("av *= 3; av ^= 5;", true, Some(false)),
        ("av <<= 1; av += a;", true, Some(true)),
        ("a = av + av;", true, Some(false)),
        ("a = av < av;", true, Some(true)),
        ("av++;", true, Some(false)),
        ("af++;", false, Some(false)),
        // GCC converts a compound assignment's scalar into an atomic vector
        // as a variable, and designates an atomic lane of one.
        ("af += 1;", false, Some(false)),
        ("af *= 2;", false, Some(false)),
        ("af += 1.0f; af = af + 1;", true, Some(false)),
        (
            "av[1] = 2; av[2] += 1; av[3]++; _Atomic int *p = &av[0];",
            true,
            Some(false),
        ),
        ("i = av[1];", true, Some(true)),
        // Lane types: both take the half-precision types; Clang a
        // power-of-two bit-precise integer and no pointer or array lanes;
        // GCC an enumeration.
        (
            "typedef _Float16 h __attribute__((vector_size(16))); h x; x = x * x;",
            true,
            Some(true),
        ),
        (
            "typedef __bf16 h __attribute__((vector_size(16))); h x;",
            true,
            Some(true),
        ),
        (
            "typedef _BitInt(32) b __attribute__((vector_size(16))); b x; x = x + x;",
            false,
            Some(true),
        ),
        (
            "typedef _BitInt(24) b __attribute__((vector_size(12)));",
            false,
            Some(false),
        ),
        (
            "typedef enum E e __attribute__((vector_size(16))); e x;",
            true,
            Some(false),
        ),
        // No scalar operand converts to GCC's enumeration lanes.
        (
            "typedef enum E e __attribute__((vector_size(16))); e x, y; x = x + y;",
            true,
            Some(false),
        ),
        (
            "typedef enum E e __attribute__((vector_size(16))); e x; x = x + 1;",
            false,
            Some(false),
        ),
        (
            "typedef enum E e __attribute__((vector_size(16))); e x; x = 1 + x;",
            false,
            Some(false),
        ),
        (
            "typedef enum E e __attribute__((vector_size(16))); e x; x = x << 1;",
            false,
            Some(false),
        ),
        (
            "typedef int *p __attribute__((vector_size(16)));",
            true,
            Some(false),
        ),
        ("a = __builtin_shufflevector(a, b);", false, Some(true)),
    ] {
        for (identity, accepted) in [
            (CompilerIdentity::Gcc, Some(gcc)),
            (CompilerIdentity::Clang, clang),
        ] {
            let Some(accepted) = accepted else {
                continue;
            };
            let source = format!("{PRELUDE}void t(void) {{ {body} }}");
            with_configuration(
                &source,
                CompilerConfiguration::default().with_compiler_identity(identity),
                |context, _| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        errors.is_empty(),
                        accepted,
                        "{identity:?} {body}: {errors:?}"
                    );
                },
            );
        }
    }
}
