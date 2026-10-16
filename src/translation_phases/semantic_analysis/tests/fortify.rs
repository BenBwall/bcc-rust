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

#[test]
fn foreign_fortify_spellings_only_report_unknown_attributes() {
    with_source(
        r#"
[[gnu::overloadable]] int plain(int n) { return n; }
int fixed(char *const p [[gnu::pass_object_size(missing_name)]]) { return 2; }
int dynamic(char *const p [[gnu::pass_dynamic_object_size(1)]]) { return 3; }
[[gnu::unavailable("blocked"), gnu::diagnose_if(missing_name,"blocked","error"),
  gnu::enable_if(missing_name,"blocked"), clang::diagnose_if(missing_name,"blocked","error")]]
int checked(int n) { return n; }
int main(void) { char data[9]; return plain(1)+fixed(data)+dynamic(data)+checked(4); }
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 7, "{errors:?}");
            assert_eq!(sema.errors, 0, "{errors:?}");
            assert!(
                errors.iter().all(|error| matches!(
                    error,
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::UnknownAttribute,
                        ..
                    })
                )),
                "{errors:?}"
            );
        },
    );
}

#[test]
fn late_entity_attributes_respect_mangled_function_definitions() {
    with_source(
        r"
int f(int n) __attribute__((overloadable)) { return n; }
int f(int n) __attribute__((overloadable,weak,always_inline,noinline));
",
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 3, "{errors:?}");
            assert_eq!(sema.errors, 0, "{errors:?}");
            assert!(
                errors.iter().all(|error| matches!(
                    error,
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::AttributeAfterDefinition,
                        ..
                    })
                )),
                "{errors:?}"
            );
            for (binding, _) in sema
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, binding)| context.string_cache.at(binding.name.name) == "f")
            {
                if let Some(attributes) = sema.entity_attributes(binding) {
                    assert!(!attributes.weak);
                    assert_eq!(attributes.inlining, Inlining::Default);
                }
            }
        },
    );
}

#[test]
fn deferred_floating_arithmetic_keeps_target_precision_and_complex_domains() {
    with_source(
        r#"
void divided(double n) __attribute__((diagnose_if(n/2.0==1.5,"division","warning")));
void multiplied(double n) __attribute__((diagnose_if(n*2.0==6.0,"multiplication","warning")));
void rounded(float n) __attribute__((diagnose_if(n+1.0f==16777216.0f,"float rounding","warning")));
void complex(double _Complex n) __attribute__((diagnose_if(n/2.0==1.5+2.0i,"complex-real","warning")));
void product(double _Complex n) __attribute__((diagnose_if(n*(2.0+0.0i)==6.0+8.0i,"complex-complex","warning")));
void test(void) { divided(3.0); multiplied(3.0); rounded(16777216.0f); complex(3.0+4.0i); product(3.0+4.0i); }
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 5, "{errors:?}");
            assert_eq!(sema.errors, 0, "{errors:?}");
            assert!(
                errors.iter().all(|error| matches!(
                    error,
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::CallAttribute,
                        ..
                    })
                )),
                "{errors:?}"
            );
        },
    );
}
