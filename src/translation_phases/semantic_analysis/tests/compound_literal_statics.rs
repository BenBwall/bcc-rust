//! A compound literal of a static object's own type as its initializer
//! (GNU extension `ConstantCompoundLiteral`): accepted, inside the wrappers
//! GCC's folder removes, when every value the literal stores is a constant
//! its destination accepts, and reported under the extension policy as GCC
//! reports it with `-pedantic`. Under the Clang identity a scalar literal's
//! value also folds into any constant expression, as Clang's evaluator
//! reads it, and nothing is reported.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

const IDENTITIES: [CompilerIdentity; 2] = [CompilerIdentity::Gcc, CompilerIdentity::Clang];

/// `(errors, extension diagnostics)` of `source` in C99 under the GCC
/// identity and `policy`.
fn diagnostics(source: &str, policy: ExtensionPolicy) -> (Vec<SemanticErrorKind>, usize) {
    diagnostics_as(source, CompilerIdentity::Gcc, policy)
}

/// `(errors, extension diagnostics)` of `source` in C99 under `identity`
/// and `policy`.
fn diagnostics_as(
    source: &str,
    identity: CompilerIdentity,
    policy: ExtensionPolicy,
) -> (Vec<SemanticErrorKind>, usize) {
    let mut result = (Vec::new(), 0);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, policy).with_compiler_identity(identity),
        |context, _| {
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(e) => result.0.push(e.kind),
                    | TranslationError::Extension(_) => result.1 += 1,
                    | other => panic!("{source}: unexpected {other:?}"),
                }
            }
        },
    );
    result
}

/// The kinds of `source`'s semantic errors, with their lines and columns.
fn positioned(source: &str) -> Vec<(SemanticErrorKind, (u32, u32))> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    let position = crate::translation_phases::GetPosition::position(
                        &error.source_vectors,
                        context,
                    );
                    result.push((error.kind, (position.line, position.column)));
                }
            }
        },
    );
    result
}

const ACCEPTED: &str = r#"struct s { int a, b; const char *n; };
    struct p { int *p; void (*f)(void); };
    static int x = 4;
    static int table[2];
    void g(void);
    struct s whole = (struct s){1, 2, "x"};
    struct w { struct s inner; int *p; } member = { ((struct s){.b = 3}), &x };
    struct s array[2] = { (struct s){5}, [1] = (struct s){.a = 6, .n = "yz"} };
    struct p file = (struct p){(int[]){1}, g};
    struct w nested = (struct w){(struct s){1, 2}, table};
    void f(void) {
        static struct s kept = (struct s){7, 8, "k"};
        static struct p addresses = (struct p){&x, &g};
        // Clang accepts a nested structure literal here; GCC rejects it.
        static struct w deep = (struct w){(struct s){.n = "d"}, &table[1]};
        (void)kept; (void)addresses; (void)deep;
    }"#;

#[test]
fn constant_compound_literals_initialize_statics_as_an_extension() {
    // GCC: "initializer element is not constant [-Wpedantic]" for each
    // literal initializing a whole static object or subobject, an error
    // with -pedantic-errors. A nested literal is reported only at file
    // scope, where its own initializer is static, as in GCC. Clang reports
    // none, even with -pedantic-errors.
    assert_eq!(diagnostics(ACCEPTED, ExtensionPolicy::Allow), (vec![], 0));
    assert_eq!(diagnostics(ACCEPTED, ExtensionPolicy::Warn), (vec![], 10));
    assert_eq!(diagnostics(ACCEPTED, ExtensionPolicy::Deny), (vec![], 10));
    for policy in [ExtensionPolicy::Warn, ExtensionPolicy::Deny] {
        assert_eq!(
            diagnostics_as(ACCEPTED, CompilerIdentity::Clang, policy),
            (vec![], 0)
        );
    }
}

#[test]
fn compound_literal_statics_require_constant_values_for_each_destination() {
    for (source, expected) in [
        // C99 §6.5.2.5p6: an array literal in a block is automatic, so its
        // address is not an address constant (GCC and Clang reject both).
        (
            "struct S { int *p; }; void f(void) { static struct S s = (struct S){(int[]){1}}; \
             (void)s; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
        (
            "struct S { int *p; }; struct W { struct S s; }; void f(void) { static struct W w = \
             (struct W){(struct S){(int[]){1}}}; (void)w; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
        (
            "struct S { int *p; }; void f(void) { static struct S s = (struct S){&(int){1}}; \
             (void)s; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
        (
            "struct T { int a; }; void f(int n) { static struct T t = (struct T){n}; (void)t; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
        // A structure member copied from an automatic object.
        (
            "struct T { int a; }; struct U { struct T t; }; void f(void) { struct T a = {1}; \
             static struct U u = (struct U){a}; (void)u; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
        // An arithmetic expression without a folded value, as for a
        // static scalar initializer.
        (
            "struct T { int a; }; void f(void) { static struct T t = (struct T){1 / 0}; (void)t; }",
            SemanticErrorKind::ConstantOverflow,
        ),
        (
            "struct T { int a; }; struct T t = (struct T){1 / 0};",
            SemanticErrorKind::ConstantOverflow,
        ),
        // An address only initializes an integer as wide as a pointer.
        (
            "struct T { signed char a : 2; }; static int x; void f(void) { static struct T t = \
             (struct T){(long)&x}; (void)t; }",
            SemanticErrorKind::NonConstantInitializer,
        ),
    ] {
        for identity in IDENTITIES {
            assert_eq!(
                diagnostics_as(source, identity, ExtensionPolicy::Allow).0,
                [expected],
                "{identity:?}: {source}"
            );
        }
    }
}

const SCALARS: &str = r"enum e { A, B };
    static int g;
    static int i = (int){3};
    static double d = ((double){2.5});
    static const int c = (int){4};
    static int k = (const int){5};
    static char ch = (char){300};
    static int braces = (int){{6}};
    static _Bool b = (_Bool){2};
    static int *p = (int *){&g};
    static enum e en = (enum e){B};
    void f(void) { static long w = (long){7}; (void)w; }";

/// GCC accepts a scalar compound literal of the object's own type as the
/// whole initializer of a static scalar, with the same `-Wpedantic`
/// "initializer element is not constant"; Clang accepts it silently.
#[test]
fn scalar_compound_literals_initialize_statics_of_their_type() {
    assert_eq!(diagnostics(SCALARS, ExtensionPolicy::Allow), (vec![], 0));
    assert_eq!(diagnostics(SCALARS, ExtensionPolicy::Warn), (vec![], 10));
    assert_eq!(diagnostics(SCALARS, ExtensionPolicy::Deny), (vec![], 10));
    assert_eq!(
        diagnostics_as(SCALARS, CompilerIdentity::Clang, ExtensionPolicy::Deny),
        (vec![], 0)
    );
    for (source, clang) in [
        // GCC: "initializer element is not constant" for each; Clang reads
        // the literal's value.
        ("static double d = (int){3};", true),
        ("static long l = (int){3};", true),
        ("static int x = (int){3} + 1;", true),
        // Neither reads an automatic value or a volatile literal, nor takes
        // an automatic literal's address as a constant.
        ("void f(int n) { static int x = (int){n}; (void)x; }", false),
        ("static int v = (volatile int){3} + 1;", false),
        (
            "void f(void) { static int *p = (int *){&(int){1}}; (void)p; }",
            false,
        ),
    ] {
        for identity in IDENTITIES {
            let accepted = identity == CompilerIdentity::Clang && clang;
            assert_eq!(
                diagnostics_as(source, identity, ExtensionPolicy::Allow).0,
                if accepted {
                    &[][..]
                } else {
                    &[SemanticErrorKind::NonConstantInitializer]
                },
                "{identity:?}: {source}"
            );
        }
    }
    // An automatic scalar takes the literal's value as before, without a
    // diagnostic.
    assert_eq!(
        diagnostics(
            "void f(void) { int x = (int){3}; (void)x; }",
            ExtensionPolicy::Deny
        ),
        (vec![], 0)
    );
}

/// GCC's folder removes casts through types that keep every value of the
/// object's type, `*&`, `__extension__` and the operand a constant `?:`
/// selects before it checks a static initializer, so a literal inside them
/// still gives the object its bytes; casts and `?:` only outside braces.
/// Clang reads through all of them.
#[test]
fn literals_inside_the_wrappers_gcc_folds_away_initialize_statics() {
    let source = r"struct s { int a; };
    enum e { A, B };
    static int g;
    static double d = (double)((double){0x1p20});
    static int i = (int)(long)(unsigned)(int){-3};
    static float f = (float)(long double)(double)(float){1.5f};
    static _Bool b = (_Bool)(int)(_Bool){2};
    static enum e en = (enum e)(int)(enum e){B};
    static int *p = (int *)(void *)(int *){&g};
    static int *q = (int *)(long)(int *){&g};
    static int c = (const int)(int){3};
    static int deref = *(&((int){3}));
    static int then = 1 ? (int){3} : g;
    static int other = 0.0 ? 4 : (int){3};
    static int nested = 1 ? (0 ? 5 : (int){3}) : 4;
    static int ext = __extension__ (int){3};
    static int elements[2] = { *&(int){1}, __extension__ ((int){2}) };
    static struct s cast = (struct s)(struct s){1};
    static struct s chosen = 1 ? (struct s){1} : (struct s){2};
    static struct s through = *&(struct s){1};
    void h(void) { static double local = (double)((double){0.5}); (void)local; }";
    // One report per literal, and one more for each record cast to its own
    // type, as GCC gives.
    assert_eq!(diagnostics(source, ExtensionPolicy::Warn), (vec![], 20));
    assert_eq!(
        diagnostics_as(source, CompilerIdentity::Clang, ExtensionPolicy::Warn),
        (vec![], 1)
    );
    for (source, clang) in [
        // A cast GCC does not fold away: a narrower or other-class
        // intermediate type, or another type than the literal's.
        ("static int x = (int)(char)(int){300};", true),
        ("static int x = (int)(double)(int){3};", true),
        ("static int x = (int)(_Bool)(int){3};", true),
        ("static long x = (long)(int){3};", true),
        ("static int g; static char *x = (char *)(int *){&g};", false),
        (
            "static __float128 x = (__float128)(long double)(__float128){1};",
            true,
        ),
        // An outermost cast to another type than the object's: GCC never
        // folds the implicit conversion to the object's type away, though
        // the value would survive it. Clang's evaluator reads through it.
        ("static int x = (unsigned)(int){-1};", true),
        ("static float x = (double)(float){1};", true),
        ("static int *x = (void *)(int *){0};", true),
        ("static _Bool x = (int)(_Bool){1};", true),
        ("static _Float16 x = (float)(_Float16){0.1};", true),
        ("static int x = (long)(int){3};", true),
        // A `?:` of another type than the object's, even one that keeps
        // its values, or whose condition is no constant.
        ("static char x = 1 ? (char){3} : 4;", true),
        ("static int x = 1 ? (int){3} : 4L;", true),
        ("int v; static int x = v ? (int){3} : 4;", false),
        // Inside braces GCC keeps casts and `?:`.
        ("static int x[1] = { (int)(int){3} };", true),
        ("static int x = { 1 ? (int){3} : 0 };", true),
        (
            "struct s { int a; }; struct t { struct s s; }; static struct t x = { (struct \
             s)(struct s){1} };",
            true,
        ),
        (
            "struct b { int x : 4; }; static struct b x = { (int)(int){3} };",
            true,
        ),
    ] {
        for identity in IDENTITIES {
            let accepted = identity == CompilerIdentity::Clang && clang;
            assert_eq!(
                diagnostics_as(source, identity, ExtensionPolicy::Allow).0,
                if accepted {
                    &[][..]
                } else {
                    &[SemanticErrorKind::NonConstantInitializer]
                },
                "{identity:?}: {source}"
            );
        }
    }
}

/// GCC's folder also removes the operand `_Generic` and
/// `__builtin_choose_expr` select, inside braces too, and compares floating
/// types by their width in bits, so `(_Float16)(__bf16)` and `(long
/// double)(__float128)` pairs fold away.
#[test]
fn gcc_folds_selections_and_equally_wide_floating_pairs_away() {
    let source = r"struct s { int a; };
    static int generic = _Generic(0, int: (int){3});
    static int braced[1] = { _Generic(0, int: (int){3}) };
    static int chosen = __builtin_choose_expr(1, (int){3}, 0);
    static int cast = (int)(long)_Generic(0, int: (int){3});
    static struct s record = _Generic(0, int: (struct s){1});
    static _Float16 halves = (_Float16)(__bf16)(_Float16){0.1};
    static __bf16 brains = (__bf16)(_Float16)(__bf16){0.1};
    static long double wide = (long double)(__float128)(long double){0.1L};";
    for identity in IDENTITIES {
        assert_eq!(
            diagnostics_as(source, identity, ExtensionPolicy::Allow).0,
            [],
            "{identity:?}"
        );
    }
}

/// A static pointer to an array in a block keeps a variable bound that
/// Clang folds nowhere, unlike a static array's own bound, which it folds
/// with `-Wgnu-folding-constant`.
#[test]
fn block_static_pointers_to_arrays_keep_variable_bounds() {
    for (source, folded) in [
        (
            "int f(void) { static char (*p)[(int){3}]; return sizeof *p; }",
            false,
        ),
        (
            "static char x[4]; int f(void) { static char (*p)[&x[3] - &x[0]]; return sizeof *p; }",
            false,
        ),
        (
            "int f(void) { static char a[(int){3}]; return sizeof a; }",
            true,
        ),
        ("static char (*q)[(int){3}];", true),
        // An extern object cannot have a variably modified type, so Clang
        // folds its pointer's bound.
        (
            "static char x[4]; int f(void) { extern char (*q)[&x[3] - &x[0]]; return sizeof *q; }",
            true,
        ),
        (
            "int f(void) { extern char (*q)[(int){3}]; return sizeof *q; }",
            true,
        ),
    ] {
        assert_eq!(
            diagnostics_as(source, CompilerIdentity::Clang, ExtensionPolicy::Warn).0,
            if folded {
                &[SemanticErrorKind::FoldedArrayBound][..]
            } else {
                &[]
            },
            "{source}"
        );
    }
}

/// GCC's `__builtin_constant_p` reads no compound literal, even
/// optimizing; Clang's reads a scalar one.
#[test]
fn constant_queries_of_compound_literals_follow_each_compiler() {
    // Each query's value, weighted by a power of two. Operands that are
    // never evaluated (a `_Generic` controlling expression, the operand
    // `?:`, `&&`, `||` or `__builtin_choose_expr` skips) read no literal.
    let queries = [
        ("(int){1}", false),
        ("(int){1} ? 2 : 3", false),
        ("sizeof (int){1}", true),
        ("_Generic((int){1}, int: 1, default: 2)", true),
        ("0 && (int){1}", true),
        ("1 ? 2 : (int){3}", true),
        ("1 || (int){1}", true),
        ("__builtin_choose_expr(1, 4, (int){3})", true),
        ("1 ?: (int){1}", true),
        ("(void *)0 ? (int){1} : 2", true),
        // The cast keeps `-Wpointer-bool-conversion` quiet.
        ("(char *)&g ? 2 : (int){1}", true),
    ];
    let sum = queries
        .iter()
        .enumerate()
        .map(|(i, (query, _))| format!("{} * __builtin_constant_p({query})", 1 << i))
        .collect::<Vec<_>>()
        .join(" + ");
    let source = format!("int g; enum {{ E = {sum} }};");
    let gcc: i128 = queries
        .iter()
        .enumerate()
        .filter(|(_, (_, gcc))| *gcc)
        .map(|(i, _)| 1 << i)
        .sum();
    for (identity, expected) in [
        (CompilerIdentity::Gcc, gcc),
        (CompilerIdentity::Clang, 0x7FF),
    ] {
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow)
                .with_compiler_identity(identity),
            |context, s| {
                assert_eq!(context.pending_error_count(), 0, "{identity:?}");
                let value = s
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == "E")
                    .and_then(|binding| binding.value)
                    .map(|value| value.value);
                assert_eq!(value, Some(expected), "{identity:?}");
            },
        );
    }
}

/// GCC removes a pair of conversions only around a literal whose type is
/// exactly the type of the outermost cast or `?:`: a cast takes its
/// type's main variant, which a literal named by a typedef or written with
/// qualifiers does not have, and an enumeration is not its compatible
/// integer type. Without a cast, the literal's type need only be
/// compatible with the object's.
#[test]
fn gcc_folds_conversion_pairs_only_around_a_literal_of_the_casts_own_type() {
    let prelude = "typedef int I; typedef long L; typedef enum { Q0 } Q; enum Neg { NA = -1, NB \
                   }; typedef struct S { int a; } T; static int g;";
    for (source, gcc) in [
        ("static int x = (I)(long)(int){3};", true),
        ("static I x = (int)(int){3};", true),
        ("static int x = (int)(L)(int){3};", true),
        ("static int x = (I){3};", true),
        ("static int x = (const int){3};", true),
        ("static const int x = (int)(long)(int){3};", true),
        ("static int x = (const int)(const long)(int){3};", true),
        ("static enum Neg x = (enum Neg)(enum Neg){NA};", true),
        ("static int x = (enum Neg)(long)(enum Neg){NA};", true),
        ("static int *const x = (int *)(int *){&g};", true),
        ("static int x = (int)(1 ? (int)(long)(int){3} : 0);", true),
        ("static I x = (int)(long)(I){3};", false),
        ("static int x = (int)(I){3};", false),
        ("static I x = (I)(I){3};", false),
        ("static int x = (int)(const int){3};", false),
        ("static int x = (int)(long)(const int){3};", false),
        (
            "static const int x = (const int)(long)(const int){3};",
            false,
        ),
        ("static double x = (double)(const double){1};", false),
        ("static int *x = (int *)(int *const){&g};", false),
        ("static int x = (int)(_Atomic int){3};", false),
        ("static int x = (int)(enum Neg){NA};", false),
        ("static int x = (int)(long)(enum Neg){NA};", false),
        ("static enum Neg x = (int)(enum Neg){NA};", false),
        ("static Q x = (Q)(Q){Q0};", false),
        ("static struct S s = (struct S)(T){1};", false),
        ("static int x = 1 ? (I){3} : 0;", false),
        ("static int x = 1 ? (const int){3} : 0;", false),
        ("static int x = 1 ? (enum Neg){NA} : NB;", false),
        ("static int x = (int)_Generic(0, int: (I){3});", false),
    ] {
        let source = format!("{prelude} {source}");
        assert_eq!(
            diagnostics_as(&source, CompilerIdentity::Gcc, ExtensionPolicy::Allow).0,
            if gcc {
                &[][..]
            } else {
                &[SemanticErrorKind::NonConstantInitializer]
            },
            "{source}"
        );
    }
    // Clang's evaluator reads through each of them but the atomic one.
    for source in [
        "static int x = (int)(I){3};",
        "static int x = (int)(enum Neg){NA};",
        "static Q x = (Q)(Q){Q0};",
        "static int x = 1 ? (const int){3} : 0;",
    ] {
        let source = format!("{prelude} {source}");
        assert_eq!(
            diagnostics_as(&source, CompilerIdentity::Clang, ExtensionPolicy::Allow).0,
            [],
            "{source}"
        );
    }
    assert_eq!(
        diagnostics_as(
            &format!("{prelude} static int x = (int)(_Atomic int){{3}};"),
            CompilerIdentity::Clang,
            ExtensionPolicy::Allow
        )
        .0,
        [SemanticErrorKind::NonConstantInitializer]
    );
}

/// Inside braces GCC takes a scalar literal of any arithmetic type and
/// converts the value its own initializer gives it to the element's type;
/// as the whole initializer, a literal of another type stays non-constant.
#[test]
fn gcc_converts_arithmetic_literals_of_other_types_inside_braces() {
    let source = r"struct b { int x : 4; };
    static double a[2] = { (int){1}, (char){300} };
    static int b[1] = { (double){2.5} };
    static _Float16 h[1] = { (__bf16){0.1} };
    static __bf16 k[1] = { *&(_Float16){0.1} };
    static struct b bits = { (double){2.5} };
    void f(void) { static long l[1] = { (_Bool){2} }; (void)l; }";
    for identity in IDENTITIES {
        assert_eq!(
            diagnostics_as(source, identity, ExtensionPolicy::Allow).0,
            [],
            "{identity:?}"
        );
    }
    // One report per literal, and two for `_Float16` before C23.
    assert_eq!(diagnostics(source, ExtensionPolicy::Warn), (vec![], 9));
    for source in [
        "static double d = (int){3};",
        "void f(int v) { static double a[1] = { (int){v} }; (void)a; }",
        "static int a[1] = { (int)(double){2.5} };",
    ] {
        assert_eq!(
            diagnostics(source, ExtensionPolicy::Allow).0,
            [SemanticErrorKind::NonConstantInitializer],
            "{source}"
        );
    }
}

/// Clang's evaluator reads a non-volatile scalar compound literal whose
/// initializer is constant inside any constant expression, at file and
/// block scope and in every mode; an integer constant context folds it as
/// `-Wgnu-folding-constant` reports. GCC folds none of these.
#[test]
fn clang_folds_scalar_compound_literal_values() {
    let source = r"static int sum = (int){3} + 1;
    static double product = 2 * (double){1.25};
    static int truncated = (int)(double){37.9};
    static int negated = -(int){39};
    static int conditional = (int){1} ? 41 : 0;
    static int braced = (int){{5}} * 2;
    static _Complex double z = (_Complex double){1} * 2;
    void f(void) { static int local = (int){54} + 1; (void)local; }";
    assert_eq!(
        diagnostics_as(source, CompilerIdentity::Clang, ExtensionPolicy::Deny),
        (vec![], 0)
    );
    assert_eq!(
        diagnostics(source, ExtensionPolicy::Allow).0,
        [SemanticErrorKind::NonConstantInitializer; 8]
    );
    // C23 (N3220) §6.7.11p11: an empty initializer gives zero.
    for (identity, expected) in [
        (CompilerIdentity::Clang, &[][..]),
        (
            CompilerIdentity::Gcc,
            &[SemanticErrorKind::NonConstantInitializer],
        ),
    ] {
        assert_eq!(
            diagnostics_as(
                "static int empty = (int){} + 1;",
                identity,
                ExtensionPolicy::Allow
            )
            .0,
            expected
        );
    }
    let integer_contexts = "enum { E = (int){3} + 1 }; _Static_assert((int){1}, \"\"); int f(int \
                            x) { switch (x) { case (int){2}: return 1; } return 0; }";
    assert_eq!(
        diagnostics_as(
            integer_contexts,
            CompilerIdentity::Clang,
            ExtensionPolicy::Allow
        ),
        (vec![], 0)
    );
    assert_eq!(
        diagnostics_as(
            integer_contexts,
            CompilerIdentity::Clang,
            ExtensionPolicy::Warn
        ),
        // `_Static_assert` is a C11 extension in C99.
        (vec![SemanticErrorKind::FoldedIntegerConstant; 3], 1)
    );
    assert_eq!(
        diagnostics(integer_contexts, ExtensionPolicy::Allow)
            .0
            .len(),
        3
    );
    with_configuration(
        "static int sum = (int){3} + 1; static double half = (double){1} / 2;",
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow),
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let folded: Vec<_> = s
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Binary { .. }))
                .map(|info| {
                    (
                        info.integer.map(|v| v.value),
                        info.floating().map(|v| v.real),
                    )
                })
                .collect();
            assert_eq!(folded.len(), 2);
            assert_eq!(folded[0].0, Some(4));
            assert!(folded[1].1.is_some());
        },
    );
}

#[test]
fn non_constant_compound_literal_values_are_reported_where_they_are() {
    // Clang: "initializer element is not a compile-time constant" at the
    // array literal.
    assert_eq!(
        positioned(
            "struct S { int *p; };\nvoid f(void) { static struct S s = (struct S){(int[]){1}}; \
             (void)s; }"
        ),
        [(SemanticErrorKind::NonConstantInitializer, (2, 47))]
    );
}
