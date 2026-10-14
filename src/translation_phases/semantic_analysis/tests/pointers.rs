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

/// Asserts the exact pointer groups `source` reports, in order.
fn assert_groups(source: &str, configuration: CompilerConfiguration, groups: &[DiagnosticGroup]) {
    with_configuration(source, configuration, |context, unit| {
        let errors = context.take_pending_errors();
        let found = errors.iter().map(|error| match error {
            | TranslationError::Semantic(error) => match error.kind {
                | SemanticErrorKind::PointerConstraint(group) => Some(group),
                | _ => None,
            },
            | _ => None,
        });
        assert!(
            found.eq(groups.iter().copied().map(Some)),
            "{source}: {errors:?}"
        );
        let expected_errors = groups
            .iter()
            .filter(|group| group.default_severity() == ErrorSeverity::Error)
            .count();
        assert_eq!(unit.errors, expected_errors, "{source}: {errors:?}");
    });
}

#[test]
#[expect(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    reason = "Test sources are built outside compiler allocation measurements."
)]
fn conditional_mismatches_warn_and_take_clang_result_types() {
    use DiagnosticGroup as G;
    let declarations = "int *ip; unsigned *up; const int *cip; volatile unsigned *vup; void *vp; \
                        const void *cvp; int (*f1)(int); int (*f2)(double); long l; int c; \
                        int (*pa)[3]; const int (*cpa)[3];";
    for (expression, ty, groups) in [
        ("c ? ip : up", "void *", &[G::PointerTypeMismatch][..]),
        ("c ? cip : vup", "void *", &[G::PointerTypeMismatch]),
        ("c ? f1 : f2", "void *", &[G::PointerTypeMismatch]),
        ("c ? cvp : f1", "void *", &[G::PointerTypeMismatch]),
        ("c ? pa : cpa", "void *", &[G::PointerTypeMismatch]),
        ("c ? ip : l", "int *", &[G::ConditionalTypeMismatch]),
        ("c ? 1 : f1", "int (*)(int)", &[G::ConditionalTypeMismatch]),
        ("c ? ip : cip", "const int *", &[]),
        ("c ? vp : cip", "const void *", &[]),
        ("c ? ip : 0", "int *", &[]),
    ] {
        let source = format!(
            "{declarations} _Static_assert(_Generic({expression}, {ty}: 1, default: 0), \
             \"type\"); void use(void) {{ (void)({expression}); }}"
        );
        // Once in the static assertion and once in the function.
        let groups: Vec<_> = groups.iter().chain(groups).copied().collect();
        assert_groups(&source, CompilerConfiguration::default(), &groups);
    }
    // The integer arm of a constant conditional is an address constant.
    assert_groups(
        "int i; int *p = 1 ? &i : 5; int *q = 0 ? &i : 5;",
        CompilerConfiguration::default(),
        &[G::ConditionalTypeMismatch, G::ConditionalTypeMismatch],
    );
    // A real floating arm stays an invalid operand pair.
    with_source(
        "int *ip; double d; int c; void f(void) { (void)(c ? ip : d); }",
        |_, unit| assert_eq!(unit.errors, 1),
    );
}

#[test]
fn conditional_mismatch_groups_follow_options_and_pedantic_errors() {
    let source = "int *ip; unsigned *up; int c; void f(void) { (void)(c ? ip : up); (void)(c ? \
                  ip : 1); }";
    let deny = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Deny);
    with_configuration(source, deny, |context, unit| {
        assert_eq!(unit.errors, 2, "{:?}", context.take_pending_errors());
    });
    let quiet = CompilerConfiguration::default()
        .with_warning_option(DiagnosticGroup::PointerTypeMismatch, WarningOption::Enable(false))
        .with_warning_option(
            DiagnosticGroup::ConditionalTypeMismatch,
            WarningOption::Enable(false),
        );
    assert_groups(source, quiet, &[]);
}

#[test]
#[expect(
    clippy::disallowed_macros,
    reason = "Test sources are built outside compiler allocation measurements."
)]
fn array_element_qualifiers_belong_to_pointer_targets() {
    use DiagnosticGroup as G;
    let declarations = "void f(const double m[][3]); void g(int n, const double m[][n]); void \
                        h(const double (*m)[]); double a[2][3]; int (*pa)[3]; const int \
                        (*cpa)[3]; int (**ppa)[3]; const int (**pcpa)[3]; const int \
                        (**pcpinc)[];";
    for (body, groups) in [
        ("f(a); g(3, a); h(a); cpa = pa;", &[][..]),
        ("(void)(cpa == pa); (void)(cpa < pa); (void)(cpa - pa);", &[]),
        ("pa = cpa;", &[G::PointerQualifiers]),
        // Nested levels still need identical array derivations, as in Clang.
        ("pcpa = ppa;", &[G::PointerQualifiers]),
        ("pcpinc = ppa;", &[G::IncompatiblePointerTypes]),
    ] {
        for standard in [CStandard::C89, CStandard::C99, CStandard::C23] {
            let configuration =
                CompilerConfiguration::new(standard, ExtensionPolicy::Allow).with_gnu_extensions(true);
            assert_groups(
                &format!("{declarations} void t(void) {{ {body} }}"),
                configuration,
                groups,
            );
        }
    }
}

#[test]
#[expect(
    clippy::disallowed_macros,
    reason = "Test sources are built outside compiler allocation measurements."
)]
fn enumeration_targets_compare_signs_through_their_compatible_type() {
    use DiagnosticGroup as G;
    use crate::target::Target;
    let declarations =
        "enum E { A, B }; enum N { M = -1 }; enum E *pe; enum N *pn; unsigned *pu; int *pi;";
    for (target, body, groups) in [
        (Target::LinuxGnu, "pu = pn;", &[G::PointerSign][..]),
        (Target::LinuxGnu, "pn = pu;", &[G::PointerSign]),
        (Target::LinuxGnu, "pi = pn;", &[]),
        (Target::LinuxGnu, "pu = pe;", &[]),
        // An unsigned enumeration stays distinct from `int`, as in Clang.
        (Target::LinuxGnu, "pi = pe;", &[G::IncompatiblePointerTypes]),
        (Target::WindowsMsvc, "pu = pe;", &[G::PointerSign]),
        (Target::WindowsMsvc, "pe = pu;", &[G::PointerSign]),
        (Target::WindowsMsvc, "pi = pe;", &[]),
    ] {
        assert_groups(
            &format!("{declarations} void t(void) {{ {body} }}"),
            CompilerConfiguration::default().with_target(target),
            groups,
        );
    }
}

#[test]
#[expect(
    clippy::disallowed_macros,
    reason = "Test sources are built outside compiler allocation measurements."
)]
fn atomic_value_operands_take_assignment_groups() {
    use DiagnosticGroup as G;
    let declarations = "int *ip; unsigned *up; const int *cip; float *fp; int i;";
    for (body, groups) in [
        ("__atomic_store_n(&ip, up, 0);", &[G::PointerSign][..]),
        ("__atomic_store_n(&ip, cip, 0);", &[G::PointerQualifiers]),
        ("__atomic_store_n(&ip, fp, 0);", &[G::IncompatiblePointerTypes]),
        ("(void)__atomic_exchange_n(&ip, up, 0);", &[G::PointerSign]),
        ("(void)__sync_val_compare_and_swap(&ip, up, ip);", &[G::PointerSign]),
        ("__c11_atomic_store((_Atomic(int *) *)&ip, up, 0);", &[G::PointerSign]),
        ("__atomic_store_n(&i, ip, 0);", &[G::IntConversion]),
    ] {
        assert_groups(
            &format!("{declarations} void t(void) {{ {body} }}"),
            CompilerConfiguration::default(),
            groups,
        );
    }
}

