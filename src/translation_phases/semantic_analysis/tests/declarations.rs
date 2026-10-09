//! Declaration, tag and layout regressions from the Stage-1 review.

use std::fmt::Write as _;

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

fn gnu17() -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow).with_gnu_extensions(true)
}

fn pedantic(standard: CStandard, gnu: bool) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, ExtensionPolicy::Deny).with_gnu_extensions(gnu)
}

/// Semantic error kinds in discovery order.
fn kinds(source: &str, configuration: CompilerConfiguration) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Semantic(e) => kinds.push(e.kind),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    kinds
}

/// Extension diagnostics, in order, as their rendered messages.
fn extensions(source: &str, configuration: CompilerConfiguration) -> Vec<String> {
    let mut messages = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Extension(e) => messages.push(format!("{e}")),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    messages
}

fn tag_layout(source: &str, configuration: CompilerConfiguration, name: &str) -> Option<Layout> {
    let mut layout = None;
    with_configuration(source, configuration, |context, s| {
        assert_eq!(context.pending_error_count(), 0, "{source}");
        layout = s
            .types
            .tags
            .iter()
            .find(|t| t.name.is_some_and(|n| context.string_cache.at(n) == name))
            .and_then(|t| t.layout.get());
    });
    layout
}

#[test]
fn nested_tag_redefinition_is_diagnosed_once_and_terminates() {
    assert_eq!(
        kinds(
            "struct S { struct S { int x; } y; int z; }; struct S s; void f(void) { s = s; } int \
             n = sizeof(s);",
            gnu17()
        ),
        [SemanticErrorKind::TagRedefinition]
    );
    assert_eq!(
        kinds("enum E { A = sizeof(enum E { B }) };", gnu17()),
        [SemanticErrorKind::TagRedefinition]
    );
}

#[test]
fn rejected_tag_bodies_still_declare_their_contents() {
    assert_eq!(
        kinds(
            "struct S { int x; }; struct S { enum { Q = 2 } q; }; int a[Q]; enum E { A }; enum E \
             { B = 3 }; int b[B]; union S { int u; enum { R } r; } v; int c[R + 1];",
            gnu17()
        ),
        [
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagKindMismatch,
        ]
    );
}

#[test]
fn self_containing_records_terminate() {
    assert_eq!(
        kinds(
            "struct S { int a; struct S s; }; struct S x; void f(void) { x = x; x.a = 1; struct S \
             y = {1}; }",
            gnu17()
        ),
        [SemanticErrorKind::InvalidMember]
    );
}

#[test]
fn declaration_attributes_never_erase_a_shared_tag_layout() {
    assert_eq!(
        kinds(
            "struct A { int a; }; struct B { double b; }; __attribute__((unused)) static struct A \
             ua; struct A g(void); struct B g(void); _Alignas(8) struct A aligned; extern struct \
             A obj; extern int obj;",
            gnu17()
        ),
        [
            SemanticErrorKind::IncompatibleDeclaration,
            SemanticErrorKind::IncompatibleDeclaration,
        ]
    );
    assert_eq!(
        tag_layout(
            "struct A { int a; }; __attribute__((unused)) static struct A ua; _Alignas(8) struct \
             A b;",
            gnu17(),
            "A"
        ),
        Some(Layout { size: 4, align: 4 })
    );
}

#[test]
fn only_layout_attributes_make_declarator_types_unanalyzed() {
    // Function, pointer and member attributes that do not change layout keep
    // the declared type, so later conflicts are still found.
    assert_eq!(
        kinds(
            "extern int printf(const char *, ...) __attribute__((__format__(__printf__, 1, 2))); \
             extern long printf(const char *, ...); int *__attribute__((unused)) p; long *p;",
            gnu17()
        ),
        [
            SemanticErrorKind::IncompatibleDeclaration,
            SemanticErrorKind::IncompatibleDeclaration,
        ]
    );
    // x86-64 ignores calling conventions; pointer-size modifiers change layout.
    assert_eq!(
        kinds(
            "void __cdecl c(void); int c(void); int *__ptr32 r; long r;",
            gnu17().with_msvc_extensions(true)
        ),
        [SemanticErrorKind::IncompatibleDeclaration]
    );
    // Alignment, packing, modes and vectors are not modeled.
    assert_eq!(
        kinds(
            "int x __attribute__((aligned(16))); extern long x; int \
             *__attribute__((__aligned__(8))) q; long q; int m __attribute__((mode(DI))); long m;",
            gnu17()
        ),
        []
    );
    assert_eq!(
        tag_layout(
            "struct S { int a __attribute__((unused)); char b; };",
            gnu17(),
            "S"
        ),
        Some(Layout { size: 8, align: 4 })
    );
    assert_eq!(
        tag_layout(
            "struct S { int a; char b __attribute__((aligned(8))); };",
            gnu17(),
            "S"
        ),
        None
    );
}

#[test]
fn unanalyzed_redeclarations_inherit_the_previous_kind() {
    assert_eq!(
        kinds(
            "int f(void); extern __typeof__(f) f; __typeof__(f) f; int g(void) { return 0; } \
             extern __typeof__(g) g __asm__(\"g_alias\"); void h(void) { f(); g(); }",
            gnu17()
        ),
        []
    );
    // GNU typeof stays an extension under the pedantic policy.
    assert_eq!(
        extensions(
            "int f(void); extern __typeof__(f) f;",
            pedantic(CStandard::C99, false)
        ),
        ["'__typeof__' is a GNU extension"]
    );
}

#[test]
fn enums_use_the_gcc_x86_64_compatible_integer_type() {
    for configuration in [CompilerConfiguration::default(), gnu17()] {
        assert_eq!(
            kinds(
                "enum E {A, B}; unsigned int f(void); enum E f(void); int x[(enum E)-1 > 0 ? 1 : \
                 -1]; enum N {M = -1}; int g(void); enum N g(void); int y[(enum N)-1 < 0 ? 1 : \
                 -1]; int z[A - 1 < 0 ? 1 : -1]; void h(enum E e) { int w[e - 1 > 0 ? 1 : 1]; }",
                configuration
            ),
            []
        );
        assert_eq!(
            kinds(
                "enum E {A}; int f(void); enum E f(void); enum N {M = -1}; unsigned g(void); enum \
                 N g(void);",
                configuration
            ),
            [
                SemanticErrorKind::IncompatibleDeclaration,
                SemanticErrorKind::IncompatibleDeclaration,
            ]
        );
    }
    assert_eq!(
        tag_layout("enum L {P = 0x100000000L};", gnu17(), "L"),
        Some(Layout { size: 8, align: 8 })
    );
    assert_eq!(
        tag_layout("enum L {P = -1, Q = 0x7fffffff};", gnu17(), "L"),
        Some(Layout { size: 4, align: 4 })
    );
}

#[test]
fn enumerators_beyond_int_are_a_policy_extension() {
    let source = "enum { HI = 0x80000000, ALL = 0xFFFFFFFF, NEXT }; int t[HI > 0 ? 1 : -1]; int \
                  u[NEXT == 0x100000000L ? 1 : -1];";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C99, false)),
        [
            "'enumerator value outside the range of int' is a C23 extension",
            "'enumerator value outside the range of int' is a C23 extension",
            "'enumerator value outside the range of int' is a C23 extension",
        ]
    );
    assert_eq!(
        extensions(source, pedantic(CStandard::C23, false)),
        Vec::<String>::new()
    );
    assert_eq!(
        kinds("enum { N = -1, U = 0xFFFFFFFFFFFFFFFF };", gnu17()),
        [SemanticErrorKind::EnumeratorRange]
    );
}

#[test]
fn exceptional_array_bounds_are_variable_length_arrays() {
    let source = "void f(void) { int b[1/0]; int c[(1 << 31) / 4 + 1]; (void)b; }";
    assert_eq!(
        kinds(source, gnu17()),
        [SemanticErrorKind::InvalidArrayBound]
    );
    with_configuration(source, gnu17(), |_, s| {
        let b = s
            .bindings
            .iter()
            .find(|b| b.duration == Duration::Automatic)
            .unwrap();
        assert!(matches!(
            s.types.nodes[b.ty.index],
            TypeKind::Array(_, ArrayBound::Variable)
        ));
    });
    assert_eq!(
        kinds(
            "int g[1/0]; enum { E = 1/0 }; void h(int x) { switch (x) { case 1/0: break; } } \
             struct S { int w : 1/0; };",
            gnu17()
        ),
        [
            SemanticErrorKind::FileScopeVariableType,
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
        ]
    );
}

#[test]
fn shifting_into_the_sign_bit_is_a_gnu_extension() {
    let source = "enum { F = 1 << 31 }; int x[(1 << 31) < 0 ? 1 : -1]; static int s = 1 << 31; \
                  int y[F < 0 ? 1 : -1];";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C99, false)),
        ["'left shift into the sign bit' is a GNU extension"; 3]
    );
    assert_eq!(
        kinds("enum { G = 3 << 31, H = 1 << 32 };", gnu17()),
        [
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
        ]
    );
}

#[test]
fn shared_deep_typedefs_answer_variably_modified_queries_once() {
    // Each declarator once walked the whole pointer chain of its typedef.
    let count = 60_000;
    let mut source = format!("typedef int {}P; P a0", "*".repeat(count));
    for index in 1..count {
        write!(source, ", a{index}").unwrap();
    }
    source.push(';');
    let start = std::time::Instant::now();
    assert_eq!(kinds(&source, gnu17()), []);
    assert!(start.elapsed().as_secs() < 15, "{:?}", start.elapsed());
}

#[test]
fn unnamed_bit_fields_do_not_align_records() {
    // Expectations come from Linux-target Clang static assertions.
    for (source, name, size, align) in [
        ("struct L1 {char a; int :4;};", "L1", 2, 1),
        ("struct L2 {char a; int :4; char b;};", "L2", 3, 1),
        ("union L3 {char c; int :3;};", "L3", 1, 1),
        ("union L4 {char c; int :12;};", "L4", 2, 1),
        ("struct L5 {char a; long :0; char b;};", "L5", 9, 1),
        ("union L6 {char c; int x:3;};", "L6", 4, 4),
    ] {
        assert_eq!(
            tag_layout(source, gnu17(), name),
            Some(Layout { size, align }),
            "{source}"
        );
    }
}

fn c11() -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow)
}

#[test]
fn anonymous_members_have_layout_names_and_initializers() {
    let records = "struct A { int a; union { int b; float c; }; int d; }; struct N { int x; \
                   struct { union { int y; char z; }; int w; }; };";
    for configuration in [c11(), gnu17()] {
        assert_eq!(
            tag_layout(records, configuration, "A"),
            Some(Layout {
                size:  12,
                align: 4,
            })
        );
        assert_eq!(
            tag_layout(records, configuration, "N"),
            Some(Layout {
                size:  12,
                align: 4,
            })
        );
    }
    with_configuration(records, c11(), |context, s| {
        let n = s
            .types
            .tags
            .iter()
            .find(|t| t.name.is_some_and(|n| context.string_cache.at(n) == "N"))
            .unwrap();
        let fields = n.fields.get();
        assert_eq!(
            fields
                .iter()
                .map(|f| (f.offset, f.path.len()))
                .collect::<Vec<_>>(),
            [(0, 1), (4, 3), (4, 3), (8, 2)]
        );
        assert!(n.members.get()[1].anonymous);
    });
    assert_eq!(
        kinds(
            &format!(
                "{records} void f(void) {{ struct A s = {{1, 2, 3}}; struct A t = {{.b = 2, .d = \
                 4}}; struct A u = {{.c = 1.0f}}; struct N n = {{.y = 1, .w = 2}}; s.b = 1; s.c = \
                 2.0f; s.d = 3; struct A *p = &s; p->b = 4; n.z = 'a'; int *q = &n.w; }}"
            ),
            c11()
        ),
        []
    );
    assert_eq!(
        kinds(
            "struct A { int a; union { int b; float c; }; int d; } e = {1, 2, 3, 4}; struct C { \
             const union { int b; }; } cs; void g(void) { cs.b = 1; cs.missing = 2; }",
            c11()
        ),
        [
            SemanticErrorKind::ExcessInitializer,
            SemanticErrorKind::ExpectedModifiableLvalue,
            SemanticErrorKind::InvalidMemberAccess,
        ]
    );
}

#[test]
fn anonymous_member_names_share_the_containing_namespace() {
    assert_eq!(
        kinds(
            "struct D { int a; union { int a; float f; }; }; struct E { union { int x; }; struct \
             { int x; }; }; struct F { struct { int y; }; int y; };",
            c11()
        ),
        [
            SemanticErrorKind::DuplicateMember,
            SemanticErrorKind::DuplicateMember,
            SemanticErrorKind::DuplicateMember,
        ]
    );
}

#[test]
fn msvc_tagged_anonymous_members_join_the_namespace() {
    assert_eq!(
        kinds(
            "struct T { int t; }; typedef struct T TT; struct S { struct T; int u; } s; struct R \
             { TT; } r; void f(void) { s.t = 1; r.t = 2; s.u = 3; }",
            gnu17().with_msvc_extensions(true)
        ),
        []
    );
}

#[test]
fn gnu_flexible_array_forms_are_policy_extensions() {
    let source = "union U { int a; int b[]; }; struct F { int n; int d[]; }; struct G { struct F \
                  f; int after; }; struct F arr[2]; struct E { int only[]; };";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C17, true)),
        [
            "'flexible array member in a union' is a GNU extension",
            "'structure with a flexible array member nested in a structure' is a GNU extension",
            "'array of structures with a flexible array member' is a GNU extension",
            "'flexible array member in an otherwise empty structure' is a GNU extension",
        ]
    );
    for (name, size, align) in [("U", 4, 4), ("G", 8, 4), ("E", 0, 4)] {
        assert_eq!(
            tag_layout(source, gnu17(), name),
            Some(Layout { size, align }),
            "{name}"
        );
    }
    // A flexible array that is not the last structure member stays invalid.
    assert_eq!(
        kinds("struct H { int d[]; int n; };", gnu17()),
        [SemanticErrorKind::InvalidMember]
    );
}

#[test]
fn gnu_folding_accepts_classic_offsetof_and_constant_p() {
    let source = "struct S { int a; char b[8]; int c; }; static char buf[((unsigned \
                  long)&((struct S*)0)->c)]; int chk[sizeof(buf) == 12 ? 1 : -1]; enum { K = \
                  (unsigned long)&((struct S*)0)->b[3] }; int chk3[K == 7 ? 1 : -1];";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C17, true)),
        ["'folded integer constant expression' is a GNU extension"; 2]
    );
    // Strict C99 keeps the integer-constant-expression constraint.
    assert_eq!(
        kinds(
            "struct S { int a; int c; }; static char buf[((unsigned long)&((struct S*)0)->c)];",
            CompilerConfiguration::default()
        ),
        [SemanticErrorKind::FileScopeVariableType]
    );
    // GCC's builtin is constant in every mode and never implicitly declared.
    let builtin = "int x; static char b2[__builtin_constant_p(1) ? 4 : 8]; int c2[sizeof(b2) == 4 \
                   ? 1 : -1]; int c4[__builtin_constant_p(x) ? -1 : 1]; void *f(void *p) { return \
                   __builtin_memcpy(p, p, 1); }";
    assert_eq!(kinds(builtin, gnu17()), []);
    assert_eq!(kinds(builtin, CompilerConfiguration::default()), []);
}

#[test]
fn members_cannot_have_variably_modified_types() {
    assert_eq!(
        kinds(
            "void f(int n) { struct { int (*p)[n]; int ok; } s; union { int a[n]; } u; }",
            gnu17()
        ),
        [
            SemanticErrorKind::FileScopeVariableType,
            SemanticErrorKind::FileScopeVariableType,
        ]
    );
}

#[test]
fn objects_larger_than_ptrdiff_max_are_rejected() {
    assert_eq!(
        kinds(
            "int big[sizeof(int) - 5]; char huge[0x7fffffffffffffffL][4]; struct H { char \
             a[0x4000000000000000L]; char b[0x4000000000000000L]; char c[0x4000000000000000L]; \
             char e; }; char fits[0x7fffffffffffffffL]; struct M { int m; char tail[sizeof(int) - \
             5]; };",
            gnu17()
        ),
        [
            SemanticErrorKind::ObjectTooLarge,
            SemanticErrorKind::ObjectTooLarge,
            SemanticErrorKind::ObjectTooLarge,
            SemanticErrorKind::ObjectTooLarge,
        ]
    );
}

#[test]
fn star_bounds_need_a_prototype_that_is_not_a_definition() {
    assert_eq!(
        kinds(
            "void ok(int a[*], int (*b)[*]); void def(int a[*]) {} void nested(int (*fp)(int \
             [*])) {} void inner(int (*p)[*]) {} int (*q)[*]; void h(void) { int (*r)[*]; }",
            gnu17()
        ),
        [
            SemanticErrorKind::DefinitionStarArray,
            SemanticErrorKind::DefinitionStarArray,
            SemanticErrorKind::InvalidStarBound,
            SemanticErrorKind::InvalidStarBound,
        ]
    );
}

#[test]
fn storage_constraints_have_distinct_kinds() {
    assert_eq!(
        kinds(
            "void f(void) { extern int a = 3; static int g(void); auto int h(void); typedef int \
             T(void); extern int ok(void); } register int r;",
            gnu17()
        ),
        [
            SemanticErrorKind::LinkedBlockInitializer,
            SemanticErrorKind::InvalidFunctionStorage,
            SemanticErrorKind::InvalidFunctionStorage,
            SemanticErrorKind::InvalidStorage,
        ]
    );
}

#[test]
fn bit_field_constraints_have_distinct_kinds() {
    assert_eq!(
        kinds(
            "int n; struct C { int a:33; float f:2; int x:0; int y:-1; int z:n; unsigned :0; };",
            gnu17()
        ),
        [
            SemanticErrorKind::InvalidBitFieldWidth,
            SemanticErrorKind::InvalidBitFieldType,
            SemanticErrorKind::NamedZeroWidthBitField,
            SemanticErrorKind::InvalidBitFieldWidth,
            SemanticErrorKind::InvalidConstant,
        ]
    );
}

#[test]
fn floating_constants_follow_iec_60559() {
    // Annex F gives infinities, NaNs and signed zeros as constant results.
    let source = "static double pinf = +1.0 / 0.0; static double ninf = -1.0 / 0.0; static double \
                  nan = 0.0 / 0.0; const double dnan = 1.0/0.0 - 1.0/0.0; long double big = \
                  5.9486574767861588254287966331400356538172e4931L + \
                  5.9486574767861588254287966331400356538172e4931L; double cmp[] = { (0.0 / 0.0), \
                  0.0 };";
    for configuration in [gnu17(), CompilerConfiguration::default()] {
        assert_eq!(kinds(source, configuration), []);
    }
    // NaN is unordered; negation keeps the sign of zero.
    assert_eq!(
        kinds(
            "int a[(0.0 / 0.0 != 0.0 / 0.0) ? 1 : -1]; int b[(0.0 / 0.0 == 0.0 / 0.0) ? -1 : 1]; \
             int c[(0.0 / 0.0 < 1.0 || 0.0 / 0.0 >= 1.0) ? -1 : 1]; int d[(1.0 / -0.0 < 0) ? 1 : \
             -1]; int e[(1.0 / 0.0 > 1e308) ? 1 : -1];",
            gnu17()
        ),
        []
    );
}

#[test]
fn pointers_to_unanalyzed_types_suppress_arithmetic_constraints() {
    assert_eq!(
        kinds(
            "typedef float V __attribute__((vector_size(16))); V f(const V *p, V *q, int i) { V a \
             = *p++; q[i] = p[0]; q[2] = (q[0] & (1 << q[1])) != 0; i = q - p; return *(p + 1); }",
            gnu17()
        ),
        []
    );
}

#[test]
fn compound_literals_may_have_pointers_to_variable_arrays() {
    assert_eq!(
        kinds(
            "void f(void) { unsigned n = 10; typedef double T[n]; (double (*)[n])((unsigned char \
             (*)[sizeof (T)]){ 0 }); }",
            gnu17()
        ),
        []
    );
    assert_eq!(
        kinds(
            "struct S { int c, e[]; }; int foo(struct S *m, int r, int c) { int (*a)[][m->c] = \
             (int (*)[][m->c])&m->e; return (*a)[r][c]; }",
            gnu17()
        ),
        []
    );
    assert_eq!(
        kinds("void g(int n) { (void)(int [3][n]){ 0 }; }", gnu17()),
        [SemanticErrorKind::InvalidCompoundLiteral]
    );
}

#[test]
fn braced_strings_initialize_the_first_nested_array() {
    assert_eq!(
        kinds(
            "const char ca[2][3] = { \"12\" }; char va[2][3] = { \"123\" }; void f(void) { char \
             l[2][3] = { \"12\" }; }",
            gnu17()
        ),
        []
    );
    assert_eq!(
        kinds("int a[3] = { \"ab\" };", gnu17()),
        [SemanticErrorKind::InvalidInitializer]
    );
}

#[test]
fn address_differences_within_one_object_are_constant() {
    let source = "struct { int a; char c; } v; static long i = ((char*)&(v.c)-(char*)&v); struct \
                  { char a, b, f[3]; } s; long j = s.f-&s.b; int z = (&\"Foobar\"[1] - \
                  &\"Foobar\"[0]); typedef struct { int x; char name[8]; } T; unsigned long o = \
                  (unsigned long) ((unsigned char *) &((T *) 0)->name - (unsigned char *) 0); int \
                  k[2]; long q = &k[1] - k;";
    for configuration in [gnu17(), CompilerConfiguration::default()] {
        assert_eq!(kinds(source, configuration), []);
    }
    for (source, expected) in [
        ("int k[4]; long q = &k[1] - k;", 1),
        ("int k[4]; long q = k - &k[3];", -3),
        (
            "struct {long a; char c;} v; long q = (char*)&v.c - (char*)&v;",
            8,
        ),
    ] {
        with_configuration(source, gnu17(), |context, s| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{source}: {errors:?}");
            let difference = s
                .expressions
                .iter()
                .find(|info| {
                    matches!(
                        info.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Subtraction,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(difference.integer.unwrap().value, expected, "{source}");
        });
    }
    assert_eq!(
        kinds("int x, y; long d = &x - &y;", gnu17()),
        [SemanticErrorKind::NonConstantInitializer]
    );
}

#[test]
fn gnu_modes_fold_static_const_integer_objects() {
    let source = "const char a = 0x42; const double b = (double) a; double e = a; double j[] = { \
                  (double) a, a, 1 + a }; static const int k = 4; enum { Z = k }; void f(void) { \
                  static const double l = 1 + a; }";
    assert_eq!(kinds(source, gnu17()), []);
    // Strict C99 reads of objects are not constant expressions (§6.6p7-8).
    assert_ne!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        kinds(
            "volatile const int v = 1; int w = v; void g(void) { const int n = 2; static int s = \
             n; }",
            gnu17()
        ),
        [
            SemanticErrorKind::NonConstantInitializer,
            SemanticErrorKind::NonConstantInitializer,
        ]
    );
}
