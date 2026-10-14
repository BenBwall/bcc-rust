//! Pointer constraints retain standard conversion facts under warning policy.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        DiagnosticGroup,
        ExtensionPolicy,
        WarningOption,
    },
    translation_phases::{
        ErrorSeverity,
        GetSeverity,
    },
};

#[test]
fn pointer_defaults_match_clang_across_modes_and_targets() {
    use DiagnosticGroup as G;
    for (source, group) in [
        (
            "int f(int *p,unsigned *q){return p==q;}",
            G::CompareDistinctPointerTypes,
        ),
        (
            "int f(int *p,float *q){return p<q;}",
            G::CompareDistinctPointerTypes,
        ),
        (
            "void f(char *p,float *q){p=q;}",
            G::IncompatiblePointerTypes,
        ),
        ("void f(float *q){char *p=q;}", G::IncompatiblePointerTypes),
        (
            "void g(char *);void f(float *q){g(q);}",
            G::IncompatiblePointerTypes,
        ),
        ("char *f(float *q){return q;}", G::IncompatiblePointerTypes),
        ("void f(int *p,unsigned *q){p=q;}", G::PointerSign),
        ("void f(char *p,signed char *q){p=q;}", G::PointerSign),
        (
            "void f(int **p,unsigned **q){p=q;}",
            G::IncompatiblePointerTypes,
        ),
        ("void f(int *p,const int *q){p=q;}", G::PointerQualifiers),
        ("void f(const int **p,int **q){p=q;}", G::PointerQualifiers),
        ("void f(int **p,const int **q){p=q;}", G::PointerQualifiers),
        ("void f(int **p,int *const *q){p=q;}", G::PointerQualifiers),
        (
            "void f(int (*p)(int),int (*q)(double)){p=q;}",
            G::IncompatibleFunctionPointerTypes,
        ),
        (
            "void f(int (*p)(int),int *q){p=q;}",
            G::IncompatiblePointerTypes,
        ),
        ("void f(int *p,int q){p=q;}", G::IntConversion),
        ("void f(int p,int *q){p=q;}", G::IntConversion),
        (
            "int f(int *p,int q){return p==q;}",
            G::PointerIntegerCompare,
        ),
        (
            "int f(int *p,int q){return p<q;}",
            G::OrderedPointerIntegerCompare,
        ),
        (
            "int f(int (*p)(int),int (*q)(int)){return p<q;}",
            G::OrderedCompareFunctionPointers,
        ),
        (
            "int f(int (*p)(int),int (*q)(double)){return p==q;}",
            G::CompareDistinctPointerTypes,
        ),
        (
            "int f(int *p,void *q){return p<q;}",
            G::CompareDistinctPointerTypes,
        ),
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
                for target in [
                    crate::target::Target::LinuxGnu,
                    crate::target::Target::WindowsGnu,
                    crate::target::Target::WindowsMsvc,
                ] {
                    let configuration =
                        CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                            .with_gnu_extensions(gnu)
                            .with_target(target);
                    with_configuration(source, configuration, |context, unit| {
                        let errors = context.take_pending_errors();
                        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                        assert!(
                            matches!(errors[0], TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(group)),
                            "{source}: {errors:?}"
                        );
                        assert_eq!(errors[0].severity(), group.default_severity());
                        assert_eq!(
                            unit.errors,
                            usize::from(group.default_severity() == ErrorSeverity::Error)
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn distinct_function_pointer_order_reports_both_groups_independently() {
    let source = "int f(int (*p)(int),int (*q)(double)){return p<q;}";
    with_source(source, |context, unit| {
        assert_eq!(unit.errors, 0);
        let errors = context.take_pending_errors();
        assert_eq!(errors.len(), 2, "{errors:?}");
        for (error, group) in errors.iter().zip([
            DiagnosticGroup::OrderedCompareFunctionPointers,
            DiagnosticGroup::CompareDistinctPointerTypes,
        ]) {
            assert!(
                matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(group))
            );
        }
    });
    let configuration = CompilerConfiguration::default()
        .with_warning_option(
            DiagnosticGroup::CompareDistinctPointerTypes,
            WarningOption::Enable(false),
        )
        .with_warning_option(
            DiagnosticGroup::OrderedCompareFunctionPointers,
            WarningOption::Error(true),
        );
    with_configuration(source, configuration, |context, unit| {
        assert_eq!(unit.errors, 1);
        assert!(!unit.lowerable(context));
        let errors = context.take_pending_errors();
        assert_eq!(errors.len(), 1);
        assert!(
            matches!(errors[0], TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(DiagnosticGroup::OrderedCompareFunctionPointers))
        );
    });
}

#[test]
fn warning_conversions_are_lowerable_and_retained_in_every_assignment_context() {
    for source in [
        "void f(const int **p,int **q){p=q;}",
        "void f(int **q){const int **p=q;}",
        "void g(const int **);void f(int **q){g(q);}",
        "const int **f(int **q){return q;}",
        "int *p; unsigned *q=(int *)0;",
    ] {
        with_source(source, |context, unit| {
            assert_eq!(unit.errors, 0);
            assert!(unit.lowerable(context), "{source}");
            assert!(
                unit.conversions
                    .iter()
                    .any(|c| c.kind == ConversionKind::Assignment)
            );
            assert!(
                context
                    .take_pending_errors()
                    .iter()
                    .all(|e| e.severity() == ErrorSeverity::Warning)
            );
        });
    }
}

#[test]
fn qualifier_warning_does_not_weaken_intrinsic_contracts_or_unrelated_types() {
    for source in [
        "void f(int **p,const float **q){p=q;}",
        "void f(int (*p)[2],const int (*q)[3]){p=q;}",
        "struct A;struct B;void f(const struct A **p,struct B **q){p=q;}",
    ] {
        with_source(source, |context, unit| {
            assert_eq!(unit.errors, 1);
            assert!(!unit.lowerable(context));
            assert!(
                matches!(context.take_pending_errors()[0],TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(DiagnosticGroup::IncompatiblePointerTypes))
            );
        });
    }
}

#[test]
fn suppressed_or_demoted_constraints_still_retain_conversion_facts() {
    for option in [WarningOption::Enable(false), WarningOption::Error(false)] {
        let configuration = CompilerConfiguration::default()
            .with_warnings_as_errors(true)
            .with_warning_option(DiagnosticGroup::IncompatiblePointerTypes, option);
        with_configuration(
            "void g(char *);void f(float *p){g(p);}",
            configuration,
            |context, unit| {
                assert!(unit.lowerable(context));
                assert_eq!(unit.errors, 0);
                assert!(
                    unit.conversions
                        .iter()
                        .any(|c| c.kind == ConversionKind::Assignment)
                );
                assert!(
                    context
                        .take_pending_errors()
                        .iter()
                        .all(|e| e.severity() == ErrorSeverity::Warning)
                );
            },
        );
    }
}

#[test]
fn conversion_errors_suppress_dependent_initializer_errors() {
    with_source("int *p=1; int following_valid;", |context, unit| {
        assert_eq!(unit.errors, 1);
        let errors = context.take_pending_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            matches!(errors[0],TranslationError::Semantic(error) if error.kind == SemanticErrorKind::PointerConstraint(DiagnosticGroup::IntConversion))
        );
    });
    let configuration = CompilerConfiguration::default()
        .with_warning_option(DiagnosticGroup::IntConversion, WarningOption::Error(false));
    with_configuration("int *p=1;", configuration, |context, unit| {
        assert_eq!(unit.errors, 0);
        assert!(unit.lowerable(context));
    });
}
