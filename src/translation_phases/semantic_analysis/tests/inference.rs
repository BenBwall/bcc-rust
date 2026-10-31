//! C23 `auto` and GNU `__auto_type`: the inferred type after lvalue
//! conversion and decay, and each identity's constraints. C23 §6.7.10p1-2,
//! §6.7.1p12, §6.2.1p7; GCC manual, "Typeof".

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        CompilerIdentity,
        ExtensionPolicy,
    },
    translation_phases::semantic_analysis::inference::InferenceFault,
};

/// The semantic error kinds of `source` in GNU C23 under `identity`.
fn kinds(source: &str, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity)
            .with_target(crate::target::Target::LinuxGnu),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    result.push(error.kind);
                }
            }
        },
    );
    result
}

/// Asserts that `source` has no semantic errors under either identity.
fn holds(source: &str) {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(kinds(source, identity), [], "{identity:?}: {source}");
    }
}

#[test]
fn inferred_types_follow_lvalue_conversion_and_decay() {
    for keyword in ["auto", "__auto_type"] {
        holds(&format!(
            "int g(int); const int c = 1; volatile double v; int a[3]; struct s {{ int m; }} s; \
             void f(int n) {{ {keyword} i = c; {keyword} d = v; {keyword} p = a; {keyword} q = g; \
             {keyword} t = \"ab\"; {keyword} r = s; {keyword} x = 1u + 2L; int w[n]; {keyword} vp \
             = &w; const {keyword} k = 'c'; static {keyword} st = 1.5f; \
             _Static_assert(_Generic(&i, int *: 1, default: 0), \"i\"); \
             _Static_assert(_Generic(&d, double *: 1, default: 0), \"d\"); \
             _Static_assert(_Generic(&p, int **: 1, default: 0), \"p\"); \
             _Static_assert(_Generic(&q, int (**)(int): 1, default: 0), \"q\"); \
             _Static_assert(_Generic(&t, char **: 1, default: 0), \"t\"); \
             _Static_assert(_Generic(&r, struct s *: 1, default: 0), \"r\"); \
             _Static_assert(_Generic(&x, long *: 1, default: 0), \"x\"); \
             _Static_assert(_Generic(&k, const int *: 1, default: 0), \"k\"); \
             _Static_assert(_Generic(&st, float *: 1, default: 0), \"st\"); (void)sizeof(*vp); }}"
        ));
    }
    // A file-scope C23 `auto` object has external linkage and static
    // storage duration, as without any storage-class specifier.
    holds(
        "static auto a = 3.5; auto p = &a; extern double *p; _Static_assert(_Generic(&p, double \
         **: 1, default: 0), \"p\");",
    );
}

#[test]
fn wide_and_extended_types_infer() {
    holds(
        "_Complex double z; __float128 q; _Float16 h; typedef int v4 \
         __attribute__((vector_size(16))); v4 lanes; void f(void) { __auto_type a = z * 2; \
         __auto_type b = q / 3; __auto_type c = h * h; __auto_type d = lanes + lanes; __auto_type \
         e = (_BitInt(37))5; __auto_type w = (unsigned _BitInt(300))1; \
         _Static_assert(_Generic(&a, _Complex double *: 1, default: 0), \"a\"); \
         _Static_assert(_Generic(&b, __float128 *: 1, default: 0), \"b\"); \
         _Static_assert(_Generic(&c, _Float16 *: 1, default: 0), \"c\"); \
         _Static_assert(_Generic(&d, v4 *: 1, default: 0), \"d\"); _Static_assert(_Generic(&e, \
         _BitInt(37) *: 1, default: 0), \"e\"); _Static_assert(_Generic(&w, unsigned _BitInt(300) \
         *: 1, default: 0), \"w\"); }",
    );
}

#[test]
fn atomic_operands_follow_each_compiler() {
    // GCC's lvalue conversion drops `_Atomic`; Clang's deduction keeps the
    // atomic type, only removing qualifiers.
    let source = "_Atomic int a; void f(void) { __auto_type x = a; auto y = a; \
                  _Static_assert(_Generic(&x, int *: 1, default: 0), \"x\"); \
                  _Static_assert(_Generic(&y, int *: 1, default: 0), \"y\"); _Atomic auto z = 1; \
                  _Static_assert(_Generic(&z, _Atomic int *: 1, default: 0), \"z\"); }";
    assert_eq!(kinds(source, CompilerIdentity::Gcc), []);
    assert_eq!(
        kinds(source, CompilerIdentity::Clang),
        [
            SemanticErrorKind::FailedAssertion,
            SemanticErrorKind::FailedAssertion
        ]
    );
}

#[test]
fn faults_follow_each_compiler() {
    use InferenceFault as F;
    for (source, gcc, clang) in [
        (
            "void f(void) { __auto_type a; }",
            &[F::MissingInitializer][..],
            &[F::MissingInitializer][..],
        ),
        (
            "void f(void) { auto a = {1}; }",
            &[F::InitializerList],
            &[F::InitializerList],
        ),
        // GCC reads only one declarator; Clang deduces each and wants one
        // type for them all.
        (
            "void f(void) { auto a = 1, b = 2; }",
            &[F::MultipleDeclarators],
            &[],
        ),
        (
            "void f(void) { auto a = 1, *b = &a, c = 3; }",
            &[F::MultipleDeclarators],
            &[],
        ),
        (
            "void f(void) { auto a = 1, b = 2.0; }",
            &[F::MultipleDeclarators],
            &[F::InconsistentDeduction],
        ),
        // GCC wants a plain identifier; Clang deduces through pointer and
        // function declarators, but not an array or a function.
        (
            "int i; void f(void) { auto *p = &i; }",
            &[F::NotPlainIdentifier],
            &[],
        ),
        (
            "int g(int); void f(void) { auto (*p)(int) = g; }",
            &[F::NotPlainIdentifier],
            &[],
        ),
        (
            "int a[3]; void f(void) { auto (*p)[3] = &a; }",
            &[F::NotPlainIdentifier],
            &[],
        ),
        (
            "void f(void) { auto *p = 0; }",
            &[F::NotPlainIdentifier],
            &[F::DeductionMismatch],
        ),
        (
            "void f(void) { __auto_type a[2] = {1, 2}; }",
            &[F::NotPlainIdentifier],
            &[F::ArrayDeclarator],
        ),
        (
            "void f(void) { auto g(void); }",
            &[F::NotPlainIdentifier],
            &[F::FunctionDeclarator],
        ),
        // GCC's `__auto_type` initializer sees the outer `x`.
        (
            "int x; void f(void) { __auto_type x = x; }",
            &[],
            &[F::SelfReference],
        ),
        (
            "int x; void f(void) { auto x = x + 1; }",
            &[F::SelfReference],
            &[F::SelfReference],
        ),
        (
            "void f(void) { auto x = x; }",
            &[F::SelfReference],
            &[F::SelfReference],
        ),
        ("void f(void) { auto x = ({ int x = 2; x; }); }", &[], &[]),
        (
            "void f(void) { auto x = sizeof(x); }",
            &[F::SelfReference],
            &[F::SelfReference],
        ),
        // Bit-fields: GCC rejects only a member access for `__auto_type`;
        // Clang rejects a member access, parenthesized or not, and deduces
        // `int` from any other operand.
        (
            "struct s { int b : 3; } s; void f(void) { auto x = (s.b); }",
            &[],
            &[F::BitField],
        ),
        (
            "struct s { int b : 3; } s, *ps; void f(void) { auto x = ps->b; }",
            &[],
            &[F::BitField],
        ),
        (
            "struct s { int b : 3; } s; void f(void) { auto x = (0, s.b); auto y = (s.b = 3); \
             auto z = ++s.b; auto w = (s.b += 1); _Static_assert(_Generic(&x, int *: 1), \"\"); }",
            &[],
            &[],
        ),
        (
            "struct s { int b : 3; } s; void f(void) { __auto_type x = s.b; }",
            &[F::BitField],
            &[F::BitField],
        ),
        (
            "struct s { int b : 3; } s; void f(void) { auto x = s.b; }",
            &[],
            &[F::BitField],
        ),
        (
            "struct s { int b : 3; } s; void f(void) { auto x = s.b + 1; }",
            &[],
            &[],
        ),
        // C23 `auto` may not redeclare a name of its scope.
        (
            "extern int e; auto e = 1;",
            &[F::Redeclared],
            &[F::Redeclared],
        ),
        ("int z; __auto_type z = 2;", &[], &[]),
        ("auto q = 1; extern int q;", &[], &[]),
        // GCC rejects a tag declared inside a C23 `auto` initializer.
        (
            "void f(void) { auto x = (struct t { int m; }){1}; }",
            &[F::TagInInitializer],
            &[],
        ),
        (
            "void f(void) { auto p = (struct u *)0; }",
            &[F::TagInInitializer],
            &[],
        ),
        (
            "void f(void) { __auto_type y = (struct v { int m; }){1}; }",
            &[],
            &[],
        ),
        // Inference belongs to initialized object declarations only.
        (
            "void f(__auto_type a);",
            &[F::NotAllowedHere],
            &[F::NotAllowedHere],
        ),
        (
            "struct m { __auto_type a; };",
            &[F::NotAllowedHere],
            &[F::NotAllowedHere],
        ),
        (
            "int n = sizeof(__auto_type);",
            &[F::NotAllowedHere],
            &[F::NotAllowedHere],
        ),
        (
            "__auto_type f(void) { return 1; }",
            &[F::FunctionDeclarator],
            &[F::FunctionDeclarator],
        ),
        (
            "typedef __auto_type T;",
            &[F::NotAllowedHere],
            &[F::NotAllowedHere],
        ),
        (
            "void f(void) { auto __auto_type x = 1; }",
            &[],
            &[F::AutoWithAutoType],
        ),
        (
            "void f(void) { auto (*p)[] = (int (*)[3])0; }",
            &[F::NotPlainIdentifier],
            &[F::DeductionMismatch],
        ),
        (
            "void f(void) { auto (*p)[] = (int (*)[])0; }",
            &[F::NotPlainIdentifier],
            &[],
        ),
        // Attributes may follow the declarator.
        (
            "auto g __attribute__((unused)) = 1; void f(void) { auto x __attribute__((unused)) = \
             1; auto y [[maybe_unused]] = 2; }",
            &[],
            &[],
        ),
        (
            "void f(void) { static auto int x = 1; }",
            &[F::StorageWithoutInference],
            &[F::StorageWithoutInference],
        ),
    ] {
        for (identity, expected) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            let faults: Vec<_> = kinds(source, identity)
                .into_iter()
                .filter_map(|kind| match kind {
                    | SemanticErrorKind::InvalidInference(fault) => Some(fault),
                    | _ => None,
                })
                .collect();
            assert_eq!(faults, expected, "{identity:?}: {source}");
        }
    }
}
