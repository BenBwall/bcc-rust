//! Regressions for operand, initializer, statement and builtin semantics.

use std::fmt::Write;

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        ErrorSeverity,
        GetSeverity,
    },
};

fn steps(source: &str) -> [usize; 3] {
    let tu = Bump::new();
    let scratch = Bump::new();
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::new(&tu);
    let unit = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        source,
        &[],
        &[],
    );
    let mut analyzer = Analyzer::new(&mut context, &scratch);
    for &root in unit.external_declarations().iter().rev() {
        analyzer.work.push(Work::Root(root));
    }
    while let Some(work) = analyzer.work.pop() {
        analyzer.step(work);
    }
    analyzer.finish_translation_unit();
    assert_eq!(
        analyzer.context.pending_error_count(),
        0,
        "{:?}",
        analyzer.context.take_pending_errors()
    );
    analyzer.review_steps.get()
}

#[test]
fn exiting_vm_paths_scale_linearly() {
    for n in [128, 512] {
        let mut source = String::from("void f(int n) { L:; int outer[n]; {\n");
        for i in 0..n {
            writeln!(source, "int a{i}[n];").unwrap();
        }
        for _ in 0..n {
            source.push_str("if(n) goto L;\n");
        }
        source.push_str("} }");
        assert!(steps(&source)[0] <= 4 * n);
        // Also retain a nonempty destination path.
        let source = source.replace("L:; int outer[n];", "int outer[n]; L:;");
        assert!(steps(&source)[0] <= 4 * n);
    }
}

#[test]
fn repeated_offset_member_lookup_scales_linearly() {
    for n in [128, 512] {
        let mut source = String::from("struct S {\n");
        for i in 0..n {
            writeln!(source, "int m{i};").unwrap();
        }
        source.push_str("};\n");
        for i in 0..n {
            writeln!(
                source,
                "int x{i}[__builtin_offsetof(struct S,m{i})=={}?1:-1];",
                4 * i
            )
            .unwrap();
        }
        assert!(steps(&source)[1] <= 2 * n);
    }
}

#[test]
fn nested_array_initializer_classification_scales_linearly() {
    for n in [128, 512] {
        let source = format!(
            "int a{}={}1{};",
            "[1]".repeat(n),
            "{".repeat(n),
            "}".repeat(n)
        );
        assert!(steps(&source)[2] <= 4 * n);
    }
}

fn clean(source: &str) {
    with_source(source, |context, _| {
        assert_eq!(
            context.pending_error_count(),
            0,
            "{source}: {:?}",
            context.take_pending_errors()
        );
    });
}

fn rejects(source: &str, kind: SemanticErrorKind) {
    with_source(source, |context, _| {
        let errors = context.take_pending_errors();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TranslationError::Semantic(e) if e.kind == kind)),
            "{source}: {errors:?}"
        );
    });
}

#[test]
fn null_pointer_alternatives_precede_pointer_pair_rules() {
    clean("void f(int *p) { *(1 ? p : (void *)0)=2; *(1 ? (void *)0 : p)=2; }");
    clean(
        "int f(void); int (*p)(void)=1 ? f : (void *)0; int (*q)(void)=1 ? (void *)0 : f; int \
         g(void) { return f==(void *)0 || (void *)0!=f; }",
    );
    with_source(
        "void f(const int *p) { 1 ? p : (void *)0; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let conditional = unit
                .expressions
                .iter()
                .find(|e| matches!(e.expression.kind, ExpressionType::Conditional(_)))
                .unwrap();
            let TypeKind::Pointer(target) = unit.types.nodes[conditional.ty.index] else {
                panic!("expected pointer")
            };
            assert_eq!(
                unit.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Int)
            );
            assert!(target.qualifiers.contains(TypeQualifiers::CONST));
        },
    );
    rejects(
        "int f(void); void g(void *p) { f==p; }",
        SemanticErrorKind::InvalidComparisonOperands,
    );
}

#[test]
fn register_pointer_does_not_transfer_storage_to_pointee() {
    clean("void f(int *q) { register int *p=q; int *r=&p[1]; r=&1[p]; r=&*p; }");
    rejects(
        "void f(void) { register int *p; &p; }",
        SemanticErrorKind::InvalidAddressOperand,
    );
    rejects(
        "void f(void) { register int a[2]; &a[1]; }",
        SemanticErrorKind::InvalidAddressOperand,
    );
}

#[test]
fn parenthesized_strings_initialize_whole_arrays() {
    with_source(
        "char a[]=(\"abc\"); char b[]={(\"abc\")}; signed char c[]=(\"abc\"); unsigned char \
         d[]={(\"abc\")}; int w[]=(L\"abc\"); struct S { char text[4]; } s={(\"abc\")};",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            for binding in &unit.bindings[..5] {
                assert!(matches!(
                    unit.types.nodes[binding.ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(4))
                ));
            }
        },
    );
}

#[test]
fn explicit_pointer_boolean_casts_keep_known_constant_truth() {
    with_source(
        "static _Bool b=(_Bool)(void *)0; static int a; static _Bool c=(_Bool)&a; int f(void); \
         static _Bool d=(_Bool)f;",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let values: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| {
                    matches!(
                        unit.types.nodes[info.ty.index],
                        TypeKind::Scalar(Scalar::Bool)
                    )
                })
                .map(|info| {
                    assert_eq!(info.constant, ConstantClass::Arithmetic);
                    assert!(!info.ice);
                    info.integer.unwrap().value
                })
                .collect();
            assert_eq!(values, [0, 1, 1]);
        },
    );
    rejects(
        "int *p; static _Bool b=(_Bool)p;",
        SemanticErrorKind::NonConstantInitializer,
    );
}

#[test]
fn runtime_offsetof_has_integer_type_without_constant_eligibility() {
    with_source(
        "struct S { int a[5]; }; unsigned long offset(int i) { return __builtin_offsetof(struct \
         S,a[i]); }",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let info = unit
                .expressions
                .iter()
                .find(|info| matches!(info.expression.kind, ExpressionType::Builtin(_)))
                .unwrap();
            assert_eq!(
                unit.types.nodes[info.ty.index],
                TypeKind::Scalar(Scalar::UnsignedLong)
            );
            assert!(!info.ice);
            assert!(info.integer.is_none());
        },
    );
    rejects(
        "struct S { int a[5]; }; void f(double i) { __builtin_offsetof(struct S,a[i]); }",
        SemanticErrorKind::InvalidOffsetof,
    );
    rejects(
        "struct S { int a[5]; }; void f(int i) { __builtin_offsetof(struct S,a[i].bad); }",
        SemanticErrorKind::InvalidOffsetof,
    );
    clean("struct S { int a[5]; }; int x[__builtin_offsetof(struct S,a[2])==8?1:-1];");
}

#[test]
fn compound_addition_enforces_directional_operand_constraints() {
    rejects(
        "void f(int *p) { int n=0; n+=p; }",
        SemanticErrorKind::InvalidAdditiveOperands,
    );
    rejects(
        "void f(void) { int a[2]; int n=0; n+=a; }",
        SemanticErrorKind::InvalidAdditiveOperands,
    );
    clean("void f(int *p) { int n=0; p+=n; n+=2; n=n+p-p; }");
}

#[test]
fn definition_parameters_preserve_register_storage() {
    rejects(
        "int f(register int a) { return *(&a); }",
        SemanticErrorKind::InvalidAddressOperand,
    );
    rejects(
        "int f(a) register int a; { return *(&a); }",
        SemanticErrorKind::InvalidAddressOperand,
    );
    clean("int f(register int *p) { return *(&p[1]); }");
}

#[test]
fn type_only_for_declarations_follow_revision_and_policy() {
    for source in [
        "void f(void) { for(enum { A=1 }; ; ) break; }",
        "void f(void) { for(struct S { int x; }; ; ) break; }",
    ] {
        for standard in [CStandard::C99, CStandard::C17, CStandard::C23] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                with_configuration(
                    source,
                    CompilerConfiguration::new(standard, policy),
                    |context, _| {
                        let errors = context.take_pending_errors();
                        assert_eq!(
                            errors.len(),
                            usize::from(
                                standard < CStandard::C23 && policy != ExtensionPolicy::Allow
                            ),
                            "{standard:?} {policy:?}: {errors:?}"
                        );
                        if let Some(error) = errors.first() {
                            assert!(matches!(error, TranslationError::Extension(_)));
                            assert_eq!(
                                error.severity(),
                                if policy == ExtensionPolicy::Warn {
                                    ErrorSeverity::Warning
                                } else {
                                    ErrorSeverity::Error
                                }
                            );
                        }
                    },
                );
            }
        }
    }
    clean(
        "void f(void) { for(enum { A=1 } x=A; x; ) break; for(struct S { int x; } s={1}; ; ) \
         break; }",
    );
}

#[test]
fn local_labels_require_unique_declarations_and_definitions() {
    rejects(
        "void f(void) { __label__ L,L; L:; }",
        SemanticErrorKind::DuplicateLocalLabel,
    );
    rejects(
        "void f(void) { __label__ L; __label__ L; L:; }",
        SemanticErrorKind::DuplicateLocalLabel,
    );
    rejects(
        "void f(void) { __label__ L; }",
        SemanticErrorKind::UndefinedLocalLabel,
    );
    clean("void f(void) { __label__ L; { __label__ L; goto L; L:; } goto L; L:; }");
    for gnu in [false, true] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            let config =
                CompilerConfiguration::new(CStandard::C17, policy).with_gnu_extensions(gnu);
            with_configuration("void f(void) { __label__ L; }", config, |context, _| {
                let errors = context.take_pending_errors();
                assert!(errors.iter().any(|error| matches!(error, TranslationError::Semantic(e) if e.kind == SemanticErrorKind::UndefinedLocalLabel)));
                let extensions: Vec<_> = errors
                    .iter()
                    .filter(|e| matches!(e, TranslationError::Extension(_)))
                    .collect();
                assert_eq!(
                    extensions.len(),
                    usize::from(policy != ExtensionPolicy::Allow)
                );
                if let Some(extension) = extensions.first() {
                    assert_eq!(
                        extension.severity(),
                        if policy == ExtensionPolicy::Warn {
                            ErrorSeverity::Warning
                        } else {
                            ErrorSeverity::Error
                        }
                    );
                }
            });
        }
    }
}
