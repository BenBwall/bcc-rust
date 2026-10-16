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
