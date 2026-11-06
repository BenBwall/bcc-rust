//! Issue #54: semantic constraints and optional Clang warning groups.
use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    DiagnosticGroup as G,
    ExtensionPolicy,
    WarningOption,
};

#[test]
fn nullptr_has_a_distinct_type_and_rejects_invalid_conversions() {
    let configuration = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow);
    for (source, expected) in [
        (
            "void f(void){int i; i=nullptr;}",
            SemanticErrorKind::InvalidAssignment,
        ),
        (
            "void f(void){int i; i==nullptr;}",
            SemanticErrorKind::InvalidComparisonOperands,
        ),
        (
            "void f(int *p){p<nullptr;}",
            SemanticErrorKind::InvalidComparisonOperands,
        ),
        (
            "void f(int *p){typeof(nullptr) n=p;}",
            SemanticErrorKind::InvalidInitializer,
        ),
        (
            "void f(void){int i=(int)nullptr;}",
            SemanticErrorKind::InvalidCast,
        ),
    ] {
        with_configuration(source, configuration, |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(
                matches!(errors[0], TranslationError::Semantic(error) if error.kind == expected),
                "{source}: {errors:?}"
            );
            assert_eq!(unit.errors, 1);
        });
    }
    with_configuration(
        "typeof(nullptr) n=nullptr, a=0, c=(void*)0; void f(void){int *p=nullptr; typeof(nullptr) \
         m=n; bool b=nullptr; p==nullptr; nullptr==nullptr; nullptr==0;}",
        configuration,
        |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(
                matches!(errors[0], TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(G::NullConversion))
            );
            let n = unit
                .bindings
                .iter()
                .find(|binding| context.string_cache.at(binding.name.name) == "n")
                .unwrap();
            assert_eq!(unit.types.kind(n.ty), TypeKind::Nullptr);
            assert_eq!(unit.types.layout(n.ty), Some(unit.types.target.pointer));
            assert_eq!(unit.errors, 0);
        },
    );
}

#[test]
fn warning_groups_cover_null_constants_casts_void_and_old_style_calls() {
    for (source, group) in [
        ("void f(int *p){p=(short)0;}", G::NonLiteralNullConversion),
        (
            "enum {ZERO=0}; void f(int *p){p=ZERO;}",
            G::NonLiteralNullConversion,
        ),
        ("void f(int *p){p=='\\0';}", G::PointerCompare),
        ("void f(void){int i; _Bool b=&i;}", G::PointerBoolConversion),
        ("void f(int *p){(int)p;}", G::PointerToIntCast),
        ("void f(void *p){*p;}", G::VoidPtrDereference),
        (
            "void f(int *) __attribute__((nonnull(1))); void g(void){f(0);}",
            G::Nonnull,
        ),
        (
            "int kr(p) int *p; {return *p;} void f(unsigned u){kr(&u);}",
            G::PointerSign,
        ),
    ] {
        let configuration = CompilerConfiguration::default()
            .with_warning_option(group, WarningOption::Enable(true));
        with_configuration(source, configuration, |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(
                matches!(errors[0], TranslationError::Semantic(error) if error.kind==SemanticErrorKind::PointerConstraint(group)),
                "{source}: {errors:?}"
            );
            assert_eq!(unit.errors, 0);
        });
        with_configuration(
            source,
            configuration.with_warning_option(group, WarningOption::Enable(false)),
            |context, _| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{source}: {:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
}

#[test]
fn nonnull_operands_and_redeclarations_retain_parameter_positions() {
    with_source(
        "enum {FIRST=1}; void f(int *, int, int *) __attribute__((nonnull(FIRST,3))); void f(int \
         *, int, int *); void g(void){f(0,0,0);} void h(int *,int) __attribute__((nonnull)); void \
         k(void){h(0,0);} void empty(int *) __attribute__((nonnull())); void m(void){empty(0);}",
        |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 4, "{errors:?}");
            assert!(errors.iter().all(|error|matches!(error,TranslationError::Semantic(error) if error.kind==SemanticErrorKind::PointerConstraint(G::Nonnull))),"{errors:?}");
            assert_eq!(unit.errors, 0);
        },
    );
}

#[test]
fn void_and_function_subtraction_are_gnu_extensions() {
    with_source(
        "void f(void); long g(void *p){return (p-p)+(f-f);}",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            assert_eq!(unit.errors, 0);
        },
    );
}

#[test]
fn address_of_cancels_void_indirection_without_a_warning() {
    with_source("void f(void *p){&*p; &(*p);}", |context, unit| {
        assert_eq!(
            context.pending_error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        assert_eq!(unit.errors, 0);
    });
}

fn pointer_bool_warnings(
    source: &str,
    identity: crate::configuration::CompilerIdentity,
    all: bool,
) -> usize {
    let mut count = 0;
    with_exact_configuration(
        source,
        CompilerConfiguration::default()
            .with_compiler_identity(identity)
            .with_gnu_extensions(true)
            .with_all_warnings(all),
        |context, _| {
            count = context
                .take_pending_errors()
                .iter()
                .filter(|error| {
                    matches!(
                        error,
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::PointerConstraint(G::PointerBoolConversion),
                            ..
                        })
                    )
                })
                .count();
        },
    );
    count
}

/// The address warning against GCC 14.2 and Clang 23.1.1: `assert(0 && "text")`
/// is the idiom, and both are silent about any string literal.
#[test]
fn string_literals_are_not_known_addresses() {
    use crate::configuration::CompilerIdentity as I;
    for source in [
        "void f(void) { if (\"s\") {} }",
        "void f(void) { if (!\"s\") {} }",
        "int f(int x) { return x && \"s\"; }",
        "int f(int x) { return \"s\" && x; }",
        "int f(void) { return 0 && \"s\"; }",
        "int f(int x) { return x || \"s\"; }",
        "int f(void) { return \"s\" ? 1 : 2; }",
        "void f(void) { _Bool b = \"s\"; (void)b; }",
        "void f(void) { while (\"s\") { break; } }",
        "void f(void) { if (L\"s\") {} }",
        "void a(void) __attribute__((noreturn)); void f(void) { (0 && \"m\") ? (void)0 : a(); }",
        // Nor about what else names no entity, or one that may be null.
        "void f(void) { if ((const char[]){\"s\"}) {} }",
        "void f(void) { if (&(int){1}) {} }",
        "void f(char (*a)[3]) { if (*a) {} }",
        "void g(void) __attribute__((weak)); void f(void) { if (g) {} }",
        "extern int w __attribute__((weak)); void f(void) { if (&w) {} }",
        "void f(int a[3]) { if (a) {} }",
    ] {
        assert_eq!(pointer_bool_warnings(source, I::Clang, true), 0, "{source}");
        assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 0, "{source}");
    }
    for source in [
        "char a[3]; void f(void) { if (a) {} }",
        "char a[3]; int f(int x) { return x && a; }",
        "char a[3]; int f(void) { return 0 && a; }",
        "struct S { char n[3]; }; void f(struct S *p) { if (p->n) {} }",
        "int v; void f(void) { if (&v) {} }",
        "void g(void); void f(void) { if (g) {} }",
    ] {
        assert_eq!(pointer_bool_warnings(source, I::Clang, true), 1, "{source}");
        assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 1, "{source}");
        // GCC's `-Waddress` is `-Wall`'s, and Clang's is on by default.
        assert_eq!(
            pointer_bool_warnings(source, I::Clang, false),
            1,
            "{source}"
        );
        assert_eq!(pointer_bool_warnings(source, I::Gcc, false), 0, "{source}");
    }
    // Clang wants one name, and the address of a function is how to silence it.
    for source in [
        "void g(void); void f(void) { if (&g) {} }",
        "char a[3][3]; void f(void) { if (a[0]) {} }",
        "void f(void) { if (__func__) {} }",
    ] {
        assert_eq!(pointer_bool_warnings(source, I::Clang, true), 0, "{source}");
        assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 1, "{source}");
    }
}

/// GCC's `-Waddress` on a truth value against GCC 14.2 and Clang 23.1.1, from
/// `maybe_warn_for_null_address`: a member or element of an object whose base
/// is a constant pointer folds to a constant (`fold_offsetof`), so `offsetof`'s
/// idiom is silent however it is spelled, and any other address is known.
#[test]
fn gcc_address_warning_folds_a_constant_base() {
    use crate::configuration::CompilerIdentity as I;
    const S: &str = "struct I { int y; int b[2]; }; struct S { int x; int a[2]; struct I i; \
                     struct I j[2]; int *q; }; struct S s; struct S *p; int g; int ga[2]; int \
                     gm[2][2]; void fn(void); int *r; int (*pia)[2]; extern struct S ws \
                     __attribute__((weak)); _Complex int z; ";
    let truth = |e: &str| format!("{S}int f(void) {{ return !!({e}); }}");
    for e in [
        // The finding, and the arrow it should have matched.
        "&(*(struct S *)0).x",
        "&((struct S *)0)->x",
        // A nested member, an element, an array member that decays.
        "&(*(struct S *)0).i.y",
        "&((struct S *)0)->i.y",
        "&((struct S *)0)->j[1].y",
        "&(*(struct S *)0).a[1]",
        "((struct S *)0)->a",
        "(*(struct S *)0).a",
        "((struct S *)0)->j[1].b",
        "&((struct S *)0)[1].x",
        "((struct S *)0)[0].a",
        // Any constant, not only null.
        "&((struct S *)100)->x",
        "&(*(struct S *)100).x",
        "&((struct S *)(0 + 0))->x",
        "&(*(struct S (*)[2])0)[1].x",
        "&(*(int (*)[2])0)[1]",
        "*(int (*)[2])0",
        "&((int *)0)[1]",
        "&*(int *)0",
        // A pointer's value.
        "p",
        "&*p",
        "*&p",
        "&p[0]",
        "p + 0",
        "(void *)r",
        "(*p).q",
        "&p->q[0]",
        "pia[0]",
        "*pia",
        // A `void *` sum, and an integer.
        "(void *)(ga + 1)",
        "(void *)r + 1",
        "(long)&g",
        "(long)ga",
        // No decl: a comma's value, a statement expression.
        "(0, ga)",
        "({ ga; })",
    ] {
        assert_eq!(pointer_bool_warnings(&truth(e), I::Gcc, true), 0, "{e}");
    }
    for e in [
        // A base that is an object, or a pointer's value.
        "&s.x",
        "&s.i.y",
        "s.a",
        "&(*p).x",
        "&p->x",
        "p->a",
        "&p->i.y",
        "&p[1].x",
        "&((struct S *)&s)->x",
        "&(*(struct S *)&s).x",
        "(struct S){0}.a",
        // Weakness asks about a declaration's own address.
        "&ws.x",
        "ws.a",
        // Elements and sums.
        "&ga[0]",
        "&ga[1]",
        "&ga[g]",
        "gm[0]",
        "*gm",
        "&gm[0][0]",
        "&(*pia)[0]",
        "ga + 0",
        "ga + 1",
        "&g + 1",
        "p + 1",
        "1 + p",
        "p - 1",
        "&p[1]",
        "&p[g]",
        "(char *)r + 1",
        "(void *)(r + 1)",
        // Casts, and what reaches a declaration.
        "(void *)&g",
        "(int *)ga",
        "(void *)fn",
        "(void *)&ga[1]",
        "*fn",
        "**fn",
        "&*fn",
        "&*ga",
        "*&ga",
        "&__real__ z",
        "&__imag__ z",
        "__func__",
        "&__func__",
    ] {
        assert_eq!(pointer_bool_warnings(&truth(e), I::Gcc, true), 1, "{e}");
    }
    // Each arm of a conditional.
    assert_eq!(
        pointer_bool_warnings(&truth("g ? ga : ga"), I::Gcc, true),
        2
    );
    assert_eq!(pointer_bool_warnings(&truth("g ? ga : r"), I::Gcc, true), 1);
    // A weak reference's address can be null, and either compiler knows.
    for source in [
        "static void h(void) __attribute__((weakref(\"q\"))); int f(void) { return !!h; }",
        "static void h(void) __attribute__((weakref(\"q\"))); int f(void) { return !!&h; }",
        "static int z __attribute__((weakref(\"q\"))); int f(void) { return !!&z; }",
    ] {
        assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 0, "{source}");
        assert_eq!(pointer_bool_warnings(source, I::Clang, true), 0, "{source}");
    }
    // A label's address, and a cast's truth value.
    for source in [
        "int f(void) { L: return !!&&L; }",
        "int g; int f(void) { return (_Bool)&g; }",
    ] {
        assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 1, "{source}");
        assert_eq!(pointer_bool_warnings(source, I::Clang, true), 0, "{source}");
    }
}

/// A macro: GCC's declaration path does not look, and the rest of its warning
/// asks about the conversion's location. Clang stays out of a macro's body and
/// away from an argument the body converts.
#[test]
fn address_warnings_look_at_macros() {
    use crate::configuration::CompilerIdentity as I;
    const P: &str = "struct S { int x; int a[2]; } s, *p; int g; int ga[2]; void \
                     fn(void);\n#define M(x) (!!(x))\n#define I(x) x\n#define IF(x) if (x) return \
                     1\n#define D (&s.x)\n#define G (&g)\n";
    for (body, in_gcc, in_clang) in [
        // Beside `!!` in a macro's body.
        ("return M(&s.x);", 0, 0),
        ("return M(p->a);", 0, 0),
        ("return M(ga + 1);", 0, 0),
        ("return M(&g);", 1, 0),
        ("return M(ga);", 1, 0),
        ("return M(fn);", 1, 0),
        // The condition's first token, against the keyword.
        ("if (D) return 1; return 0;", 0, 0),
        ("if (G) return 1; return 0;", 1, 0),
        ("if (!D) return 1; return 0;", 1, 0),
        ("if (!G) return 1; return 0;", 1, 0),
        ("IF(&s.x); return 0;", 0, 0),
        ("IF(&g); return 0;", 1, 0),
        // A conversion to `_Bool`, whose location is not the expression's.
        ("_Bool b = D; return b;", 1, 0),
        ("_Bool b = I(&s.x); return b;", 1, 1),
        ("_Bool b; b = D; return b;", 1, 0),
        ("return (_Bool)D;", 1, 0),
        ("return (_Bool)I(&s.x);", 1, 0),
        // The operators': `&&`'s left operand at its first token, its right
        // operand at the operator, and `?` at itself.
        ("return D && g;", 0, 0),
        ("return g && D;", 1, 0),
        ("return I(&s.x) && g;", 0, 1),
        ("return G && g;", 1, 0),
        ("return D ? 1 : 0;", 1, 0),
        ("return I(&s.x) ? 1 : 0;", 1, 1),
        // An argument the user wrote and nothing in a body converts.
        ("if (I(&s.x)) return 1; return 0;", 0, 1),
        ("if (I(&g)) return 1; return 0;", 1, 1),
    ] {
        let source = format!("{P}int f(void) {{ {body} }}");
        assert_eq!(
            pointer_bool_warnings(&source, I::Gcc, true),
            in_gcc,
            "GCC: {body}"
        );
        assert_eq!(
            pointer_bool_warnings(&source, I::Clang, true),
            in_clang,
            "Clang: {body}"
        );
    }
}

/// Clang's warning names what is written after `&`, not what is in parentheses
/// there, and leaves a GNU `?:`'s condition alone.
#[test]
fn clang_address_warning_reads_the_operand_as_written() {
    use crate::configuration::CompilerIdentity as I;
    const P: &str = "struct S { int x; } s, *p; int g; ";
    for (e, in_clang) in [
        ("&g", 1),
        ("&(g)", 0),
        ("&s.x", 1),
        ("&(s.x)", 0),
        ("&(s).x", 1),
        ("&p->x", 1),
        ("&(p->x)", 0),
    ] {
        let source = format!("{P}int f(void) {{ return !!{e}; }}");
        assert_eq!(
            pointer_bool_warnings(&source, I::Clang, true),
            in_clang,
            "{e}"
        );
        assert_eq!(pointer_bool_warnings(&source, I::Gcc, true), 1, "{e}");
    }
    let source = "int g; int *f(void) { return &g ?: 0; }";
    assert_eq!(pointer_bool_warnings(source, I::Clang, true), 0);
    assert_eq!(pointer_bool_warnings(source, I::Gcc, true), 1);
}
