//! Expressions, type names, and initializers.

use super::{
    block_items,
    constant_expression_text,
    declaration,
    expression_text,
    function_definition,
    parser_errors,
    return_expression,
    sourced_text,
    with_parse,
};
use crate::translation_phases::{
    parsing::{
        declaration_syntax::{
            Designator,
            DesignatorType,
            Initializer,
            InitializerElement,
            InitializerType,
            TypeName,
        },
        errors::ParserErrorType,
        syntax::{
            BinaryOperator,
            BlockItem,
            Constant,
            Expression,
            ExpressionSlot,
            ExpressionType,
            ExternalDeclaration,
            Statement,
            StatementType,
            UnaryOperator,
        },
    },
    preprocessing::{
        IntegerTokenType,
        OperatorTokenType,
        TokenType,
    },
};

#[test]
fn primary_expressions_are_parsed_in_expression_and_return_statements() {
    with_parse("int f(void) { value; return 1; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        let BlockItem::Statement(expression_statement) = items[0] else {
            panic!("expected expression statement");
        };
        let StatementType::Expression(ExpressionSlot::Parsed(identifier)) =
            parsed.parser.syntax[expression_statement].kind
        else {
            panic!("expected parsed identifier expression");
        };
        assert!(matches!(
            parsed.parser.syntax[identifier].kind,
            ExpressionType::Identifier(_)
        ));

        let BlockItem::Statement(return_statement) = items[1] else {
            panic!("expected return statement");
        };
        let StatementType::Return(Some(ExpressionSlot::Parsed(integer))) =
            parsed.parser.syntax[return_statement].kind
        else {
            panic!("expected parsed return expression");
        };
        assert!(matches!(
            parsed.parser.syntax[integer].kind,
            ExpressionType::Constant(Constant::Integer(IntegerTokenType::Int(1)))
        ));
        assert!(parser_errors(parsed).next().is_none());
    });
}

#[test]
fn postfix_expression_suffixes_compose_left_to_right() {
    with_parse(
        "int f(void) { factory()(x)[i].field->member++; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            let BlockItem::Statement(statement) = items[0] else {
                panic!("expected expression statement");
            };
            let StatementType::Expression(ExpressionSlot::Parsed(expression)) =
                parsed.parser.syntax[statement].kind
            else {
                panic!("expected parsed postfix expression");
            };
            let ExpressionType::Unary {
                operator: UnaryOperator::PostIncrement,
                operand_expression,
            } = parsed.parser.syntax[expression].kind
            else {
                panic!("expected postfix increment at the root");
            };
            assert!(matches!(
                parsed.parser.syntax[operand_expression].kind,
                ExpressionType::IndirectMember { .. }
            ));
            assert!(parser_errors(parsed).next().is_none());
        },
    );
}

#[test]
fn function_arguments_keep_postfix_calls_and_increments() {
    with_parse(
        "int f(void) { return outer(inner(), value++); }\n",
        |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one return statement")
            };
            let root = return_expression(parsed, *statement);
            let ExpressionType::Call { arguments, .. } = parsed.parser.syntax[root].kind else {
                panic!("expected an outer call")
            };
            let arguments = &parsed.parser.syntax[arguments];

            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                parsed.parser.syntax[arguments[0]].kind,
                ExpressionType::Call { .. }
            ));
            assert!(matches!(
                parsed.parser.syntax[arguments[1]].kind,
                ExpressionType::Unary {
                    operator: UnaryOperator::PostIncrement,
                    ..
                }
            ));
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn function_arguments_keep_binary_operators_inside_each_argument() {
    with_parse(
        "int f(void) { return outer(1+2, 3-4, 5*6, 7&8); }\n",
        |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one return statement")
            };
            let root = return_expression(parsed, *statement);
            let ExpressionType::Call { arguments, .. } = parsed.parser.syntax[root].kind else {
                panic!("expected a function call")
            };
            let arguments = &parsed.parser.syntax[arguments];

            assert_eq!(arguments.len(), 4);
            assert_eq!(
                arguments
                    .iter()
                    .map(|expression| expression_text(parsed, expression))
                    .collect::<Vec<_>>(),
                ["1+2", "3-4", "5*6", "7&8"]
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
fn return_expression_keeps_a_typedef_spelled_call() {
    with_parse(
        "typedef int T; int f(int *p) { return T(*p); }\n",
        |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(parsed, function_definition(parsed, 1).body)
            else {
                panic!("expected one return statement")
            };
            let root = return_expression(parsed, *statement);

            assert!(matches!(
                parsed.parser.syntax[root].kind,
                ExpressionType::Call { .. }
            ));
            assert_eq!(expression_text(parsed, root), "T(*p)");
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn nested_expression_recovery_honors_the_enclosing_semicolon() {
    with_parse("int f(void) { ( ; return 1; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2, "{items:#?}");
        assert!(matches!(items[0], BlockItem::Statement(_)));
        assert!(
            matches!(items[1], BlockItem::Statement(statement) if matches!(
                parsed.parser.syntax[statement].kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            ))
        );
        assert!(!parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement("expression statement", _)
        )));
    });
}

#[test]
fn conditional_middle_recovery_honors_enclosing_boundaries() {
    with_parse("int f(void) { return 1 ? ; return 2; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2, "{items:#?}");
        assert!(matches!(items[0], BlockItem::Statement(_)));
        let BlockItem::Statement(following_return) = items[1] else {
            panic!("expected the following return statement")
        };
        let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) =
            parsed.parser.syntax[following_return].kind
        else {
            panic!("expected an exact following return expression")
        };
        assert_eq!(expression_text(parsed, expression), "2");
        assert!(!parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement("return statement", _)
        )));
    });
    with_parse("int f(void) { if (1 ? ) return 2; }\n", |parenthesis| {
        let [BlockItem::Statement(if_statement)] =
            block_items(parenthesis, function_definition(parenthesis, 0).body)
        else {
            panic!("expected one recovered if statement")
        };
        let StatementType::If { then_statement, .. } = parenthesis.parser.syntax[if_statement].kind
        else {
            panic!("expected recovered if syntax")
        };
        assert!(matches!(
            parenthesis.parser.syntax[then_statement].kind,
            StatementType::Return(Some(ExpressionSlot::Parsed(_)))
        ));
        assert!(!parser_errors(parenthesis).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingParenthesisInStatement("if statement", _)
        )));
    });
    with_parse(
        "int f(void) { return array[1 ? ]; return 2; }\n",
        |bracket| {
            let items = block_items(bracket, function_definition(bracket, 0).body);
            assert_eq!(items.len(), 2, "{items:#?}");
            assert!(
                matches!(items[1], BlockItem::Statement(statement) if matches!(
                    bracket.parser.syntax[statement].kind,
                    StatementType::Return(Some(ExpressionSlot::Parsed(_)))
                ))
            );
            assert!(!parser_errors(bracket).any(|error| matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInSubscript(_)
            )));
        },
    );
    with_parse("int values[] = { 1 ? , 2 };\n", |initializer| {
        let declaration = declaration(initializer, 0);
        let [init_declarator] = declaration.init_declarators else {
            panic!("expected one initialized declarator")
        };
        let outer = init_declarator
            .initializer
            .expect("expected an initializer list");
        let InitializerType::InitializerList(elements) = initializer.parser.syntax[outer].kind
        else {
            panic!("expected an initializer list")
        };
        assert_eq!(elements.len(), 2);
        assert!(!parser_errors(initializer).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(_)
        )));
    });
}

#[test]
fn missing_square_brackets_name_the_owning_construct() {
    with_parse("int f(void) { return array[1; }\n", |subscript| {
        assert!(parser_errors(subscript).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInSubscript(Some(TokenType::Operator(
                OperatorTokenType::Semicolon
            )))
        )));
    });
    with_parse("int array[] = { [1 = 2 };\n", |designator| {
        assert!(parser_errors(designator).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(Some(
                TokenType::Operator(OperatorTokenType::Equals)
            ))
        )));
    });
}

#[test]
fn unary_cast_and_sizeof_forms_use_parsed_type_names() {
    with_parse(
        "typedef int T; int f(void) { -(T)value; sizeof value; return sizeof(T); }\n",
        |parsed| {
            let definition = function_definition(parsed, 1);
            let items = block_items(parsed, definition.body);
            let BlockItem::Statement(statement) = items[2] else {
                panic!("expected return statement");
            };
            let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) =
                parsed.parser.syntax[statement].kind
            else {
                panic!(
                    "expected parsed return expression: {:#?}",
                    parsed
                        .parser
                        .syntax
                        .iter::<Statement<'_>>()
                        .collect::<Vec<_>>()
                );
            };
            assert!(matches!(
                parsed.parser.syntax[expression].kind,
                ExpressionType::SizeofType(_)
            ));
            assert_eq!(parsed.parser.syntax.count::<TypeName<'_>>(), 2);
            assert!(parser_errors(parsed).next().is_none());
        },
    );
}

#[test]
fn type_names_retain_typedef_specifiers_after_primitive_specifiers() {
    with_parse(
        "typedef int T; int f(void) { return sizeof(int T) + sizeof(T int); }\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
            let definition = function_definition(parsed, 1);
            let items = block_items(parsed, definition.body);
            assert_eq!(items.len(), 1, "{items:#?}");
            assert_eq!(parsed.parser.syntax.count::<TypeName<'_>>(), 2);
            assert_eq!(
                sourced_text(
                    parsed,
                    parsed.parser.syntax.nth::<TypeName<'_>>(0).source_vectors
                ),
                "intT"
            );
            assert_eq!(
                sourced_text(
                    parsed,
                    parsed.parser.syntax.nth::<TypeName<'_>>(1).source_vectors
                ),
                "Tint"
            );
            assert_eq!(parser_errors(parsed).count(), 2, "{:#?}", parsed.errors);
        },
    );
}

#[test]
fn cast_and_sizeof_classification_tracks_typedef_shadowing() {
    with_parse(
        "typedef int T;\nint unshadowed(void) { return (T)1; }\nint parameter(int T) { return \
         sizeof(T); }\nint block(void) { int T; return (T); }\nint iteration(void) { for (int T; \
         ; ) return (T); }\n",
        |parsed| {
            let unshadowed = block_items(parsed, function_definition(parsed, 1).body);
            let BlockItem::Statement(unshadowed_return) = unshadowed[0] else {
                panic!("expected unshadowed return statement")
            };
            assert!(matches!(
                parsed.parser.syntax[return_expression(parsed, unshadowed_return)].kind,
                ExpressionType::Cast { .. }
            ));

            let parameter = block_items(parsed, function_definition(parsed, 2).body);
            let BlockItem::Statement(parameter_return) = parameter[0] else {
                panic!("expected parameter-shadowed return statement")
            };
            assert!(matches!(
                parsed.parser.syntax[return_expression(parsed, parameter_return)].kind,
                ExpressionType::SizeofExpr(_)
            ));

            let block = block_items(parsed, function_definition(parsed, 3).body);
            let BlockItem::Statement(block_return) = block[1] else {
                panic!("expected block-shadowed return statement")
            };
            assert!(matches!(
                parsed.parser.syntax[return_expression(parsed, block_return)].kind,
                ExpressionType::Parenthesized { .. }
            ));

            let iteration = block_items(parsed, function_definition(parsed, 4).body);
            let BlockItem::Statement(for_statement) = iteration[0] else {
                panic!("expected for statement")
            };
            let StatementType::For { body_statement, .. } =
                parsed.parser.syntax[for_statement].kind
            else {
                panic!("expected for statement syntax")
            };
            assert!(matches!(
                parsed.parser.syntax[return_expression(parsed, body_statement)].kind,
                ExpressionType::Parenthesized { .. }
            ));
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn declarator_binding_shadows_a_typedef_inside_its_own_initializer() {
    with_parse(
        "typedef int T; int f(void) { int T = sizeof(T); }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 1).body);
            let BlockItem::Declaration(declaration) = items[0] else {
                panic!("expected a block declaration")
            };
            let [init_declarator] = declaration.init_declarators else {
                panic!("expected one initialized declarator")
            };
            let initializer = init_declarator.initializer.expect("parsed initializer");
            let InitializerType::AssignmentExpression(expression) =
                parsed.parser.syntax[initializer].kind
            else {
                panic!("expected a scalar initializer")
            };

            assert!(matches!(
                parsed.parser.syntax[expression].kind,
                ExpressionType::SizeofExpr(_)
            ));
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn precedence_conditional_assignment_and_comma_contexts_are_distinct() {
    with_parse(
        "int f(void) { a = b = c; a ? (b, c) : d ? e : f; call(a, b); call((a, b)); }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            let roots = items
                .iter()
                .map(|item| {
                    let BlockItem::Statement(statement) = *item else {
                        panic!("expected expression statement");
                    };
                    let StatementType::Expression(ExpressionSlot::Parsed(expression)) =
                        parsed.parser.syntax[statement].kind
                    else {
                        panic!("expected parsed expression");
                    };
                    expression
                })
                .collect::<Vec<_>>();

            assert!(matches!(
                parsed.parser.syntax[roots[0]].kind,
                ExpressionType::Binary {
                    operator: BinaryOperator::Assignment,
                    right_expression,
                    ..
                } if matches!(
                    parsed.parser.syntax[right_expression].kind,
                    ExpressionType::Binary {
                        operator: BinaryOperator::Assignment,
                        ..
                    }
                )
            ));
            assert!(matches!(
                parsed.parser.syntax[roots[1]].kind,
                ExpressionType::Conditional { else_expression, .. }
                    if matches!(
                        parsed.parser.syntax[else_expression].kind,
                        ExpressionType::Conditional { .. }
                    )
            ));
            let argument_lengths = roots[2..]
                .iter()
                .map(|root| match parsed.parser.syntax[root].kind {
                    | ExpressionType::Call { arguments, .. } => arguments.len(),
                    | _ => panic!("expected call expression"),
                })
                .collect::<Vec<_>>();
            assert_eq!(argument_lengths, [2, 1]);
            assert!(parser_errors(parsed).next().is_none());
        },
    );
}

#[test]
fn conditional_middle_accepts_an_unparenthesized_comma_expression() {
    with_parse("int f(void) { return a ? b, c : d; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);
        let BlockItem::Statement(statement) = items[0] else {
            panic!("expected a return statement")
        };
        let root = return_expression(parsed, statement);
        let ExpressionType::Conditional {
            then_expression,
            else_expression,
            ..
        } = parsed.parser.syntax[root].kind
        else {
            panic!("expected a conditional expression")
        };

        assert!(matches!(
            parsed.parser.syntax[then_expression].kind,
            ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                ..
            }
        ));
        assert_eq!(expression_text(parsed, then_expression), "b,c");
        assert_eq!(expression_text(parsed, else_expression), "d");
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
    });
}

#[test]
fn missing_call_argument_comma_preserves_later_arguments() {
    with_parse("int f(void) { foo(a b, c); return; }\n", |parsed| {
        let items = block_items(parsed, function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2, "{items:#?}");
        let BlockItem::Statement(statement) = items[0] else {
            panic!("expected expression statement")
        };
        let StatementType::Expression(ExpressionSlot::Parsed(root)) =
            parsed.parser.syntax[statement].kind
        else {
            panic!("expected parsed expression")
        };
        let ExpressionType::Call { arguments, .. } = parsed.parser.syntax[root].kind else {
            panic!("expected call expression")
        };
        assert_eq!(
            parsed.parser.syntax[arguments]
                .iter()
                .map(|expression| expression_text(parsed, expression))
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionCall(Some(
                TokenType::Identifier
            ))
        )));
    });
}

#[test]
fn equal_precedence_chains_nested_conditionals_and_casts_keep_their_associativity() {
    with_parse(
        "int f(void) { a-b-c; a ? b ? c : d : e; (int)(long)a; a+b=c; }\n",
        |parsed| {
            let items = block_items(parsed, function_definition(parsed, 0).body);
            let roots = items
                .iter()
                .map(|item| {
                    let BlockItem::Statement(statement) = *item else {
                        panic!("expected expression statement");
                    };
                    let StatementType::Expression(ExpressionSlot::Parsed(expression)) =
                        parsed.parser.syntax[statement].kind
                    else {
                        panic!("expected parsed expression");
                    };
                    expression
                })
                .collect::<Vec<_>>();

            assert!(matches!(
                parsed.parser.syntax[roots[0]].kind,
                ExpressionType::Binary {
                    operator: BinaryOperator::Subtraction,
                    left_expression,
                    ..
                } if matches!(
                    parsed.parser.syntax[left_expression].kind,
                    ExpressionType::Binary {
                        operator: BinaryOperator::Subtraction,
                        ..
                    }
                )
            ));
            assert!(matches!(
                parsed.parser.syntax[roots[1]].kind,
                ExpressionType::Conditional { then_expression, .. }
                    if matches!(
                        parsed.parser.syntax[then_expression].kind,
                        ExpressionType::Conditional { .. }
                    )
            ));
            assert!(matches!(
                parsed.parser.syntax[roots[2]].kind,
                ExpressionType::Cast { operand_expression, .. }
                    if matches!(
                        parsed.parser.syntax[operand_expression].kind,
                        ExpressionType::Cast { .. }
                    )
            ));
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedStatementExpression(
                    "unary-expression left operand of assignment",
                    _
                )
            )));
        },
    );
}

#[test]
fn every_equal_precedence_operator_pair_has_explicit_tree_coverage() {
    let left_associative_groups: &[&[(&str, BinaryOperator)]] = &[
        &[
            ("*", BinaryOperator::Multiplication),
            ("/", BinaryOperator::Division),
            ("%", BinaryOperator::Modulo),
        ],
        &[
            ("+", BinaryOperator::Addition),
            ("-", BinaryOperator::Subtraction),
        ],
        &[
            ("<<", BinaryOperator::LeftShift),
            (">>", BinaryOperator::RightShift),
        ],
        &[
            ("<", BinaryOperator::LessThan),
            (">", BinaryOperator::GreaterThan),
            ("<=", BinaryOperator::LessThanOrEqual),
            (">=", BinaryOperator::GreaterThanOrEqual),
        ],
        &[
            ("==", BinaryOperator::Equal),
            ("!=", BinaryOperator::NotEqual),
        ],
        &[("&", BinaryOperator::BitwiseAnd)],
        &[("^", BinaryOperator::BitwiseXor)],
        &[("|", BinaryOperator::BitwiseOr)],
        &[("&&", BinaryOperator::LogicalAnd)],
        &[("||", BinaryOperator::LogicalOr)],
        &[(",", BinaryOperator::Comma)],
    ];
    for group in left_associative_groups {
        for &(first_spelling, first_operator) in *group {
            for &(second_spelling, second_operator) in *group {
                let source = format!("int f(void) {{ a{first_spelling}b{second_spelling}c; }}\n");
                with_parse(&source, |parsed| {
                    let [BlockItem::Statement(statement)] =
                        block_items(parsed, function_definition(parsed, 0).body)
                    else {
                        panic!("expected one expression statement for {source:?}")
                    };
                    let StatementType::Expression(ExpressionSlot::Parsed(root)) =
                        parsed.parser.syntax[statement].kind
                    else {
                        panic!("expected a parsed expression for {source:?}")
                    };
                    let ExpressionType::Binary {
                        operator,
                        left_expression,
                        ..
                    } = parsed.parser.syntax[root].kind
                    else {
                        panic!("expected a binary root for {source:?}")
                    };
                    assert_eq!(operator, second_operator, "wrong root for {source:?}");
                    assert!(matches!(
                        parsed.parser.syntax[left_expression].kind,
                        ExpressionType::Binary { operator, .. } if operator == first_operator
                    ));
                    assert!(
                        parser_errors(parsed).next().is_none(),
                        "{:#?}",
                        parsed.errors
                    );
                });
            }
        }
    }

    let assignment_operators = [
        ("=", BinaryOperator::Assignment),
        ("*=", BinaryOperator::MultiplicationAssignment),
        ("/=", BinaryOperator::DivisionAssignment),
        ("%=", BinaryOperator::ModuloAssignment),
        ("+=", BinaryOperator::AdditionAssignment),
        ("-=", BinaryOperator::SubtractionAssignment),
        ("<<=", BinaryOperator::LeftShiftAssignment),
        (">>=", BinaryOperator::RightShiftAssignment),
        ("&=", BinaryOperator::BitwiseAndAssignment),
        ("^=", BinaryOperator::BitwiseXorAssignment),
        ("|=", BinaryOperator::BitwiseOrAssignment),
    ];
    for (first_spelling, first_operator) in assignment_operators {
        for (second_spelling, second_operator) in assignment_operators {
            let source = format!("int f(void) {{ a{first_spelling}b{second_spelling}c; }}\n");
            with_parse(&source, |parsed| {
                let [BlockItem::Statement(statement)] =
                    block_items(parsed, function_definition(parsed, 0).body)
                else {
                    panic!("expected one assignment statement for {source:?}")
                };
                let StatementType::Expression(ExpressionSlot::Parsed(root)) =
                    parsed.parser.syntax[statement].kind
                else {
                    panic!("expected a parsed assignment for {source:?}")
                };
                let ExpressionType::Binary {
                    operator,
                    right_expression,
                    ..
                } = parsed.parser.syntax[root].kind
                else {
                    panic!("expected an assignment root for {source:?}")
                };
                assert_eq!(operator, first_operator, "wrong root for {source:?}");
                assert!(matches!(
                    parsed.parser.syntax[right_expression].kind,
                    ExpressionType::Binary { operator, .. } if operator == second_operator
                ));
                assert!(
                    parser_errors(parsed).next().is_none(),
                    "{:#?}",
                    parsed.errors
                );
            });
        }
    }
}

#[test]
fn typedef_spelled_expressions_remain_inside_braced_initializers() {
    for expression in ["T + 1", "T[0]", "T()", "T * value"] {
        with_parse(
            &format!("typedef int T; int values[] = {{ {expression} }}; int tail;\n"),
            |parsed| {
                assert!(matches!(
                    parsed.items.as_slice(),
                    [
                        ExternalDeclaration::Declaration(_),
                        ExternalDeclaration::Declaration(_),
                        ExternalDeclaration::Declaration(_)
                    ]
                ));
                let [init_declarator] = declaration(parsed, 1).init_declarators else {
                    panic!("expected one initialized declarator")
                };
                let initializer = init_declarator.initializer.expect("braced initializer");
                let InitializerType::InitializerList(elements) =
                    parsed.parser.syntax[initializer].kind
                else {
                    panic!("expected an initializer list")
                };
                let [element] = &parsed.parser.syntax[elements] else {
                    panic!("expected one initializer element")
                };
                let InitializerType::AssignmentExpression(expression_index) =
                    parsed.parser.syntax[element.initializer].kind
                else {
                    panic!("expected a scalar initializer element")
                };

                assert_eq!(
                    expression_text(parsed, expression_index),
                    expression.replace(' ', "")
                );
                assert!(
                    parser_errors(parsed).next().is_none(),
                    "{expression}: {:#?}",
                    parsed.errors
                );
            },
        );
    }
}

#[test]
fn scalar_list_and_designated_initializers_have_stable_arena_children() {
    with_parse(
        "struct S { int field; int tail; }; struct S value = { .field = { [2] = 3, 4, }, .tail = \
         5 };\n",
        |parsed| {
            let declaration = declaration(parsed, 1);
            let [init_declarator] = declaration.init_declarators else {
                panic!("expected one initialized declarator");
            };
            let initializer = init_declarator
                .initializer
                .expect("initializer must be attached to its declarator");
            let outer_initializer = &parsed.parser.syntax[initializer];
            let InitializerType::InitializerList(elements) = outer_initializer.kind else {
                panic!("expected outer initializer list");
            };
            assert_eq!(
                sourced_text(
                    parsed,
                    outer_initializer
                        .opening_brace_source_vectors
                        .expect("opening initializer brace"),
                ),
                "{"
            );
            assert_eq!(
                sourced_text(
                    parsed,
                    outer_initializer
                        .closing_brace_source_vectors
                        .expect("closing initializer brace"),
                ),
                "}"
            );
            let outer = &parsed.parser.syntax[elements];
            assert_eq!(outer.len(), 2);
            assert!(outer.iter().all(|element| element.designation.is_some()));

            let field_designation = outer[0].designation.expect("field designation");
            let field_designation = parsed.parser.syntax[field_designation];
            let field_designators = &parsed.parser.syntax[field_designation.designators];
            assert!(matches!(
                field_designators,
                [Designator {
                    kind: DesignatorType::Field(identifier),
                    ..
                }] if parsed.context.string_cache.at(identifier.name) == "field"
            ));
            assert_eq!(
                sourced_text(parsed, field_designators[0].operator_source_vectors),
                "."
            );
            assert_eq!(
                sourced_text(
                    parsed,
                    field_designation
                        .equals_source_vectors
                        .expect("designation equals"),
                ),
                "="
            );

            let nested_initializer = &parsed.parser.syntax[outer[0].initializer];
            let InitializerType::InitializerList(nested_elements) = nested_initializer.kind else {
                panic!("expected nested initializer list");
            };
            assert_eq!(nested_elements.len(), 2);
            let nested = &parsed.parser.syntax[nested_elements];
            let array_designation = nested[0].designation.expect("array designation");
            let array_designation = parsed.parser.syntax[array_designation];
            let array_designator = parsed.parser.syntax[array_designation.designators][0];
            assert!(matches!(
                array_designator.kind,
                DesignatorType::Array(expression) if constant_expression_text(parsed, expression) == "2"
            ));
            assert_eq!(
                sourced_text(parsed, array_designator.operator_source_vectors),
                "["
            );
            assert_eq!(
                sourced_text(
                    parsed,
                    array_designator
                        .closing_bracket_source_vectors
                        .expect("closing designator bracket"),
                ),
                "]"
            );
            let scalar_text = |element: InitializerElement<'_>| {
                let InitializerType::AssignmentExpression(expression) =
                    parsed.parser.syntax[element.initializer].kind
                else {
                    panic!("expected scalar initializer")
                };
                expression_text(parsed, expression)
            };
            assert_eq!(scalar_text(nested[0]), "3");
            assert_eq!(scalar_text(nested[1]), "4");
            assert_eq!(
                sourced_text(
                    parsed,
                    nested[1]
                        .comma_source_vectors
                        .expect("trailing initializer comma"),
                ),
                ","
            );
            assert_eq!(scalar_text(outer[1]), "5");
            assert_eq!(
                sourced_text(parsed, array_designation.source_vectors),
                "[2]="
            );
            assert!(sourced_text(parsed, outer[0].source_vectors).contains(".field"));
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn chained_designators_retain_their_order_and_initializer() {
    with_parse(
        "struct S { int member[2][2]; }; struct S value = { .member[0][1] = 3 };\n",
        |parsed| {
            let declaration = declaration(parsed, 1);
            let [init_declarator] = declaration.init_declarators else {
                panic!("expected one initialized declarator")
            };
            let initializer = init_declarator
                .initializer
                .expect("initializer must be attached to its declarator");
            let InitializerType::InitializerList(elements) = parsed.parser.syntax[initializer].kind
            else {
                panic!("expected an initializer list")
            };
            let [element] = &parsed.parser.syntax[elements] else {
                panic!("expected one designated element")
            };
            let designation = element.designation.expect("expected a designation");
            let designation = parsed.parser.syntax[designation];
            let designators = &parsed.parser.syntax[designation.designators];

            assert!(matches!(
                designators,
                [
                    Designator {
                        kind: DesignatorType::Field(identifier),
                        ..
                    },
                    Designator {
                        kind: DesignatorType::Array(first),
                        ..
                    },
                    Designator {
                        kind: DesignatorType::Array(second),
                        ..
                    }
                ] if parsed.context.string_cache.at(identifier.name) == "member"
                    && constant_expression_text(parsed, *first) == "0"
                    && constant_expression_text(parsed, *second) == "1"
            ));
            assert_eq!(
                sourced_text(parsed, designation.source_vectors),
                ".member[0][1]="
            );
            let InitializerType::AssignmentExpression(expression) =
                parsed.parser.syntax[element.initializer].kind
            else {
                panic!("expected a scalar initializer")
            };
            assert_eq!(expression_text(parsed, expression), "3");
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn array_designators_accept_conditional_and_parenthesized_comma_expressions() {
    with_parse(
        "int values[] = { [x ? y : z] = 1, [(x, y)] = 2 };\n",
        |parsed| {
            let declaration = declaration(parsed, 0);
            let [init_declarator] = declaration.init_declarators else {
                panic!("expected one initialized declarator")
            };
            let initializer = init_declarator.initializer.expect("parsed initializer");
            let InitializerType::InitializerList(elements) = parsed.parser.syntax[initializer].kind
            else {
                panic!("expected an initializer list")
            };
            let elements = &parsed.parser.syntax[elements];
            let expressions = elements
                .iter()
                .map(|element| {
                    let designation = element.designation.expect("array designation");
                    let designation = parsed.parser.syntax[designation];
                    let designator = parsed.parser.syntax[designation.designators][0];
                    let DesignatorType::Array(expression) = designator.kind else {
                        panic!("expected an array designator")
                    };
                    assert_eq!(
                        sourced_text(parsed, designator.operator_source_vectors),
                        "["
                    );
                    assert_eq!(
                        sourced_text(
                            parsed,
                            designator
                                .closing_bracket_source_vectors
                                .expect("closing designator bracket"),
                        ),
                        "]"
                    );
                    expression.expression()
                })
                .collect::<Vec<_>>();

            assert!(matches!(
                parsed.parser.syntax[expressions[0]].kind,
                ExpressionType::Conditional { .. }
            ));
            assert!(matches!(
                parsed.parser.syntax[expressions[1]].kind,
                ExpressionType::Parenthesized { expression }
                    if matches!(
                        parsed.parser.syntax[expression].kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Comma,
                            ..
                        }
                    )
            ));
            assert_eq!(expression_text(parsed, expressions[0]), "x?y:z");
            assert_eq!(expression_text(parsed, expressions[1]), "(x,y)");
            assert_eq!(elements.len(), 2);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn every_c99_expression_operator_is_represented_by_the_syntax_model() {
    with_parse(
        "struct S { int member; }; int f(int a, int b, int c, int *p, struct S s) { a*b; a/b; \
         a%b; a+b; a-b; a<<b; a>>b; a<b; a>b; a<=b; a>=b; a==b; a!=b; a&b; a^b; a|b; a&&b; a||b; \
         a=b; a*=b; a/=b; a%=b; a+=b; a-=b; a<<=b; a>>=b; a&=b; a^=b; a|=b; a,b; a?b:c; p[a]; \
         f(a); s.member; p->member; &a; *p; +a; -a; ~a; !a; ++a; --a; a++; a--; (int)a; sizeof a; \
         sizeof(int); return 0; }\n",
        |parsed| {
            let binary = [
                BinaryOperator::Multiplication,
                BinaryOperator::Division,
                BinaryOperator::Modulo,
                BinaryOperator::Addition,
                BinaryOperator::Subtraction,
                BinaryOperator::LeftShift,
                BinaryOperator::RightShift,
                BinaryOperator::LessThan,
                BinaryOperator::GreaterThan,
                BinaryOperator::LessThanOrEqual,
                BinaryOperator::GreaterThanOrEqual,
                BinaryOperator::Equal,
                BinaryOperator::NotEqual,
                BinaryOperator::BitwiseAnd,
                BinaryOperator::BitwiseXor,
                BinaryOperator::BitwiseOr,
                BinaryOperator::LogicalAnd,
                BinaryOperator::LogicalOr,
                BinaryOperator::Assignment,
                BinaryOperator::MultiplicationAssignment,
                BinaryOperator::DivisionAssignment,
                BinaryOperator::ModuloAssignment,
                BinaryOperator::AdditionAssignment,
                BinaryOperator::SubtractionAssignment,
                BinaryOperator::LeftShiftAssignment,
                BinaryOperator::RightShiftAssignment,
                BinaryOperator::BitwiseAndAssignment,
                BinaryOperator::BitwiseXorAssignment,
                BinaryOperator::BitwiseOrAssignment,
                BinaryOperator::Comma,
                BinaryOperator::Subscript,
            ];
            for expected in binary {
                assert!(
                    parsed
                        .parser
                        .syntax
                        .iter::<Expression<'_>>()
                        .any(|expression| matches!(
                            expression.kind,
                            ExpressionType::Binary { operator, .. } if operator == expected
                        )),
                    "missing binary operator {expected:?}"
                );
            }
            let unary = [
                UnaryOperator::AddressOf,
                UnaryOperator::Indirection,
                UnaryOperator::Plus,
                UnaryOperator::Minus,
                UnaryOperator::BitwiseNot,
                UnaryOperator::LogicalNot,
                UnaryOperator::PreIncrement,
                UnaryOperator::PreDecrement,
                UnaryOperator::PostIncrement,
                UnaryOperator::PostDecrement,
            ];
            for expected in unary {
                assert!(
                    parsed
                        .parser
                        .syntax
                        .iter::<Expression<'_>>()
                        .any(|expression| matches!(
                            expression.kind,
                            ExpressionType::Unary { operator, .. } if operator == expected
                        )),
                    "missing unary operator {expected:?}"
                );
            }
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Expression<'_>>()
                    .any(|expression| matches!(
                        expression.kind,
                        ExpressionType::Conditional { .. }
                    ))
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Expression<'_>>()
                    .any(|expression| matches!(expression.kind, ExpressionType::Cast { .. }))
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Expression<'_>>()
                    .any(|expression| matches!(expression.kind, ExpressionType::SizeofExpr(_)))
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Expression<'_>>()
                    .any(|expression| matches!(expression.kind, ExpressionType::SizeofType(_)))
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
fn each_unary_spelling_maps_to_its_ast_operator() {
    let cases = [
        ("&value", "&", UnaryOperator::AddressOf),
        ("*value", "*", UnaryOperator::Indirection),
        ("+value", "+", UnaryOperator::Plus),
        ("-value", "-", UnaryOperator::Minus),
        ("~value", "~", UnaryOperator::BitwiseNot),
        ("!value", "!", UnaryOperator::LogicalNot),
        ("++value", "++", UnaryOperator::PreIncrement),
        ("--value", "--", UnaryOperator::PreDecrement),
        ("value++", "++", UnaryOperator::PostIncrement),
        ("value--", "--", UnaryOperator::PostDecrement),
    ];

    for (source_expression, operator_spelling, expected_operator) in cases {
        let source = format!("int f(void) {{ {source_expression}; }}\n");
        with_parse(&source, |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(parsed, function_definition(parsed, 0).body)
            else {
                panic!("expected one expression statement for {source_expression}")
            };
            let StatementType::Expression(ExpressionSlot::Parsed(expression)) =
                parsed.parser.syntax[statement].kind
            else {
                panic!("expected a parsed expression for {source_expression}")
            };
            let expression = &parsed.parser.syntax[expression];
            assert!(matches!(
                expression.kind,
                ExpressionType::Unary { operator, .. } if operator == expected_operator
            ));
            assert_eq!(
                sourced_text(
                    parsed,
                    expression
                        .operator_source_vectors
                        .expect("unary expression must retain operator provenance"),
                ),
                operator_spelling
            );
            assert!(
                parser_errors(parsed).next().is_none(),
                "unexpected diagnostics for {source_expression}: {:#?}",
                parsed.errors
            );
        });
    }
}

#[test]
fn deeply_nested_expressions_use_heap_backed_parser_frames() {
    let depth = 4_096;
    let source = format!(
        "int f(void) {{ return {}1{}; }}\n",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    with_parse(&source, |parsed| {
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
                .is_some_and(|maximum| maximum > depth)
        );
    });
}

#[test]
fn call_argument_and_initializer_nesting_translation_floors_are_heap_backed() {
    let arguments = (0..127).map(|_| "1").collect::<Vec<_>>().join(",");
    let initializer_depth = 512;
    let source = format!(
        "int nested = {}1{}; int f(void) {{ return f({arguments}); }}\n",
        "{".repeat(initializer_depth),
        "}".repeat(initializer_depth),
    );
    with_parse(&source, |parsed| {
        assert!(
            parser_errors(parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(
            parsed
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .any(|expression| matches!(
                    expression.kind,
                    ExpressionType::Call { arguments, .. } if arguments.len() == 127
                ))
        );
        assert!(
            parsed
                .parser
                .syntax
                .iter::<Initializer<'_>>()
                .filter(|initializer| matches!(
                    initializer.kind,
                    InitializerType::InitializerList(_)
                ))
                .count()
                >= initializer_depth
        );
    });
}

#[test]
fn abstract_type_names_cover_pointer_array_function_and_parenthesized_forms() {
    with_parse(
        "int f(int *p) { (int *)p; sizeof(int [4]); sizeof(int (*)(int)); return sizeof(int \
         (*)[4]); }\n",
        |parsed| {
            assert_eq!(parsed.parser.syntax.count::<TypeName<'_>>(), 4);
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<TypeName<'_>>()
                    .all(|type_name| type_name.declarator.is_some())
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
fn repeated_qualifiers_are_coalesced_in_type_names_and_abstract_declarators() {
    with_parse(
        "int f(void) { sizeof(const const int); sizeof(int *volatile volatile); sizeof(int \
         [restrict restrict 4]); return; }\n",
        |parsed| {
            assert_eq!(parsed.parser.syntax.count::<TypeName<'_>>(), 3);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn unterminated_initializer_lists_stop_at_caller_boundaries_and_eof() {
    with_parse("int x = {1; int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items[0],
            ExternalDeclaration::RecoveredDeclaration(_)
        ));
        assert!(matches!(
            parsed.items[1],
            ExternalDeclaration::Declaration(_)
        ));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(Some(TokenType::Operator(
                OperatorTokenType::Semicolon
            )))
        )));
    });
    with_parse("int x = {1", |eof| {
        assert_eq!(eof.items.len(), 1);
        assert!(parser_errors(eof).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(None)
        )));
    });
}
