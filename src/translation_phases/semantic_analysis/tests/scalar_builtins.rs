//! Documented GCC scalar intrinsic types and invalid operand diagnostics.
use super::*;

#[test]
fn gcc_scalar_builtin_types_follow_the_target() {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::WindowsGnu,
    ] {
        with_configuration(
            r"
unsigned short a;unsigned b;unsigned long long c;long l;void *p;__SIZE_TYPE__ size;
int main(void) {int r; a=__builtin_bswap16(1);b=__builtin_bswap32(1);c=__builtin_bswap64(1);l=__builtin_expect(1,1);p=__builtin_alloca(4);size=__builtin_object_size(p,0);r=__builtin_clz(1);return __builtin_add_overflow(1,2,&r);}
",
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, s| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                let calls = s
                    .expressions
                    .iter()
                    .filter(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                    .map(|i| s.types.nodes[i.ty.index])
                    .collect::<Vec<_>>();
                assert_eq!(
                    &calls[..4],
                    &[
                        TypeKind::Scalar(Scalar::UnsignedShort),
                        TypeKind::Scalar(Scalar::UnsignedInt),
                        TypeKind::Scalar(if target == crate::target::Target::LinuxGnu {
                            Scalar::UnsignedLong
                        } else {
                            Scalar::UnsignedLongLong
                        }),
                        TypeKind::Scalar(Scalar::Long)
                    ]
                );
                assert!(matches!(calls[4], TypeKind::Pointer(_)));
                assert_eq!(calls[5], TypeKind::Scalar(s.types.target.size_t));
                assert_eq!(calls[6], TypeKind::Scalar(Scalar::Int));
                assert_eq!(calls[7], TypeKind::Scalar(Scalar::Bool));
            },
        );
    }
}

#[test]
fn gcc_scalar_builtin_counts_types_and_immediates_are_checked() {
    for (call, expected) in [
        ("__builtin_clz()", SemanticErrorKind::InvalidArgumentCount),
        (
            "__builtin_clzll(1,2)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_bswap16(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        ("__builtin_ctz(&r)", SemanticErrorKind::InvalidArgumentType),
        (
            "__builtin_popcount(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_parity(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        ("__builtin_ffs(&r)", SemanticErrorKind::InvalidArgumentType),
        (
            "__builtin_clrsb(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_expect(1)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_expect_with_probability(1,1,1.5)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_assume_aligned(&r,3)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_unreachable(1)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        ("__builtin_trap(1)", SemanticErrorKind::InvalidArgumentCount),
        (
            "__builtin_prefetch(&r,2,3)",
            SemanticErrorKind::InvalidImmediate,
        ),
        (
            "__builtin_prefetch(&r,0,4)",
            SemanticErrorKind::InvalidImmediate,
        ),
        (
            "__builtin_prefetch(1.0)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_constant_p()",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_object_size(&r,4)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_dynamic_object_size(&r,r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_alloca(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_add_overflow(1,2)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_sub_overflow(1,2,&b)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow(1.0,2,&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_add_overflow(1,2,&e)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow(1,2,&c)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_uadd_overflow(1,2,&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow_p(1,2,b)",
            SemanticErrorKind::InvalidArgumentType,
        ),
    ] {
        let source = format!("void f(void) {{int r;_Bool b;enum E {{A}} e;const int c=0;{call};}}");
        with_source(&source, |context, _| {
            let errors = context.take_pending_errors();
            assert!(errors.iter().any(|error|matches!(error,TranslationError::Semantic(SemanticError {kind,..}) if *kind==expected)),"{call}: {errors:?}");
        });
    }
}

#[test]
fn gcc_scalar_constant_queries_and_overflow_predicates_are_ices() {
    with_source(
        r#"
_Static_assert(__builtin_constant_p(1+2),"ICE");
_Static_assert(!__builtin_constant_p(1.0),"conservative");
_Static_assert(__builtin_add_overflow_p(127,1,(signed char)0),"narrow");
_Static_assert(!__builtin_add_overflow_p(-1,1u,0u),"mixed");
_Static_assert(__builtin_sub_overflow_p(0u,1u,0u),"negative");
_Static_assert(__builtin_mul_overflow_p((unsigned __int128)1<<127,2,(unsigned __int128)0),"wide");
int r; volatile int v;
_Static_assert(!__builtin_add_overflow_p(1,2,r),"ignored lvalue");
_Static_assert(!__builtin_add_overflow_p(1,2,(__INTPTR_TYPE__)&v),"ignored address");
int main(void){return 0;}
"#,
        |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
        },
    );
}

#[test]
fn gcc_scalar_builtin_policy_uses_the_shared_extension_gate() {
    with_configuration(
        "int f(unsigned x){return __builtin_clz(x);}",
        crate::configuration::CompilerConfiguration::new(
            CStandard::C99,
            crate::configuration::ExtensionPolicy::Deny,
        ),
        |context, _| {
            assert!(context.pending_error_count() > 0);
        },
    );
}
