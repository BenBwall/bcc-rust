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

/// A folded switch's case labels are analyzed in source order and scope, so
/// their diagnostics follow the statements before them and their types are
/// visible only from their position on (#113 review, findings 1 and 10).
/// C99: §6.2.1 paragraphs 4 and 7, pp. 29-30; PDF pp. 41-42.
#[test]
fn folded_switch_case_labels_keep_source_order_and_scope() {
    let names = |source: &str, expected: &[&str]| {
        with_source(source, |context, _| {
            let errors = context.take_pending_errors();
            let names = errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Semantic(error) => error.name,
                    | _ => None,
                })
                .map(|name| context.string_cache.at(name))
                .collect::<Vec<_>>();
            assert_eq!(names, expected, "{source}");
        });
    };
    names(
        "int f(void) { switch (1) { case 1: bad = 0; break; case nope: break; } return 0; }",
        &["bad", "nope"],
    );
    names(
        "int f(void) { switch (1) { case x: y = 0; case z: w = 0; } return 0; }",
        &["x", "y", "z", "w"],
    );
    // An enumerator a case label declares in one block is not visible in
    // the next one.
    assert!(
        kinds(
            "int main(void) { switch (1) { { case sizeof(enum { K = 1 }): break; } { case K: \
             return 1; } } return 0; }"
        )
        .contains(&SemanticErrorKind::UndeclaredIdentifier)
    );
    clean(
        "int main(void) { switch (1) { { case sizeof(enum { K = 1 }): break; } { case sizeof(enum \
         { K = 2 }) + 1: break; } } return 0; }",
    );
    clean(
        "struct T { char v[2]; }; int main(void) { int r = 0; switch (2) { { case sizeof(struct T \
         { char v[4]; }): r = 4; break; } { case sizeof(struct T): r = 2; break; } } return r != \
         2; }",
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
        [SemanticErrorKind::PointerConstraint(
            crate::configuration::DiagnosticGroup::PointerQualifiers
        )]
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
    // An unused one is reported only under -Wunused-function, as in Clang.
    clean("static int f(void);");
    clean("static int f(void); int g(void) {return f();} static int f(void) {return 0;}");
    clean("static int f(void); int g(void) {return sizeof &f;}");
    assert_eq!(
        kinds("static int f(void); void g(int n) {sizeof(int[f()]);}"),
        [SemanticErrorKind::UndefinedInternal]
    );
    // A constant query's operand is used, even where only its value is
    // emitted.
    for source in [
        "static int f(void); int g(void) {return __builtin_constant_p(f());}",
        "static int f(void); int g(void) {static int s = __builtin_constant_p(f()); return s;}",
    ] {
        assert_eq!(
            kinds(source),
            [SemanticErrorKind::UndefinedInternal],
            "{source}"
        );
    }
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

/// A case label in a statement expression of a type name, which the label
/// walk of a folded switch does not record, is rejected after a tentative
/// `default` as anywhere else (PR #117 review).
#[test]
fn folded_switch_rejects_a_case_label_in_a_type_name() {
    for source in [
        "void t(void) { switch (1) { default: break; case 2: { __typeof__(({ case 1: ; 0; })) x = \
         0; (void)x; } } }",
        "void t(void) { switch (1) { default: break; case 2: (void)sizeof(int[({ case 1: ; 1; \
         })]); } }",
    ] {
        assert!(
            kinds(source).contains(&SemanticErrorKind::SwitchIntoStatementExpression),
            "{source}"
        );
    }
}

/// Whether a call does not return is decided without native recursion
/// through nested `__typeof__` operands and casts (PR #117 review).
#[test]
fn noreturn_types_have_no_native_stack_limit() {
    let operand = format!("{}p{}", "(__typeof__(".repeat(10_000), "))0".repeat(10_000));
    clean(&format!(
        "void (*p)(void); void t(void) {{ __typeof__({operand}) x = 0; (void)x; ({operand})(); }}"
    ));
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
        [SemanticErrorKind::PointerConstraint(
            crate::configuration::DiagnosticGroup::IntConversion
        )]
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
            assert_eq!(kinds[0], DefinitionKind::Tentative);
            assert_eq!(kinds[1], DefinitionKind::Function);
            assert!(matches!(kinds[2], DefinitionKind::FunctionName(_)));
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

#[test]
fn unused_static_functions_follow_their_warning_group() {
    use crate::{
        configuration::{
            CompilerConfiguration,
            DiagnosticGroup,
            WarningOption,
        },
        translation_phases::ErrorSeverity,
    };
    for (option, severity) in [
        (WarningOption::Enable(true), ErrorSeverity::Warning),
        (WarningOption::Error(true), ErrorSeverity::Error),
    ] {
        let configuration = CompilerConfiguration::default()
            .with_warning_option(DiagnosticGroup::UnusedFunction, option);
        with_configuration("static int f(void);", configuration, |context, unit| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(errors.as_slice(), [TranslationError::Semantic(error)] if error.kind == SemanticErrorKind::UnusedStaticFunction),
                "{errors:?}"
            );
            assert_eq!(errors[0].configured_severity(context), severity);
            assert_eq!(unit.errors, usize::from(severity == ErrorSeverity::Error));
        });
    }
}

/// The emission model's queries stay linear on inputs that made them
/// quadratic during PR #117's reviews: `strlen` of a long constant array,
/// many reads of a large constant table, nested blocks that each begin
/// with a call that does not return, and nested assignments and Microsoft
/// call arguments after such a call, which code generation emits in
/// another order.
#[test]
fn emission_model_queries_stay_linear() {
    let length = 64_000;
    let size = 20_000;
    let depth = 16_000;
    let mut table = format!("static const int t[{size}] = {{");
    for index in 0..size {
        write!(table, "{},", index % 7).unwrap();
    }
    table.push_str("}; int f(void) { int r = 0; ");
    for index in 0..size {
        write!(table, "if (t[{index}]) r++; ").unwrap();
    }
    table.push_str("return r; }");
    let mut blocks = String::from("__attribute__((noreturn)) void die(void); void f(void) { ");
    for index in 0..depth {
        write!(blocks, "l{index}: {{ die(); ").unwrap();
    }
    blocks.push_str(&"}".repeat(depth + 1));
    let assignment = format!(
        "void abort(void); int a[4]; void f(void) {{ abort(), {}a[0]{} = 1; }}",
        "a[".repeat(depth),
        " = 1]".repeat(depth)
    );
    let compound = format!(
        "void abort(void); int a[4]; void f(void) {{ abort(), {}a[0]{} += 1; }}",
        "a[".repeat(depth),
        " += 1]".repeat(depth)
    );
    let arguments = format!(
        "void abort(void); int g(int, int); void f(void) {{ abort(), {}0{}; }}",
        "g(".repeat(depth),
        ", 0)".repeat(depth)
    );
    let msvc = crate::configuration::CompilerConfiguration::default()
        .with_target(crate::target::Target::WindowsMsvc);
    for (source, configuration) in [
        (
            format!(
                "static const char s[] = \"{}\"; int f(void) {{ if (__builtin_strlen(s) == 0) \
                 return 1; return 0; }}",
                "a".repeat(length)
            ),
            None,
        ),
        (table, None),
        (blocks, None),
        (assignment, None),
        (compound, None),
        (arguments, Some(msvc)),
    ] {
        let start = std::time::Instant::now();
        match configuration {
            | Some(configuration) => with_configuration(&source, configuration, |_, _| {}),
            | None => with_source(&source, |_, _| {}),
        }
        assert!(
            start.elapsed().as_secs() < 15,
            "{:?}: {}",
            start.elapsed(),
            &source[..80]
        );
    }
}

/// The fold evaluator recurses only as deep as keeps it within a 1 MiB
/// stack, the default reserve of a Windows program's main thread, even
/// unoptimized: through operands, `const` arrays read element by element
/// and `strlen` of such arrays (PR #117's third review).
#[test]
fn fold_evaluation_fits_a_one_mebibyte_stack() {
    let mut sum = String::from("int x; void h(void); void t(void) { if (x");
    sum.push_str(&" + x".repeat(1000));
    sum.push_str(") h(); }");
    let mut arrays = String::from("void h(void); void t(void) { const int a0[1] = {0}; ");
    for index in 1..1000 {
        write!(arrays, "const int a{index}[1] = {{a{}[0]}}; ", index - 1).unwrap();
    }
    arrays.push_str("if (a999[0]) h(); }");
    let mut strings = String::from("void h(void); void t(void) { const char s0[2] = \"a\"; ");
    for index in 1..400 {
        write!(
            strings,
            "const char s{index}[2] = {{(char)__builtin_strlen(s{})}}; ",
            index - 1
        )
        .unwrap();
    }
    strings.push_str("if (s399[0]) h(); }");
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            for source in [sum, arrays, strings] {
                clean(&source);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
