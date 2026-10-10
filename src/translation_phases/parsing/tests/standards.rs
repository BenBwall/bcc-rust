//! Mode-specific ISO syntax, policy, provenance, and recovery regressions.

use super::{
    block_items,
    declaration,
    extensions,
    function_definition,
    parser_errors,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        ErrorSeverity,
        GetSeverity,
        parsing::{
            declaration_syntax::{
                DirectDeclarator,
                TypeSpecifiers,
            },
            modern::{
                ExtendedType,
                SyntaxOperand,
            },
            syntax::{
                BlockItem,
                ExpressionSlot,
                ExpressionType,
                StatementType,
            },
        },
    },
};

fn with_parse_configuration<R>(
    source: &str,
    configuration: CompilerConfiguration,
    inspect: impl FnOnce(&super::Parsed<'_, '_>) -> R,
) -> R {
    super::with_parse_configuration(&format!("{source}\n"), configuration, inspect)
}
fn mode(standard: CStandard, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, policy)
}
fn clean(source: &str, standard: CStandard) {
    with_parse_configuration(source, mode(standard, ExtensionPolicy::Warn), |p| {
        assert!(p.errors.is_empty(), "{source}\n{:?}", p.errors);
    });
}

#[test]
fn c99_constructs_follow_policy_in_every_pre_c99_mode() {
    let source = "long long wide; enum E { A, }; struct S { int n; int tail[]; }; int f(void) { \
                  int x=0; x++; int y=1; for(int i=0;i<2;i++) x+=i; struct S s={.n=1}; return \
                  ((struct S){.n=y}).n; }";
    for standard in [CStandard::C89, CStandard::C95] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(source, mode(standard, policy), |p| {
                assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                let ext = extensions(p);
                if policy == ExtensionPolicy::Allow {
                    assert_eq!(ext, Vec::<String>::new());
                } else {
                    for feature in [
                        "long long",
                        "trailing enum comma",
                        "flexible array member",
                        "mixed declarations and code",
                        "for declaration",
                        "designated initializer",
                        "compound literal",
                    ] {
                        assert!(
                            ext.iter().any(|x| x.contains(feature)),
                            "{feature}: {ext:?}"
                        );
                    }
                    for error in &p.errors {
                        assert_eq!(
                            error.severity(),
                            if policy == ExtensionPolicy::Deny {
                                ErrorSeverity::Error
                            } else {
                                ErrorSeverity::Warning
                            }
                        );
                    }
                }
                assert_eq!(p.items.len(), 4);
            });
        }
    }
    clean(source, CStandard::C99);
}

#[test]
fn implicit_int_is_native_in_c90_and_a_policy_extension_afterward() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C23,
    ] {
        with_parse_configuration(
            "extern object; f(void) { return 0; }",
            mode(standard, ExtensionPolicy::Warn),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                assert_eq!(
                    extensions(p).len(),
                    if standard < CStandard::C99 { 0 } else { 2 }
                );
                assert_eq!(
                    declaration(p, 0).declaration_specifiers.type_specifiers,
                    TypeSpecifiers::Int
                );
            },
        );
    }
}

#[test]
fn c11_and_c17_syntax_and_reserved_older_spellings() {
    let source = "_Thread_local static int t; _Alignas(int) _Alignas(16) int x; _Atomic(int *) a; \
                  const _Atomic int b; _Noreturn void f(void) { _Static_assert(1,\"yes\"); \
                  _Generic(x,int:x++,default:x--); } struct S { _Static_assert(1,\"member\"); \
                  union { int a; float b; }; }; int g(void) { return _Alignof(int *); }";
    for standard in [CStandard::C11, CStandard::C17] {
        clean(source, standard);
    }
    for standard in [CStandard::C89, CStandard::C99] {
        with_parse_configuration(source, mode(standard, ExtensionPolicy::Warn), |p| {
            assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
            for feature in [
                "_Thread_local",
                "_Alignas",
                "_Atomic",
                "_Noreturn",
                "_Static_assert",
                "_Generic",
                "anonymous struct or union member",
                "_Alignof",
            ] {
                assert!(
                    extensions(p).iter().any(|x| x.contains(feature)),
                    "{feature}: {:?}",
                    p.errors
                );
            }
        });
    }
}

#[test]
fn c23_types_constants_initializers_and_compound_literal_storage() {
    clean(
        "constexpr bool yes=true; bool no=false; typeof(yes) a; typeof_unqual(yes) b; typeof(int \
         *) p; unsigned _BitInt(17) bits; _BitInt(9) signed more; _Decimal32 d; _Decimal64 e; \
         _Decimal128 f; auto inferred=1; static_assert(1); thread_local int tls; alignas(8) int \
         aligned; int empty[2]={}; int g(int) { start: int v=alignof(int); void *p=nullptr; int \
         n=(static int){1}; return v+n; end: } enum E : unsigned int { A, B=2 };",
        CStandard::C23,
    );
}

#[test]
fn c23_nodes_preserve_types_and_generic_children() {
    with_parse_configuration(
        "_Atomic(int *) a; typeof_unqual(1+2) b; unsigned _BitInt(17) c; int f(void){return \
         _Generic(1,int:2,default:3);}",
        mode(CStandard::C23, ExtensionPolicy::Warn),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            assert!(matches!(
                declaration(p, 0).declaration_specifiers.type_specifiers,
                TypeSpecifiers::Extended(ExtendedType::Atomic(_))
            ));
            assert!(matches!(
                declaration(p, 1).declaration_specifiers.type_specifiers,
                TypeSpecifiers::Extended(ExtendedType::Typeof {
                    operand:     SyntaxOperand::Expression(_),
                    unqualified: true,
                })
            ));
            assert!(matches!(
                declaration(p, 2).declaration_specifiers.type_specifiers,
                TypeSpecifiers::Extended(ExtendedType::BitInt {
                    signedness: Some(false),
                    ..
                })
            ));
            let BlockItem::Statement(statement) = block_items(function_definition(p, 3).body)[0]
            else {
                panic!("return")
            };
            let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) = statement.kind
            else {
                panic!("return")
            };
            let ExpressionType::Generic(selection) = expression.kind else {
                panic!("generic")
            };
            assert_eq!(selection.associations.len(), 2);
            assert!(selection.associations[1].type_name.is_none());
        },
    );
}

#[test]
fn attributes_cover_iso_grammar_positions_and_balanced_vendor_arguments() {
    let source = "[[deprecated(\"old\")]] int object [[vendor::tag({[x](y)},\"a\")]]; struct \
                  [[vendor::record]] S { [[maybe_unused]] int member [[vendor::field]]; }; enum \
                  [[vendor::kind]] E { A [[deprecated]], B }; int f([[maybe_unused]] int x \
                  [[vendor::parameter]]) { [[likely]] if(x) return 1; return sizeof(int \
                  [[vendor::type]]); }";
    clean(source, CStandard::C23);
    with_parse_configuration(source, mode(CStandard::C17, ExtensionPolicy::Warn), |p| {
        assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
        assert!(
            extensions(p)
                .iter()
                .filter(|x| x.contains("[[...]]"))
                .count()
                >= 10
        );
    });
    with_parse_configuration(
        "int * [[vendor::pointer]] p;",
        mode(CStandard::C23, ExtensionPolicy::Warn),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let declarator = declaration(p, 0).init_declarators[0].declarator;
            assert!(declarator.pointer.levels[0].attributes.is_some());
            assert!(
                !declarator
                    .kind
                    .iter()
                    .any(|x| matches!(x, DirectDeclarator::Attributes(_)))
            );
        },
    );
}

#[test]
fn c2y_syntax_retains_selection_declarations_ranges_and_named_jumps() {
    let source = "int f(void) { int a[3]; int n=_Countof a + _Countof(int[4]); \
                  n+=_Generic(int,int:1,default:0); if(int x=1) n=x; else n=0; if(int x=2;x) n=x; \
                  switch(int x=2) {case 1 ... 3: n=x; break;} outer: for(;;) {continue outer; \
                  break outer;} return n; }";
    clean(source, CStandard::C2y);
    with_parse_configuration(
        source,
        mode(CStandard::C23, ExtensionPolicy::Warn).with_gnu_extensions(true),
        |p| {
            assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
            for feature in [
                "_Countof",
                "type-controlling _Generic",
                "selection declaration",
                "case range",
                "named loop control",
            ] {
                assert!(
                    extensions(p).iter().any(|x| x.contains(feature)),
                    "{feature}: {:?}",
                    p.errors
                );
            }
        },
    );
}

#[test]
fn case_ranges_require_native_or_gnu_support_and_preserve_following_input() {
    for (standard, gnu) in [
        (CStandard::C99, false),
        (CStandard::C17, false),
        (CStandard::C17, true),
        (CStandard::C2y, false),
    ] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            for suppression in ["", "__extension__ "] {
                let source = format!(
                    "{suppression}int f(int x) {{switch(x) {{case 1 ... 3: return 1; case 4: \
                     return 2; default: return 0;}}}} int following;"
                );
                let configuration = mode(standard, policy).with_gnu_extensions(gnu);
                with_parse_configuration(&source, configuration, |p| {
                    let accepted = standard == CStandard::C2y || gnu;
                    let errors: Vec<_> = parser_errors(p).collect();
                    if accepted {
                        assert!(errors.is_empty(), "{:?}", p.errors);
                    } else {
                        assert!(
                            matches!(
                                errors.as_slice(),
                                [super::super::errors::ParserErrorType::ExpectedIsoSyntax(
                                    "a single case value in this language mode",
                                    _
                                )]
                            ),
                            "{:?}",
                            p.errors
                        );
                        assert!(
                            p.errors
                                .iter()
                                .any(|e| e.severity() == ErrorSeverity::Error)
                        );
                    }
                    let reported = extensions(p);
                    if gnu && policy != ExtensionPolicy::Allow && suppression.is_empty() {
                        assert_eq!(reported, ["'case range' is a C2y extension"]);
                        let severity = if policy == ExtensionPolicy::Warn {
                            ErrorSeverity::Warning
                        } else {
                            ErrorSeverity::Error
                        };
                        assert!(p.errors.iter().any(|e| e.severity() == severity));
                    } else {
                        assert!(reported.is_empty(), "{reported:?}");
                    }
                    assert!(
                        p.parser
                            .syntax
                            .iter::<super::super::syntax::Statement<'_>>()
                            .any(|s| matches!(s.kind, StatementType::CaseRange(_)))
                    );
                    assert_eq!(p.items.len(), 2);
                    let following = declaration(p, 1);
                    assert!(!following.recovered);
                    assert_eq!(
                        super::identifier_name(p, following.init_declarators[0].declarator)
                            .as_deref(),
                        Some("following")
                    );
                    assert!(p.parser.frames.is_empty());
                    assert_eq!(p.parser.pedantic_suppression, 0);
                });
            }
        }
    }
}

#[test]
fn malformed_iso_constructs_preserve_following_declarations() {
    for source in [
        "_Static_assert(,\"x\"); int following;",
        "_Atomic() broken; int following;",
        "_Generic(1,int:,default:2); int following;",
        "[[vendor::bad(]] int broken; int following;",
        "enum E : { A }; int following;",
        "_BitInt() bits; int following;",
    ] {
        with_parse_configuration(source, mode(CStandard::C23, ExtensionPolicy::Warn), |p| {
            assert!(!p.errors.is_empty(), "{source}");
            assert!(p.items.iter().any(|x|matches!(x,super::ExternalDeclaration::Declaration(d) if d.init_declarators.iter().any(|x|x.declarator.identifier().is_some_and(|i|p.parser.context.string_cache.at(i.name)=="following")))),"{source}: {:?}",p.items);
        });
    }
}

#[test]
fn mode_edges_preserve_identifiers_and_only_diagnose_new_syntax() {
    clean(
        "int bool,true,false,nullptr,constexpr,typeof,typeof_unqual,static_assert,alignas,alignof,\
         thread_local;",
        CStandard::C17,
    );
    clean(
        "struct S { int (*pointer)[]; int *array[3]; };",
        CStandard::C89,
    );
    clean(
        "typeof((1,2)) x; [[maybe_unused]]; int f(void) [[vendor::function]] { [[maybe_unused]] \
         int x; [[vendor::statement]] x=1; return x; }",
        CStandard::C23,
    );
    for standard in [CStandard::C99, CStandard::C11, CStandard::C17] {
        with_parse_configuration(
            "int f(int) {return 0;} _Static_assert(1); int x={};",
            mode(standard, ExtensionPolicy::Warn),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                for feature in [
                    "unnamed parameter",
                    "static assertion without message",
                    "empty initializer",
                ] {
                    assert!(
                        extensions(p).iter().any(|x| x.contains(feature)),
                        "{feature}: {:?}",
                        p.errors
                    );
                }
            },
        );
    }
}

#[test]
fn modern_grammar_truncations_terminate_with_restored_state() {
    for source in [
        "int f(void){return _Generic(1,int:2,default:3);}",
        "_Atomic(int *) object;",
        "[[vendor::tag(([x]{y}))]] int object;",
        "enum E : unsigned int { A [[deprecated]]=1 };",
        "int f(void){if(int x=1;x) return x;}",
        "int f(void){switch(1){case 1 ... 3:break;} end:}",
        "int f(void){return (static struct { struct { int x; } inner; }){{1}}.inner.x;}",
        "int f(void){return _Countof (static struct { int x; }[1]){{1}};}",
    ] {
        for end in 0..=source.len() {
            with_parse_configuration(
                &source[..end],
                mode(CStandard::C2y, ExtensionPolicy::Warn),
                |p| {
                    assert_eq!(p.parser.scopes.depth(), 0, "{source:?} at {end}");
                    assert!(p.parser.frames.is_empty(), "{source:?} at {end}");
                    assert!(p.parser.returned.is_none(), "{source:?} at {end}");
                },
            );
        }
    }
}

#[test]
fn nested_iso_operands_and_balanced_attributes_do_not_recurse() {
    let nested = format!("{}int{} x;", "_Atomic(".repeat(256), ")".repeat(256));
    clean(&nested, CStandard::C11);
    let attributes = format!(
        "[[vendor::tag({}1{})]] int object;",
        "(".repeat(2_000),
        ")".repeat(2_000)
    );
    clean(&attributes, CStandard::C23);
}

#[test]
fn inspection_shows_every_iso_child_without_semantic_evaluation() {
    let source = "[[vendor::tag(1)]] _Alignas(16) _Atomic(int) object; enum E : unsigned int { A \
                  [[deprecated]] }; _Static_assert(0,\"later analysis\"); _Noreturn void f(int) { \
                  int x=alignof(int); x=_Generic(x,int:1,default:2); label: int y=(static int){}; \
                  if(int n=1;n) x=n; outer: for(;;){break outer;} switch(x){case 1 ... 3: break;} \
                  return; }\n";
    super::with_parsed_in(
        source,
        mode(CStandard::C2y, ExtensionPolicy::Warn),
        |unit, context| {
            let output = unit.inspect(
                context.tu_arena(),
                context,
                super::super::InspectionOptions::default(),
            );
            for text in [
                "attribute-specifier [[...]]",
                "alignment",
                "atomic-type",
                "underlying-type",
                "static-assert",
                "function-specifiers=_Noreturn",
                "alignof type",
                "generic-selection",
                "generic-association default",
                "labeled-declaration",
                "storage=static",
                "selection-declaration",
                "break outer",
                "case-range",
            ] {
                assert!(output.contains(text), "{text}: {output}");
            }
            assert!(!output.contains("missing"), "{output}");
            assert!(context.pop_pending_error().is_none());
        },
    );
}

#[test]
fn declaration_only_iso_specifiers_are_rejected_in_type_names_and_members() {
    for source in [
        "struct S { _Thread_local int member; }; int following;",
        "struct S { constexpr int member; }; int following;",
        "struct S { _Noreturn int member; }; int following;",
        "int x=sizeof(_Thread_local int); int following;",
        "int x=sizeof(_Noreturn int); int following;",
        "int x=sizeof(constexpr int); int following;",
        "enum E : unsigned int * { A }; int following;",
        "[[vendor::broken(a]b)]] int broken; int following;",
        "[[vendor::]] int broken; int following;",
        "[[first second]] int broken; int following;",
    ] {
        with_parse_configuration(source, mode(CStandard::C23, ExtensionPolicy::Warn), |p| {
            assert!(parser_errors(p).count() > 0, "{source}: {:?}", p.errors);
            assert!(p.items.iter().any(|x|matches!(x,super::ExternalDeclaration::Declaration(d) if d.init_declarators.iter().any(|x|x.declarator.identifier().is_some_and(|i|p.parser.context.string_cache.at(i.name)=="following")))),"{source}: {:?}",p.items);
        });
    }
}

#[test]
fn modern_reserved_spellings_remain_structured_under_every_policy() {
    let source = "_Alignas(16) _Atomic(int) object; _Thread_local int thread; _Noreturn void \
                  f(int) { int a[3]; int n=_Generic(a,int*:1,default:0); \
                  n+=_Alignof(int)+_Countof a; label: int x={}; return; } _Static_assert(1); \
                  [[vendor::tag(1)]] unsigned _BitInt(8) bits;";
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(source, mode(standard, policy), |p| {
                assert_eq!(
                    parser_errors(p).count(),
                    0,
                    "{standard:?}/{policy:?}: {:?}",
                    p.errors
                );
                assert_eq!(p.items.len(), 5, "{standard:?}/{policy:?}: {:?}", p.items);
                if policy == ExtensionPolicy::Allow || standard == CStandard::C2y {
                    assert!(extensions(p).is_empty(), "{:?}", p.errors);
                } else {
                    assert_ne!(extensions(p), Vec::<String>::new());
                    assert!(p.errors.iter().all(|e| e.severity()
                        == if policy == ExtensionPolicy::Deny {
                            ErrorSeverity::Error
                        } else {
                            ErrorSeverity::Warning
                        }));
                }
            });
        }
    }
}

#[test]
fn attributes_attach_to_prefix_parameters_abstract_declarators_and_labels() {
    clean(
        "int f([[vendor::parameter]] int x, int (* [[vendor::pointer]] cb)(int)) { \
         [[vendor::label]] label: [[vendor::statement]] x++; return sizeof(int [3] \
         [[vendor::array]]) + sizeof(int (* [[vendor::abstract]])(int)); }",
        CStandard::C23,
    );
}

#[test]
fn array_parameter_additions_follow_the_c99_syntax_policy() {
    let source = "void f(int a[static const 3], int b[*]);";
    clean(source, CStandard::C99);
    with_parse_configuration(source, mode(CStandard::C89, ExtensionPolicy::Warn), |p| {
        assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
        for text in [
            "qualified array parameter",
            "static array parameter",
            "variable length array marker",
        ] {
            assert!(
                extensions(p).iter().any(|x| x.contains(text)),
                "{text}: {:?}",
                p.errors
            );
        }
    });
}

#[test]
fn iso_nodes_retain_macro_provenance_after_compaction() {
    with_parse_configuration(
        "#define ATTR [[vendor::tag(42)]]\n#define TYPE _Atomic(int)\nATTR TYPE \
         object;\n_Static_assert(1,\"message\");",
        mode(CStandard::C23, ExtensionPolicy::Warn),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let specifiers = declaration(p, 0).declaration_specifiers;
            let super::super::modern::SpecifierExtensionKind::Attributes(attributes) =
                specifiers.extensions.unwrap().kind
            else {
                panic!("attributes")
            };
            let vectors = p
                .parser
                .context
                .get_source_vectors(attributes.source_vectors);
            assert_ne!(vectors, []);
            assert!(
                vectors
                    .iter()
                    .all(|v| v.line == 1 && v.source_file_index == 0)
            );
            let token = attributes
                .tokens
                .iter()
                .find(|t| {
                    matches!(
                        t.kind,
                        crate::translation_phases::preprocessing::TokenType::Integer(_)
                    )
                })
                .unwrap();
            assert_eq!(
                p.parser.context.get_source_vectors(token.source_vectors)[0].line,
                1
            );
            let TypeSpecifiers::Extended(ExtendedType::Atomic(type_name)) =
                specifiers.type_specifiers
            else {
                panic!("atomic")
            };
            assert!(
                p.parser
                    .context
                    .get_source_vectors(type_name.source_vectors)
                    .iter()
                    .all(|v| v.line == 2)
            );
            assert!(declaration(p, 1).assertion.unwrap().message.is_some());
        },
    );
}

#[test]
fn c23_standalone_labels_require_compound_block_positions() {
    clean(
        "int f(void) { first: second: [[vendor::tag]] int x; third: fourth: }",
        CStandard::C23,
    );
    for source in [
        "int f(void){if(1) label: int x;}",
        "int f(void){if(1) label:}",
        "int f(void){while(1) label: int x;}",
    ] {
        with_parse_configuration(source, mode(CStandard::C23, ExtensionPolicy::Warn), |p| {
            assert!(
                parser_errors(p).any(|e| matches!(
                    e,
                    super::super::errors::ParserErrorType::ExpectedIsoSyntax(
                        "a statement after a label outside a compound block",
                        _
                    )
                )),
                "{source}: {:?}",
                p.errors
            );
        });
    }
}

#[test]
fn selection_headers_retain_complete_provenance_in_enclosing_nodes() {
    for (statement_source, expected) in [
        ("if (1) ;", "if(1);"),
        ("if (int x = 1; x) ;", "if(intx=1;x);"),
        ("if (int x = 1) ;", "if(intx=1);"),
        ("switch (int x = 1; x + 2) ;", "switch(intx=1;x+2);"),
        ("switch (int x = 1) ;", "switch(intx=1);"),
    ] {
        for standard in [CStandard::C99, CStandard::C23, CStandard::C2y] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                let source = format!("void f(void) {{ {{ {statement_source} }} }}");
                with_parse_configuration(&source, mode(standard, policy), |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                    let definition = function_definition(p, 0);
                    let BlockItem::Statement(inner_block) = block_items(definition.body)[0] else {
                        panic!("expected an inner compound statement")
                    };
                    let BlockItem::Statement(statement) = block_items(inner_block)[0] else {
                        panic!("expected a selection statement")
                    };
                    for (vectors, expected) in [
                        (statement.source_vectors, expected.to_owned()),
                        (inner_block.source_vectors, format!("{{{expected}}}")),
                        (
                            definition.body.source_vectors,
                            format!("{{{{{expected}}}}}"),
                        ),
                        (
                            definition.source_vectors,
                            format!("voidf(void){{{{{expected}}}}}"),
                        ),
                    ] {
                        assert_eq!(
                            super::sourced_text(p, vectors),
                            expected,
                            "{standard:?}/{policy:?}: {source}"
                        );
                    }
                });
            }
        }
    }
}

#[test]
fn c2y_selection_declarations_restore_typedef_scope_and_validate_shape() {
    clean(
        "typedef int T; int f(void){ if(int T=1;T) T=2; else T=3; T after; switch(int T=1){case \
         1: T=2;break;} T again; return 0; }",
        CStandard::C2y,
    );
    clean(
        "int f(void){ if(int x=1,y=2;x+y) return x+y; return 0; }",
        CStandard::C2y,
    );
    for source in [
        "int f(void){if(int x) return 1;}",
        "int f(void){if(int x=1,y=2) return 1;}",
    ] {
        with_parse_configuration(source, mode(CStandard::C2y, ExtensionPolicy::Warn), |p| {
            assert!(
                parser_errors(p).any(|e| matches!(
                    e,
                    super::super::errors::ParserErrorType::ExpectedIsoSyntax(
                        "a single initialized declaration in selection header",
                        _
                    )
                )),
                "{source}: {:?}",
                p.errors
            );
        });
    }
}

#[test]
fn implicit_return_types_support_pointer_and_parenthesized_declarators() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C23,
    ] {
        with_parse_configuration(
            "*f(void){return 0;} (g)(void){return 0;}",
            mode(standard, ExtensionPolicy::Warn),
            |p| {
                assert_eq!(parser_errors(p).count(), 0, "{standard:?}: {:?}", p.errors);
                assert_eq!(p.items.len(), 2);
                assert_eq!(
                    extensions(p).len(),
                    if standard < CStandard::C99 { 0 } else { 2 }
                );
                assert_eq!(
                    function_definition(p, 0)
                        .declaration_specifiers
                        .type_specifiers,
                    TypeSpecifiers::Int
                );
            },
        );
    }
}

#[test]
fn c23_standalone_ellipsis_retains_variadic_prototypes_and_definitions() {
    for standard in [CStandard::C23, CStandard::C2y] {
        clean("int apply(int (...)); int (*pointer)(...);", standard);
    }
    for standard in [CStandard::C17, CStandard::C23, CStandard::C2y] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(
                "int f(...); int g(...){return 0;} int after;",
                mode(standard, policy),
                |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                    let prototype = declaration(p, 0).init_declarators[0].declarator;
                    let definition = function_definition(p, 1).declarator;
                    for declarator in [prototype, definition] {
                        assert!(matches!(
                            declarator.function_suffix(),
                            Some(DirectDeclarator::Function { parameter_list, is_variadic: true })
                                if parameter_list.is_empty()
                        ));
                    }
                    assert_eq!(
                        extensions(p).len(),
                        if standard == CStandard::C17 && policy != ExtensionPolicy::Allow {
                            2
                        } else {
                            0
                        }
                    );
                    assert_eq!(p.items.len(), 3);
                },
            );
        }
    }
}

#[test]
fn c23_removed_identifier_lists_diagnose_and_preserve_legacy_recovery() {
    for standard in [CStandard::C17, CStandard::C23, CStandard::C2y] {
        with_parse_configuration(
            "int f(a); int g(a) int a; {return a;} int after;",
            mode(standard, ExtensionPolicy::Allow),
            |p| {
                assert_eq!(
                    parser_errors(p).count(),
                    if standard == CStandard::C17 { 0 } else { 2 },
                    "{:?}",
                    p.errors
                );
                assert_eq!(p.items.len(), 3);
                assert_eq!(declaration(p, 0).recovered, standard != CStandard::C17);
                assert_eq!(
                    function_definition(p, 1).recovered,
                    standard != CStandard::C17
                );
                assert_eq!(p.parser.scopes.depth(), 0);
            },
        );
    }
}

#[test]
fn repeated_enum_underlying_types_diagnose_without_overwriting_the_first() {
    for standard in [CStandard::C17, CStandard::C23, CStandard::C2y] {
        with_parse_configuration(
            "enum E : int : unsigned {A}; int after;",
            mode(standard, ExtensionPolicy::Allow),
            |p| {
                assert_eq!(parser_errors(p).count(), 1, "{:?}", p.errors);
                assert!(declaration(p, 0).recovered);
                let TypeSpecifiers::Enum(enumeration) =
                    declaration(p, 0).declaration_specifiers.type_specifiers
                else {
                    panic!("expected enum");
                };
                assert_eq!(
                    enumeration
                        .underlying_type
                        .unwrap()
                        .declaration_specifiers
                        .type_specifiers,
                    TypeSpecifiers::Int
                );
                assert_eq!(p.items.len(), 2);
                assert!(!declaration(p, 1).recovered);
            },
        );
    }
}

#[test]
fn variadic_and_fixed_enum_recovery_terminates_at_every_prefix() {
    for source in [
        "int f(..., int x);",
        "enum E : int : unsigned : {A};",
        "int f(a) int a; {return a;}",
    ] {
        for end in 0..=source.len() {
            with_parse_configuration(
                &source[..end],
                mode(CStandard::C23, ExtensionPolicy::Deny),
                |p| {
                    assert_eq!(p.parser.scopes.depth(), 0, "{source} at {end}");
                    assert!(p.parser.frames.is_empty());
                    assert!(p.parser.returned.is_none());
                },
            );
        }
    }
}

#[test]
fn c23_empty_named_and_abstract_function_declarators_are_prototypes() {
    for standard in [CStandard::C17, CStandard::C23, CStandard::C2y] {
        with_parse_configuration(
            "int f(); int size=sizeof(int ());",
            mode(standard, ExtensionPolicy::Deny),
            |p| {
                assert!(p.errors.is_empty(), "{:?}", p.errors);
                let abstract_type = p
                    .parser
                    .syntax
                    .iter::<super::super::syntax::Expression<'_>>()
                    .find_map(|x| match x.kind {
                        | ExpressionType::SizeofType(type_name) => Some(type_name),
                        | _ => None,
                    })
                    .expect("sizeof abstract function type");
                let named = declaration(p, 0).init_declarators[0].declarator;
                let abstract_declarator = abstract_type.declarator.unwrap();
                for declarator in [named, abstract_declarator] {
                    let suffix = declarator
                        .kind
                        .iter()
                        .find(|x| {
                            matches!(
                                x,
                                DirectDeclarator::Function { .. }
                                    | DirectDeclarator::KAndRStyleFunction { .. }
                            )
                        })
                        .expect("function declarator");
                    if standard == CStandard::C17 {
                        assert!(
                            matches!(suffix, DirectDeclarator::KAndRStyleFunction { parameters } if parameters.is_empty())
                        );
                    } else {
                        assert!(
                            matches!(suffix, DirectDeclarator::Function { parameter_list, is_variadic: false } if parameter_list.is_empty())
                        );
                    }
                }
            },
        );
    }
}

#[test]
fn enum_colons_without_a_following_type_belong_to_the_enclosing_grammar() {
    for standard in [
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for source in [
            "enum E {A}; int f(enum E e){ return _Generic(e, enum E: 1, default: 0); }",
            "enum E {A}; int f(enum E e){ return _Generic(e, enum E : e, default: 0); }",
            "enum E {A}; struct S { enum E : 3; enum E named : 2; };",
        ] {
            with_parse_configuration(source, mode(standard, ExtensionPolicy::Deny), |p| {
                assert!(p.errors.is_empty(), "{standard:?} {source}\n{:?}", p.errors);
            });
        }
    }
    with_parse_configuration(
        "enum E : unsigned char {A}; int x;",
        mode(CStandard::C23, ExtensionPolicy::Deny),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let TypeSpecifiers::Enum(enumeration) =
                declaration(p, 0).declaration_specifiers.type_specifiers
            else {
                panic!("expected enum");
            };
            assert!(enumeration.underlying_type.is_some());
        },
    );
}

fn inspect(source: &str, configuration: CompilerConfiguration) -> String {
    super::syntax_tree(
        source,
        configuration,
        super::super::InspectionOptions::default(),
    )
}

#[test]
fn pointer_attributes_belong_to_the_pointer_level_they_follow() {
    let output = inspect(
        "int * [[a]] * const [[b]] [[c]] p;\n",
        mode(CStandard::C23, ExtensionPolicy::Deny),
    );
    let lines: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|x| !x.starts_with("token [") && !x.starts_with("token ]"))
        .collect();
    let start = lines
        .iter()
        .position(|x| x.starts_with("pointer 0"))
        .expect("pointer level");
    assert_eq!(
        lines[start..],
        [
            "pointer 0 qualifiers=none",
            "attribute-specifier [[...]]",
            "token a",
            "pointer 1 qualifiers=const",
            "attribute-specifier [[...]]",
            "token b",
            "attribute-specifier [[...]]",
            "token c",
            "identifier p",
        ],
        "{output}"
    );
}

#[test]
fn pointer_levels_keep_gnu_and_iso_attribute_chains_in_place() {
    with_parse_configuration(
        "int * __attribute__((a)) * const __attribute__((b)) [[c]] p, q;",
        mode(CStandard::C23, ExtensionPolicy::Allow).with_gnu_extensions(true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let declarator = declaration(p, 0).init_declarators[0].declarator;
            let chain = |level: usize| {
                let mut names = Vec::new();
                let mut link = declarator.pointer.levels[level].attributes;
                while let Some(x) = link {
                    let super::super::modern::SpecifierExtensionKind::Attributes(attributes) =
                        x.kind
                    else {
                        panic!("pointer attributes");
                    };
                    names.push(super::sourced_text(p, attributes.source_vectors));
                    link = x.next;
                }
                names.reverse();
                names
            };
            assert_eq!(chain(0), ["__attribute__((a))"]);
            assert_eq!(chain(1), ["__attribute__((b))", "[[c]]"]);
            assert!(
                declarator
                    .kind
                    .iter()
                    .all(|x| !matches!(x, DirectDeclarator::Attributes(_)))
            );
            let plain = declaration(p, 0).init_declarators[1].declarator;
            assert!(plain.pointer.levels.is_empty());
        },
    );
}

#[test]
fn countof_takes_a_compound_literal_operand() {
    with_parse_configuration(
        "int n = _Countof (int[]){1, 2, 3}; int m = _Countof(int[4]); int k = _Countof (int[]){1} \
         + 1;",
        mode(CStandard::C2y, ExtensionPolicy::Deny),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let operands: Vec<_> = p
                .parser
                .syntax
                .iter::<super::super::syntax::Expression<'_>>()
                .filter_map(|x| match x.kind {
                    | ExpressionType::Countof(operand) => Some(operand),
                    | _ => None,
                })
                .collect();
            assert_eq!(operands.len(), 3);
            assert!(
                matches!(operands[0], SyntaxOperand::Expression(x) if matches!(x.kind, ExpressionType::CompoundLiteral { .. }))
            );
            assert!(matches!(operands[1], SyntaxOperand::Type(_)));
            assert!(
                matches!(operands[2], SyntaxOperand::Expression(x) if matches!(x.kind, ExpressionType::CompoundLiteral { .. }))
            );
        },
    );
}

#[test]
fn compound_literal_storage_survives_aggregate_bodies_in_type_names() {
    for (standard, operand) in [
        (CStandard::C23, "(static struct { int x; }){1}.x"),
        (CStandard::C23, "(static union { int x; int y; }){1}.x"),
        (
            CStandard::C23,
            "(static struct { struct { int x; } inner; }){{1}}.inner.x",
        ),
        (CStandard::C23, "(static enum { E = (1) }){E}"),
        (
            CStandard::C2y,
            "_Countof (static struct { int x; }[1]){{1}}",
        ),
    ] {
        let source = format!("int f(void) {{ return {operand}; }} int after;");
        with_parse_configuration(&source, mode(standard, ExtensionPolicy::Deny), |p| {
            assert!(p.errors.is_empty(), "{source}: {:?}", p.errors);
            assert_eq!(p.items.len(), 2, "following declaration survives");
            let literal = p
                .parser
                .syntax
                .iter::<super::super::syntax::Expression<'_>>()
                .find(|x| matches!(x.kind, ExpressionType::CompoundLiteral { .. }))
                .expect("compound literal");
            let ExpressionType::CompoundLiteral { type_name, .. } = literal.kind else {
                unreachable!("selected compound literal");
            };
            assert!(!literal.recovered, "{source}");
            assert_eq!(
                type_name.declaration_specifiers.storage_class,
                Some(super::super::syntax::StorageClass::Static),
                "{source}"
            );
        });
    }
}

#[test]
fn constexpr_and_thread_local_compound_literals_are_recognized() {
    for source in [
        "int f(void){ return (constexpr int){1}; }",
        "int *f(void){ return &(thread_local int){1}; }",
        "int *f(void){ return &(static thread_local int){1}; }",
        "int f(void){ return (constexpr static int){1}; }",
    ] {
        with_parse_configuration(source, mode(CStandard::C23, ExtensionPolicy::Deny), |p| {
            assert!(p.errors.is_empty(), "{source}: {:?}", p.errors);
            assert!(
                p.parser
                    .syntax
                    .iter::<super::super::syntax::Expression<'_>>()
                    .any(|x| matches!(x.kind, ExpressionType::CompoundLiteral { .. })),
                "{source}"
            );
        });
    }
}

#[test]
fn alignof_expression_is_accepted_as_a_gnu_extension() {
    for policy in [
        ExtensionPolicy::Allow,
        ExtensionPolicy::Warn,
        ExtensionPolicy::Deny,
    ] {
        for (source, keyword) in [
            (
                "int x; int a = _Alignof(x); int b = _Alignof((x));",
                "_Alignof",
            ),
            ("int x; int a = alignof(x[0] + 1);", "alignof"),
        ] {
            let standard = if keyword == "alignof" {
                CStandard::C23
            } else {
                CStandard::C11
            };
            with_parse_configuration(source, mode(standard, policy), |p| {
                assert_eq!(parser_errors(p).count(), 0, "{source}: {:?}", p.errors);
                assert!(
                    p.parser
                        .syntax
                        .iter::<super::super::syntax::Expression<'_>>()
                        .any(|x| matches!(x.kind, ExpressionType::AlignofExpr(_))),
                    "{source}"
                );
                let found = extensions(p);
                if policy == ExtensionPolicy::Allow {
                    assert!(found.is_empty(), "{found:?}");
                } else {
                    assert!(
                        !found.is_empty() && found.iter().all(|x| x.contains("GNU extension")),
                        "{source}: {found:?}"
                    );
                }
            });
        }
    }
    with_parse_configuration(
        "int a = _Alignof(int);",
        mode(CStandard::C11, ExtensionPolicy::Deny),
        |p| assert!(p.errors.is_empty(), "{:?}", p.errors),
    );
}

#[test]
fn identifier_list_declarators_keep_the_c23_hard_error_and_legacy_shape() {
    for standard in [
        CStandard::C89,
        CStandard::C99,
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
                    "int f(a,b); int following;",
                    mode(standard, policy).with_gnu_extensions(gnu),
                    |p| {
                        assert_eq!(
                            parser_errors(p).count(),
                            usize::from(standard >= CStandard::C23),
                            "{:?}",
                            p.errors
                        );
                        assert_eq!(extensions(p), Vec::<String>::new());
                        assert!(declaration(p, 0).init_declarators[0].declarator.kind.iter()
                        .any(|d| matches!(d, DirectDeclarator::KAndRStyleFunction { parameters }
                            if parameters.len() == 2)));
                        assert_eq!(p.items.len(), 2);
                        assert!(!declaration(p, 1).recovered);
                    },
                );
            }
        }
    }
}

#[test]
fn member_declares_nothing_quality_diagnostic_follows_shared_policy() {
    for policy in [
        ExtensionPolicy::Allow,
        ExtensionPolicy::Warn,
        ExtensionPolicy::Deny,
    ] {
        with_parse_configuration(
            "struct S { int; int field; }; int following;",
            mode(CStandard::C17, policy),
            |p| {
                assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
                assert_eq!(
                    p.errors[0].to_string(),
                    "declaration does not declare anything"
                );
                assert_eq!(
                    p.errors[0].severity(),
                    if policy == ExtensionPolicy::Deny {
                        ErrorSeverity::Error
                    } else {
                        ErrorSeverity::Warning
                    }
                );
                assert_eq!(p.items.len(), 2);
            },
        );
    }
}
