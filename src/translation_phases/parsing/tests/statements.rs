//! Function definitions, compound blocks, and statement parsing.

use super::{
    block_items,
    constant_expression_text,
    declaration,
    expression_text,
    function_definition,
    identifier_name,
    init_declarators,
    parser_errors,
    sourced_text,
    with_parse,
};
use crate::translation_phases::{
    TranslationError,
    parsing::{
        declaration_syntax::{
            InitializerType,
            ParameterDeclaration,
            StructDeclarator,
        },
        errors::ParserErrorType,
        scope::ScopeKind,
        syntax::{
            BlockItem,
            ConstantExpressionSlot,
            ExpressionSlot,
            ExternalDeclaration,
            ForInitializer,
            StatementType,
        },
    },
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        TokenType,
    },
};

#[test]
fn prototype_style_definition_produces_real_function_syntax() {
    with_parse("int defined(int value) { return; }\n", |parsed| {
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let ExternalDeclaration::FunctionDefinition(index) = parsed.items[0] else {
            panic!("expected a function definition: {:#?}", parsed.items);
        };
        let definition = &parsed.parser.syntax[index];
        assert_eq!(
            identifier_name(parsed, definition.declarator).as_deref(),
            Some("defined")
        );
        assert_eq!(
            sourced_text(parsed, definition.source_vectors),
            "intdefined(intvalue){return;}"
        );
        assert!(!definition.recovered);
        assert!(matches!(
            parsed.parser.syntax[definition.body].kind,
            StatementType::Compound { .. }
        ));
    });
}

#[test]
fn compound_blocks_preserve_mixed_items_and_every_statement_family() {
    with_parse(
        "int all(int value) {\nint local;\n; value; label: ; case 1: ; default: ;\nif (value) ; \
         else ; switch (value) { case 1: ; default: ; }\nwhile (value) ; do ; while (value);\nfor \
         (;;) ; for (value; value; value) ; for (int i; i; i) ;\ngoto label; continue; break; \
         return; return value;\n}\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 1);
            let definition = function_definition(parsed, 0);
            let items = block_items(parsed, definition.body);
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert!(
                items[1..]
                    .iter()
                    .all(|item| matches!(item, BlockItem::Statement(_)))
            );

            let kinds = items
                .iter()
                .filter_map(|item| match item {
                    | BlockItem::Statement(index) => Some(parsed.parser.syntax[index].kind),
                    | BlockItem::Declaration(_) => None,
                })
                .collect::<Vec<_>>();
            assert!(kinds.iter().any(|kind| matches!(kind, StatementType::Null)));
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Expression(_)))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Label(..)))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Case(..)))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Default(..)))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::If { .. }))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Switch { .. }))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::While { .. }))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::DoWhile { .. }))
            );
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, StatementType::For { .. }))
                    .count(),
                3
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Goto(_)))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Continue))
            );
            assert!(
                kinds
                    .iter()
                    .any(|kind| matches!(kind, StatementType::Break))
            );
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, StatementType::Return(_)))
                    .count(),
                2
            );
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(parsed.parser.scopes.nested_scopes.is_empty());
            assert!(parsed.parser.label_scopes.is_empty());
            assert!(parsed.parser.switch_scopes.is_empty());
        },
    );
}

#[test]
fn old_style_definition_keeps_its_parameter_declaration_list() {
    with_parse(
        "int declared(int value);\nint (*pointer)(int value);\nint old_style(left, right) int \
         left; int right; { return; }\n",
        |parsed| {
            assert!(matches!(
                parsed.items[0],
                ExternalDeclaration::Declaration(_)
            ));
            assert!(matches!(
                parsed.items[1],
                ExternalDeclaration::Declaration(_)
            ));
            let definition = function_definition(parsed, 2);
            assert_eq!(definition.declaration_list.length, 2);
            let names = parsed.parser.syntax[definition.declaration_list]
                .iter()
                .map(|index| {
                    identifier_name(
                        parsed,
                        init_declarators(parsed, &parsed.parser.syntax[index])[0].declarator,
                    )
                    .expect("old-style declaration name")
                })
                .collect::<Vec<_>>();
            assert_eq!(names, ["left", "right"]);
            assert!(
                parsed.parser.syntax[definition.declaration_list]
                    .iter()
                    .all(|index| !parsed.parser.syntax[index].recovered)
            );
            assert_eq!(
                sourced_text(parsed, definition.source_vectors),
                "intold_style(left,right)intleft;intright;{return;}"
            );
            assert!(!definition.recovered);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );

            let nested_statements = format!(
                "int control(void) {{ {} ; }}\n",
                (0..127)
                    .map(|index| if index % 2 == 0 {
                        "if (condition)"
                    } else {
                        "while (condition)"
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            with_parse(&nested_statements, |parsed| {
                assert!(matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::FunctionDefinition(_))
                ));
                assert!(
                    parser_errors(parsed).next().is_none(),
                    "{:#?}",
                    parsed.errors
                );
                assert!(parsed.parser.scopes.nested_scopes.is_empty());
                assert!(
                    parsed
                        .parser
                        .trace
                        .iter()
                        .map(|event| event.depth)
                        .max()
                        .is_some_and(|depth| depth > 127)
                );
            });
        },
    );
}

#[test]
fn function_definition_publishes_identifier_bound_parameters() {
    with_parse("int (*legacy(a))(int) int a; { return; }\n", |old_style| {
        let definition = function_definition(old_style, 0);
        assert_eq!(definition.declaration_list.length, 1);
        assert!(!definition.recovered);
        assert!(
            parser_errors(old_style).next().is_none(),
            "{:#?}",
            old_style.errors
        );
    });
    with_parse(
        "typedef int Y;\nint (*nested(int x))(int Y) { Y value; return 0; }\n",
        |nested| {
            let definition = function_definition(nested, 1);
            let items = block_items(nested, definition.body);
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert_eq!(
                identifier_name(
                    nested,
                    init_declarators(
                        nested,
                        &nested.parser.syntax[match items[0] {
                            | BlockItem::Declaration(index) => index,
                            | BlockItem::Statement(_) => unreachable!(),
                        }],
                    )[0]
                    .declarator,
                )
                .as_deref(),
                Some("value")
            );
            assert!(
                parser_errors(nested).next().is_none(),
                "{:#?}",
                nested.errors
            );
        },
    );
}

#[test]
fn old_style_parameter_recovery_preserves_the_function_body() {
    for source in [
        "int f(a) int a { return; }\n",
        "int f(a) int a = value { return; }\n",
    ] {
        with_parse(source, |parsed| {
            let definition = function_definition(parsed, 0);
            assert_eq!(definition.declaration_list.length, 1);
            let declaration = parsed.parser.syntax[definition.declaration_list][0];
            assert!(parsed.parser.syntax[declaration].recovered);
            let [BlockItem::Statement(statement)] = block_items(parsed, definition.body) else {
                panic!("expected the recovered function body to retain its return statement")
            };
            assert!(matches!(
                parsed.parser.syntax[statement].kind,
                StatementType::Return(None)
            ));
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                    Some(TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)),
                    _
                )
            )));
        });
    }
}

#[test]
fn dangling_else_binds_to_the_nearest_unmatched_if() {
    with_parse(
        "int f(void) { if (outer) if (inner) ; else ; }\n",
        |parsed| {
            let definition = function_definition(parsed, 0);
            let [BlockItem::Statement(outer)] = block_items(parsed, definition.body) else {
                panic!("expected one outer if statement")
            };
            let StatementType::If {
                then_statement: inner,
                else_statement: outer_else,
                ..
            } = parsed.parser.syntax[outer].kind
            else {
                panic!("expected outer if")
            };
            let StatementType::If { else_statement, .. } = parsed.parser.syntax[inner].kind else {
                panic!("expected inner if")
            };
            assert!(outer_else.is_none());
            assert!(else_statement.is_some());
        },
    );
}

#[test]
fn missing_if_body_leaves_else_for_its_enclosing_if() {
    with_parse("int f(void) { if (condition) else value; }\n", |parsed| {
        let [BlockItem::Statement(if_statement)] =
            block_items(parsed, function_definition(parsed, 0).body)
        else {
            panic!("expected one if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: Some(else_statement),
            ..
        } = parsed.parser.syntax[if_statement].kind
        else {
            panic!("expected a recovered if statement with an else branch")
        };
        assert!(matches!(
            parsed.parser.syntax[then_statement].kind,
            StatementType::Null
        ));
        assert!(matches!(
            parsed.parser.syntax[else_statement].kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        ));
    });
}

#[test]
fn nested_missing_if_body_leaves_else_for_its_enclosing_if() {
    with_parse(
        "int f(void) { if (outer) while (inner) else value; }\n",
        |parsed| {
            let [BlockItem::Statement(if_statement)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one if statement")
            };
            let StatementType::If {
                then_statement,
                else_statement: Some(else_statement),
                ..
            } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected a recovered if statement with an else branch")
            };
            let StatementType::While { body_statement, .. } =
                parsed.parser.syntax[then_statement].kind
            else {
                panic!("expected the if then-branch to be a while statement")
            };
            assert!(matches!(
                parsed.parser.syntax[body_statement].kind,
                StatementType::Null
            ));
            assert!(matches!(
                parsed.parser.syntax[else_statement].kind,
                StatementType::Expression(ExpressionSlot::Parsed(_))
            ));
        },
    );
}

#[test]
fn missing_else_body_leaves_a_later_else_for_its_enclosing_if() {
    with_parse(
        "int f(void) { if (outer) if (inner) ; else else value; }\n",
        |parsed| {
            let [BlockItem::Statement(if_statement)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one if statement")
            };
            let StatementType::If {
                then_statement,
                else_statement: Some(outer_else),
                ..
            } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected the outer if to retain its else branch")
            };
            let StatementType::If {
                else_statement: Some(inner_else),
                ..
            } = parsed.parser.syntax[then_statement].kind
            else {
                panic!("expected the inner if to retain its first else branch")
            };
            assert!(matches!(
                parsed.parser.syntax[inner_else].kind,
                StatementType::Null
            ));
            assert!(matches!(
                parsed.parser.syntax[outer_else].kind,
                StatementType::Expression(ExpressionSlot::Parsed(_))
            ));
        },
    );
}

#[test]
fn case_recovery_distinguishes_a_conditional_colon_from_the_label_colon() {
    with_parse(
        "int f(void) { switch (value) { case a ? b : c: ; } }\n",
        |parsed| {
            let [BlockItem::Statement(switch)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one switch statement")
            };
            let StatementType::Switch { body_statement, .. } = parsed.parser.syntax[switch].kind
            else {
                panic!("expected switch syntax")
            };
            let [BlockItem::Statement(case)] = block_items(parsed, body_statement) else {
                panic!("expected one case label")
            };
            let StatementType::Case(ConstantExpressionSlot::Parsed(expression), _) =
                parsed.parser.syntax[case].kind
            else {
                panic!("expected a parsed case expression")
            };

            assert_eq!(constant_expression_text(parsed, expression), "a?b:c");
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn case_recovery_does_not_steal_a_colon_after_a_closed_conditional_delimiter() {
    with_parse(
        "int f(void) { switch (value) { case (a ? b) : ; } }\n",
        |parsed| {
            let [BlockItem::Statement(switch)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one switch statement")
            };
            let StatementType::Switch { body_statement, .. } = parsed.parser.syntax[switch].kind
            else {
                panic!("expected switch syntax")
            };
            let [BlockItem::Statement(case)] = block_items(parsed, body_statement) else {
                panic!("expected one case label")
            };
            let StatementType::Case(ConstantExpressionSlot::Parsed(expression), _) =
                parsed.parser.syntax[case].kind
            else {
                panic!("expected a parsed case expression")
            };

            assert_eq!(constant_expression_text(parsed, expression), "(a?b)");
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedColonInLabel("conditional expression", _)
            )));
        },
    );
}

#[test]
fn case_recovery_matches_an_outer_question_after_closed_inner_nesting() {
    with_parse(
        "int f(void) { switch (value) { case a ? (b ? c) : d : ; } }\n",
        |parsed| {
            let [BlockItem::Statement(switch)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one switch statement")
            };
            let StatementType::Switch { body_statement, .. } = parsed.parser.syntax[switch].kind
            else {
                panic!("expected switch syntax")
            };
            let [BlockItem::Statement(case)] = block_items(parsed, body_statement) else {
                panic!("expected one case label")
            };
            let StatementType::Case(ConstantExpressionSlot::Parsed(expression), _) =
                parsed.parser.syntax[case].kind
            else {
                panic!("expected a parsed case expression")
            };

            assert_eq!(constant_expression_text(parsed, expression), "a?(b?c):d");
        },
    );
}

#[test]
fn stray_else_consumes_its_token_and_preserves_following_items() {
    with_parse("int f(void) { else; return; }\nint after;\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(first) if matches!(
            parsed.parser.syntax[first].kind,
            StatementType::Null
        )));
        assert!(matches!(items[1], BlockItem::Statement(second) if matches!(
            parsed.parser.syntax[second].kind,
            StatementType::Null
        )));
        assert!(matches!(items[2], BlockItem::Statement(third) if matches!(
            parsed.parser.syntax[third].kind,
            StatementType::Return(None)
        )));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedStatement(Some(TokenType::Keyword(KeywordTokenType::Else)))
        )));
        assert_eq!(
            identifier_name(
                parsed,
                init_declarators(parsed, declaration(parsed, 1))[0].declarator,
            )
            .as_deref(),
            Some("after")
        );
    });
}

#[test]
fn duplicate_default_is_diagnosed_per_innermost_switch() {
    with_parse(
        "int f(void) { switch (outer) { default: ; switch (inner) { default: ; } default: ; } }\n",
        |parsed| {
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(error, ParserErrorType::DuplicateDefaultLabel))
                    .count(),
                1
            );
            assert!(matches!(
                parsed.items.first(),
                Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
            ));
            assert!(parsed.parser.switch_scopes.is_empty());
        },
    );
}

#[test]
fn for_slots_distinguish_absent_expressions_and_declarations() {
    with_parse(
        "int f(void) {\nfor (;;) ;\nfor (start; ; ) ;\nfor (; condition; ) ;\nfor (; ; step) \
         ;\nfor (int item; condition; step) ;}\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            let forms = items
                .iter()
                .map(|item| {
                    let BlockItem::Statement(index) = item else {
                        panic!("expected for statement")
                    };
                    let StatementType::For {
                        initializer,
                        condition_expression,
                        iteration_expression,
                        ..
                    } = parsed.parser.syntax[index].kind
                    else {
                        panic!("expected for syntax")
                    };
                    (
                        initializer.map(|initializer| {
                            matches!(initializer, ForInitializer::Declaration(_))
                        }),
                        condition_expression.is_some(),
                        iteration_expression.is_some(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                forms,
                [
                    (None, false, false),
                    (Some(false), false, false),
                    (None, true, false),
                    (None, false, true),
                    (Some(true), true, true),
                ]
            );
        },
    );
}

#[test]
fn missing_expressions_remain_distinct_from_present_children() {
    with_parse(
        "int f(void) { for () ; if (;) ; case ; return }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            let [
                BlockItem::Statement(for_statement),
                BlockItem::Statement(if_statement),
                BlockItem::Statement(case_statement),
                BlockItem::Statement(return_statement),
            ] = items
            else {
                panic!("expected four recovered statements: {items:#?}")
            };

            let StatementType::For {
                initializer,
                condition_expression,
                iteration_expression,
                ..
            } = parsed.parser.syntax[for_statement].kind
            else {
                panic!("expected a for statement")
            };
            assert!(initializer.is_none());
            assert!(condition_expression.is_none());
            assert!(iteration_expression.is_none());

            let StatementType::If {
                condition_expression: ExpressionSlot::Missing(if_expression),
                ..
            } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected an if statement with a missing expression")
            };
            let StatementType::Case(ConstantExpressionSlot::Missing(case_expression), _) =
                parsed.parser.syntax[case_statement].kind
            else {
                panic!("expected a case statement with a missing expression")
            };
            assert_eq!(sourced_text(parsed, if_expression), "");
            assert_eq!(sourced_text(parsed, case_expression), "");
            assert!(matches!(
                parsed.parser.syntax[return_statement].kind,
                StatementType::Return(None)
            ));
        },
    );
}

#[test]
fn expression_recovery_preserves_following_statement_keywords() {
    with_parse("int f(void) { return value break; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(Some(ExpressionSlot::Parsed(_)))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement("return statement", Some(_))
        )));
    });
}

#[test]
fn typedef_named_members_remain_inside_expressions() {
    with_parse(
        "typedef int T; struct S { int T; }; int f(struct S *p) { return p->T; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 2).body);

            assert_eq!(items.len(), 1);
            let BlockItem::Statement(statement) = items[0] else {
                panic!("expected a return statement")
            };
            let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) =
                parsed.parser.syntax[statement].kind
            else {
                panic!("expected a parsed return expression")
            };
            assert_eq!(expression_text(parsed, expression), "p->T");
        },
    );
}

#[test]
fn typedef_spellings_remain_primary_expressions_until_semantic_analysis() {
    with_parse(
        "typedef int T; int value = T + 1; int product = T * ptr; int sum = 1 + T * ptr; int \
         f(void) { return T * ptr; }\n",
        |parsed| {
            let initializer = init_declarators(parsed, declaration(parsed, 1))[0]
                .initializer
                .expect("value must have an initializer");
            let InitializerType::AssignmentExpression(initializer_expression) =
                parsed.parser.syntax[initializer].kind
            else {
                panic!("expected a scalar initializer")
            };
            assert_eq!(expression_text(parsed, initializer_expression), "T+1");

            let initializer = init_declarators(parsed, declaration(parsed, 2))[0]
                .initializer
                .expect("product must have an initializer");
            let InitializerType::AssignmentExpression(initializer_expression) =
                parsed.parser.syntax[initializer].kind
            else {
                panic!("expected a scalar initializer")
            };
            assert_eq!(expression_text(parsed, initializer_expression), "T*ptr");

            let initializer = init_declarators(parsed, declaration(parsed, 3))[0]
                .initializer
                .expect("sum must have an initializer");
            let InitializerType::AssignmentExpression(initializer_expression) =
                parsed.parser.syntax[initializer].kind
            else {
                panic!("expected a scalar initializer")
            };
            assert_eq!(expression_text(parsed, initializer_expression), "1+T*ptr");

            let items = block_items(parsed, function_definition(parsed, 4).body);
            let [BlockItem::Statement(statement)] = items else {
                panic!("expected one return statement: {items:#?}")
            };
            let StatementType::Return(Some(ExpressionSlot::Parsed(return_expression))) =
                parsed.parser.syntax[statement].kind
            else {
                panic!("expected a parsed return expression")
            };
            assert_eq!(expression_text(parsed, return_expression), "T*ptr");
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn unary_compound_literal_classification_recovers_without_scanning_ahead() {
    with_parse(
        "typedef int T; int f(void) { ++(T; int after; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 1).body);

            assert_eq!(items.len(), 2, "{items:#?}");
            assert!(matches!(items[0], BlockItem::Statement(_)));
            assert!(matches!(items[1], BlockItem::Declaration(_)));
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisInStatement("type name", _)
            )));
        },
    );
}

#[test]
fn bare_return_recovery_preserves_following_statement_keywords() {
    with_parse("int f(void) { return break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement(
                "return statement",
                Some(TokenType::Keyword(KeywordTokenType::Break))
            )
        )));
    });
}

#[test]
fn bare_return_recovery_preserves_following_declarations() {
    with_parse("int f(void) { return int saved; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[1], BlockItem::Declaration(_)));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn bare_return_recovery_preserves_following_identifier_labels() {
    with_parse("int f(void) { return label: ; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Label(_, _)
        )));
    });
}

#[test]
fn block_declaration_recovery_preserves_following_statement_keywords() {
    with_parse("int f(void) { int value return; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Declaration(_)));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                Some(TokenType::Keyword(KeywordTokenType::Return)),
                _
            )
        )));
    });
}

#[test]
fn expression_recovery_preserves_following_declarations() {
    with_parse("int f(void) { value int saved; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        )));
        assert!(matches!(items[1], BlockItem::Declaration(_)));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
    });
}

#[test]
fn initializer_recovery_preserves_following_typedef_led_declarations() {
    with_parse(
        "typedef int T; int f(void) { int x = 1 + T y; return y; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 1).body);

            assert_eq!(items.len(), 3);
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert!(matches!(items[1], BlockItem::Declaration(_)));
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            )));
        },
    );
}

#[test]
fn typedef_spelled_struct_member_declarators_are_not_consumed_as_specifiers() {
    with_parse(
        "typedef int T; struct First { int T; }; struct Second { T T; };\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 3, "{:#?}", parsed.items);
            assert_eq!(
                parsed
                    .parser
                    .syntax
                    .iter::<StructDeclarator<'_>>()
                    .filter_map(|member| member
                        .declarator
                        .and_then(|declarator| identifier_name(parsed, declarator)))
                    .collect::<Vec<_>>(),
                ["T", "T"]
            );
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn expression_recovery_preserves_following_identifier_labels() {
    with_parse("int f(void) { value label: ; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Label(_, _)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
    });
}

#[test]
fn malformed_expression_recovery_preserves_following_identifier_labels() {
    with_parse("int f(void) { x = 1 + label: return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Label(_, _)
        )));
    });
}

#[test]
fn grouped_expression_recovery_preserves_following_identifier_labels() {
    with_parse("int f(void) { return (1 + label: ; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(Some(ExpressionSlot::Parsed(_)))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Label(_, _)
        )));
    });
}

#[test]
fn conditional_operands_are_not_recovered_as_identifier_labels() {
    with_parse(
        "int f(int c, int x, int y) { c ? x : y; return; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 2);
            let BlockItem::Statement(expression) = items[0] else {
                panic!("expected an expression statement")
            };
            let StatementType::Expression(ExpressionSlot::Parsed(source)) =
                parsed.parser.syntax[expression].kind
            else {
                panic!("expected a parsed expression")
            };
            assert_eq!(expression_text(parsed, source), "c?x:y");
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(None)
            )));
        },
    );
}

#[test]
fn block_declaration_recovery_preserves_following_identifier_labels() {
    with_parse("int f(void) { int value label: ; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Declaration(_)));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Label(_, _)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
    });
}

#[test]
fn malformed_block_items_preserve_following_compound_statements() {
    for source in [
        "int f(void) { int x { return; } break; }\n",
        "int f(void) { value { return; } break; }\n",
        "int f(void) { return + { return; } break; }\n",
    ] {
        with_parse(source, |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 3, "{items:#?}");
            let BlockItem::Statement(compound) = items[1] else {
                panic!("expected a preserved compound statement: {items:#?}")
            };
            let nested_items = block_items(parsed, compound);
            assert!(
                matches!(nested_items, [BlockItem::Statement(index)] if matches!(
                    parsed.parser.syntax[index].kind,
                    StatementType::Return(None)
                ))
            );
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Break
            )));
        });
    }
}

#[test]
fn block_brace_initializers_remain_in_the_declaration() {
    with_parse("int f(void) { int x = { 1 }; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        let BlockItem::Declaration(declaration) = items[0] else {
            panic!("expected a block declaration")
        };
        let initializer = *init_declarators(parsed, &parsed.parser.syntax[declaration])[0]
            .initializer
            .as_ref()
            .expect("parsed initializer");
        let initializer = &parsed.parser.syntax[initializer];
        assert!(matches!(
            initializer.kind,
            InitializerType::InitializerList(_)
        ));
        assert_eq!(sourced_text(parsed, initializer.source_vectors), "{1}");
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
    });
}

#[test]
fn block_initializer_recovery_preserves_following_statement_keywords() {
    with_parse("int f(void) { int x = 1 return; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 3);
        let BlockItem::Declaration(declaration) = items[0] else {
            panic!("expected a recovered block declaration")
        };
        assert!(parsed.parser.syntax[declaration].recovered);
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn block_initializer_recovery_preserves_statement_keyword_after_operator() {
    with_parse("int f(void) { int x = 1 + return; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 3, "{items:#?}");
        let BlockItem::Declaration(declaration) = items[0] else {
            panic!("expected a recovered block declaration")
        };
        assert!(parsed.parser.syntax[declaration].recovered);
        assert!(matches!(
            items[1],
            BlockItem::Statement(index)
                if matches!(
                    parsed.parser.syntax[index].kind,
                    StatementType::Return(None)
                )
        ));
        assert!(matches!(
            items[2],
            BlockItem::Statement(index)
                if matches!(
                    parsed.parser.syntax[index].kind,
                    StatementType::Break
                )
        ));
    });
}

#[test]
fn block_initializer_recovery_preserves_a_following_compound_statement() {
    with_parse(
        "int f(void) { int x = {1} { return; } break; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 3);
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Compound { .. }
            )));
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Break
            )));
        },
    );
}

#[test]
fn call_parentheses_do_not_hide_a_following_statement_body() {
    with_parse("int f(void) { if (foo() { return; } break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        let BlockItem::Statement(if_statement) = items[0] else {
            panic!("expected an if statement")
        };
        let StatementType::If { then_statement, .. } = parsed.parser.syntax[if_statement].kind
        else {
            panic!("expected an if statement")
        };
        assert!(matches!(
            parsed.parser.syntax[then_statement].kind,
            StatementType::Compound { .. }
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn sizeof_type_parentheses_do_not_hide_a_following_statement_body() {
    with_parse(
        "int f(void) { if (sizeof(int) { return; } break; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 2);
            let BlockItem::Statement(if_statement) = items[0] else {
                panic!("expected an if statement")
            };
            let StatementType::If { then_statement, .. } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected an if statement")
            };
            assert!(matches!(
                parsed.parser.syntax[then_statement].kind,
                StatementType::Compound { .. }
            ));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Break
            )));
        },
    );
}

#[test]
fn sizeof_type_parentheses_do_not_hide_a_declaration_led_body() {
    with_parse(
        "int f(void) { if (sizeof(int) { int y; } break; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 2);
            let BlockItem::Statement(if_statement) = items[0] else {
                panic!("expected an if statement")
            };
            let StatementType::If { then_statement, .. } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected an if statement")
            };
            assert!(matches!(
                parsed.parser.syntax[then_statement].kind,
                StatementType::Compound { .. }
            ));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Break
            )));
        },
    );
}

#[test]
fn sizeof_type_parentheses_can_introduce_a_compound_literal_operand() {
    with_parse("int f(void) { return sizeof (int){1}; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 1);
        let BlockItem::Statement(statement) = items[0] else {
            panic!("expected a return statement")
        };
        let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) =
            parsed.parser.syntax[statement].kind
        else {
            panic!("expected a parsed return expression")
        };
        assert_eq!(expression_text(parsed, expression), "sizeof(int){1}");
    });
}

#[test]
fn statement_recovery_preserves_following_identifier_labels() {
    with_parse(
        "int f(void) { if (x) int y label: ; return; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 3);
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Label(_, _)
            )));
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(None)
            )));
        },
    );
}

#[test]
fn statement_recovery_preserves_following_statement_keywords() {
    with_parse("int f(void) { if (x) int y return; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 3);
        let BlockItem::Statement(if_statement) = items[0] else {
            panic!("expected an if statement")
        };
        let StatementType::If { then_statement, .. } = parsed.parser.syntax[if_statement].kind
        else {
            panic!("expected an if statement")
        };
        assert!(parsed.parser.syntax[then_statement].recovered);
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn compound_literals_survive_parenthesized_expression_recovery() {
    with_parse(
        "struct S { int x; };\nint f(void) { if ((struct S){0}.x) ; return; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 1).body);
            assert_eq!(items.len(), 2);
            let BlockItem::Statement(if_statement) = items[0] else {
                panic!("expected an if statement")
            };
            let StatementType::If {
                condition_expression: ExpressionSlot::Parsed(expression),
                then_statement,
                else_statement: None,
            } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected a parsed if condition without an else branch")
            };
            assert!(expression_text(parsed, expression).contains("{0}"));
            assert!(matches!(
                parsed.parser.syntax[then_statement].kind,
                StatementType::Null
            ));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(None)
            )));
        },
    );
}

#[test]
fn compound_literal_type_names_do_not_mark_outer_grouping_parentheses() {
    with_parse(
        "struct S { int x; };\nint f(void) { ((struct S){0}) { return; } break; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 1).body);

            assert_eq!(items.len(), 3);
            assert!(matches!(items[0], BlockItem::Statement(_)));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Compound { .. }
            )));
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Break
            )));
        },
    );
}

#[test]
fn malformed_condition_preserves_the_following_body_brace() {
    with_parse("int f(void) { if (value { return; } break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(if_statement) = items[0] else {
            panic!("expected an if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: None,
            ..
        } = parsed.parser.syntax[if_statement].kind
        else {
            panic!("expected a recovered if statement")
        };
        assert!(matches!(
            parsed.parser.syntax[then_statement].kind,
            StatementType::Compound { .. }
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn malformed_for_declaration_recovery_preserves_the_header_close() {
    with_parse("int f(void) { for (int i) ; return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(for_statement) = items[0] else {
            panic!("expected a for statement")
        };
        let StatementType::For {
            initializer: Some(ForInitializer::Declaration(_)),
            condition_expression: None,
            iteration_expression: None,
            body_statement,
        } = parsed.parser.syntax[for_statement].kind
        else {
            panic!("expected a recovered declaration-form for statement")
        };
        assert!(matches!(
            parsed.parser.syntax[body_statement].kind,
            StatementType::Null
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Return(None)
        )));
    });
}

#[test]
fn malformed_for_declaration_preserves_a_statement_body() {
    with_parse("int f(void) { for (int x return; break; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2);
        let BlockItem::Statement(for_statement) = items[0] else {
            panic!("expected a for statement")
        };
        let StatementType::For {
            initializer: Some(ForInitializer::Declaration(_)),
            body_statement,
            ..
        } = parsed.parser.syntax[for_statement].kind
        else {
            panic!("expected a recovered declaration-form for statement")
        };
        assert!(
            matches!(
                parsed.parser.syntax[body_statement].kind,
                StatementType::Return(None)
            ),
            "{:#?}",
            parsed.parser.syntax[body_statement]
        );
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax[index].kind,
            StatementType::Break
        )));
    });
}

#[test]
fn malformed_for_initializer_recovery_preserves_the_header_close() {
    with_parse(
        "int f(void) { for (int i = value) ; return; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            assert_eq!(items.len(), 2);
            let BlockItem::Statement(for_statement) = items[0] else {
                panic!("expected a for statement")
            };
            let StatementType::For {
                initializer: Some(ForInitializer::Declaration(declaration)),
                condition_expression: None,
                iteration_expression: None,
                body_statement,
            } = parsed.parser.syntax[for_statement].kind
            else {
                panic!("expected a recovered declaration-form for statement")
            };
            assert_eq!(parsed.parser.syntax[declaration].init_declarators.length, 1);
            assert!(!parsed.parser.syntax[declaration].recovered);
            assert!(
                init_declarators(parsed, &parsed.parser.syntax[declaration],)[0]
                    .initializer
                    .is_some()
            );
            assert!(matches!(
                parsed.parser.syntax[body_statement].kind,
                StatementType::Null
            ));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(None)
            )));
        },
    );
}

#[test]
fn malformed_for_declaration_preserves_a_compound_body() {
    with_parse(
        "int f(void) { for (int x junk { x++; } return 0; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);

            assert_eq!(items.len(), 2, "{items:#?}");
            let BlockItem::Statement(for_statement) = items[0] else {
                panic!("expected a for statement")
            };
            let StatementType::For { body_statement, .. } =
                parsed.parser.syntax[for_statement].kind
            else {
                panic!("expected a recovered for statement")
            };
            assert!(matches!(
                parsed.parser.syntax[body_statement].kind,
                StatementType::Compound { .. }
            ));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                parsed.parser.syntax[index].kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            )));
        },
    );
}

#[test]
fn function_block_implicit_and_label_scopes_restore_typedef_classification() {
    with_parse(
        "typedef int T; typedef int __func__;\nint f(int T) { T; __func__; T: ; goto T; { typedef \
         int U; U value; }\nif (T) ; while (T) ; for (int T; ; ) ; }\nT after; __func__ outside;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 5);
            let definition = function_definition(parsed, 2);
            let items = block_items(parsed, definition.body);
            assert!(matches!(items[0], BlockItem::Statement(_)));
            assert!(matches!(items[1], BlockItem::Statement(_)));
            assert!(matches!(items[2], BlockItem::Statement(_)));
            assert!(matches!(items[3], BlockItem::Statement(_)));
            assert!(parsed.parser.scopes.nested_scopes.is_empty());
            assert!(parsed.parser.label_scopes.is_empty());

            for kind in [
                ScopeKind::FunctionPrototype,
                ScopeKind::Function,
                ScopeKind::Block,
                ScopeKind::ImplicitSelection,
                ScopeKind::ImplicitIteration,
            ] {
                assert!(
                    parsed
                        .parser
                        .scopes
                        .trace
                        .iter()
                        .any(|event| event.kind == kind && event.enter)
                );
                assert_eq!(
                    parsed
                        .parser
                        .scopes
                        .trace
                        .iter()
                        .filter(|event| event.kind == kind && event.enter)
                        .count(),
                    parsed
                        .parser
                        .scopes
                        .trace
                        .iter()
                        .filter(|event| event.kind == kind && !event.enter)
                        .count(),
                    "scope kind {kind:?} leaked"
                );
            }
            assert!(
                declaration(parsed, 3)
                    .declaration_specifiers
                    .type_specifiers
                    .is_typedef_name()
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
fn malformed_statement_delimiters_recover_the_body_and_next_file_item() {
    for source in [
        "int f(void) { expression } int after;\n",
        "int f(void) { return value } int after;\n",
        "int f(void) { if (value ; } int after;\n",
        "int f(void) { case value ; } int after;\n",
        "int f(void) { do ; while (value) } int after;\n",
        "int f(void) { for (value; value; value ; } int after;\n",
    ] {
        with_parse(source, |parsed| {
            assert!(
                matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
                ),
                "missing recovered function for {source:?}: {:#?}",
                parsed.items
            );
            assert!(
                matches!(
                    parsed.items.get(1),
                    Some(ExternalDeclaration::Declaration(_))
                ),
                "recovery swallowed the next declaration for {source:?}: {:#?}",
                parsed.items
            );
            assert_eq!(
                identifier_name(
                    parsed,
                    init_declarators(parsed, declaration(parsed, 1))[0].declarator,
                )
                .as_deref(),
                Some("after")
            );
            assert!(parsed.parser.scopes.nested_scopes.is_empty());
            assert!(parsed.parser.label_scopes.is_empty());
            assert!(parsed.errors.iter().all(|error| match error {
                | TranslationError::Parsing(error) => error.source_vectors.length > 0,
                | _ => true,
            }));
        });
    }

    with_parse("int f(void) { if (value) return;", |eof| {
        assert!(matches!(
            eof.items.first(),
            Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
        ));
        assert!(parser_errors(eof).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None)
        )));
        assert!(eof.parser.scopes.nested_scopes.is_empty());
        assert!(eof.parser.label_scopes.is_empty());
    });
}

#[test]
fn premature_eof_unwinds_every_phase_03_frame_family() {
    for (source, expected_error) in [
        (
            "int f(void) {",
            ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None),
        ),
        (
            "int f(void) { if (",
            ParserErrorType::ExpectedStatementExpression("if statement", None),
        ),
        (
            "int f(void) { if (condition",
            ParserErrorType::ExpectedClosingParenthesisInStatement("if statement", None),
        ),
        (
            "int f(void) { case value",
            ParserErrorType::ExpectedColonInLabel("case label", None),
        ),
        (
            "int f(void) { label:",
            ParserErrorType::ExpectedStatement(None),
        ),
        (
            "int f(void) { goto",
            ParserErrorType::ExpectedGotoLabel(None),
        ),
        (
            "int f(void) { do ;",
            ParserErrorType::ExpectedWhileAfterDoBody(None),
        ),
        (
            "int f(void) { for (",
            ParserErrorType::ExpectedSemicolonInStatement("for initializer", None),
        ),
        (
            "int f(parameter) int parameter;",
            ParserErrorType::ExpectedFunctionBody(None),
        ),
    ] {
        with_parse(source, |parsed| {
            assert!(
                matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
                ),
                "expected a recovered function for {source:?}: {:#?}",
                parsed.items
            );
            assert!(
                parser_errors(parsed).any(|error| error == &expected_error),
                "missing {expected_error:?} for {source:?}: {:#?}",
                parsed.errors
            );
            assert!(parsed.errors.iter().all(|error| match error {
                | TranslationError::Parsing(error) => {
                    error.source_vectors.length > 0
                        && !matches!(
                            error.error_type,
                            ParserErrorType::ParserFrameConsumedAtEndOfInput(_)
                        )
                },
                | _ => true,
            }));
            let definition = function_definition(parsed, 0);
            assert_ne!(
                parsed.context.get_source_vectors(definition.source_vectors),
                []
            );
            assert_ne!(
                parsed
                    .context
                    .get_source_vectors(parsed.parser.syntax[definition.body].source_vectors),
                []
            );
            assert!(parsed.parser.scopes.nested_scopes.is_empty(), "{source:?}");
            assert!(parsed.parser.label_scopes.is_empty(), "{source:?}");
            assert!(parsed.parser.switch_scopes.is_empty(), "{source:?}");
        });
    }
}

#[test]
fn synthesized_missing_statements_are_anchored_at_the_recovery_point() {
    for source in ["int f(void) { if (x) }", "int f(void) { if (x)"] {
        with_parse(source, |parsed| {
            let definition = function_definition(parsed, 0);
            let [BlockItem::Statement(if_statement)] = block_items(parsed, definition.body) else {
                panic!("expected one if statement for {source:?}")
            };
            let StatementType::If { then_statement, .. } = parsed.parser.syntax[if_statement].kind
            else {
                panic!("expected an if statement for {source:?}")
            };
            let [missing_source] = parsed
                .context
                .get_source_vectors(parsed.parser.syntax[then_statement].source_vectors)
            else {
                panic!("expected one missing-statement anchor for {source:?}")
            };

            // At the token that ends the statement, or at the end of input; not
            // wherever the preprocessor happens to have read ahead to.
            let recovery_point = source.find('}').unwrap_or(source.len());
            assert_eq!(missing_source.index as usize, recovery_point);
            assert_eq!(missing_source.length, 0);
        });
    }
}

#[test]
fn missing_function_body_at_eof_retains_a_zero_width_source_location() {
    let source = "int f(parameter) int parameter;";
    with_parse(source, |parsed| {
        let definition = function_definition(parsed, 0);
        let body = &parsed.parser.syntax[definition.body];
        let [body_source] = parsed.context.get_source_vectors(body.source_vectors) else {
            panic!("expected one source vector for the recovered function body")
        };

        assert!(body.recovered);
        assert_eq!(body_source.index as usize, source.len());
        assert_eq!(body_source.length, 0);
    });
}

#[test]
fn statement_delimiter_trace_names_the_owning_frame() {
    with_parse(
        "int f(void) { int item; if (condition) return; }\n",
        |parsed| {
            let brace_events = parsed.parser.trace.iter().filter(|event| {
                event.action == "consume"
                    && matches!(
                        event.token,
                        Some(TokenType::Operator(
                            OperatorTokenType::OpeningCurlyBrace
                                | OperatorTokenType::ClosingCurlyBrace
                        ))
                    )
            });
            assert!(
                brace_events
                    .clone()
                    .all(|event| event.frame == "compound-statement")
            );
            assert_eq!(brace_events.count(), 2);

            assert!(parsed.parser.trace.iter().any(|event| {
                event.frame == "declaration"
                    && event.action == "consume"
                    && event.token == Some(TokenType::Operator(OperatorTokenType::Semicolon))
            }));
            assert!(parsed.parser.trace.iter().any(|event| {
                event.frame == "statement"
                    && event.action == "consume"
                    && event.token
                        == Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
            }));
        },
    );
}

#[test]
fn blocks_and_definition_parameters_meet_the_c99_translation_floor() {
    let nested = format!(
        "int deep(void) {{{}{}{} }}\n",
        "{".repeat(127),
        ";",
        "}".repeat(127)
    );
    with_parse(&nested, |parsed| {
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::FunctionDefinition(_))
        ));
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .map(|event| event.depth)
                .max()
                .is_some_and(|depth| depth > 127)
        );

        let parameters = (0..127)
            .map(|index| format!("int parameter_{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        with_parse(
            &format!("int many({parameters}) {{ return; }}\n"),
            |parsed| {
                assert!(matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::FunctionDefinition(_))
                ));
                assert_eq!(
                    parsed.parser.syntax.count::<ParameterDeclaration<'_>>(),
                    127
                );
                assert!(
                    parser_errors(parsed).next().is_none(),
                    "{:#?}",
                    parsed.errors
                );
            },
        );
    });
}
