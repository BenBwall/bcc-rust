//! GNU parser grammar regressions.
use std::fmt::Write as _;

use super::{
    parser_errors,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::parsing::errors::ParserErrorType,
};

#[test]
fn alignof_aliases_accept_unary_operands_and_preserve_precedence() {
    use super::super::syntax::{
        Expression,
        ExpressionType,
    };

    for spelling in ["__alignof__", "__alignof"] {
        for operand in [
            "x", "*p", "++x", "sizeof x", "x++", "(x + 1)", "(int)", "(int){1}",
        ] {
            for gnu in [false, true] {
                for policy in [
                    ExtensionPolicy::Allow,
                    ExtensionPolicy::Warn,
                    ExtensionPolicy::Deny,
                ] {
                    let source =
                        format!("int f(int x, int *p) {{ return {spelling} {operand} + 1; }}\n");
                    with_parse_configuration(
                        &source,
                        CompilerConfiguration::new(CStandard::C17, policy).with_gnu_extensions(gnu),
                        |p| {
                            assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                            let expression = p
                                .parser
                                .syntax
                                .iter::<Expression<'_>>()
                                .find(|x| {
                                    matches!(
                                        x.kind,
                                        ExpressionType::AlignofExpr(_)
                                            | ExpressionType::AlignofType(_)
                                    )
                                })
                                .expect("alignment expression");
                            assert!(!expression.recovered, "{source}");
                            assert_eq!(
                                super::sourced_text(
                                    p,
                                    expression
                                        .operator_source_vectors
                                        .expect("alignment keyword source")
                                ),
                                spelling,
                                "{source}"
                            );
                            assert!(p.parser.syntax.iter::<Expression<'_>>().any(|x| {
                                matches!(x.kind, ExpressionType::Binary { left_expression, .. } if std::ptr::eq(left_expression, expression))
                            }), "alignment must leave the addition outside its operand: {source}");
                            assert_eq!(
                                p.errors.len(),
                                usize::from(policy != ExtensionPolicy::Allow),
                                "{:?}",
                                p.errors
                            );
                            assert!(
                                p.errors
                                    .iter()
                                    .all(|x| x.to_string().contains("GNU extension")),
                                "{:?}",
                                p.errors
                            );
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn reserved_asm_alias_remains_gnu_with_or_without_msvc_assembly() {
    let source = "__asm(\"file\"); int label __asm(\"external\"); int f(void){ __asm(\"nop\"); \
                  __asm __volatile__ __inline__ (\"nop\"); __asm goto(\"\" : : : : L); L: return \
                  0; }\n";
    for standard in [CStandard::C89, CStandard::C17, CStandard::C23] {
        for gnu in [false, true] {
            for msvc in [false, true] {
                for policy in [
                    ExtensionPolicy::Allow,
                    ExtensionPolicy::Warn,
                    ExtensionPolicy::Deny,
                ] {
                    with_parse_configuration(
                        source,
                        CompilerConfiguration::new(standard, policy)
                            .with_gnu_extensions(gnu)
                            .with_msvc_feature(crate::configuration::MsvcFeature::Asm, msvc),
                        |p| {
                            assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                            assert_eq!(
                                p.parser.syntax.iter::<super::super::gnu::Asm<'_>>().count(),
                                5
                            );
                            assert_eq!(
                                p.parser
                                    .syntax
                                    .iter::<super::super::msvc::MsAsm<'_>>()
                                    .count(),
                                0
                            );
                            let extensions = super::extensions(p);
                            if policy == ExtensionPolicy::Allow {
                                assert_eq!(extensions, Vec::<String>::new());
                            } else {
                                assert_eq!(
                                    extensions
                                        .iter()
                                        .filter(|x| *x == "'__asm' is a GNU extension")
                                        .count(),
                                    5
                                );
                                assert!(extensions.iter().all(|x| x.contains("GNU extension")));
                            }
                        },
                    );
                }
            }
        }
    }
}
#[test]
fn gnu_surface_smoke() {
    let samples = [
        "int f(__attribute__((unused))); __extension__ _Static_assert(1,\"ok\");",
        "void f(int x) { x = (__attribute__((unused)) float){2.}; }",
        "__attribute__((unused)) int x; int * __attribute__((aligned(8))) p;",
        "struct __attribute__((packed)) S { int x __attribute__((unused)); }; enum E { A \
         __attribute__((unused)) };",
        "__asm__(\"nop\"); int x __asm__(\"other\") __attribute__((used));",
        "int f(int x) { __asm__ __volatile__ __inline__ (\"nop\" : [out] \"=r\"(x) : \"r\"(x) : \
         \"memory\"); __asm__ goto (\"\" : : : : L); L: return x; }",
        "__typeof__(int) x; __typeof__(x) y; __int128 a; unsigned __int128 b; __int128 unsigned \
         c; __auto_type d=1; struct Empty {};",
        "int f(void) { __label__ L, M; void *p=&&L; goto *p; L: M: return ({ int x=1; x; }) ?: 2; \
         }",
        "int f(void) { int nested(int x) { return x; } return nested(1); }",
        "int f(void) { int nested(x) int x; { return x; } return nested(1); }",
        "int f(void) { return __builtin_va_arg(ap, int) + __builtin_offsetof(struct S, a[1].b) + \
         __builtin_types_compatible_p(int, long) + __builtin_choose_expr(1,2,3); }",
        "union U { int i; }; int f(void) { union U x=(union U)1; return __real__ x.i + __imag__ \
         x.i; }",
        "__extension__ __int128 x; int f(void) { __extension__ ({ int x=0; x; }); return \
         __alignof__(x); }",
        "int a[4]={[1 ... 3]=2}; struct S {int x;}; struct S s={x: 1}; int b[3]={[1] 2};",
        "int a[(0)]; int f(int x) { switch(x) { case 1 ... 3: return 1; } return 0; }",
    ];
    for source in samples {
        with_parse_configuration(
            &format!("{source}\n"),
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{source}\n{:?}", p.errors);
            },
        );
    }
}

#[test]
fn reserved_gnu_syntax_keeps_ast_under_every_policy_and_mode() {
    let source = "__attribute__((used)) __int128 x[0]; __typeof__(x) y; __auto_type a=1; struct \
                  Empty {}; int f(void) { __label__ L; __asm__(\"nop\"); void *p=&&L; goto *p; L: \
                  switch(1){case 1 ... 3:break;} int nested(int n){return n;} return \
                  __builtin_choose_expr(1, ({1;}), 2) ?: 3; }\n";
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
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                with_parse_configuration(
                    source,
                    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu),
                    |p| {
                        assert_eq!(
                            parser_errors(p).count(),
                            0,
                            "{standard:?} {gnu} {policy:?}: {:?}",
                            p.errors
                        );
                        assert_eq!(p.items.len(), 5);
                        let extensions = super::extensions(p);
                        if policy == ExtensionPolicy::Allow {
                            assert_eq!(extensions, Vec::<String>::new());
                        } else {
                            for spelling in [
                                "__attribute__",
                                "__int128",
                                "zero-length array",
                                "__typeof__",
                                "__auto_type",
                                "empty struct",
                                "__label__",
                                "__asm__",
                                "label address",
                                "computed goto",
                                "nested function",
                                "__builtin_choose_expr",
                                "statement expression",
                                "omitted conditional",
                            ] {
                                assert!(
                                    extensions.iter().any(|x| x.contains(spelling)),
                                    "{spelling}: {extensions:?}"
                                );
                            }
                            assert!(super::extension_severities(p).iter().all(
                                |severity| *severity
                                    == if policy == ExtensionPolicy::Deny {
                                        crate::translation_phases::ErrorSeverity::Error
                                    } else {
                                        crate::translation_phases::ErrorSeverity::Warning
                                    }
                            ));
                            assert_eq!(
                                extensions.iter().any(|x| x.contains("case range")),
                                standard < CStandard::C2y
                            );
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn non_reserved_spellings_are_gated_without_stealing_identifiers() {
    for standard in [CStandard::C89, CStandard::C99, CStandard::C17] {
        with_parse_configuration(
            "int asm; int typeof; int f(void){return asm+typeof;}\n",
            CompilerConfiguration::new(standard, ExtensionPolicy::Allow),
            |p| assert!(p.errors.is_empty(), "{:?}", p.errors),
        );
        with_parse_configuration(
            "asm(\"nop\"); typeof(int) x;\n",
            CompilerConfiguration::new(standard, ExtensionPolicy::Warn).with_gnu_extensions(true),
            |p| assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors),
        );
        for source in ["asm(\"nop\");\n", "typeof(int) x;\n"] {
            with_parse_configuration(
                source,
                CompilerConfiguration::new(standard, ExtensionPolicy::Warn),
                |p| {
                    assert!(
                        parser_errors(p).count() > 0,
                        "strict mode accepted {source}"
                    );
                },
            );
        }
    }
}

#[test]
fn attributes_cover_gcc_attachment_positions_and_share_standard_nodes() {
    let source = "__attribute__((unused)) int a, __attribute__((used)) b; int \
                  __attribute__((aligned(8))) c; struct __attribute__((packed)) S { int x \
                  __attribute__((unused)); int y:3 __attribute__((unused)); } \
                  __attribute__((aligned(8))); union __attribute__((transparent_union)) U {int \
                  x;}; enum __attribute__((packed)) E {A __attribute__((deprecated))=1} \
                  __attribute__((unused)); int (* __attribute__((unused)) p)(int \
                  __attribute__((unused)) arg); int (__attribute__((unused)) *q); void f(int \
                  a[__attribute__((unused)) const 3], int (__attribute__((unused)) *b)) { L: \
                  __attribute__((unused)); __attribute__((fallthrough)); }\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
        let attrs = p
            .parser
            .syntax
            .iter::<super::super::modern::AttributeSpecifier<'_>>()
            .collect::<Vec<_>>();
        assert_eq!(attrs.len(), 18);
        assert!(
            attrs
                .iter()
                .all(|x| x.syntax == super::super::modern::AttributeSyntax::Gnu && !x.recovered)
        );
        let aggregate = super::declaration(p, 2)
            .declaration_specifiers
            .type_specifiers;
        assert!(
            matches!(aggregate,super::super::declaration_syntax::TypeSpecifiers::StructOrUnion(x) if x.attributes.is_some())
        );
    });
}

#[test]
fn extension_marker_suppresses_only_its_operand_or_declaration() {
    let source = "__extension__ __int128 x[0]; __int128 y; int f(void){ __extension__ ({ __int128 \
                  z; z ?: 1; }); return ({2;}); }\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        |p| {
            assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
            let extensions = super::extensions(p);
            assert_eq!(extensions.len(), 2, "{extensions:?}");
            assert!(extensions[0].contains("__int128"));
            assert!(extensions[1].contains("statement expression"));
            assert_eq!(p.parser.pedantic_suppression, 0);
        },
    );
}

#[test]
fn malformed_gnu_prefixes_terminate_and_restore_machine_state() {
    for source in [
        "__attribute__((unused(1))) int x;",
        "__asm__ goto(\"\": [x] \"=r\"(1): \"r\"(2): \"memory\":L);",
        "int f(void){ __label__ L,M; goto *&&L; L: return ({int x=1;x;}) ?: 2; }",
        "int f(void){return __builtin_va_arg(ap,int (*)[3])+__builtin_offsetof(struct \
         S,a[1].b)+__builtin_types_compatible_p(int,long)+__builtin_choose_expr(1,2,3);}",
        "int a[4]={[1 ... 3]=2};",
        "__extension__ int f(void){int nested(int x){return x;} return 1;}",
        "int f(int x,int *p){return __extension__ __alignof__ *p + __alignof(x);}",
    ] {
        for end in 0..=source.len() {
            with_parse_configuration(
                &format!("{}\n", &source[..end]),
                CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
                |p| {
                    assert_eq!(p.parser.scopes.depth(), 0, "{source} at {end}");
                    assert!(p.parser.frames.is_empty());
                    assert!(p.parser.returned.is_none());
                    assert_eq!(p.parser.pedantic_suppression, 0, "{source} at {end}");
                    assert_eq!(p.parser.switch_floor, 0);
                },
            );
        }
    }
}

#[test]
fn ambiguous_asm_prefixes_restore_frames_and_scopes() {
    for msvc in [false, true] {
        for source in [
            "int f(void){__asm __volatile__(\"\" : \"r\"(1));}",
            "int f(void){__asm { mov eax, [ebx+(1)] }}",
        ] {
            for end in 0..=source.len() {
                with_parse_configuration(
                    &format!("{}\n", &source[..end]),
                    CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Deny)
                        .with_msvc_feature(crate::configuration::MsvcFeature::Asm, msvc),
                    |p| {
                        assert_eq!(p.parser.scopes.depth(), 0, "{source} at {end}");
                        assert!(p.parser.frames.is_empty());
                        assert!(p.parser.returned.is_none());
                    },
                );
            }
        }
    }
}

#[test]
fn malformed_gnu_children_preserve_following_declarations() {
    for source in [
        "__attribute__((unused);",
        "__asm__(123);",
        "int f(void){__asm__ goto(\"\": : : : );}",
        "int f(void){__asm__(\"\":\"r\"());}",
        "int f(void){return __builtin_va_arg(ap,);}",
        "int f(void){return __builtin_offsetof(int,);}",
        "int f(void){return __builtin_choose_expr(1,,3);}",
        "int f(void){goto *;}",
        "int f(void){__label__ ;}",
        "int f(void){return ({int x=; x;});}",
        "int a[4]={[1 ... ]=2};",
    ] {
        with_parse_configuration(
            &format!("{source} int after;\n"),
            CompilerConfiguration::default(),
            |p| {
                assert!(parser_errors(p).count() > 0, "{source}: {:?}", p.errors);
                let declaration = super::declaration(p, p.items.len() - 1);
                assert_eq!(
                    p.parser
                        .context
                        .string_cache
                        .at(declaration.init_declarators[0]
                            .declarator
                            .identifier()
                            .unwrap()
                            .name),
                    "after"
                );
            },
        );
    }
}

#[test]
fn nested_functions_have_separate_typedef_label_and_switch_contexts() {
    let source = "typedef int T; int f(void){ switch(1){default:; int nested(int T){default:; \
                  goto L; L: return T;} default:;} T x=1; return x; }\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        let errors: Vec<_> = parser_errors(p).collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(matches!(errors[0], ParserErrorType::DuplicateDefaultLabel));
        assert_eq!(p.parser.switch_scopes.len(), 0);
    });
}

#[test]
fn gnu_nodes_preserve_typed_children_and_inspection() {
    use super::super::{
        declaration_syntax::{
            Designator,
            DesignatorType,
        },
        gnu::{
            Asm,
            Builtin,
        },
        modern::SyntaxOperand,
    };
    let source = include_str!("../../../../tests/fixtures/diagnostics/language/gnu-parser.c");
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
        let assemblies: Vec<_> = p.parser.syntax.iter::<Asm<'_>>().collect();
        assert_eq!(assemblies.len(), 4);
        assert_eq!(assemblies[2].sections, 3);
        assert_eq!(assemblies[2].operands.len(), 2);
        assert!(assemblies[2].operands[0].output);
        assert!(!assemblies[2].operands[1].output);
        assert_eq!(
            p.parser
                .context
                .string_cache
                .at(assemblies[2].operands[0].name.unwrap().name),
            "out"
        );
        assert_eq!(assemblies[3].sections, 4);
        assert_eq!(assemblies[3].labels.len(), 1);
        let builtins: Vec<_> = p.parser.syntax.iter::<Builtin<'_>>().collect();
        assert_eq!(builtins.len(), 4);
        assert!(matches!(
            builtins[0].operands.as_slice(),
            [SyntaxOperand::Expression(_), SyntaxOperand::Type(_)]
        ));
        assert_eq!(builtins[1].members.len(), 1);
        assert!(
            builtins[2]
                .operands
                .iter()
                .all(|x| matches!(x, SyntaxOperand::Type(_)))
        );
        assert_eq!(builtins[3].operands.len(), 3);
        let designators: Vec<_> = p.parser.syntax.iter::<Designator<'_>>().collect();
        assert!(matches!(designators[0].kind, DesignatorType::Range(_)));
        assert!(matches!(designators[1].kind, DesignatorType::GnuField(_)));
    });
    super::with_parsed(source, |unit, context| {
        assert!(context.pop_pending_error().is_none());
        let output = unit.inspect(
            context.tu_arena(),
            context,
            super::super::InspectionOptions {
                show_locations: true,
            },
        );
        for text in [
            "__attribute__((...))",
            "__int128",
            "__auto_type",
            "asm sections=0",
            "asm sections=3",
            "asm sections=4",
            "symbolic-name out",
            "goto-label L",
            "local-label L",
            "nested-function",
            "label-address L",
            "computed-goto",
            "case-range",
            "cast",
            "old-field-designator x",
            "array-range-designator",
            "builtin __builtin_va_arg",
            "builtin __builtin_offsetof",
            "builtin __builtin_types_compatible_p",
            "builtin __builtin_choose_expr",
            "statement-expression",
            "omitted middle",
            "__real__",
            "__imag__",
        ] {
            assert!(output.contains(text), "missing {text}: {output}");
        }
    });
}

#[test]
fn gnu_nesting_and_builtin_expression_delimiters_use_frames() {
    let mut expression = "1".to_owned();
    for _ in 0..256 {
        expression = format!("({{ {expression}; }})");
    }
    let source = format!("int f(void){{return {expression};}}\n");
    with_parse_configuration(&source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    });
    let mut expression = "1".to_owned();
    for _ in 0..256 {
        expression = format!("__builtin_choose_expr(1,{expression},3)");
    }
    let source = format!("int f(void){{return {expression};}}\n");
    with_parse_configuration(&source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    });
    let source = "int f(int x){__asm__(\"\":\"=r\"((x,x)):\"r\"(x,x)); return \
                  __builtin_offsetof(struct S,a[1,2].b);}\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    });
}

#[test]
fn macro_extension_suppression_preserves_unsuppressed_occurrences() {
    let source = "#define GNU __int128\n__extension__ GNU a;\nGNU b;\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Warn),
        |p| {
            assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
            assert!(p.errors[0].to_string().contains("__int128"));
        },
    );
}

#[test]
fn alternate_keyword_spellings_reuse_iso_syntax_in_every_revision() {
    let source = "__inline __inline__ int f(__const __const__ __volatile __volatile__ __signed \
                  int * __restrict __restrict__ p){return __alignof(*p)+__alignof__(int); } \
                  __signed__ int x;\n";
    for standard in [CStandard::C89, CStandard::C99, CStandard::C23] {
        with_parse_configuration(
            source,
            CompilerConfiguration::new(standard, ExtensionPolicy::Warn)
                .with_repeated_specifier_warnings(false),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                assert!(p.errors.iter().any(|x| x.to_string().contains("__inline")));
            },
        );
    }
}

#[test]
fn gnu_designator_and_macro_nodes_retain_complete_provenance() {
    with_parse_configuration(
        "int a[4]={[1 ... 3]=2}; struct S{int x;}; struct S s={x:1};\n",
        CompilerConfiguration::default(),
        |p| {
            let nodes: Vec<_> = p
                .parser
                .syntax
                .iter::<super::super::declaration_syntax::Designator<'_>>()
                .collect();
            assert_eq!(super::sourced_text(p, nodes[0].source_vectors), "[1...3]");
            assert_eq!(super::sourced_text(p, nodes[1].source_vectors), "x:");
        },
    );
    let source = "#define GNU(expr) ({expr;})\nint f(void){return GNU(1);}\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let expression = p
            .parser
            .syntax
            .iter::<super::super::syntax::Expression<'_>>()
            .find(|x| {
                matches!(
                    x.kind,
                    super::super::syntax::ExpressionType::StatementExpression(_)
                )
            })
            .expect("macro statement expression survives");
        assert!(matches!(
            expression.kind,
            super::super::syntax::ExpressionType::StatementExpression(_)
        ));
        assert_eq!(super::sourced_text(p, expression.source_vectors), "({1;})");
    });
}

#[test]
fn extension_marked_nested_definition_suppresses_its_head_and_body_only() {
    let source = "int f(void){ __extension__ int nested(void){return ({1;});} return ({2;}); }\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Warn),
        |p| {
            assert_eq!(parser_errors(p).count(), 0);
            assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
            assert!(p.errors[0].to_string().contains("statement expression"));
        },
    );
}

#[test]
fn allocation_fixture_has_complete_gnu_syntax() {
    let source = "__attribute__((used)) unsigned __int128 wide[0]; __typeof__(wide) copy; \
                  __auto_type value=1; struct Empty {}; __asm__(\"nop\"); int f(void){__label__ \
                  L; int nested(int x){return x;} int a[4]={[1 ... 3]=2}; struct S{int x;}; \
                  struct S s={x:1}; __asm__ \
                  volatile(\"\":[out]\"=r\"(value):\"r\"(value):\"memory\"); __asm__ \
                  goto(\"\"::::L); void *p=&&L; goto *p; L: return __extension__ ({ \
                  __builtin_va_arg(ap,int)+__builtin_offsetof(struct \
                  S,x)+__builtin_types_compatible_p(int,long)+__builtin_choose_expr(1,__real__ \
                  value,__imag__ value); }) ?: 2;}\n";

    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    });
}

#[test]
fn omitted_conditional_retains_written_source_without_duplicating_condition() {
    with_parse_configuration(
        "int f(int a){return a ?: 2;}\n",
        CompilerConfiguration::default(),
        |p| {
            let expression = p
                .parser
                .syntax
                .iter::<super::super::syntax::Expression<'_>>()
                .find(|x| {
                    matches!(
                        x.kind,
                        super::super::syntax::ExpressionType::OmittedConditional(_)
                    )
                })
                .unwrap();
            assert_eq!(super::sourced_text(p, expression.source_vectors), "a?:2");
        },
    );
}

#[test]
fn label_attributes_before_extension_expressions_preserve_statement_ownership() {
    with_parse_configuration(
        "int f(void){ L: __attribute__((unused)) __extension__ ({1;}); return 0; }\n",
        CompilerConfiguration::default(),
        |p| {
            assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
            assert!(
                p.parser
                    .syntax
                    .iter::<super::super::syntax::Expression<'_>>()
                    .any(|x| matches!(
                        x.kind,
                        super::super::syntax::ExpressionType::StatementExpression(_)
                    ))
            );
        },
    );
}

#[test]
fn imaginary_integer_components_survive_parsing_and_inspection() {
    use crate::translation_phases::{
        parsing::syntax::{
            Constant,
            Expression,
            ExpressionType,
        },
        preprocessing::IntegerTokenType,
    };

    let source = "double _Complex a[]={1i,2Li,3LLj,4ui,5ULj,6ULLi,7.0fi,8.0j,9.0Li};\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let integers: Vec<_> = p
            .parser
            .syntax
            .iter::<Expression<'_>>()
            .filter_map(|expression| match expression.kind {
                | ExpressionType::Constant(Constant::Integer(IntegerTokenType::Imaginary(
                    value,
                    kind,
                ))) => Some((value.get(), kind.type_name())),
                | _ => None,
            })
            .collect();
        assert_eq!(
            integers,
            [
                (1, "int _Complex"),
                (2, "long _Complex"),
                (3, "long long _Complex"),
                (4, "unsigned int _Complex"),
                (5, "unsigned long _Complex"),
                (6, "unsigned long long _Complex")
            ]
        );
    });
    super::with_parsed(source, |unit, context| {
        assert!(context.pop_pending_error().is_none());
        let output = unit.inspect(
            context.tu_arena(),
            context,
            super::super::InspectionOptions::default(),
        );
        for text in [
            "1i (int _Complex)",
            "2i (long _Complex)",
            "3i (long long _Complex)",
            "4i (unsigned int _Complex)",
            "5i (unsigned long _Complex)",
            "6i (unsigned long long _Complex)",
            "7i (float _Complex)",
            "8i (double _Complex)",
            "0x1.2p+3i (long double _Complex)",
        ] {
            assert!(output.contains(text), "{text}: {output}");
        }
    });
}

#[test]
fn many_extension_markers_preserve_occurrence_order_and_later_diagnostics() {
    let mut source = String::from("#define WIDE __int128\n");
    for index in 0..4_096 {
        writeln!(source, "__extension__ WIDE suppressed{index};").expect("write to String");
    }
    source.push_str("WIDE unsuppressed;\n__extension__ WIDE another;\nWIDE last;\n");
    with_parse_configuration(
        &source,
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Warn),
        |p| {
            assert_eq!(p.items.len(), 4_099);
            assert_eq!(p.errors.len(), 2, "{:?}", p.errors);
            assert!(
                p.errors
                    .iter()
                    .all(|error| error.to_string().contains("__int128"))
            );
        },
    );
}

#[test]
fn extension_markers_suppress_numeric_diagnostics_per_macro_occurrence() {
    for (standard, literal) in [
        (CStandard::C89, "1LL"),
        (CStandard::C89, "0x1p0"),
        (CStandard::C17, "0b1"),
        (CStandard::C17, "1i"),
    ] {
        for policy in [ExtensionPolicy::Warn, ExtensionPolicy::Deny] {
            let source = format!(
                "#define VALUE {literal}\n__extension__ double silent = VALUE;\ndouble loud = \
                 VALUE;\n__extension__ double another = VALUE;\ndouble last = VALUE;\n"
            );
            with_parse_configuration(
                &source,
                CompilerConfiguration::new(standard, policy).with_gnu_extensions(true),
                |p| {
                    assert_eq!(p.items.len(), 4);
                    assert_eq!(p.errors.len(), 2, "{literal}: {:?}", p.errors);
                    assert!(p.errors.iter().all(|e| matches!(
                        e,
                        crate::translation_phases::TranslationError::Extension(_)
                    )));
                },
            );
        }
    }
}

#[test]
fn numeric_suppression_preserves_preprocessing_occurrences_and_invocation_sites() {
    let source =
        "#define VALUE 0b1\n#if VALUE\n#endif\n__extension__ int silent=VALUE;\nint loud=VALUE;\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Warn).with_gnu_extensions(true),
        |p| {
            assert_eq!(p.errors.len(), 2, "{:?}", p.errors);
            // The parser discards completed macro-location hint metadata,
            // but its occurrence index keeps stable invocation keys.
            let remaining: Vec<_> = p
                .parser
                .token_diagnostics
                .iter()
                .filter(|(_, occurrences)| !occurrences.is_empty())
                .map(|(key, occurrences)| (key.2.as_ref().unwrap().line, occurrences.len()))
                .collect();
            assert_eq!(remaining, [(1, 1)]);
        },
    );
}

/// Names declared by clean (unrecovered) top-level declarations.
fn clean_declared_names(p: &super::Parsed<'_, '_>) -> Vec<String> {
    p.items
        .iter()
        .filter_map(|x| match x {
            | super::super::syntax::ExternalDeclaration::Declaration(x) => Some(x),
            | _ => None,
        })
        .flat_map(|x| x.init_declarators.iter())
        .filter_map(|x| super::identifier_name(p, x.declarator))
        .collect()
}

#[test]
fn unclosed_attribute_arguments_leave_semicolons_and_braces_to_the_declaration() {
    for source in [
        "int x __attribute__((aligned(8; int after; int more;",
        "int x __attribute__((aligned((8; int after; int more;",
        "[[gnu::aligned(8; int after; int more;",
        "[[gnu::aligned([8; int after; int more;",
        "__declspec(align(8 ; int after; int more;",
        "void f(void) { int x __attribute__((aligned(8) } int after; int more;",
        "void f(void) __attribute__((noinline(1 { return; } int after; int more;",
        "void f(void) { [[vendor::tag(1)] } int after; int more;",
    ] {
        with_parse_configuration(
            &format!("{source}\n"),
            CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_msvc_extensions(true),
            |p| {
                // The attribute reports its missing closer at the boundary
                // it leaves to its owner, and nothing reaches end of input.
                let errors: Vec<_> = parser_errors(p).collect();
                assert!(
                    matches!(
                        errors.first(),
                        Some(
                            ParserErrorType::ExpectedGnuSyntax(closer, Some(_))
                                | ParserErrorType::ExpectedIsoSyntax(closer, Some(_))
                                | ParserErrorType::ExpectedMsSyntax(closer, Some(_))
                        ) if closer.contains(" in ")
                    ),
                    "{source}: {errors:?}"
                );
                assert!(errors.len() <= 2, "{source}: {errors:?}");
                let names = clean_declared_names(p);
                assert!(
                    names.ends_with(&["after".to_owned(), "more".to_owned()]),
                    "{source}: {names:?}"
                );
            },
        );
    }
}

#[test]
fn attribute_missing_its_second_closer_finishes_before_the_declaration() {
    for source in [
        "__attribute__((noreturn) void die(void);\nint following;",
        "[[noreturn] void die(void);\nint following;",
    ] {
        with_parse_configuration(
            &format!("{source}\n"),
            CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
            |p| {
                assert_eq!(parser_errors(p).count(), 1, "{source}: {:?}", p.errors);
                let names = clean_declared_names(p);
                assert_eq!(names.last().map(String::as_str), Some("following"));
                assert!(
                    p.items.iter().any(|x| match x {
                        | super::super::syntax::ExternalDeclaration::Declaration(x)
                        | super::super::syntax::ExternalDeclaration::RecoveredDeclaration(x) =>
                            x.init_declarators.iter().any(|x| {
                                super::identifier_name(p, x.declarator).as_deref() == Some("die")
                            }),
                        | _ => false,
                    }),
                    "{source}: {:?}",
                    p.items
                );
            },
        );
    }
}

#[test]
fn leading_attributes_do_not_turn_a_grouping_parenthesis_into_parameters() {
    use super::super::declaration_syntax::DirectDeclarator;
    let parameter_shape = |source: &str| {
        with_parse_configuration(
            &format!("{source}\n"),
            CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Warn)
                .with_gnu_extensions(true),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                assert!(
                    !p.errors
                        .iter()
                        .any(|x| x.to_string().contains("implicit int")),
                    "{source}: {:?}",
                    p.errors
                );
                let DirectDeclarator::Function { parameter_list, .. } =
                    super::declaration(p, 0).init_declarators[0].declarator.kind[1]
                else {
                    panic!("{source}: expected a function declarator");
                };
                let declarator = parameter_list[0].declarator.expect("parameter declarator");
                declarator
                    .kind
                    .iter()
                    .map(|x| match x {
                        | DirectDeclarator::Parenthesized(x) => format!(
                            "grouped({} {:?})",
                            x.declarator.pointer.levels.len(),
                            super::identifier_name(p, x.declarator)
                        ),
                        | DirectDeclarator::Function { .. } => "function".to_owned(),
                        | other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
            },
        )
    };
    assert_eq!(
        parameter_shape("void f(int (__attribute__((unused)) *b));"),
        ["grouped(1 Some(\"b\"))"]
    );
    assert_eq!(
        parameter_shape("void f(int (__attribute__((unused)) __attribute__((x)) *));"),
        ["grouped(1 None)"]
    );
    assert_eq!(
        parameter_shape("void f(int (__attribute__((unused)) int x));"),
        ["function"]
    );
    assert_eq!(
        parameter_shape("void f(int ([[maybe_unused]] int x));"),
        ["function"]
    );
}

#[test]
fn omitted_designation_equals_requires_a_single_array_designator() {
    for standard in [CStandard::C17, CStandard::C23] {
        for gnu in [false, true] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                for source in [
                    "struct S { int x; }; struct S s = { .x 1 }; int after;\n",
                    "struct S { int x; }; struct S a[1] = { [0].x 1 }; int after;\n",
                    "struct S { int x[1]; }; struct S s = { .x[0] 1 }; int after;\n",
                    "int a[1][1] = { [0][0] 1 }; int after;\n",
                ] {
                    with_parse_configuration(
                        source,
                        CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu),
                        |p| {
                            assert!(
                                matches!(
                                    parser_errors(p).collect::<Vec<_>>().as_slice(),
                                    [ParserErrorType::ExpectedEqualsAfterInitializerDesignation(
                                        _
                                    )]
                                ),
                                "{source}: {:?}",
                                p.errors
                            );
                            let designation = p
                                .parser
                                .syntax
                                .iter::<super::super::declaration_syntax::Designation<'_>>()
                                .next()
                                .expect("recovered designation");
                            assert!(designation.recovered);
                            assert!(designation.equals_source_vectors.is_none());
                            assert!(clean_declared_names(p).iter().any(|name| name == "after"));
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn gnu_designation_forms_preserve_syntax_and_extension_policy() {
    for (source, designator_source, extension_count) in [
        ("int a[1] = { [0] 1 };\n", "[0]", 1),
        ("int a[2] = { [0 ... 1] 1 };\n", "[0...1]", 2),
        ("struct S { int x; }; struct S s = { x: 1 };\n", "x:", 1),
        ("struct S { int x; }; struct S s = { .x = 1 };\n", ".x", 0),
    ] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, policy).with_gnu_extensions(true),
                |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                    assert_eq!(
                        p.errors.len(),
                        if policy == ExtensionPolicy::Allow {
                            0
                        } else {
                            extension_count
                        }
                    );
                    let designation = p
                        .parser
                        .syntax
                        .iter::<super::super::declaration_syntax::Designation<'_>>()
                        .next()
                        .expect("designation");
                    assert!(!designation.recovered);
                    assert_eq!(designation.designators.len(), 1);
                    assert_eq!(
                        super::sourced_text(p, designation.designators[0].source_vectors),
                        designator_source
                    );
                    assert_eq!(
                        designation.equals_source_vectors.is_some(),
                        extension_count == 0
                    );
                },
            );
        }
    }
}

#[test]
fn old_field_designator_keeps_nested_statement_expression_values() {
    use super::super::{
        declaration_syntax::{
            Designation,
            DesignatorType,
        },
        syntax::{
            BlockItem,
            Expression,
            ExpressionType,
            StatementType,
        },
    };

    for value in [
        "({int t=0; if(t) t=1; t;})",
        "{({int t=0; if(t) t=1; t;})}",
        "({int t=0; while(t) {break;} t;})",
        "({int t=0; for(;t;) {continue;} t;})",
        "({int t=0; switch(t) {case 1: break; default: break;} t;})",
    ] {
        let source = format!(
            "struct S {{ int x; }}; void f(void) {{ struct S s = {{ x: {value} }}; int after; }} \
             int tail;\n"
        );
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(
                &source,
                CompilerConfiguration::new(CStandard::C17, policy).with_gnu_extensions(true),
                |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                    assert_eq!(
                        p.errors.len(),
                        usize::from(policy != ExtensionPolicy::Allow) * 2
                    );
                    let designation = p.parser.syntax.iter::<Designation<'_>>().next().unwrap();
                    assert!(!designation.recovered);
                    assert!(matches!(
                        designation.designators[0].kind,
                        DesignatorType::GnuField(_)
                    ));
                    assert_eq!(super::sourced_text(p, designation.source_vectors), "x:");
                    let expression = p
                        .parser
                        .syntax
                        .iter::<Expression<'_>>()
                        .find(|x| matches!(x.kind, ExpressionType::StatementExpression(_)))
                        .expect("statement expression remains an initializer value");
                    assert!(!expression.recovered);
                    let function = super::function_definition(p, 1);
                    assert!(!function.recovered);
                    let StatementType::Compound { items } = function.body.kind else {
                        panic!("expected compound function body");
                    };
                    assert_eq!(
                        items.len(),
                        2,
                        "initializer contents stay in the declaration"
                    );
                    let BlockItem::Declaration(after) = items[1] else {
                        panic!("expected following declaration");
                    };
                    assert_eq!(
                        super::identifier_name(p, after.init_declarators[0].declarator).as_deref(),
                        Some("after")
                    );
                    assert!(clean_declared_names(p).iter().any(|name| name == "tail"));
                },
            );
        }
    }
}

#[test]
fn label_after_an_unclosed_initializer_list_stays_a_label() {
    for (source, labels) in [
        (
            "void f(void) {\n int x;\n int a[] = { 1, 2,\n out: x = 0;\n return;\n}\nint g;\n",
            1,
        ),
        (
            "void f(void) {\n int x;\n int a[] = { 1,\n out: { x = 0; }\n return;\n}\nint g;\n",
            1,
        ),
        // A complete GNU designator element keeps its meaning.
        (
            "struct S { int x, y; }; void f(void) { struct S s = { x: 1, y: (2) }; struct S t = { \
             x: {1}\n}; }\nint g;\n",
            0,
        ),
    ] {
        with_parse_configuration(
            source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
            |p| {
                let found = p
                    .parser
                    .syntax
                    .iter::<super::super::syntax::Statement<'_>>()
                    .filter(|x| matches!(x.kind, super::super::syntax::StatementType::Label(..)))
                    .count();
                assert_eq!(found, labels, "{source}: {:?}", p.errors);
                // The missing `}` and `;` are both reported at the label.
                assert!(
                    parser_errors(p).count() <= 2 * labels,
                    "{source}: {:?}",
                    p.errors
                );
                assert!(
                    super::identifier_name(
                        p,
                        super::declaration(p, p.items.len() - 1).init_declarators[0].declarator
                    )
                    .is_some_and(|x| x == "g")
                );
            },
        );
    }
}

#[test]
fn gnu_asm_keeps_template_qualifiers_and_clobbers_as_typed_fields() {
    use super::super::gnu::{
        Asm,
        AsmQualifiers,
    };
    let source = "int f(int x) { __asm__ volatile goto (\"jmp %l0\" : : \"r\"(x) : \"memory\", \
                  \"cc\" : L); __asm__ __inline__ (\"nop\"); L: return x; }\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let assemblies: Vec<_> = p.parser.syntax.iter::<Asm<'_>>().collect();
            assert_eq!(assemblies.len(), 2);
            let text = |token: Option<crate::translation_phases::preprocessing::Token>| {
                super::sourced_text(p, token.expect("token").source_vectors)
            };
            assert_eq!(
                assemblies[0].qualifiers,
                AsmQualifiers {
                    volatile: true,
                    inline:   false,
                    goto:     true,
                }
            );
            assert_eq!(text(assemblies[0].template), "\"jmp %l0\"");
            assert_eq!(assemblies[0].operands.len(), 1);
            let clobbers: Vec<_> = assemblies[0]
                .clobbers
                .iter()
                .map(|x| text(Some(*x)))
                .collect();
            assert_eq!(clobbers, ["\"memory\"", "\"cc\""]);
            assert_eq!(assemblies[0].labels.len(), 1);
            assert_eq!(
                assemblies[1].qualifiers,
                AsmQualifiers {
                    volatile: false,
                    inline:   true,
                    goto:     false,
                }
            );
            assert_eq!(text(assemblies[1].template), "\"nop\"");
            assert!(assemblies[1].clobbers.is_empty());
        },
    );
}
