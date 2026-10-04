//! Recovery inside declarations, declarators, and parameter lists.

use super::{
    block_items,
    declaration,
    function_definition,
    identifier_name,
    init_declarators,
    parse,
    parser_errors,
    sourced_text,
    with_parse,
};
use crate::translation_phases::{
    ErrorSeverity,
    GetSeverity,
    TranslationError,
    parsing::{
        declaration_syntax::{
            DirectDeclarator,
            Enumerator,
            InitDeclarator,
            ParameterDeclaration,
            StructDeclaration,
            StructDeclarator,
            TypeSpecifiers,
        },
        errors::ParserErrorType,
        scope::NameClass,
        syntax::{
            BlockItem,
            ExpressionSlot,
            ExpressionType,
            ExternalDeclaration,
            Identifier,
            StatementType,
            UnaryOperator,
        },
    },
    preprocessing::{
        IntegerTokenType,
        KeywordTokenType,
        OperatorTokenType,
        TokenType,
    },
};

#[test]
fn pure_external_garbage_yields_an_error_node_and_continues() {
    with_parse("}\nint after;\n", |parsed| {
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::Error(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert!(
            parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::EmptyDeclarationSpecifiers(..)))
        );
    });
}

#[test]
fn missing_declarators_skip_post_declarator_diagnostics() {
    for source in ["int", "int + int after;\n"] {
        with_parse(source, |parsed| {
            assert!(
                parser_errors(parsed).any(|error| matches!(
                    error,
                    ParserErrorType::ExpectedDeclaratorInDeclaration(_)
                ))
            );
            assert!(!parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(..)
            )));
        });
    }

    with_parse("int + int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });
}

#[test]
fn malformed_parameter_recovery_stops_at_comma_and_keeps_the_next_parameter() {
    with_parse("int f(int x +, char y);\nint after;\n", |parsed| {
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert_eq!(parsed.parser.syntax.count::<ParameterDeclaration>(), 2);
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<ParameterDeclaration>()
                .map(|parameter| sourced_text(parsed, parameter.source_vectors))
                .collect::<Vec<_>>(),
            ["intx", "chary"]
        );
        assert_eq!(
            parsed
                .parser
                .syntax
                .nth::<ParameterDeclaration>(1)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::Char
        );
        assert_eq!(
            parsed
                .parser
                .syntax
                .nth::<ParameterDeclaration>(1)
                .declarator
                .and_then(|declarator| identifier_name(parsed, declarator))
                .as_deref(),
            Some("y")
        );
    });
}

#[test]
fn omitted_parameter_comma_reprocesses_the_next_declaration_starter() {
    with_parse("int f(int a int b);\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                Some(TokenType::Keyword(KeywordTokenType::Int))
            )
        )));
        assert_eq!(parsed.parser.syntax.count::<ParameterDeclaration>(), 2);
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<ParameterDeclaration>()
                .filter_map(|parameter| parameter
                    .declarator
                    .and_then(|declarator| identifier_name(parsed, declarator)))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    });
}

#[test]
fn omitted_struct_member_semicolon_reprocesses_the_next_declaration_starter() {
    with_parse("struct S { int first int second; };\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(Some(
                TokenType::Keyword(KeywordTokenType::Int)
            ))
        )));
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<StructDeclarator>()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    });
}

#[test]
fn omitted_enumerator_comma_reprocesses_the_next_identifier() {
    with_parse("enum E { A B, C };\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                TokenType::Identifier
            ))
        )));
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<Enumerator>()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    });
}

#[test]
fn named_parameter_declarators_retain_nested_k_and_r_identifier_lists() {
    with_parse("int outer(int callback(arg));\n", |parsed| {
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let callback = parsed
            .parser
            .syntax
            .nth::<ParameterDeclaration>(0)
            .declarator
            .expect("named callback declarator");
        let parameters = parsed.parser.syntax[callback.kind]
            .iter()
            .find_map(|direct| match direct {
                | DirectDeclarator::KAndRStyleFunction { parameters } => Some(*parameters),
                | _ => None,
            })
            .expect("callback retains a K&R identifier-list suffix");
        assert_eq!(parameters.length, 1);
        assert_eq!(
            parsed
                .context
                .string_cache
                .at(parsed.parser.syntax[parameters][0].name),
            "arg"
        );
    });
}

#[test]
fn nested_recovery_stops_before_grammar_starters() {
    with_parse("int f(int a + int b);\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<ParameterDeclaration>()
                .filter_map(|parameter| parameter
                    .declarator
                    .and_then(|declarator| identifier_name(parsed, declarator)))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    });

    with_parse("struct S { int first + int second; };\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<StructDeclarator>()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    });

    with_parse("struct S { int first : int; int second; };\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<StructDeclarator>()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    });

    with_parse("enum E { A + B, C };\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<Enumerator>()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    });

    with_parse("int f(a + b, c);\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<Identifier>()
                .map(|identifier| parsed.context.string_cache.at(identifier.name))
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    });

    with_parse("enum E { A = + int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("enum E { A = VALUE + OTHER, B };\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<Enumerator>()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    });

    with_parse("enum E { A = int, B };\n", |parsed| {
        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<Enumerator>()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    });

    with_parse("int array[int];\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("int array[* int];\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("int initialized = int;\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("int f(int a, ... + int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("int f(int a, ... int b);\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 0))[0].declarator
            )
            .as_deref(),
            Some("f")
        );
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });

    with_parse("int f(int a,);\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parser_errors(parsed).count(), 1);
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(Some(
                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            ))
        )));
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });
}

#[test]
fn omitted_k_and_r_comma_reprocesses_the_next_identifier() {
    let parsed = parse("int f(a b, c);\n");

    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
            Some(TokenType::Identifier)
        )
    )));
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<Identifier>()
            .map(|identifier| parsed.context.string_cache.at(identifier.name))
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
}

#[test]
fn malformed_parameter_after_ellipsis_terminates() {
    let parsed = parse("int f(int, ..., char trailing);\nint after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
            TokenType::Operator(OperatorTokenType::Comma)
        )
    )));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn malformed_array_bound_recovery_stops_at_the_owning_bracket() {
    let parsed = parse("int a[(1];\nint after;\n");

    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(matches!(
        parsed.items.get(1),
        Some(ExternalDeclaration::Declaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn malformed_children_stop_at_unambiguous_owning_delimiters() {
    for source in [
        "int x = (1; int after;\n",
        "enum E { A = (1, B }; int after;\n",
        "int f(int x + [); int after;\n",
        "struct S { int x + ( ; }; int after;\n",
    ] {
        let parsed = parse(source);

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after"),
            "recovery swallowed the declaration after {source:?}"
        );
    }
}

#[test]
fn malformed_initializer_recovery_preserves_the_next_declaration() {
    let parsed = parse("int x = + int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn malformed_array_bound_recovery_preserves_the_next_declaration() {
    let parsed = parse("int a[+ int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn expression_recovery_keeps_semicolons_inside_nested_braces() {
    for source in [
        "int array[sizeof(struct Inner { int member; })], after;\n",
        "int initialized = sizeof(struct Inner { int member; }), after;\n",
    ] {
        let parsed = parse(source);

        assert_eq!(
            parsed
                .parser
                .syntax
                .iter::<InitDeclarator>()
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            if source.starts_with("int array") {
                vec!["array", "after"]
            } else {
                vec!["initialized", "after"]
            },
            "nested member semicolon escaped recovery for {source:?}"
        );
    }

    let parsed = parse("enum E { A = sizeof(struct Inner { int member; }), B };\n");
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<Enumerator>()
            .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
            .collect::<Vec<_>>(),
        ["A", "B"]
    );

    let parsed = parse(
        "struct Outer { unsigned width : sizeof(struct Inner { int member; }); int after; };\n",
    );
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<StructDeclarator>()
            .filter_map(|declarator| {
                declarator
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator))
            })
            .collect::<Vec<_>>(),
        ["member", "width", "after"]
    );
}

#[test]
fn declaration_recovery_keeps_semicolons_inside_nested_braces() {
    let parsed = parse("int x + sizeof(struct Inner { int member; }); int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn initializer_recovery_unwinds_at_a_top_level_closing_brace() {
    let parsed = parse("int x = 1 } int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
            Some(TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)),
            _
        )
    )));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn initializer_recovery_preserves_an_enclosing_brace_despite_unbalanced_children() {
    let parsed = parse("int x = (1 } int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn array_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
    let parsed = parse("int a[1; int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(Some(
            TokenType::Operator(OperatorTokenType::Semicolon)
        ))
    )));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn array_recovery_unwinds_at_a_top_level_declarator_comma() {
    let parsed = parse("int a[1, b;\n");

    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<InitDeclarator>()
            .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(Some(
            TokenType::Operator(OperatorTokenType::Comma)
        ))
    )));
}

#[test]
fn array_recovery_preserves_an_enclosing_closing_brace() {
    let parsed = parse("struct S { int a[1 } int after;\n");

    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<InitDeclarator>()
            .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
            .collect::<Vec<_>>(),
        ["after"]
    );
}

#[test]
fn array_recovery_preserves_an_enclosing_closing_parenthesis() {
    let parsed = parse("int f(int a[1) int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn struct_recovery_preserves_an_enclosing_closing_parenthesis() {
    let parsed = parse("int f(struct S { int x + ) int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn array_recovery_unwinds_at_semicolons_despite_unbalanced_children() {
    let parsed = parse("int a[(1; int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn parameter_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
    let parsed = parse("int f(int x; int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(Some(
            TokenType::Operator(OperatorTokenType::Semicolon)
        ))
    )));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn parameter_recovery_unwinds_at_semicolons_despite_unbalanced_children() {
    let parsed = parse("int f(int x + (1; int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn parameter_recovery_preserves_an_enclosing_closing_brace() {
    let parsed = parse("int f(int x } int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(Some(
            TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
        ))
    )));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn enum_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
    for source in ["enum E { A; int after;\n", "enum E { A = (1; int after;\n"] {
        let parsed = parse(source);

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                TokenType::Operator(OperatorTokenType::Semicolon)
            ))
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after"),
            "enum recovery swallowed the declaration after {source:?}"
        );
    }
}

#[test]
fn enum_recovery_preserves_an_enclosing_closing_parenthesis() {
    let parsed = parse("int f(enum E { A + ) int after;\n");

    assert_eq!(parsed.items.len(), 2);
    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn malformed_declaration_recovery_stops_at_comma_and_keeps_next_declarator() {
    let parsed = parse("int x +, y; int after;\n");

    assert!(matches!(
        parsed.items.first(),
        Some(ExternalDeclaration::RecoveredDeclaration(_))
    ));
    assert!(matches!(
        parsed.items.get(1),
        Some(ExternalDeclaration::Declaration(_))
    ));
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<InitDeclarator>()
            .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
            .collect::<Vec<_>>(),
        ["x", "y", "after"]
    );
}

#[test]
fn direct_recovery_sources_are_retained_by_the_recovered_declaration() {
    for (source, expected) in [
        ("int x +;\n", "intx+;"),
        ("}\nint after;\n", "}"),
        ("int x }\nint after;\n", "intx}"),
    ] {
        let parsed = parse(source);
        let source_vectors = match parsed.items.first() {
            | Some(ExternalDeclaration::RecoveredDeclaration(index)) =>
                parsed.parser.syntax[index].source_vectors,
            | Some(ExternalDeclaration::Error(source_vectors)) => *source_vectors,
            | root => panic!("expected recovered syntax or an error root: {root:?}"),
        };

        assert_eq!(
            sourced_text(&parsed, source_vectors),
            expected,
            "error-node provenance did not retain all owned tokens for {source:?}"
        );
    }
}

#[test]
fn recovery_trace_identifies_the_owner_of_every_consumed_token() {
    let parsed = parse("int x + (1);\n");
    let recovery = parsed
        .parser
        .trace
        .iter()
        .filter(|event| event.action == "recover-consume")
        .collect::<Vec<_>>();

    assert!(recovery.iter().all(|event| event.frame == "declaration"));
    assert_eq!(
        recovery
            .iter()
            .filter_map(|event| event.token)
            .collect::<Vec<_>>(),
        [
            TokenType::Operator(OperatorTokenType::Plus),
            TokenType::Operator(OperatorTokenType::OpeningParenthesis),
            TokenType::Integer(IntegerTokenType::Int(1)),
            TokenType::Operator(OperatorTokenType::ClosingParenthesis),
        ]
    );
}

#[test]
fn unnamed_bit_field_after_member_comma_does_not_require_a_declarator() {
    let parsed = parse("struct S { int named, : 3; };\n");

    assert!(!parser_errors(&parsed).any(|error| matches!(
        error,
        ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
    )));
    assert_eq!(parsed.parser.syntax.count::<StructDeclarator>(), 2);
    assert!(
        parsed
            .parser
            .syntax
            .nth::<StructDeclarator>(1)
            .declarator
            .is_none()
    );
    assert_eq!(
        parsed
            .parser
            .syntax
            .iter::<StructDeclarator>()
            .map(|declarator| sourced_text(&parsed, declarator.source_vectors))
            .collect::<Vec<_>>(),
        ["named", ":3"]
    );
}

#[test]
fn prototype_enumerators_stop_hiding_file_scope_typedefs_at_the_closing_parenthesis() {
    let parsed = parse("typedef int A; int f(enum { A } x); A y;\n");

    assert_eq!(parsed.items.len(), 3);
    assert!(
        parser_errors(&parsed).next().is_none(),
        "{:#?}",
        parsed.errors
    );
    let a = parsed
        .context
        .string_cache
        .get_id_from_string("A")
        .expect("interned A");
    assert_eq!(
        parsed.parser.scopes.file_scope.get(&a),
        Some(&NameClass::Typedef)
    );
    assert!(matches!(
        declaration(&parsed, 2)
            .declaration_specifiers
            .type_specifiers,
        TypeSpecifiers::TypedefName(identifier) if identifier.name == a
    ));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 2))[0].declarator
        )
        .as_deref(),
        Some("y")
    );
}

#[test]
fn definition_parameter_enumerators_are_visible_in_the_function_body() {
    let parsed = parse("typedef int A; int f(enum { A } x) { A; return 0; }\n");
    let items = block_items(&parsed, function_definition(&parsed, 1).body);

    assert_eq!(items.len(), 2);
    assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
        parsed.parser.syntax[index].kind,
        StatementType::Expression(ExpressionSlot::Parsed(_))
    )));
    assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
        parsed.parser.syntax[index].kind,
        StatementType::Return(Some(ExpressionSlot::Parsed(_)))
    )));
}

#[test]
fn nested_definition_parameter_enumerators_are_visible_in_the_function_body() {
    let parsed = parse("typedef int A; int f(struct { enum { A } e; } x) { A; return 0; }\n");
    let items = block_items(&parsed, function_definition(&parsed, 1).body);

    assert_eq!(items.len(), 2);
    assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
        parsed.parser.syntax[index].kind,
        StatementType::Expression(ExpressionSlot::Parsed(_))
    )));
    assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
        parsed.parser.syntax[index].kind,
        StatementType::Return(Some(ExpressionSlot::Parsed(_)))
    )));
}

#[test]
fn named_parameters_hide_typedefs_for_later_prototype_parameters() {
    let parsed = parse("typedef int T; int f(int T, T x);\n");

    // The parameter name `T` hides the typedef, so `T x` names an unknown
    // type rather than declaring `T` without one.
    assert!(parser_errors(&parsed).any(|error| matches!(error, ParserErrorType::UnknownTypeName)));
    assert_eq!(parsed.parser.syntax.count::<ParameterDeclaration>(), 2);
    assert_eq!(
        parsed
            .parser
            .syntax
            .nth::<ParameterDeclaration>(1)
            .declaration_specifiers
            .type_specifiers,
        TypeSpecifiers::Empty
    );
}

#[test]
fn array_recovery_consumes_nested_brackets_before_the_owning_bracket() {
    let parsed = parse("int a[sizeof(int[2])][*];\n");
    let declarator = parsed.parser.syntax.nth::<InitDeclarator>(0).declarator;

    assert_eq!(
        parsed.parser.syntax[declarator.kind]
            .iter()
            .filter(|direct| matches!(direct, DirectDeclarator::Array { .. }))
            .count(),
        2
    );
}

#[test]
fn parameter_recovery_consumes_nested_parentheses_before_the_owning_separator() {
    let parsed = parse("int f(int x + (1), char y);\nint after;\n");

    assert_eq!(parsed.parser.syntax.count::<ParameterDeclaration>(), 2);
    assert_eq!(
        parsed
            .parser
            .syntax
            .nth::<ParameterDeclaration>(1)
            .declaration_specifiers
            .type_specifiers,
        TypeSpecifiers::Char
    );
    assert_eq!(
        parsed
            .parser
            .syntax
            .nth::<ParameterDeclaration>(1)
            .declarator
            .and_then(|declarator| identifier_name(&parsed, declarator))
            .as_deref(),
        Some("y")
    );
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
        )
        .as_deref(),
        Some("after")
    );
}

#[test]
fn array_star_is_a_vla_marker_only_immediately_before_the_closing_bracket() {
    let parsed = parse("void f(int *p, int marker[*]) { int bound[*p]; }\n");

    let mut saw_marker = false;
    let mut saw_bound = false;
    for direct in parsed.parser.syntax.iter::<DirectDeclarator>() {
        let DirectDeclarator::Array {
            is_pointer,
            assignment_expression,
            ..
        } = *direct
        else {
            continue;
        };
        saw_marker |= is_pointer && assignment_expression.is_none();
        if let Some(expression) = assignment_expression {
            saw_bound |= matches!(
                parsed.parser.syntax[expression].kind,
                ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    ..
                }
            );
        }
    }

    assert!(saw_marker);
    assert!(saw_bound);
    assert!(
        parser_errors(&parsed).next().is_none(),
        "{:#?}",
        parsed.errors
    );
}

#[test]
fn hard_syntax_errors_retain_an_explicitly_recovered_declaration() {
    let parsed = parse("int array[+;\n");
    let Some(ExternalDeclaration::RecoveredDeclaration(index)) = parsed.items.first() else {
        panic!("hard syntax errors should retain a recovered declaration");
    };

    let declaration = &parsed.parser.syntax[index];
    assert_eq!(declaration.init_declarators.length, 1);
    assert!(parser_errors(&parsed).any(|error| {
        matches!(error, ParserErrorType::ExpectedStatementExpression(..))
            && error.severity() == ErrorSeverity::Error
    }));
    assert_eq!(
        identifier_name(
            &parsed,
            init_declarators(&parsed, declaration)[0].declarator
        )
        .as_deref(),
        Some("array")
    );
}

#[test]
fn struct_recovery_distinguishes_a_closing_parenthesis_from_eof() {
    let parsed = parse("int f(struct S { int x + ) int after;\n");
    let error = parsed
        .errors
        .iter()
        .find_map(|error| match error {
            | TranslationError::Parsing(error)
                if matches!(
                    error.error_type,
                    ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(Some(
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    ))
                ) =>
                Some(error),
            | _ => None,
        })
        .expect("missing closing-curly diagnostic anchored to `)`");

    assert_eq!(
        error.to_string(),
        "expected `}` to close the member list, found `)`"
    );
}

#[test]
fn missing_parameter_after_comma_avoids_specifier_cascade_diagnostics() {
    with_parse("int function(int,\n", |parsed| {
        let errors = parser_errors(parsed).collect::<Vec<_>>();

        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|error| matches!(
            error,
            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(None)
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(None, _)
        )));
        assert!(!errors.iter().any(|error| matches!(
            error,
            ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier
                | ParserErrorType::UnexpectedEndBeforeTypeSpecifier
        )));
    });
}

#[test]
fn pointer_without_direct_declarator_keeps_the_legacy_diagnostic() {
    for source in ["int *;\n", "int *\n"] {
        with_parse(source, |parsed| {
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(
                        error,
                        ParserErrorType::TypeQualifiersWithoutDeclarator
                    ))
                    .count(),
                1,
                "missing pointer-without-declarator diagnostic for {source:?}"
            );
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(
                    error,
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        ..
                    )
                ))
                    .count(),
                1,
                "generic direct-declarator diagnostic count changed for {source:?}"
            );
        });
    }
}

#[test]
fn adjacent_array_stars_are_parsed_as_unary_indirection() {
    with_parse("int array[**];\n", |parsed| {
        assert!(
            parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::ExpectedStatementExpression(..)))
        );
        assert!(
            !parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::PointerSpecifiedTwice))
        );
    });
}

#[test]
fn qualifiers_before_an_abstract_array_pointer_keep_their_legacy_diagnostic() {
    with_parse("int function(int [const *]);\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
        )));
    });
}

#[test]
fn mixed_k_and_r_and_prototype_parameters_keep_their_legacy_diagnostic() {
    with_parse("int function(first, int second);\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator
        )));
    });
}

#[test]
fn array_qualifiers_on_both_sides_of_static_keep_the_legacy_diagnostic() {
    with_parse("int values[const static volatile 4];\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
        )));
    });
}

#[test]
fn migrated_nodes_retain_their_exact_owned_token_provenance() {
    let parsed = parse(
        "int (*value);\nint function(const char *name, unsigned count);\nstruct S { int first, \
         *second; unsigned bits:3; };\n",
    );

    let parenthesized = init_declarators(&parsed, declaration(&parsed, 0))[0].declarator;
    let parenthesized_source = parenthesized.source_vectors;
    assert_eq!(sourced_text(&parsed, parenthesized_source), "(*value)");
    assert_eq!(
        identifier_name(&parsed, parenthesized).as_deref(),
        Some("value")
    );

    let parameter_text = parsed
        .parser
        .syntax
        .iter::<ParameterDeclaration>()
        .map(|parameter| sourced_text(&parsed, parameter.source_vectors))
        .collect::<Vec<_>>();
    assert_eq!(parameter_text, ["constchar*name", "unsignedcount"]);

    let member_text = parsed
        .parser
        .syntax
        .iter::<StructDeclaration>()
        .map(|declaration| sourced_text(&parsed, declaration.source_vectors))
        .collect::<Vec<_>>();
    assert_eq!(member_text, ["intfirst,*second;", "unsignedbits:3;"]);
    let member_declarator_text = parsed
        .parser
        .syntax
        .iter::<StructDeclarator>()
        .map(|declarator| sourced_text(&parsed, declarator.source_vectors))
        .collect::<Vec<_>>();
    assert_eq!(member_declarator_text, ["first", "*second", "bits:3"]);
}

#[test]
fn empty_abstract_function_declarator_owns_both_parentheses() {
    let parsed = parse("int f(int ());\n");
    let declarator = parsed
        .parser
        .syntax
        .nth::<ParameterDeclaration>(0)
        .declarator
        .expect("abstract function declarator");

    assert_eq!(sourced_text(&parsed, declarator.source_vectors), "()");
}

#[test]
fn malformed_and_eof_paths_terminate_with_source_backed_diagnostics() {
    type DiagnosticMatcher = fn(&ParserErrorType) -> bool;
    let cases: &[(&str, DiagnosticMatcher)] = &[
        ("int\n", |error| {
            matches!(
                error,
                ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(None)
            )
        }),
        ("int *\n", |error| {
            matches!(
                error,
                ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(None)
            )
        }),
        ("int (value\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(None)
            )
        }),
        ("int (value;\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(Some(
                    TokenType::Operator(OperatorTokenType::Semicolon)
                ))
            )
        }),
        ("int array[\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None)
            )
        }),
        ("int array[*;\n", |error| {
            matches!(error, ParserErrorType::ExpectedStatementExpression(..))
        }),
        ("int function(int\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    None
                )
            )
        }),
        ("int function(int, ...;\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    TokenType::Operator(OperatorTokenType::Semicolon)
                )
            )
        }),
        ("struct S { int member\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(None)
            )
        }),
        ("struct S { int first,\n", |error| {
            matches!(
                error,
                ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(None)
            )
        }),
        ("struct S { int member }\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList
            )
        }),
        ("struct {\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(..)
            )
        }),
        ("struct S {};\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace
            )
        }),
        ("union U {};\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace
            )
        }),
        ("enum E { A\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(None)
            )
        }),
        ("enum E { A,, B };\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(Some(
                    TokenType::Operator(OperatorTokenType::Comma)
                ))
            )
        }),
        ("enum {\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(None)
            )
        }),
        ("enum E {};\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace
            )
        }),
        ("typedef ;\n", |error| {
            matches!(error, ParserErrorType::ExpectedDeclaratorInTypedef(..))
        }),
        ("}\nint after;\n", |error| {
            matches!(error, ParserErrorType::EmptyDeclarationSpecifiers(..))
        }),
    ];

    for (source, expected_diagnostic) in cases {
        let parsed = parse(source);
        let errors = parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Parsing(error) => Some(error),
                | _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            errors
                .iter()
                .any(|error| expected_diagnostic(&error.error_type)),
            "missing expected parser diagnostic for {source:?}: {errors:#?}"
        );
        assert!(errors.iter().all(|error| error.source_vectors.length > 0));
    }

    for source in ["struct S {};\n", "union U {};\n", "enum E {};\n"] {
        let parsed = parse(source);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
    }
}
