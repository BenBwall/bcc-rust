//! Selected call diagnostics keep the attribute's own message and provenance.
use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

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
            // Nine predicates hold; `integer(4.5)` also drops a fraction
            // (Clang's `-Wliteral-conversion`).
            assert_eq!(errors.len(), 10, "{errors:?}");
            assert_eq!(
                errors
                    .iter()
                    .filter(|error| matches!(error, TranslationError::Semantic(error) if matches!(error.kind, SemanticErrorKind::LiteralConversion(_))))
                    .count(),
                1
            );
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

#[test]
fn cleanup_diagnostics_substitute_the_automatic_object_address() {
    with_source(
        r#"
void nonnull(int *p) __attribute__((diagnose_if(p!=0,"nonnull","error")));
void null(int *p) __attribute__((diagnose_if(p==0,"null","error")));
void constant(int *p) __attribute__((diagnose_if(1,"constant","warning")));
void known(int *p) __attribute__((diagnose_if(__builtin_object_size(p,0)==sizeof(int),"size","warning")));
void offset(void *p) __attribute__((diagnose_if(__builtin_object_size((char*)p+2,1)==5,"offset","warning")));
void runtime(int *p) __attribute__((diagnose_if(__builtin_constant_p(p),"constant address","error")));
void blocked(int *p) __attribute__((unavailable("blocked")));
[[clang::unavailable("scoped")]] void scoped(int *p);
void renamed(int *p) __attribute__((diagnose_if(p!=0,"renamed","warning")));
void renamed(int *other);
void test(void) {
    int a __attribute__((cleanup(nonnull))) = 0;
    int b __attribute__((cleanup(null))) = 0;
    int c __attribute__((cleanup(constant))) = 0;
    int d __attribute__((cleanup(known))) = 0;
    char data[7] __attribute__((cleanup(offset)));
    int e __attribute__((cleanup(runtime))) = 0;
    int f __attribute__((cleanup(blocked))) = 0;
    int g __attribute__((cleanup(scoped))) = 0;
    int h __attribute__((cleanup(renamed))) = 0;
}
"#,
        |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 7, "{errors:?}");
            assert_eq!(sema.errors, 3, "{errors:?}");
            let messages: Vec<_> = errors
                .iter()
                .map(|error| {
                    let TranslationError::Semantic(error) = error else {
                        panic!("{error:?}")
                    };
                    assert_eq!(error.kind, SemanticErrorKind::CallAttribute);
                    assert!(error.previous.is_some());
                    context.string_cache.at(error.name.unwrap())
                })
                .collect();
            assert_eq!(
                messages,
                [
                    "nonnull", "constant", "size", "offset", "blocked", "scoped", "renamed"
                ]
            );
        },
    );
}

#[test]
fn cleanup_rejects_distinct_overloads_without_selecting_a_candidate() {
    for source in [
        "void hook(int*) __attribute__((overloadable)); void hook(double*) \
         __attribute__((overloadable)); void test(void){int n __attribute__((cleanup(hook)))=0;}",
        "void hook(int*) __attribute__((overloadable)); void hook(void*) \
         __attribute__((overloadable)); void test(void){int n __attribute__((cleanup(hook)))=0;}",
        "void hook(int *p) __attribute__((overloadable,enable_if(p!=0,\"nonnull\"))); void \
         hook(int *p) __attribute__((overloadable,enable_if(1,\"constant\"))); void \
         test(void){int n __attribute__((cleanup(hook)))=0;}",
    ] {
        with_source(source, |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}\n{errors:?}");
            assert_eq!(sema.errors, 1, "{errors:?}");
            assert!(
                matches!(
                    errors[0],
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::CleanupOverloadSet,
                        previous: Some(_),
                        ..
                    })
                ),
                "{errors:?}"
            );
        });
    }
}

#[test]
fn cleanup_accepts_single_overloads_redeclarations_and_disabled_enable_predicates() {
    for source in [
        "void hook(int*) __attribute__((overloadable)); void test(void){int n \
         __attribute__((cleanup(hook)))=0;}",
        "void hook(int*) __attribute__((overloadable)); void hook(int*) \
         __attribute__((overloadable)); void test(void){int n __attribute__((cleanup(hook)))=0;}",
        "void hook(int *p) __attribute__((enable_if(0,\"disabled\"))); void test(void){int n \
         __attribute__((cleanup(hook)))=0;}",
        "void hook(int *p) __attribute__((enable_if(p!=0,\"nonnull\"))); void hook(int *p) \
         __attribute__((enable_if(1,\"constant\"))); void test(void){int n \
         __attribute__((cleanup(hook)))=0;}",
    ] {
        with_source(source, |context, sema| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{source}\n{errors:?}");
            assert_eq!(sema.errors, 0, "{errors:?}");
        });
    }
}

#[test]
fn overload_floating_complex_and_qualification_sequences_match_clang() {
    // The return type identifies the selected declaration without executing
    // binary80/binary128 or complex arithmetic in the interpreter.
    for (first, second, argument, expected) in [
        ("double", "int", "1.0f", "char"),
        ("float", "long double", "1.0", "long"),
        ("int", "__float128", "1.0", "long"),
        (
            "float _Complex",
            "double _Complex",
            "(float _Complex)1",
            "char",
        ),
        (
            "float _Complex",
            "long double _Complex",
            "(double _Complex)1",
            "long",
        ),
        ("float _Complex", "int", "1.0", "long"),
        ("float", "double _Complex", "(float _Complex)1", "long"),
        ("_Bool", "float", "(double _Complex)1", "char"),
        ("_Bool", "void *", "(int *)0", "long"),
        ("const int *", "void *", "(int *)0", "char"),
        ("const int *", "const volatile int *", "(int *)0", "char"),
        ("int", "enum E", "(enum E)0", "long"),
    ] {
        let source = format!(
            "enum E{{zero}}; char f({first}) __attribute__((overloadable)); long f({second}) \
             __attribute__((overloadable)); \
             _Static_assert(__builtin_types_compatible_p(__typeof__(f({argument})),{expected}), \
             \"selected overload\");"
        );
        with_source(&source, |context, sema| {
            assert_eq!(
                sema.errors,
                0,
                "{source}\n{:?}",
                context.take_pending_errors()
            );
        });
    }
}

#[test]
fn overload_floating_promotions_and_crossed_qualifiers_remain_ambiguous() {
    for (first, second, argument) in [
        ("double", "long double", "1.0f"),
        (
            "double _Complex",
            "long double _Complex",
            "(float _Complex)1",
        ),
        ("float _Complex", "double _Complex", "1.0"),
        ("const int *", "volatile int *", "(int *)0"),
        ("int *", "void *", "0"),
    ] {
        let source = format!(
            "int f({first}) __attribute__((overloadable)); int f({second}) \
             __attribute__((overloadable)); int test(void){{return f({argument});}}"
        );
        with_source(&source, |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(sema.errors, 1, "{source}\n{errors:?}");
            assert!(
                matches!(
                    errors.as_slice(),
                    [TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::AmbiguousOverload,
                        ..
                    })]
                ),
                "{source}\n{errors:?}"
            );
            assert!(sema.function_extensions.calls.is_empty());
        });
    }
}

#[test]
fn overload_ellipsis_beats_incompatible_pointers_and_reports_unavailability() {
    for argument in ["(char *)0", "(const int *)0"] {
        let source = format!(
            "int f(int *) __attribute__((overloadable)); long f(...) \
             __attribute__((overloadable,unavailable(\"non arithmetic\"))); void \
             test(void){{f({argument});}}"
        );
        with_source(&source, |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(sema.errors, 1, "{source}\n{errors:?}");
            assert!(
                matches!(
                    errors.as_slice(),
                    [TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::CallAttribute,
                        ..
                    })]
                ),
                "{source}\n{errors:?}"
            );
        });
    }
}

#[test]
fn overload_declarations_require_prototypes_and_one_unmarked_entity() {
    for source in [
        "int f() __attribute__((overloadable));",
        "int f(int) __attribute__((overloadable)); int f(double); int f(float);",
    ] {
        with_source(source, |context, sema| {
            assert_eq!(
                sema.errors,
                1,
                "{source}\n{:?}",
                context.take_pending_errors()
            );
        });
    }
}

#[test]
fn overload_null_pointer_conversions_do_not_prefer_added_qualifiers() {
    for argument in ["0", "(void *)0", "(void *)1"] {
        let source = format!(
            "int f(int *) __attribute__((overloadable)); int f(const int *) \
             __attribute__((overloadable)); int test(void){{return f({argument});}}"
        );
        with_source(&source, |context, sema| {
            let errors = context.take_pending_errors();
            assert_eq!(sema.errors, 1, "{argument}: {errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::AmbiguousOverload,
                    ..
                })
            ));
        });
    }
    with_source(
        r#"
char f(int *) __attribute__((overloadable));
long f(const void *) __attribute__((overloadable));
_Static_assert(__builtin_types_compatible_p(__typeof__(f((void *)0)), char), "null pointer conversion");
_Static_assert(__builtin_types_compatible_p(__typeof__(f((void *)1)), long), "qualification conversion");
"#,
        |context, sema| {
            assert_eq!(sema.errors, 0, "{:?}", context.take_pending_errors());
        },
    );
}

#[test]
fn overload_array_element_qualification_keeps_exact_conversion_rank() {
    with_source(
        r#"
char f(const int (*)[3]) __attribute__((overloadable));
long f(void *) __attribute__((overloadable));
char g(const int (*)[2][3]) __attribute__((overloadable));
long g(const volatile int (*)[2][3]) __attribute__((overloadable));
int a[3], b[2][3];
_Static_assert(__builtin_types_compatible_p(__typeof__(f(&a)), char), "array qualification");
_Static_assert(__builtin_types_compatible_p(__typeof__(g(&b)), char), "nested qualifiers");
"#,
        |context, sema| {
            assert_eq!(sema.errors, 0, "{:?}", context.take_pending_errors());
        },
    );
}

#[test]
fn overload_hidden_block_externs_compare_mangled_entities() {
    with_source(
        r"
void a(void) { extern int f(int) __attribute__((overloadable)); f(1); }
void b(void) { extern int f(double) __attribute__((overloadable)); f(2.0); }
",
        |context, sema| {
            assert_eq!(sema.errors, 0, "{:?}", context.take_pending_errors());
        },
    );
}

/// Frozen against LLVM 23.1.1 `MicrosoftMangle`, including the separate
/// dictionaries shared by nested signatures and vendor template names.
#[test]
fn overload_msvc_symbols_match_clang_for_supported_c_signatures() {
    for (source, expected) in [
        (
            "int f(int) __attribute__((overloadable));",
            "?f@@$$J0YAHH@Z",
        ),
        (
            "static int f(int x) __attribute__((overloadable)){return x;}",
            "?f@@YAHH@Z",
        ),
        (
            "long f(float,double,long double) __attribute__((overloadable));",
            "?f@@$$J0YAJMNO@Z",
        ),
        (
            "int f(void) __attribute__((overloadable));",
            "?f@@$$J0YAHXZ",
        ),
        ("int f(...) __attribute__((overloadable));", "?f@@$$J0YAHZZ"),
        (
            "int f(int,...) __attribute__((overloadable));",
            "?f@@$$J0YAHHZZ",
        ),
        (
            "int f(char*,const char*,volatile char*,char*restrict) __attribute__((overloadable));",
            "?f@@$$J0YAHPEADPEBDPECDPEIAD@Z",
        ),
        (
            "int f(int**,const int**,int*const*,int*volatile*) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAPEAHPEAPEBHPEBQEAHPECREAH@Z",
        ),
        (
            "struct R{int x;}; int f(struct R,struct R*,struct R) __attribute__((overloadable));",
            "?f@@$$J0YAHUR@@PEAU1@0@Z",
        ),
        (
            "union U{int x;}; int f(union U) __attribute__((overloadable));",
            "?f@@$$J0YAHTU@@@Z",
        ),
        (
            "enum E{A};int f(enum E) __attribute__((overloadable));",
            "?f@@$$J0YAHW4E@@@Z",
        ),
        (
            "typedef struct{int x;} Name;int f(Name) __attribute__((overloadable));",
            "?f@@$$J0YAHUName@@@Z",
        ),
        (
            "int f(int(*)(int),int(*)(),int(*)(void),int(*)[3]) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHH@ZP6AH@ZP6AHXZPEAY02H@Z",
        ),
        (
            "int f(float _Complex,float _Complex,double _Complex,long double _Complex) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHU?$_Complex@M@__clang@@0U?$_Complex@N@2@U?$_Complex@O@2@@Z",
        ),
        (
            "int f(_Atomic(int),_Atomic(int)) __attribute__((overloadable));",
            "?f@@$$J0YAHU?$_Atomic@H@__clang@@0@Z",
        ),
        (
            "typedef int V __attribute__((vector_size(16)));int f(V,V) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHT?$__vector@H$03@__clang@@0@Z",
        ),
        (
            "int f(char *const p __attribute__((pass_object_size(1)))) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHQEADW4__pass_object_size1@__clang@@@Z",
        ),
        (
            "int f(int *const) __attribute__((overloadable));",
            "?f@@$$J0YAHQEAH@Z",
        ),
        (
            "int f(long long,long long) __attribute__((overloadable));",
            "?f@@$$J0YAH_J0@Z",
        ),
        (
            "int f(_Bool,_Bool) __attribute__((overloadable));",
            "?f@@$$J0YAH_N0@Z",
        ),
        (
            "int f(int(*)(char*,char*),char*,char*) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHPEAD0@Z00@Z",
        ),
        (
            "int f(int(*)(int*const)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);int f(F*) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);int f(int(*)(int*)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHPEAH@Z@Z",
        ),
        (
            "int f(int(*)(int*),int(*)(int*const)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHPEAH@Z1@Z",
        ),
        (
            "typedef int I __attribute__((aligned(32)));int f(I*,int*) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHPEAH0@Z",
        ),
        (
            "typedef int F(int*const);int f(_Atomic(F*)) __attribute__((overloadable));",
            "?f@@$$J0YAHU?$_Atomic@P6AHQEAH@Z@__clang@@@Z",
        ),
        (
            "typedef int F(int*const);int f(__typeof__(F)*) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);F*value;int f(__typeof__(value)) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "struct S{int(*cb)(int*const);};int f(__typeof__(((struct S*)0)->cb)) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "struct S{struct{int(*cb)(int*const);};};int f(__typeof__(((struct S*)0)->cb)) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "int callback(int*const);int f(__typeof__(&callback)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "int(*produce(void))(int*const);int f(__typeof__(produce())) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);F**pointer;int f(__typeof__(*pointer)) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "int f(int callback(int*const)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);typedef int G(int*);F*a;G*b;int f(__typeof__(1?a:b)) \
             __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHPEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);F*a;int f(__typeof__(1?a:a)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "void outer(int(*cb)(int*const)){extern int f(__typeof__(cb)) \
             __attribute__((overloadable));f(cb);}",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "void outer(int cb(int*const)){extern int f(__typeof__(cb)) \
             __attribute__((overloadable));f(cb);}",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "void outer(cb) int(*cb)(int*const); {extern int f(__typeof__(cb)) \
             __attribute__((overloadable));f(cb);}",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "int f(int(*)[2][11]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY11L@H@Z",
        ),
        (
            "int cb(int*const);int f(__typeof__((0,cb))) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "int cb(int*const);int f(__typeof__(1?cb:cb)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "void outer(void){extern int f(__typeof__(({typedef int F(int*const);(F*)0;}))) \
             __attribute__((overloadable));f(0);}",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);F*a;int f(__typeof__(1?0:a)) __attribute__((overloadable));",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "void outer(void){typedef int F(int*const);F*a;extern int f(__typeof__(({a;}))) \
             __attribute__((overloadable));f(a);}",
            "?f@@$$J0YAHP6AHQEAH@Z@Z",
        ),
        (
            "typedef int F(int*const);F**a;int f(__typeof__(a+=0)) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAP6AHQEAH@Z@Z",
        ),
        (
            "struct R {int x;};struct R f(int) __attribute__((overloadable));",
            "?f@@$$J0YA?AUR@@H@Z",
        ),
        (
            "enum E{A};enum E f(enum E) __attribute__((overloadable));",
            "?f@@$$J0YA?AW4E@@W41@@Z",
        ),
        (
            "int f(int(*)[]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY0A@H@Z",
        ),
        (
            "int f(const int(*)[3]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY02$$CBH@Z",
        ),
        (
            "int f(volatile int(*)[3]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY02$$CCH@Z",
        ),
        (
            "int f(const int(*)[2][3]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY112$$CBH@Z",
        ),
        (
            "int f(int*const(*)[3]) __attribute__((overloadable));",
            "?f@@$$J0YAHPEAY02QEAH@Z",
        ),
        (
            "int f(const int[3]) __attribute__((overloadable));",
            "?f@@$$J0YAHQEBH@Z",
        ),
    ] {
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(Target::WindowsMsvc),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{source}: {:?}",
                    context.take_pending_errors()
                );
                let binding = sema
                    .bindings
                    .iter()
                    .enumerate()
                    .find(|(_, b)| context.string_cache.at(b.name.name) == "f")
                    .unwrap()
                    .0;
                let actual = sema.function_extensions.function(binding).unwrap().abi_name;
                assert_eq!(context.string_cache.at(actual), expected, "{source}");
            },
        );
    }
}
