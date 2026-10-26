//! A compound literal of a static structure or union's own type as its
//! initializer (GNU extension): accepted only when every value the literal
//! stores is a constant its destination accepts, as GCC and Clang require.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

/// `(errors, extension diagnostics)` of `source` in C99 under `policy`.
fn diagnostics(source: &str, policy: ExtensionPolicy) -> (Vec<SemanticErrorKind>, usize) {
    let mut result = (Vec::new(), 0);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, policy),
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
fn constant_compound_literals_initialize_statics() {
    assert_eq!(diagnostics(ACCEPTED, ExtensionPolicy::Allow), (vec![], 0));
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
        assert_eq!(
            diagnostics(source, ExtensionPolicy::Allow).0,
            [expected],
            "{source}"
        );
    }
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
