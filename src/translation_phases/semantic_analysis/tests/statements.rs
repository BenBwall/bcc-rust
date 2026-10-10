//! Statement and function constraints, C99 §6.8-§6.9.

use std::fmt::Write;

use super::{
    super::expressions::ConversionKind,
    *,
};

fn kinds(source: &str) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_source(source, |context, _| {
        kinds.extend(
            context
                .take_pending_errors()
                .iter()
                .filter_map(|e| match e {
                    | TranslationError::Semantic(e) => Some(e.kind),
                    | _ => None,
                }),
        );
    });
    kinds
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

#[test]
fn definition_parameters_follow_the_identifier_derivation() {
    clean("int (*fp(int a))(int b) { int b = a; return 0; }");
    assert_eq!(
        kinds("int (g(int a)) { int a = 1; return a; }"),
        [SemanticErrorKind::DuplicateDeclaration]
    );
    clean("int g(int); int (((g(int a)))) { return a; }");
    clean("int (*f(int a, int b))(int x, int y, int z) { return 0; }");
    clean("int f(int); int (f(a)) short a; { return a; }");
    with_source(
        "int f(int); int (f(a)) double a; { return 0; }",
        |context, _| {
            let errors = context.take_pending_errors();
            let mismatch = errors
                .iter()
                .find_map(|e| match e {
                    | TranslationError::Semantic(e)
                        if e.kind == SemanticErrorKind::IncompatibleDeclaration =>
                        Some(e),
                    | _ => None,
                })
                .unwrap();
            assert!(mismatch.previous.is_some());
        },
    );
}

#[test]
fn implicit_old_style_parameters_bind_and_shadow_in_c89() {
    let config = crate::configuration::CompilerConfiguration::new(
        CStandard::C89,
        crate::configuration::ExtensionPolicy::Allow,
    );
    with_configuration(
        "int b[2]; int f(a,b) int a; { return a+b; }",
        config,
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let b = s
                .bindings
                .iter()
                .rev()
                .find(|b| {
                    b.kind == BindingKind::Parameter && context.string_cache.at(b.name.name) == "b"
                })
                .unwrap();
            assert!(matches!(
                s.types.nodes[b.ty.index],
                TypeKind::Scalar(Scalar::Int)
            ));
        },
    );
    with_configuration(
        "int f(a,b) int a; { int b=0; return a; }",
        config,
        |context, _| {
            assert!(context.take_pending_errors().iter().any(|e| matches!(e, TranslationError::Semantic(e) if e.kind == SemanticErrorKind::DuplicateDeclaration)));
        },
    );
    assert_eq!(
        kinds("int f(a,b) int a; { return b; }"),
        [SemanticErrorKind::InvalidDefinitionParameterList]
    );
}

#[test]
fn function_definition_constraints() {
    for (source, expected) in [
        (
            "typedef int F(void); F f { return 0; }",
            SemanticErrorKind::InvalidFunctionDefinition,
        ),
        (
            "typedef int f(void) { return 0; }",
            SemanticErrorKind::FunctionDefinitionStorage,
        ),
        (
            "int f(int) { return 0; }",
            SemanticErrorKind::UnnamedDefinitionParameter,
        ),
        (
            "struct S; void f(struct S s) {}",
            SemanticErrorKind::IncompleteDefinitionParameter,
        ),
        (
            "struct S; struct S f(void) {}",
            SemanticErrorKind::IncompleteFunctionReturn,
        ),
        (
            "void f(int a[*]) {}",
            SemanticErrorKind::DefinitionStarArray,
        ),
        (
            "int f(a) int b; { return 0; }",
            SemanticErrorKind::InvalidDefinitionParameterList,
        ),
        (
            "int f(a) int a=1; { return a; }",
            SemanticErrorKind::InvalidDefinitionParameterList,
        ),
        (
            "int f(a,a) int a; { return a; }",
            SemanticErrorKind::DuplicateDeclaration,
        ),
    ] {
        assert!(
            kinds(source).contains(&expected),
            "{source}: {:?}",
            kinds(source)
        );
    }
    clean("struct S; void f(struct S *s) {} void g(int a[], int (*p)[2]) {}");
    clean("int main(void) { return 0; }");
    clean("int main(int argc, char **argv) { return argc; }");
    assert_eq!(
        kinds("void main(void) {}"),
        [SemanticErrorKind::MainSignature]
    );
    // A freestanding startup function is implementation-defined (C99
    // §5.1.2.1p1), so `main` has no required signature there.
    for source in [
        "void main(void) {}",
        "static int main(char c) { return c; }",
    ] {
        with_configuration(
            source,
            crate::configuration::CompilerConfiguration::default().with_hosted(false),
            |context, _| assert_eq!(context.pending_error_count(), 0, "{source}"),
        );
    }
}

#[test]
fn predefined_function_object_exists_before_its_first_use() {
    clean("int __func__; int f(void) { return sizeof __func__ == 2; }");
    clean("int f(void) { {int __func__=0;} return sizeof __func__ == 2; }");
    assert_eq!(
        kinds("int f(void) { int __func__; return 0; }"),
        [SemanticErrorKind::DuplicateDeclaration]
    );
    with_source("int f(void) { return sizeof __func__; }", |context, s| {
        assert_eq!(context.pending_error_count(), 0);
        let b = s
            .bindings
            .iter()
            .find(|b| context.string_cache.at(b.name.name) == "__func__")
            .unwrap();
        assert_eq!(b.duration, Duration::Static);
        assert_eq!(s.types.layout(b.ty).unwrap().size, 2);
    });
}

#[test]
fn labels_and_loop_placement_are_function_local() {
    clean("void f(void) {goto L; L:;} void g(void) {goto L; L:;}");
    assert_eq!(
        kinds("void f(void) { L:; { L:; } }"),
        [SemanticErrorKind::DuplicateLabel]
    );
    assert_eq!(
        kinds("void f(void) { goto absent; goto absent; }"),
        [SemanticErrorKind::UndefinedLabel]
    );
    assert_eq!(
        kinds("void f(void) { break; continue; }"),
        [
            SemanticErrorKind::BreakOutsideLoopOrSwitch,
            SemanticErrorKind::ContinueOutsideLoop
        ]
    );
    assert_eq!(
        kinds("void f(int n) { switch(n) {case 0: continue;} }"),
        [SemanticErrorKind::ContinueOutsideLoop]
    );
    clean(
        "void f(int n) {while(n) {switch(n) {case 0: continue; default: break;} break;} do \
         {continue;} while(n); for(;;) break;}",
    );
}

#[test]
fn switch_promotion_conversion_and_ranges() {
    assert_eq!(
        kinds("void f(unsigned x) { switch(x) {case -1:; case 4294967295U:;} }"),
        [SemanticErrorKind::DuplicateCase]
    );
    clean("void f(unsigned char x) {switch(x) {case -1:; case 255:;}} ");
    assert_eq!(
        kinds("void f(void) {case 1:; default:;}"),
        [
            SemanticErrorKind::CaseOutsideSwitch,
            SemanticErrorKind::CaseOutsideSwitch
        ]
    );
    clean("void f(int n) { switch(n) {case 1: switch(n) {case 1:; default:;} break; default:;} }");
    for configuration in [
        crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true),
        crate::configuration::CompilerConfiguration::new(
            CStandard::C2y,
            crate::configuration::ExtensionPolicy::Allow,
        ),
    ] {
        for (source, expected) in [
            (
                "void f(int n) {switch(n) {case 1 ... 3:; case 3 ... 5:;}}",
                Some(SemanticErrorKind::DuplicateCase),
            ),
            (
                "void f(int n) {switch(n) {case 5 ... 3:;}}",
                Some(SemanticErrorKind::EmptyCaseRange),
            ),
            (
                "void f(int n) {switch(n) {case 1 ... 3:; case 4 ... 9:;}}",
                None,
            ),
        ] {
            with_configuration(source, configuration, |context, _| {
                let errors = context.take_pending_errors();
                assert_eq!(
                    errors.len(),
                    usize::from(expected.is_some()),
                    "{source}: {errors:?}"
                );
                if let Some(kind) = expected {
                    assert!(
                        matches!(errors.as_slice(), [TranslationError::Semantic(e)] if e.kind == kind),
                        "{source}: {errors:?}"
                    );
                }
            });
        }
    }
    assert_eq!(
        kinds("void f(int n) {switch(n) {case n:;}}"),
        [SemanticErrorKind::InvalidConstant]
    );
}

#[test]
fn jumps_compare_declaration_scope_paths() {
    for source in [
        "void f(int n) {goto L; int a[n]; L:;}",
        "void f(int n) {goto L; {int a[n]; L:;} }",
        "void f(int n) {{int a[n]; goto L;} {int b[n]; L:;} }",
        "void f(int n) {goto L; typedef int A[n]; L:;}",
        "void f(int n) {goto L; int (*p)[n]; L:;}",
    ] {
        assert_eq!(
            kinds(source),
            [SemanticErrorKind::JumpIntoVariableScope],
            "{source}"
        );
    }
    clean("void f(int n) {L:; int a[n]; goto L;}");
    clean("void f(int n) {int a[n]; goto L; L:;}");
    clean("void f(int n) {{int a[n]; goto L;} L:;}");
    assert_eq!(
        kinds("void f(int n) {switch(n) {int a[n]; case 1:; default:;}}"),
        [
            SemanticErrorKind::SwitchIntoVariableScope,
            SemanticErrorKind::SwitchIntoVariableScope
        ]
    );
    clean("void f(int n) {int a[n]; switch(n) {case 1:; default:;}}");
    clean("void f(int n) {switch(n) {case 1: {int a[n];} case 2:;}}");
}

#[test]
fn return_assignment_conversion_is_retained() {
    assert_eq!(
        kinds("void f(void) {return 1;}"),
        [SemanticErrorKind::VoidReturnValue]
    );
    assert_eq!(
        kinds("int f(void) {return;}"),
        [SemanticErrorKind::MissingReturnValue]
    );
    assert_eq!(
        kinds("int *f(const int *p) {return p;}"),
        [SemanticErrorKind::InvalidReturnConversion]
    );
    clean("const int *f(int *p) {return p;} int *g(void) {return 0;}");
    clean("struct S {int x;}; struct S f(struct S s) {return s;}");
    with_source("long f(int n) {return n;}", |context, s| {
        assert_eq!(context.pending_error_count(), 0);
        assert!(
            s.conversions
                .iter()
                .any(|c| c.kind == ConversionKind::Assignment
                    && matches!(s.types.nodes[c.ty.index], TypeKind::Scalar(Scalar::Long)))
        );
    });
}

#[test]
fn return_modes_preserve_c89_and_gnu_diagnostics() {
    for configuration in [
        crate::configuration::CompilerConfiguration::new(
            CStandard::C89,
            crate::configuration::ExtensionPolicy::Allow,
        ),
        crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true),
    ] {
        with_configuration("int f(void) {return;}", configuration, |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(errors.as_slice(), [TranslationError::Semantic(e)] if e.kind == SemanticErrorKind::MissingReturnValueWarning)
            );
        });
    }
    with_configuration(
        "void g(void) {} void f(void) {return g();}",
        crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true),
        |context, _| assert_eq!(context.pending_error_count(), 0),
    );
    assert_eq!(
        kinds("void g(void) {} void f(void) {return g();}"),
        [SemanticErrorKind::VoidReturnValue]
    );
}

#[test]
fn for_initializers_declare_only_automatic_objects() {
    clean(
        "void f(void) {for(auto int i=0;;) break; for(register int j=0;;) break; for(int \
         (*p)(void);;) break;}",
    );
    for source in [
        "void f(void) {for(static int i=0;;) break;}",
        "void f(void) {for(extern int i;;) break;}",
        "void f(void) {for(typedef int I;;) break;}",
        "void f(void) {for(int g(void);;) break;}",
    ] {
        assert_eq!(
            kinds(source),
            [SemanticErrorKind::InvalidForDeclaration],
            "{source}"
        );
    }
}

#[test]
fn tentative_and_external_definitions() {
    clean("int a; int a; extern int a; static int b; extern int b;");
    clean("int a[]; extern int a[3];");
    assert_eq!(
        kinds("int a=1; int a=2;"),
        [SemanticErrorKind::DuplicateDefinition]
    );
    assert_eq!(
        kinds("int f(void) {return 1;} int f(void) {return 2;}"),
        [SemanticErrorKind::DuplicateDefinition]
    );
    assert_eq!(
        kinds("static int a[];"),
        [SemanticErrorKind::IncompleteInternalTentative]
    );
    assert_eq!(
        kinds("struct S; struct S s;"),
        [SemanticErrorKind::IncompleteTentativeDefinition]
    );
    with_source("int a[]; extern int a[];", |context, s| {
        assert_eq!(
            kinds("int a[];"),
            [SemanticErrorKind::TentativeArrayAssumedOne]
        );
        assert_eq!(context.pending_error_count(), 1);
        assert_eq!(
            s.types.layout(s.bindings.last().unwrap().ty).unwrap().size,
            4
        );
    });
    clean("extern int a[];");
}

#[test]
fn internal_definitions_exclude_constant_sizeof_uses() {
    assert_eq!(
        kinds("static int f(void); int g(void) {return f();}"),
        [SemanticErrorKind::UndefinedInternal]
    );
    assert_eq!(
        kinds("static int f(void);"),
        [SemanticErrorKind::UnusedStaticFunction]
    );
    clean("static int f(void); int g(void) {return f();} static int f(void) {return 0;}");
    assert_eq!(
        kinds("static int f(void); int g(void) {return sizeof &f;}"),
        [SemanticErrorKind::UnusedStaticFunction]
    );
    assert_eq!(
        kinds("static int f(void); void g(int n) {sizeof(int[f()]);}"),
        [SemanticErrorKind::UndefinedInternal]
    );
}

#[test]
fn inline_constraints_depend_on_all_file_scope_declarations() {
    assert_eq!(
        kinds("inline int f(void) {static int x; return x;}"),
        [SemanticErrorKind::InlineStaticObject]
    );
    assert_eq!(
        kinds("static int x; inline int f(void) {return x;}"),
        [SemanticErrorKind::InlineInternalReference]
    );
    clean("static inline int f(void) {static int x; return x;}");
    clean("inline int f(void) {static int x; return x;} extern int f(void);");
    clean("int f(void); inline int f(void) {static int x; return x;}");
    clean("inline int f(void) {static const int x=1; return x;}");
    assert_eq!(
        kinds("static int x; inline int f(void) {return sizeof x;}"),
        [SemanticErrorKind::InlineInternalReference]
    );
}

#[test]
fn gnu_statement_forms_keep_their_scopes_and_targets() {
    let config = crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true);
    with_configuration(
        "int f(void) { __label__ L; void *p=&&L; goto *p; L:; {__label__ L; goto L; L:;} return \
         ({int x=1; x;}); }",
        config,
        |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
        },
    );
    with_configuration(
        "void f(void) {while(1) {void g(void) {break;}}}",
        config,
        |context, _| {
            assert!(context.take_pending_errors().iter().any(|e| matches!(e, TranslationError::Semantic(e) if e.kind == SemanticErrorKind::BreakOutsideLoopOrSwitch)));
        },
    );
}

#[test]
fn statement_analysis_has_no_native_stack_limit() {
    for prefix in ["if(1)", "while(1)", "{"] {
        let mut source = String::from("void f(void) {");
        source.push_str(&prefix.repeat(10_000));
        source.push(';');
        if prefix == "{" {
            source.push_str(&"}".repeat(10_000));
        }
        source.push('}');
        clean(&source);
    }
    let mut source = String::from("void f(int n) {switch(n) {");
    for n in 0..10_000 {
        write!(source, "case {n}:").unwrap();
    }
    source.push_str(";}}");
    clean(&source);
}

#[test]
fn malformed_statements_suppress_dependent_constraints() {
    let cases: &[(&str, &[&str])] = &[
        (
            "void f(void) {if() return 1;}",
            &["ExpectedStatementExpression"],
        ),
        (
            "void f(void) {switch() {case :break;}}",
            &["ExpectedStatementExpression", "ExpectedStatementExpression"],
        ),
        ("void f(void) {goto ;}", &["ExpectedGotoLabel"]),
        (
            "int f(int a,) {return +;}",
            &[
                "ExpectedParameterDeclarationAfterCommaInFunctionDeclarator",
                "ExpectedStatementExpression",
            ],
        ),
    ];
    for &(source, expected_parser_kinds) in cases {
        let source = format!("{source} void following(void) {{return 1;}}");
        with_source(&source, |context, _| {
            let errors = context.take_pending_errors();
            let (last, parser_errors) = errors.split_last().unwrap();
            assert!(
                matches!(last, TranslationError::Semantic(e)
                    if e.kind == SemanticErrorKind::VoidReturnValue),
                "{source}: {errors:?}"
            );
            let parser_kinds: Vec<_> = parser_errors
                .iter()
                .map(|error| match error {
                    | TranslationError::Parsing(e) => {
                        // The parser's kind enum is private. Check its variant
                        // name, excluding the grammar-specific payload.
                        let mut kind = format!("{:?}", e.error_type);
                        if let Some(payload) = kind.find('(') {
                            kind.truncate(payload);
                        }
                        kind
                    },
                    | _ => panic!("{source}: unexpected dependent diagnostic: {error:?}"),
                })
                .collect();
            assert_eq!(parser_kinds, expected_parser_kinds, "{source}: {errors:?}");
        });
    }
}

#[test]
fn opaque_compiler_builtins_do_not_acquire_implicit_int_results() {
    clean(
        "char *f(char *p) {return __builtin_strdup(p);} void *g(void *p, void *q) {return \
         __builtin___memcpy_chk(p,q,1,1);} void *h(void) {return __builtin_return_address(0);}",
    );
    assert_eq!(
        kinds("int *f(void) {return 1;}"),
        [SemanticErrorKind::InvalidReturnConversion]
    );
}

#[test]
fn nested_function_definitions_have_no_translation_unit_linkage() {
    clean(
        "void f(void) {int nested(void) {return 1;} nested();} void g(void) {int nested(void) \
         {return 2;} nested();}",
    );
    clean("void f(int n) {__label__ L; int a[n]; void nested(void) {goto L;} nested(); L:;}");
}

#[test]
fn finalized_definitions_retain_synthesized_objects() {
    with_source(
        "int a[]; int f(void) {return sizeof __func__;}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 1);
            let kinds: Vec<_> = s.definitions.iter().map(|d| d.kind).collect();
            assert_eq!(kinds.len(), 3);
            assert_eq!(kinds[0], functions::DefinitionKind::Tentative);
            assert_eq!(kinds[1], functions::DefinitionKind::Function);
            assert!(matches!(
                kinds[2],
                functions::DefinitionKind::FunctionName(_)
            ));
        },
    );
}

#[test]
fn unnamed_parameters_follow_c23_and_existing_parser_policy() {
    with_configuration(
        "int f(int) {return 0;}",
        crate::configuration::CompilerConfiguration::new(
            CStandard::C23,
            crate::configuration::ExtensionPolicy::Allow,
        ),
        |context, _| {
            assert_eq!(context.pending_error_count(), 0);
        },
    );
    with_configuration(
        "int f(int) {return 0;}",
        crate::configuration::CompilerConfiguration::default()
            .with_extension_policy(crate::configuration::ExtensionPolicy::Deny),
        |context, _| {
            assert_eq!(context.pending_error_count(), 1);
        },
    );
}

#[test]
fn associated_substatements_have_independent_c99_scopes() {
    assert_eq!(
        kinds("void f(int n) {if(n) sizeof(enum {A=1}); else A;}"),
        [SemanticErrorKind::UndeclaredIdentifier]
    );
    assert_eq!(
        kinds("void f(int n) {do sizeof(enum {A=1}); while(A);}"),
        [SemanticErrorKind::UndeclaredIdentifier]
    );
    clean("void f(int n) {if(n) sizeof(enum {A=1}); else sizeof(enum {A=2});}");
    clean("void f(int n) {while(n) sizeof(enum {A=1}); int A;}");
}

#[test]
fn invalid_definition_types_do_not_cascade_into_body_expressions() {
    assert_eq!(
        kinds("struct S; void f(struct S s) {s.x;}"),
        [SemanticErrorKind::IncompleteDefinitionParameter]
    );
    assert_eq!(
        kinds("struct S; struct S f(void) {return 1;}"),
        [SemanticErrorKind::IncompleteFunctionReturn]
    );
    assert_eq!(
        kinds("void f(void p) {p+1;}"),
        [SemanticErrorKind::InvalidParameter]
    );
    assert_eq!(
        kinds("register int f(void) {return 0;}"),
        [SemanticErrorKind::InvalidStorage]
    );
}

#[test]
fn inline_static_objects_use_object_const_qualification() {
    assert_eq!(
        kinds("inline void f(void) {static struct {const int a; int b;} s; s.b=1;}"),
        [SemanticErrorKind::InlineStaticObject]
    );
    clean("inline void f(void) {static const int a[1]={0}; sizeof a;}");
}

#[test]
fn unnamed_definition_parameter_fallback_matches_clang_severity_without_duplicates() {
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
    for standard in [CStandard::C17, CStandard::C23] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            for source in [
                "int f(int) {return 0;}",
                "typedef int I; int f(I) {return 0;}",
            ] {
                with_configuration(
                    source,
                    CompilerConfiguration::new(standard, policy),
                    |context, _| {
                        let errors = context.take_pending_errors();
                        assert_eq!(
                            errors.len(),
                            usize::from(standard < CStandard::C23),
                            "{errors:?}"
                        );
                        if let Some(error) = errors.first() {
                            assert_eq!(
                                error.severity(),
                                if policy == ExtensionPolicy::Deny {
                                    ErrorSeverity::Error
                                } else {
                                    ErrorSeverity::Warning
                                }
                            );
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn implicit_old_style_parameters_keep_strict_error_and_gnu_extension_policy() {
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
    for standard in [CStandard::C89, CStandard::C17] {
        for gnu in [false, true] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                with_configuration(
                    "int f(x) {return x;}",
                    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu),
                    |context, _| {
                        let errors = context.take_pending_errors();
                        let expected = if standard == CStandard::C89
                            || (gnu && policy == ExtensionPolicy::Allow)
                        {
                            None
                        } else if gnu && policy == ExtensionPolicy::Warn {
                            Some(ErrorSeverity::Warning)
                        } else {
                            Some(ErrorSeverity::Error)
                        };
                        assert_eq!(errors.len(), usize::from(expected.is_some()), "{errors:?}");
                        if let Some(expected) = expected {
                            assert_eq!(errors[0].severity(), expected);
                        }
                    },
                );
            }
        }
    }
}
