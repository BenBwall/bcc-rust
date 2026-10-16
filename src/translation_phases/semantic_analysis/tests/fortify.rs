//! Selected call diagnostics keep the attribute's own message and provenance.
use super::*;

#[test]
fn call_predicates_are_deferred_and_unavailable_declarations_can_be_unused() {
    with_source(
        r#"
int checked(int flags) __attribute__((diagnose_if((flags & 64) != 0,"mode required","error")));
int trap(void) __attribute__((unavailable("do not call")));
int unknown(int flags) { return checked(flags); }
int good(void) { return checked(0); }
int bad(void) { return checked(64); }
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            let calls: Vec<_> = errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Semantic(error)
                        if error.kind == SemanticErrorKind::CallAttribute =>
                        Some(error),
                    | _ => None,
                })
                .collect();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(calls.len(), 1);
            assert_eq!(
                context.string_cache.at(calls[0].name.unwrap()),
                "mode required"
            );
            assert!(calls[0].previous.is_some());
            assert_eq!(sema.errors, 1);
        },
    );
}

#[test]
fn diagnose_if_warning_preserves_warning_severity_and_lowering_gate() {
    with_source(
        r#"int f(int n) __attribute__((diagnose_if(n>8,"too large","warning"))); int g(void){return f(9);}"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert_eq!(sema.errors, 0);
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::CallAttribute,
                    extension_severity: Some(crate::translation_phases::ErrorSeverity::Warning),
                    ..
                })
            ));
        },
    );
}

#[test]
fn redeclared_diagnostics_keep_original_parameter_environment_and_object_offsets() {
    with_source(
        r#"
int checked(int n) __attribute__((diagnose_if(n>3,"too large","warning")));
int checked(int renamed);
void object(char *p) __attribute__((diagnose_if(__builtin_object_size(p+2,1)==6,"known offset","warning")));
void test(void) { char data[8]; checked(4); object(data); }
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 2, "{errors:?}");
            assert_eq!(sema.errors, 0);
        },
    );
}

#[test]
fn overload_ambiguity_and_unavailable_address_are_diagnosed() {
    for (source, kind) in [
        (
            "int f(int,int,double) __attribute__((overloadable)); int f(long,long,int) \
             __attribute__((overloadable)); int test(void){return f(1,1,1);}",
            SemanticErrorKind::AmbiguousOverload,
        ),
        (
            "int f(int n) __attribute__((overloadable,enable_if(n!=0,\"nonzero\"))); int f(int n) \
             __attribute__((overloadable,enable_if(n>0,\"positive\"),enable_if(n<10,\"small\"))); \
             int test(void){return f(5);}",
            SemanticErrorKind::AmbiguousOverload,
        ),
        (
            "int f(int) __attribute__((unavailable(\"blocked\"))); int(*p)(int)=f;",
            SemanticErrorKind::CallAttribute,
        ),
        (
            "int f(char *const p __attribute__((pass_object_size(1)))); void *test(void){return \
             (void*)f;}",
            SemanticErrorKind::InvalidAddressOperand,
        ),
    ] {
        with_source(source, |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(sema.errors, 1, "{errors:?}");
            assert!(
                errors.iter().any(
                    |error| matches!(error,TranslationError::Semantic(error) if error.kind==kind)
                ),
                "{errors:?}"
            );
        });
    }
}

#[test]
fn deferred_predicates_apply_boolean_and_floating_parameter_conversions() {
    with_source(
        r#"
int boolean(_Bool n) __attribute__((diagnose_if(n,"boolean","warning")));
int casted(int n) __attribute__((diagnose_if((_Bool)n,"casted","warning")));
int integer(int n) __attribute__((diagnose_if(n>3,"integer","warning")));
int floating(double n) __attribute__((diagnose_if(n>0.0,"floating","warning")));
int rounded(double n) __attribute__((diagnose_if(n==9007199254740993LL,"rounded","warning")));
void null(const char *p) __attribute__((diagnose_if(p==0,"null pointer","warning")));
void nonnull(const char *p) __attribute__((diagnose_if(p!=0,"nonnull pointer","warning")));
char global[4];
void test(void) { char local[4]; boolean(2); casted(2); integer(4.5); floating(2.0); rounded(9007199254740992.0); null((void*)0); nonnull("x"); nonnull(global); nonnull(local); }
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 9, "{errors:?}");
            assert_eq!(sema.errors, 0);
        },
    );
}

#[test]
fn unsupported_enable_expressions_do_not_receive_a_fabricated_symbol() {
    with_source(
        r#"int f(int *p) __attribute__((overloadable,enable_if(p[0]>0,"positive")));"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(sema.errors, 1, "{errors:?}");
            assert!(errors.iter().any(|error| matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::AttributeArguments)), "{errors:?}");
        },
    );
}
