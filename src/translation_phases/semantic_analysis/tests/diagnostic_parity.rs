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
