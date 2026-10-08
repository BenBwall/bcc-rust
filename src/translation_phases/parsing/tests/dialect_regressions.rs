//! Regressions for language-mode syntax: GNU `__extension__` scoping, C23
//! storage-class and alignment rules, and the lookahead that separates
//! specifiers, attributes, and labels.

use super::{
    Parsed,
    declaration,
    extensions,
    identifier_name,
    parser_errors,
    sourced_text,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        TranslationError,
        parsing::{
            InspectionOptions,
            Parser,
            declaration_syntax::{
                DeclarationSpecifiers,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            modern::{
                AttributeSyntax,
                ExtendedType,
                SpecifierExtensionKind,
            },
            syntax::{
                ExternalDeclaration,
                StorageClass,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            TokenType,
        },
    },
};

fn mode(standard: CStandard, gnu: bool, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu)
}

fn assert_clean_parse(parsed: &Parsed<'_, '_>, source: &str) {
    assert_eq!(
        parser_errors(parsed).count(),
        0,
        "{source}\n{:?}",
        parsed.errors
    );
    assert!(parsed.parser.frames.is_empty(), "{source}");
    assert_eq!(parsed.parser.pedantic_suppression, 0, "{source}");
}

#[test]
fn auto_type_name_enters_scope_after_its_initializer() {
    let source = "typedef int T; void f(void) { { __auto_type T = (T){1}; T + 1; } T following; } \
                  T outside;\n";
    with_parse_configuration(
        source,
        mode(CStandard::C17, true, ExtensionPolicy::Allow),
        |p| {
            assert_clean_parse(p, source);
            assert_eq!(p.items.len(), 3);
            assert!(matches!(
                p.items[1],
                ExternalDeclaration::FunctionDefinition(_)
            ));
            assert!(matches!(
                declaration(p, 2).declaration_specifiers.type_specifiers,
                TypeSpecifiers::TypedefName(_)
            ));
        },
    );
}

#[test]
fn nested_function_parameter_bindings_extend_through_its_body() {
    let source = "typedef int T; void f(void) { void prototype(int a[sizeof(enum { T=2 })]); T \
                  after_prototype; void g(int a[sizeof(enum { T=1 })]) { T + 1; } T \
                  after_definition; } T outside;\n";
    with_parse_configuration(
        source,
        mode(CStandard::C11, true, ExtensionPolicy::Allow),
        |p| {
            assert_clean_parse(p, source);
            assert_eq!(p.items.len(), 3);
            assert!(matches!(
                p.items[1],
                ExternalDeclaration::FunctionDefinition(_)
            ));
            assert!(matches!(
                declaration(p, 2).declaration_specifiers.type_specifiers,
                TypeSpecifiers::TypedefName(_)
            ));
        },
    );
}

#[test]
fn extension_marker_before_a_member_terminates_and_suppresses_only_that_member() {
    // glibc's `bits/atomic_wide_counter.h` shape.
    let glibc = "typedef union { __extension__ unsigned long long int __value64; struct { \
                 unsigned int __low, __high; } __value32; } W;\nstruct S { __extension__ union { \
                 int a; float b; }; int c; };\n";
    for (standard, gnu) in [
        (CStandard::C89, false),
        (CStandard::C99, false),
        (CStandard::C11, true),
    ] {
        with_parse_configuration(glibc, mode(standard, gnu, ExtensionPolicy::Deny), |p| {
            assert_clean_parse(p, glibc);
            assert_eq!(p.items.len(), 2);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        });
    }
    let scoped = "struct S { __extension__ long long a; long long b; };\n";
    with_parse_configuration(
        scoped,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, scoped);
            let extensions = extensions(p);
            assert_eq!(extensions.len(), 1, "{extensions:?}");
            assert!(extensions[0].contains("long long"));
        },
    );
}

#[test]
fn extension_marker_suppresses_flexible_member_checks_until_the_member_finishes() {
    for (source, unsuppressed) in [
        ("struct S { int n; __extension__ int data[]; };\n", false),
        (
            "struct S { int n; __extension__ int data[]; };\nstruct T { int n; int data[]; \
             };\nint tail;\n",
            true,
        ),
    ] {
        for standard in [CStandard::C89, CStandard::C99, CStandard::C23] {
            for gnu in [false, true] {
                for policy in [ExtensionPolicy::Warn, ExtensionPolicy::Deny] {
                    with_parse_configuration(source, mode(standard, gnu, policy), |p| {
                        assert_clean_parse(p, source);
                        let expected = if standard == CStandard::C89 && unsuppressed {
                            vec!["'flexible array member' is a C99 extension"]
                        } else {
                            vec![]
                        };
                        assert_eq!(extensions(p), expected, "{source}: {:?}", p.errors);
                        assert_eq!(p.items.len(), if unsuppressed { 3 } else { 1 });
                        let TypeSpecifiers::StructOrUnion(aggregate) =
                            declaration(p, 0).declaration_specifiers.type_specifiers
                        else {
                            panic!("expected a struct specifier");
                        };
                        let members = aggregate.struct_declaration_list.unwrap();
                        assert_eq!(members.len(), 2);
                        assert!(
                            members[1].struct_declarator_list[0]
                                .declarator
                                .unwrap()
                                .is_unsized_array()
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn bracket_after_an_identifier_is_not_an_attribute() {
    let source = "typedef int T;\nstruct S { char T[16]; };\nvoid g(void) { long T[3]; }\n";
    for standard in [CStandard::C99, CStandard::C23] {
        with_parse_configuration(source, mode(standard, false, ExtensionPolicy::Warn), |p| {
            assert_clean_parse(p, source);
        });
    }
    let implicit = "static buf[10];\n";
    with_parse_configuration(
        implicit,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, implicit);
            assert!(declaration(p, 0).declaration_specifiers.implicit_int);
        },
    );
}

#[test]
fn semicolons_in_balanced_attribute_arguments_stay_attached_to_the_declaration() {
    // C23 §6.7.13.2p1 allows every non-delimiter token, including `;`.
    for arguments in [";", "1;2", "(;)", "[1;2]", "{;}", "[{(;)}]"] {
        let attribute = format!("[[vendor::attr({arguments})]]");
        let source = format!("{attribute} int x;\nint tail;\n");
        for gnu in [false, true] {
            with_parse_configuration(
                &source,
                mode(CStandard::C23, gnu, ExtensionPolicy::Deny),
                |p| {
                    assert_clean_parse(p, &source);
                    assert!(p.errors.is_empty(), "{source}: {:?}", p.errors);
                    assert_eq!(p.items.len(), 2);
                    let head = declaration(p, 0);
                    assert!(!head.recovered);
                    assert_eq!(
                        identifier_name(p, head.init_declarators[0].declarator).as_deref(),
                        Some("x")
                    );
                    let SpecifierExtensionKind::Attributes(attributes) =
                        head.declaration_specifiers.extensions.unwrap().kind
                    else {
                        panic!("expected an attribute on x");
                    };
                    assert_eq!(attributes.syntax, AttributeSyntax::Standard);
                    assert!(!attributes.recovered);
                    assert_eq!(sourced_text(p, attributes.source_vectors), attribute);
                    let semicolon = attributes
                        .tokens
                        .iter()
                        .find(|token| {
                            token.kind == TokenType::Operator(OperatorTokenType::Semicolon)
                        })
                        .expect("attribute argument retains its semicolon");
                    assert_eq!(sourced_text(p, semicolon.source_vectors), ";");
                    let tail = declaration(p, 1);
                    assert!(!tail.recovered);
                    assert_eq!(
                        identifier_name(p, tail.init_declarators[0].declarator).as_deref(),
                        Some("tail")
                    );
                    assert!(tail.declaration_specifiers.extensions.is_none());
                },
            );
        }
    }
}

#[test]
fn incomplete_standard_attributes_recover_at_semicolons_outside_arguments() {
    for attribute in [
        "[[vendor::attr",
        "[[vendor::attr(1;2)",
        "[[vendor::attr([1;2])",
        "[[vendor::attr(1;2)]",
    ] {
        let source = format!("int x {attribute};\nint tail;\n");
        with_parse_configuration(
            &source,
            mode(CStandard::C23, false, ExtensionPolicy::Deny),
            |p| {
                assert_eq!(
                    parser_errors(p).collect::<Vec<_>>(),
                    [&ParserErrorType::ExpectedIsoSyntax(
                        "`]]` in attribute specifier",
                        Some(TokenType::Operator(OperatorTokenType::Semicolon))
                    )],
                    "{source}: {:?}",
                    p.errors
                );
                assert!(p.parser.frames.is_empty());
                assert_eq!(p.parser.pedantic_suppression, 0);
                assert_eq!(p.items.len(), 2);
                assert!(declaration(p, 0).recovered);
                let tail = declaration(p, 1);
                assert!(!tail.recovered);
                assert_eq!(
                    identifier_name(p, tail.init_declarators[0].declarator).as_deref(),
                    Some("tail")
                );
            },
        );
    }
}

#[test]
fn extension_operand_in_control_headers_is_an_expression() {
    let source = "int f(int x) { if (__extension__ x) return 1; switch (__extension__ x) { \
                  default: break; } for (__extension__ x; x; ) break; if (__extension__ ({ x; })) \
                  x = 2; return 0; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C17, true, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
        },
    );
}

#[test]
fn block_scope_function_definition_requires_its_body() {
    let source = "void g(void) {\n int f(void)\n int x;\n x = 0;\n}\nint h(void) { return 0; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Allow),
        |p| {
            assert_eq!(parser_errors(p).count(), 1, "{:?}", p.errors);
            assert_eq!(p.items.len(), 2);
            assert!(matches!(
                p.items[1],
                ExternalDeclaration::FunctionDefinition(_)
            ));
        },
    );
}

#[test]
fn c23_auto_accompanies_another_storage_class_and_infers_the_type() {
    let source = "static auto a = 3.5;\nauto static b = 3.5;\nextern auto int c;\n";
    with_parse_configuration(
        source,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            for item in 0..2 {
                let specifiers = declaration(p, item).declaration_specifiers;
                assert_eq!(specifiers.storage_class, Some(StorageClass::Static));
                assert!(specifiers.auto_with_storage_class);
                assert!(
                    matches!(
                        specifiers.type_specifiers,
                        TypeSpecifiers::Extended(ExtendedType::Inferred)
                    ),
                    "{:?}",
                    specifiers.type_specifiers
                );
            }
            let explicit = declaration(p, 2).declaration_specifiers;
            assert_eq!(explicit.storage_class, Some(StorageClass::Extern));
            assert!(explicit.auto_with_storage_class);
            assert_eq!(explicit.type_specifiers, TypeSpecifiers::Int);
        },
    );
    for (source, standard) in [
        ("typedef auto t = 1;\n", CStandard::C23),
        ("static auto a = 1;\n", CStandard::C17),
        ("auto auto a = 1;\n", CStandard::C23),
    ] {
        with_parse_configuration(source, mode(standard, false, ExtensionPolicy::Warn), |p| {
            assert!(
                parser_errors(p)
                    .any(|error| matches!(error, ParserErrorType::StorageClassRedefinition(..))),
                "{source}: {:?}",
                p.errors
            );
        });
    }
}

#[test]
fn c23_auto_storage_recovery_keeps_following_syntax() {
    for (invalid, storage) in [
        ("static auto auto x=1;", "auto"),
        ("auto static auto int x;", "auto"),
        ("static auto typedef int T;", "typedef"),
    ] {
        for standard in [CStandard::C17, CStandard::C23] {
            for gnu in [false, true] {
                for block_scope in [false, true] {
                    let source = if block_scope {
                        format!(
                            "void f(void) {{ {invalid} int following_block; }}\nint following;\n"
                        )
                    } else {
                        format!("{invalid}\nint following;\n")
                    };
                    let tu = crate::util::bump::Bump::new();
                    let mut context = crate::translation_phases::Context::with_configuration(
                        &tu,
                        mode(standard, gnu, ExtensionPolicy::Deny),
                    );
                    let pp = crate::util::bump::Bump::new();
                    let parse = crate::util::bump::Bump::new();
                    let preprocessor = crate::translation_phases::preprocessing::Preprocessor::new(
                        &pp,
                        &mut context,
                        std::path::PathBuf::from("<auto-storage-recovery>").into_boxed_path(),
                        &source,
                        crate::util::shared::SharedVec::default(),
                        crate::util::shared::SharedVec::default(),
                    );
                    let unit =
                        Parser::new(preprocessor, &mut context, &parse).parse_translation_unit();
                    let output = unit.inspect(&tu, &context, InspectionOptions::default());
                    assert!(
                        output.contains(&format!("storage={storage} type=")),
                        "{standard:?}, GNU={gnu}: {source}\n{output}"
                    );
                    assert!(
                        output
                            .lines()
                            .any(|line| line.trim() == "declarator following"),
                        "{source}\n{output}"
                    );
                    if block_scope {
                        assert!(
                            output.contains("declarator following_block"),
                            "{source}\n{output}"
                        );
                    }
                    assert_eq!(unit.external_declarations().len(), 2, "{source}");
                    if !block_scope {
                        let (ExternalDeclaration::Declaration(declaration)
                        | ExternalDeclaration::RecoveredDeclaration(declaration)) =
                            unit.external_declarations()[0]
                        else {
                            panic!("expected a declaration: {source}");
                        };
                        assert!(
                            !declaration.declaration_specifiers.auto_with_storage_class,
                            "{source}"
                        );
                    }
                    assert!(
                        matches!(
                            unit.external_declarations()[1],
                            ExternalDeclaration::Declaration(_)
                        ),
                        "{source}"
                    );
                    let mut conflicts = 0;
                    while let Some(error) = context.pop_pending_error() {
                        if matches!(error, TranslationError::Parsing(error)
                            if matches!(error.error_type, ParserErrorType::StorageClassRedefinition(..)))
                        {
                            conflicts += 1;
                        }
                    }
                    assert_eq!(
                        conflicts,
                        if standard == CStandard::C23 { 1 } else { 2 },
                        "{source}"
                    );
                }
            }
        }
    }
}

#[test]
fn c23_auto_storage_recovery_spelling_is_total() {
    for (storage_class, spelling) in [
        (None, "none"),
        (Some(StorageClass::Auto), "auto"),
        (Some(StorageClass::Typedef), "typedef"),
        (Some(StorageClass::Static), "auto static"),
        (Some(StorageClass::Extern), "auto extern"),
        (Some(StorageClass::Register), "auto register"),
    ] {
        let specifiers = DeclarationSpecifiers {
            storage_class,
            auto_with_storage_class: true,
            ..DeclarationSpecifiers::new()
        };
        assert_eq!(specifiers.storage_spelling(), spelling);
    }
}

#[test]
fn extension_marker_suppression_ends_with_its_parameter() {
    for (source, expected) in [
        ("void f(__extension__ long long a, long long b);\n", 1),
        (
            "void g(void *q) { void (*p)(long long); p = (void (*)(__extension__ long long))q; { \
             long long y; } }\n",
            2,
        ),
        (
            "void h(a, b) __extension__ long long a; long long b; { }\n",
            1,
        ),
    ] {
        with_parse_configuration(
            source,
            mode(CStandard::C89, false, ExtensionPolicy::Warn),
            |p| {
                assert_clean_parse(p, source);
                let extensions = extensions(p);
                assert_eq!(extensions.len(), expected, "{source}: {extensions:?}");
                assert!(extensions.iter().all(|x| x.contains("long long")));
            },
        );
    }
}

#[test]
fn local_label_declaration_is_not_a_statement_before_declarations() {
    let source = "int f(void){ __label__ L; int x = 0; L: return x; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C89, true, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            let extensions = extensions(p);
            assert_eq!(extensions.len(), 1, "{extensions:?}");
            assert!(extensions[0].contains("__label__"));
        },
    );
}

#[test]
fn typedef_name_label_may_follow_another_label() {
    let source = "typedef int T;\nvoid f(void) { a: T: return; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
        },
    );
}

#[test]
fn named_jump_needs_its_semicolon_after_the_label() {
    let source = "void f(void){ for(;;){ break\n x = 1; } }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Warn),
        |p| {
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
            let errors: Vec<_> = parser_errors(p).collect();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                ParserErrorType::ExpectedSemicolonInStatement(
                    "jump statement",
                    Some(TokenType::Identifier)
                )
            ));
        },
    );
    let named = "void f(void){ outer: for(;;){ break outer; continue outer; } }\n";
    with_parse_configuration(
        named,
        mode(CStandard::C2y, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, named);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}

#[test]
fn function_specifiers_are_not_compound_literal_storage() {
    for source in [
        "int *p = &(inline int){0};\n",
        "int *p = &(inline const int){0};\n",
    ] {
        with_parse_configuration(
            source,
            mode(CStandard::C23, false, ExtensionPolicy::Warn),
            |p| {
                assert_eq!(
                    parser_errors(p).collect::<Vec<_>>(),
                    [&ParserErrorType::DeclarationSpecifierNotAllowedHere(
                        TokenType::Keyword(KeywordTokenType::Inline)
                    )],
                    "{source}: {:?}",
                    p.errors
                );
            },
        );
    }
    let storage = "int *p = &(static int){0};\n";
    with_parse_configuration(
        storage,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, storage);
        },
    );
}

#[test]
fn alignment_specifier_belongs_only_to_declarations_and_compound_literals() {
    let source = "int n = sizeof(alignas(8) int);\n";
    for policy in [ExtensionPolicy::Allow, ExtensionPolicy::Warn] {
        with_parse_configuration(source, mode(CStandard::C23, false, policy), |p| {
            assert_eq!(
                parser_errors(p).collect::<Vec<_>>(),
                [&ParserErrorType::DeclarationSpecifierNotAllowedHere(
                    TokenType::Keyword(KeywordTokenType::Alignas)
                )],
                "{:?}",
                p.errors
            );
        });
    }
    let literal = "int *p = &(alignas(8) int){0};\nstruct S { alignas(8) int m; };\n";
    with_parse_configuration(
        literal,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, literal);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}

#[test]
fn implicit_int_after_c89_is_a_removed_feature() {
    let source = "static x = 1;\n";
    for gnu in [false, true] {
        with_parse_configuration(
            source,
            mode(CStandard::C99, gnu, ExtensionPolicy::Warn),
            |p| {
                assert_clean_parse(p, source);
                assert_eq!(
                    extensions(p),
                    ["'implicit int' is a C89 feature removed in C99"]
                );
            },
        );
    }
    with_parse_configuration(
        source,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}

#[test]
fn typedef_name_singleton_parameter_may_name_void() {
    let source = "typedef void V;\nint f(V) { return 0; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Deny),
        |p| {
            assert_clean_parse(p, source);
            assert!(p.errors.is_empty(), "{:?}", p.errors);
        },
    );
    let unnamed = "int f(int) { return 0; }\n";
    with_parse_configuration(
        unnamed,
        mode(CStandard::C99, false, ExtensionPolicy::Warn),
        |p| {
            assert_eq!(extensions(p).len(), 1, "{:?}", p.errors);
        },
    );
}

#[test]
fn ms_anonymous_structs_accept_typedef_name_members() {
    let source = "typedef struct { int a; } A;\nstruct S { A; int b; };\n";
    let configuration = mode(CStandard::C17, false, ExtensionPolicy::Allow);
    with_parse_configuration(
        source,
        configuration.with_msvc_feature(crate::configuration::MsvcFeature::AnonymousStructs, true),
        |p| {
            assert_clean_parse(p, source);
        },
    );
    with_parse_configuration(source, configuration, |p| {
        assert!(
            parser_errors(p).any(|error| *error == ParserErrorType::EmptyStructDeclarator),
            "{:?}",
            p.errors
        );
    });
}
