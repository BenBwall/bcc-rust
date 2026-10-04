//! Declarations, specifiers, and declarators.

use super::{
    block_items,
    declaration,
    function_definition,
    identifier_name,
    parser_errors,
    sourced_text,
    with_parse,
};
use crate::{
    translation_phases::{
        TranslationError,
        TranslationPhase,
        parsing::{
            declaration_syntax::{
                DirectDeclarator,
                EnumSpecifier,
                Enumerator,
                InitDeclarator,
                StructDeclaration,
                StructDeclarator,
                StructOrUnion,
                StructOrUnionSpecifier,
                TypeQualifiers,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            syntax::{
                BlockItem,
                ExternalDeclaration,
                Identifier,
                StatementType,
                StorageClass,
            },
        },
        preprocessing::{
            OperatorTokenType,
            TokenType,
        },
    },
    util::vector_slice::VectorSlice,
};

#[test]
fn specifier_only_declaration_runs_through_the_machine() {
    with_parse("int;\n", |parsed| {
        assert_eq!(parsed.items.len(), 1);
        let declaration = declaration(parsed, 0);
        assert_eq!(
            declaration.declaration_specifiers.type_specifiers,
            TypeSpecifiers::Int
        );
        assert_eq!(declaration.init_declarators.len(), 0);
        assert!(parser_errors(parsed).next().is_none());
        assert!(
            parsed.parser.trace.iter().any(|event| {
                event.frame == "declaration-specifiers" && event.action == "reduce"
            })
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .any(|event| event.token.is_some())
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .any(|event| { event.frame == "external-declaration" && event.action == "reduce" })
        );
    });
}

#[test]
fn empty_translation_unit_emits_one_dedicated_diagnostic() {
    with_parse("", |parsed| {
        assert_eq!(parsed.items, []);
        assert_eq!(
            parser_errors(parsed).collect::<Vec<_>>(),
            [&ParserErrorType::EmptyTranslationUnit]
        );
        assert_eq!(parsed.parser.next_item(parsed.context), None);
        assert!(parsed.context.pop_pending_error().is_none());
    });
}

#[test]
fn ordinary_pointer_and_typedef_declarations_are_reachable() {
    with_parse(
        "int x;\nint x2, y;\nconst unsigned long *p;\nint *const *volatile q;\ntypedef int T;\nT \
         value;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 6);
            assert!(parser_errors(parsed).next().is_none());
            let names = parsed
                .items
                .iter()
                .enumerate()
                .flat_map(|(item, _)| (declaration(parsed, item)).init_declarators)
                .map(|init| identifier_name(parsed, init.declarator).expect("named declarator"))
                .collect::<Vec<_>>();
            assert_eq!(names, ["x", "x2", "y", "p", "q", "T", "value"]);
            let TypeSpecifiers::TypedefName(identifier) = declaration(parsed, 5)
                .declaration_specifiers
                .type_specifiers
            else {
                panic!("expected a typedef-name specifier")
            };
            assert_eq!(
                identifier.name,
                parsed
                    .context
                    .string_cache
                    .get_id_from_string("T")
                    .expect("interned T")
            );

            let pointer_declaration = declaration(parsed, 3);
            let pointer = pointer_declaration.init_declarators[0]
                .declarator
                .pointer
                .type_qualifiers_list;
            assert_eq!(pointer, &[TypeQualifiers::CONST, TypeQualifiers::VOLATILE]);
        },
    );
}

#[test]
fn name_classification_is_published_after_each_declarator() {
    with_parse(
        "typedef int T, Prototype(T);\nint T, OldStyle(T);\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 2);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(parsed
        .parser
        .syntax
        .iter::<DirectDeclarator<'_>>()
        .any(|direct| matches!(direct, DirectDeclarator::Function { parameter_list, .. } if parameter_list.len() == 1)));
            assert!(parsed
        .parser
        .syntax
        .iter::<DirectDeclarator<'_>>()
        .any(|direct| matches!(direct, DirectDeclarator::KAndRStyleFunction { parameters } if parameters.len() == 1)));
            let t = parsed
                .context
                .string_cache
                .get_id_from_string("T")
                .expect("interned T");
            assert!(
                !parsed.parser.scopes.is_file_scope_typedef(t),
                "the ordinary declaration of T ends its typedef status"
            );
        },
    );
}

#[test]
fn duplicate_storage_class_keeps_the_last_class_for_typedef_publication() {
    with_parse("extern typedef int T;\nT x;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            declaration(parsed, 0).declaration_specifiers.storage_class,
            Some(StorageClass::Typedef)
        );
        assert!(
            parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::StorageClassRedefinition(..)))
        );
        let t = parsed
            .context
            .string_cache
            .get_id_from_string("T")
            .expect("interned T");
        assert!(matches!(
            declaration(parsed, 1)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::TypedefName(identifier) if identifier.name == t
        ));
        assert_eq!(
            identifier_name(
                parsed,
                declaration(parsed, 1).init_declarators[0].declarator
            )
            .as_deref(),
            Some("x")
        );
    });
}

#[test]
fn conflicting_type_specifiers_are_anchored_to_the_conflicting_token() {
    with_parse(
        "int float primitive; int struct S { int member; } tagged; int union U { int member; } \
         united; int enum E { A } enumerated;\n",
        |parsed| {
            let mut conflict_sources = parsed
                .errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Parsing(error)
                        if matches!(
                            error.error_type,
                            ParserErrorType::ConflictingTypeSpecifiers { .. }
                        ) =>
                        Some(sourced_text(parsed, error.source_vectors)),
                    | _ => None,
                })
                .collect::<Vec<_>>();
            conflict_sources.sort_unstable();
            assert_eq!(conflict_sources, ["enum", "float", "struct", "union"]);
        },
    );
}

#[test]
fn conflicting_type_specifier_messages_use_source_spellings() {
    with_parse(
        "typedef int T; T long a; struct S { int m; } long b; enum E { A } T c; long T d;
",
        |parsed| {
            let messages = parsed
                .errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Parsing(error)
                        if matches!(
                            error.error_type,
                            ParserErrorType::ConflictingTypeSpecifiers { .. }
                        ) =>
                        Some(error.to_string()),
                    | _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                messages,
                [
                    "cannot combine `long` with `T`",
                    "cannot combine `long` with `struct S`",
                    "cannot combine `T` with `enum E`",
                    "cannot combine `T` with `long`",
                ]
            );
            for message in &messages {
                assert!(!message.contains("VectorSlice"), "{message}");
                assert!(!message.contains("StringCacheId"), "{message}");
            }
        },
    );
}

#[test]
fn conflicting_typedef_names_remain_specifiers_and_preserve_following_declarations() {
    with_parse(
        "typedef int T; unsigned T x; unsigned T (*pointer); unsigned T ((*nested)); T y;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 5);
            let conflict = parsed
                .errors
                .iter()
                .find_map(|error| match error {
                    | TranslationError::Parsing(error)
                        if matches!(
                            &error.error_type,
                            ParserErrorType::ConflictingTypeSpecifiers {
                                existing,
                                conflicting,
                            } if &**existing == "unsigned" && &**conflicting == "T"
                        ) =>
                        Some(error),
                    | _ => None,
                })
                .expect("typedef conflict diagnostic");
            assert_eq!(sourced_text(parsed, conflict.source_vectors), "T");
            assert_eq!(conflict.to_string(), "cannot combine `T` with `unsigned`");
            assert_eq!(
                parsed
                    .parser
                    .syntax
                    .iter::<InitDeclarator<'_>>()
                    .filter_map(|declarator| identifier_name(parsed, declarator.declarator))
                    .collect::<Vec<_>>(),
                ["T", "x", "pointer", "nested", "y"]
            );
            assert!(
                declaration(parsed, 4)
                    .declaration_specifiers
                    .type_specifiers
                    .is_typedef_name()
            );
        },
    );
}

#[test]
fn parenthesized_identifier_lists_preserve_typedef_shadowing() {
    with_parse("typedef int T; unsigned T (x); T y;\n", |parsed| {
        assert_eq!(parsed.items.len(), 3);
        assert_eq!(
            [declaration(parsed, 0), declaration(parsed, 1)]
                .into_iter()
                .flat_map(|declaration| declaration.init_declarators)
                .filter_map(|declarator| identifier_name(parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["T", "T"]
        );
        assert!(matches!(
            parsed.items[2],
            ExternalDeclaration::RecoveredDeclaration(_)
        ));
        let typedef_name = parsed
            .context
            .string_cache
            .get_id_from_string("T")
            .expect("interned typedef name");
        assert!(!parsed.parser.scopes.is_typedef(typedef_name));
    });
}

#[test]
fn arrays_functions_abstract_parameters_variadics_and_k_and_r_parse() {
    with_parse(
        "int a[];\nint matrix[][];\nint f(int, const char *, ...);\nint old(a,b);\nint \
         (*factory(void))(int);\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 5);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<DirectDeclarator<'_>>()
                    .any(|direct| {
                        matches!(
                            direct,
                            DirectDeclarator::Array {
                                assignment_expression: None,
                                ..
                            }
                        )
                    })
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<DirectDeclarator<'_>>()
                    .any(|direct| {
                        matches!(
                            direct,
                            DirectDeclarator::Function {
                                is_variadic: true,
                                ..
                            }
                        )
                    })
            );
            assert!(parsed.parser.syntax.iter::<DirectDeclarator<'_>>().any(|direct| {
        matches!(direct, DirectDeclarator::KAndRStyleFunction { parameters } if parameters.len() == 2)
    }));
        },
    );
}

#[test]
fn union_kind_and_enum_arena_slice_are_correct() {
    with_parse(
        "struct S;\nunion Forward;\nstruct { int anonymous; };\nunion U { int x; char y; \
         };\nstruct Outer { union { int nested; } value; };\nenum E { A, B, };\nenum { C, D };\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 7);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<StructOrUnionSpecifier<'_>>()
                    .any(|specifier| specifier.struct_or_union == StructOrUnion::Union)
            );
            let enum_specifier = parsed
                .parser
                .syntax
                .iter::<EnumSpecifier<'_>>()
                .find(|specifier| specifier.name.is_some())
                .expect("named enum specifier");
            let enumeration_list = enum_specifier.enumeration_list.expect("enum body");
            assert_eq!(enumeration_list.len(), 2);
            let names = enumeration_list
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>();
            assert_eq!(names, ["A", "B"]);
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Enumerator<'_>>()
                    .all(|enumerator| enumerator.source_vectors.length > 0)
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<StructDeclaration<'_>>()
                    .map(|declaration| declaration.source_vectors)
                    .chain(
                        parsed
                            .parser
                            .syntax
                            .iter::<StructDeclarator<'_>>()
                            .map(|declarator| declarator.source_vectors)
                    )
                    .all(|source_vectors| source_vectors.length > 0)
            );
        },
    );
}

#[test]
fn specifier_combinations_and_conflicts_keep_legacy_diagnostics() {
    with_parse(
        "extern const unsigned long int x;\ninline static double f(void);\nconst const int \
         duplicate;\nlong long double conflict;\ndouble long long reordered;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 5);
            assert_eq!(
                declaration(parsed, 0)
                    .declaration_specifiers
                    .type_specifiers,
                TypeSpecifiers::UnsignedLongInt
            );
            assert!(
                declaration(parsed, 0)
                    .declaration_specifiers
                    .type_qualifiers
                    .contains(TypeQualifiers::CONST)
            );
            assert!(
                declaration(parsed, 1)
                    .declaration_specifiers
                    .function_specifiers
                    .is_inline
            );
            assert!(
                parser_errors(parsed)
                    .any(|error| { matches!(error, ParserErrorType::ConstSpecifiedTwice) })
            );
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(error, ParserErrorType::LongLongDoubleSpecified))
                    .count(),
                2
            );
            assert!(TypeSpecifiers::Long.is_long());
            assert!(TypeSpecifiers::LongDouble.is_long_double());
            let structure = StructOrUnionSpecifier {
                struct_or_union:         StructOrUnion::Struct,
                identifier:              None,
                struct_declaration_list: None,
                source_vectors:          VectorSlice::empty(),
            };
            assert!(TypeSpecifiers::StructOrUnion(&structure).is_struct_or_union());
            let enumeration = EnumSpecifier {
                name:             None,
                enumeration_list: None,
                source_vectors:   VectorSlice::empty(),
            };
            assert!(TypeSpecifiers::Enum(&enumeration).is_enum());
            let duplicate = parsed
                .context
                .string_cache
                .get_id_from_string("duplicate")
                .expect("interned identifier");
            assert!(
                TypeSpecifiers::TypedefName(Identifier::new(duplicate, VectorSlice::empty()))
                    .is_typedef_name()
            );
        },
    );
}

#[test]
fn expression_dependent_positions_store_typed_syntax_children() {
    with_parse(
        "int bounded[4];\nstruct Bits { unsigned value:3; };\nenum Values { A=1, B };\nint \
         initialized=42;\nint function(void) { return 0; }\nint after;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 6);
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<DirectDeclarator<'_>>()
                    .any(|direct| matches!(
                        direct,
                        DirectDeclarator::Array {
                            assignment_expression: Some(_),
                            ..
                        }
                    ))
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<StructDeclarator<'_>>()
                    .any(|declarator| declarator.bitfield_width.is_some())
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Enumerator<'_>>()
                    .any(|enumerator| enumerator.expression.is_some())
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<InitDeclarator<'_>>()
                    .any(|declarator| declarator.initializer.is_some())
            );
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert_eq!(
                identifier_name(
                    parsed,
                    declaration(parsed, 5).init_declarators[0].declarator
                )
                .as_deref(),
                Some("after")
            );
        },
    );
}

#[test]
fn assignments_are_rejected_in_constant_expression_owners() {
    with_parse("enum E { A = value = 1, B }; int after;\n", |parsed| {
        assert!(matches!(
            parsed.items.as_slice(),
            [
                ExternalDeclaration::RecoveredDeclaration(_),
                ExternalDeclaration::Declaration(_)
            ]
        ));
        assert_eq!(
            identifier_name(
                parsed,
                declaration(parsed, 1).init_declarators[0].declarator
            )
            .as_deref(),
            Some("after")
        );
        let errors = parser_errors(parsed).collect::<Vec<_>>();
        assert!(matches!(
            errors.as_slice(),
            [
                ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                    TokenType::Operator(OperatorTokenType::Equals)
                ))
            ]
        ));
    });
}

#[test]
fn braced_declarators_retain_function_definition_syntax_before_constraint_checking() {
    with_parse("int object { int retained; } int after;\n", |parsed| {
        assert!(!function_definition(parsed, 0).recovered);
        let [BlockItem::Declaration(retained)] = block_items(function_definition(parsed, 0).body)
        else {
            panic!("expected the braced declarator to retain its compound body")
        };
        assert_eq!(
            identifier_name(parsed, retained.init_declarators[0].declarator).as_deref(),
            Some("retained")
        );
        assert_eq!(
            identifier_name(
                parsed,
                declaration(parsed, 1).init_declarators[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse(
        "int object, f() { int swallowed; } int after;\n",
        |parsed| {
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                    Some(TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)),
                    _
                )
            )));
            assert_eq!(declaration(parsed, 0).init_declarators.len(), 2);
            assert_eq!(
                identifier_name(
                    parsed,
                    declaration(parsed, 1).init_declarators[0].declarator
                )
                .as_deref(),
                Some("after")
            );
        },
    );
}

#[test]
fn declaration_lists_retain_constraint_invalid_function_definitions() {
    with_parse("int f int parameter; { return 0; }\n", |parsed| {
        assert!(matches!(
            parsed.items.as_slice(),
            [ExternalDeclaration::FunctionDefinition(_)]
        ));
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let definition = function_definition(parsed, 0);
        assert_eq!(definition.declaration_list.len(), 1);
        assert!(matches!(
            definition.body.kind,
            StatementType::Compound { .. }
        ));
    });
}

#[test]
fn function_body_dispatch_follows_parenthesized_pointer_binding() {
    with_parse("int (f()) { return 0; } int after;\n", |function| {
        assert!(
            parser_errors(function).next().is_none(),
            "{:#?}",
            function.errors
        );
        assert!(!parser_errors(function).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                Some(TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)),
                _
            )
        )));
    });

    with_parse(
        "int (*fp)(void) { int swallowed; } int after;\n",
        |pointer| {
            assert!(!function_definition(pointer, 0).recovered);
            assert_eq!(block_items(function_definition(pointer, 0).body).len(), 1);
            assert_eq!(
                identifier_name(
                    pointer,
                    declaration(pointer, 1).init_declarators[0].declarator
                )
                .as_deref(),
                Some("after")
            );
        },
    );
}
