//! Recovery inside expressions, designations, and initializers.

use super::{
    block_items,
    declaration,
    expression_text,
    function_definition,
    identifier_name,
    parser_errors,
    return_expression,
    sourced_text,
    with_parse,
};
use crate::translation_phases::{
    parsing::{
        declaration_syntax::{
            Declaration,
            Designation,
            Designator,
            DesignatorType,
            DirectDeclarator,
            EnumSpecifier,
            Enumerator,
            InitializerElement,
            InitializerType,
            ParameterDeclaration,
            StructDeclaration,
            StructOrUnionSpecifier,
            TypeName,
            TypeSpecifiers,
        },
        errors::ParserErrorType,
        syntax::{
            BinaryOperator,
            BlockItem,
            Expression,
            ExpressionSlot,
            ExpressionType,
            ExternalDeclaration,
            Identifier,
            Statement,
            StatementType,
            StorageClass,
            UnaryOperator,
        },
    },
    preprocessing::{
        OperatorTokenType,
        StringTokenType,
        TokenType,
    },
};

#[test]
fn adjacent_strings_merge_across_macro_expansion_and_preserve_width() {
    with_parse(
        "#define PREFIX \"a\"\nchar *ordinary = PREFIX \"b\"; char *wide = \"x\" L\"y\";\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 2);
            let literal = |item: usize| {
                let initializer = (declaration(parsed, item)).init_declarators[0]
                    .initializer
                    .expect("string initializer");
                let InitializerType::AssignmentExpression(expression) = initializer.kind else {
                    panic!("expected scalar string initializer")
                };
                let ExpressionType::StringLiteral(literal) = expression.kind else {
                    panic!("expected string literal expression")
                };
                (literal, expression.source_vectors)
            };
            let (StringTokenType::String(ordinary), ordinary_source_vectors) = literal(0) else {
                panic!("ordinary concatenation must remain ordinary")
            };
            let (StringTokenType::WideString(wide), wide_source_vectors) = literal(1) else {
                panic!("a mixed concatenation must become wide")
            };
            assert_eq!(
                parsed
                    .context
                    .literal_text_in(parsed.context.tu_arena(), ordinary, false)
                    .unwrap(),
                "ab"
            );
            assert_eq!(
                parsed
                    .context
                    .literal_text_in(parsed.context.tu_arena(), wide, true)
                    .unwrap(),
                "xy"
            );
            assert_eq!(sourced_text(parsed, ordinary_source_vectors), "\"a\"\"b\"");
            assert_eq!(sourced_text(parsed, wide_source_vectors), "\"x\"L\"y\"");
            assert!(parser_errors(parsed).next().is_none());
        },
    );
}

#[test]
fn prefix_increment_accepts_a_compound_literal_postfix_operand() {
    with_parse(
        "typedef struct { int x; } T; int f(void) { ++(T){1}.x; return 0; }\n",
        |parsed| {
            let items = block_items(function_definition(parsed, 1).body);
            let BlockItem::Statement(statement) = items[0] else {
                panic!("expected expression statement")
            };
            let StatementType::Expression(ExpressionSlot::Parsed(root)) = statement.kind else {
                panic!("expected parsed prefix expression")
            };
            let ExpressionType::Unary {
                operator: UnaryOperator::PreIncrement,
                operand_expression,
            } = root.kind
            else {
                panic!("expected prefix increment")
            };
            assert!(matches!(
                operand_expression.kind,
                ExpressionType::DirectMember { base_expression, .. }
                    if matches!(
                        base_expression.kind,
                        ExpressionType::CompoundLiteral { .. }
                    )
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
fn sizeof_owns_the_complete_compound_literal_postfix_operand() {
    with_parse(
        "typedef struct { int x; } T; int f(void) { return sizeof (T){1}.x; }\n",
        |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(function_definition(parsed, 1).body)
            else {
                panic!("expected one return statement")
            };
            let StatementType::Return(Some(ExpressionSlot::Parsed(root))) = statement.kind else {
                panic!("expected parsed return expression")
            };
            let ExpressionType::SizeofExpr(operand) = root.kind else {
                panic!("expected sizeof expression")
            };
            let ExpressionType::DirectMember {
                base_expression, ..
            } = operand.kind
            else {
                panic!("sizeof must own the member suffix")
            };
            assert!(matches!(
                base_expression.kind,
                ExpressionType::CompoundLiteral { .. }
            ));
            assert_eq!(expression_text(parsed, base_expression), "(T){1}");
            assert_eq!(expression_text(parsed, root), "sizeof(T){1}.x");
            assert!(parser_errors(parsed).next().is_none());
        },
    );

    // The invalid call suffix stays in the expression, marked recovered.
    with_parse("int f(void) { return sizeof(int)(); }\n", |invalid| {
        assert_eq!(parser_errors(invalid).count(), 1, "{:#?}", invalid.errors);
        assert!(
            invalid
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .filter(|expression| matches!(expression.kind, ExpressionType::Call { .. }))
                .all(|expression| expression.recovered)
        );
    });
}

#[test]
fn repaired_expressions_and_designations_retain_recovery_metadata() {
    with_parse("int x[] = { [(1] = 1, [2] 3 }; int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        assert!(
            parsed
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .any(|expression| expression.recovered
                    && matches!(expression.kind, ExpressionType::Parenthesized { .. }))
        );
        let designations = parsed
            .parser
            .syntax
            .iter::<Designation<'_>>()
            .collect::<Vec<_>>();
        let [first, second] = designations[..] else {
            panic!("expected two designations")
        };
        assert!(first.recovered);
        assert!(second.recovered);
        let first_designator = first.designators[0];
        assert!(first_designator.recovered);
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedEqualsAfterInitializerDesignation(_)
        )));
    });
}

#[test]
fn adjacent_precedence_levels_and_parentheses_have_explicit_tree_tests() {
    let cases = [
        (
            "a*b+c",
            BinaryOperator::Addition,
            BinaryOperator::Multiplication,
        ),
        (
            "a+b<<c",
            BinaryOperator::LeftShift,
            BinaryOperator::Addition,
        ),
        (
            "a<<b<c",
            BinaryOperator::LessThan,
            BinaryOperator::LeftShift,
        ),
        ("a<b==c", BinaryOperator::Equal, BinaryOperator::LessThan),
        ("a==b&c", BinaryOperator::BitwiseAnd, BinaryOperator::Equal),
        (
            "a&b^c",
            BinaryOperator::BitwiseXor,
            BinaryOperator::BitwiseAnd,
        ),
        (
            "a^b|c",
            BinaryOperator::BitwiseOr,
            BinaryOperator::BitwiseXor,
        ),
        (
            "a|b&&c",
            BinaryOperator::LogicalAnd,
            BinaryOperator::BitwiseOr,
        ),
        (
            "a&&b||c",
            BinaryOperator::LogicalOr,
            BinaryOperator::LogicalAnd,
        ),
    ];
    for (source, outer, inner) in cases {
        with_parse(&format!("int f(void) {{ {source}; }}\n"), |parsed| {
            let [BlockItem::Statement(statement)] =
                block_items(function_definition(parsed, 0).body)
            else {
                panic!("expected one expression statement")
            };
            let StatementType::Expression(ExpressionSlot::Parsed(root)) = statement.kind else {
                panic!("expected parsed expression")
            };
            let ExpressionType::Binary {
                operator,
                left_expression,
                ..
            } = root.kind
            else {
                panic!("expected binary root for {source}")
            };
            assert_eq!(operator, outer, "wrong outer operator for {source}");
            assert!(matches!(
                left_expression.kind,
                ExpressionType::Binary { operator, .. } if operator == inner
            ));
            assert!(parser_errors(parsed).next().is_none());
        });
    }

    with_parse("int f(void) { (a+b)*c; }\n", |parsed| {
        let [BlockItem::Statement(statement)] = block_items(function_definition(parsed, 0).body)
        else {
            panic!("expected one expression statement")
        };
        let StatementType::Expression(ExpressionSlot::Parsed(root)) = statement.kind else {
            panic!("expected parsed expression")
        };
        assert!(matches!(
            root.kind,
            ExpressionType::Binary {
                operator: BinaryOperator::Multiplication,
                left_expression,
                ..
            } if matches!(
                left_expression.kind,
                ExpressionType::Parenthesized { expression }
                    if matches!(
                        expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Addition,
                            ..
                        }
                    )
            )
        ));
    });
}

#[test]
fn unowned_expression_closers_are_consumed_once() {
    for source in [
        "int f(void) { 1 ]; } int after;\n",
        "int f(void) { return (1 2); } int after;\n",
    ] {
        with_parse(source, |parsed| {
            assert_eq!(parsed.items.len(), 2, "{source}: {:#?}", parsed.items);
            assert!(matches!(
                parsed.items[1],
                ExternalDeclaration::Declaration(_)
            ));
            assert!(parser_errors(parsed).next().is_some());
        });
    }
}

#[test]
fn binary_expression_recovery_synthesizes_missing_operands() {
    with_parse("int f(void) { return 1 + ); return; }\n", |parsed| {
        let items = block_items(function_definition(parsed, 0).body);

        assert_eq!(items.len(), 2, "{items:#?}");
        assert!(matches!(
            items[0],
            BlockItem::Statement(index)
                if matches!(
                    index.kind,
                    StatementType::Return(Some(ExpressionSlot::Parsed(_)))
                )
        ));
        assert!(matches!(
            items[1],
            BlockItem::Statement(index)
                if matches!(
                    index.kind,
                    StatementType::Return(None)
                )
        ));
        assert!(parser_errors(parsed).next().is_some());
    });
}

#[test]
fn missing_operand_error_is_anchored_at_the_current_boundary_token() {
    let source = "int f(void) { return 1 + ; return; }\n";
    with_parse(source, |parsed| {
        let error = parsed
            .parser
            .syntax
            .iter::<Expression<'_>>()
            .find(|expression| matches!(expression.kind, ExpressionType::Error))
            .expect("missing operand must produce an error expression");
        let [anchor] = parsed.context.get_source_vectors(error.source_vectors) else {
            panic!("error expression must have one source anchor")
        };

        assert_eq!(
            anchor.index as usize,
            source.find(';').expect("boundary semicolon")
        );
        assert_eq!(anchor.length, 0);
    });
}

#[test]
fn unexpected_braces_in_an_expression_do_not_close_the_function_body() {
    with_parse(
        "int f(void) { int x = 1 + {} int after; return; }\n",
        |parsed| {
            let items = block_items(function_definition(parsed, 0).body);

            assert_eq!(items.len(), 3, "{items:#?}");
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert!(matches!(items[1], BlockItem::Declaration(_)));
            assert!(
                matches!(items[2], BlockItem::Statement(statement) if matches!(
                    statement.kind,
                    StatementType::Return(None)
                ))
            );
            assert!(
                parsed
                    .parser
                    .syntax
                    .iter::<Expression<'_>>()
                    .any(|expression| {
                        matches!(expression.kind, ExpressionType::Error)
                            && sourced_text(parsed, expression.source_vectors) == "{}"
                    })
            );
        },
    );
}

#[test]
fn missing_member_names_retain_the_consumed_operator_provenance() {
    with_parse("int f(void) { return a.; return p->; }\n", |parsed| {
        let items = block_items(function_definition(parsed, 0).body);
        let roots = items
            .iter()
            .map(|item| {
                let BlockItem::Statement(statement) = *item else {
                    panic!("expected return statement")
                };
                return_expression(statement)
            })
            .collect::<Vec<_>>();

        assert_eq!(roots.len(), 2);
        for (root, expression_source, operator_source) in
            [(roots[0], "a.", "."), (roots[1], "p->", "->")]
        {
            let expression = root;
            assert!(matches!(expression.kind, ExpressionType::Error));
            assert!(expression.recovered);
            assert_eq!(expression_text(parsed, root), expression_source);
            assert_eq!(
                sourced_text(
                    parsed,
                    expression
                        .operator_source_vectors
                        .expect("member operator provenance"),
                ),
                operator_source
            );
        }

        let errors = parser_errors(parsed).collect::<Vec<_>>();
        assert!(errors.iter().all(|error| matches!(
            error,
            ParserErrorType::ExpectedMemberIdentifier(Some(TokenType::Operator(
                OperatorTokenType::Semicolon
            )))
        )));
        assert_eq!(
            errors[0].to_string(),
            "expected a member name after `.` or `->`, found `;`"
        );
    });
}

#[test]
fn unterminated_initializer_list_stops_before_a_following_declaration() {
    with_parse("int x = {1 int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
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
            ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(_)
        )));
    });
}

#[test]
fn array_designator_recovery_synchronizes_to_its_closing_bracket() {
    with_parse("int x[] = { [1 2] = 3, 4 }; int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
        assert!(matches!(
            parsed.items[1],
            ExternalDeclaration::Declaration(_)
        ));
        assert_eq!(parsed.parser.syntax.count::<Designator<'_>>(), 1);
        assert!(parsed.parser.syntax.nth::<Designator<'_>>(0).recovered);
        assert_eq!(parsed.parser.syntax.count::<InitializerElement<'_>>(), 2);
        // One stray run inside the brackets is one diagnostic.
        assert_eq!(parser_errors(parsed).count(), 1, "{:#?}", parsed.errors);
    });
}

#[test]
fn array_designator_recovery_ignores_nested_commas() {
    with_parse(
        "int a[] = { [1 junk (2,3)] = 4, 5 }; int after;\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
            assert!(matches!(
                parsed.items[1],
                ExternalDeclaration::Declaration(_)
            ));
            assert_eq!(parsed.parser.syntax.count::<Designator<'_>>(), 1);
            assert!(parsed.parser.syntax.nth::<Designator<'_>>(0).recovered);
            assert_eq!(parsed.parser.syntax.count::<InitializerElement<'_>>(), 2);
            // One stray run inside the brackets is one diagnostic.
            assert_eq!(parser_errors(parsed).count(), 1, "{:#?}", parsed.errors);
        },
    );
}

#[test]
fn array_designator_recovery_preserves_following_declarations() {
    with_parse(
        "int f(void) { int x = { [1 + int after; int later; }\n",
        |parsed| {
            let items = block_items(function_definition(parsed, 0).body);

            assert_eq!(items.len(), 3, "{items:#?}");
            let names = items
                .iter()
                .map(|item| {
                    let BlockItem::Declaration(index) = item else {
                        panic!("expected a declaration block item")
                    };
                    let declaration = index;
                    identifier_name(parsed, declaration.init_declarators[0].declarator)
                        .expect("named declarator")
                })
                .collect::<Vec<_>>();
            assert_eq!(names, ["x", "after", "later"]);
        },
    );

    with_parse("int x[] = { [1 foo [ 2 } ; int after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
        assert!(matches!(
            parsed.items[0],
            ExternalDeclaration::RecoveredDeclaration(_)
        ));
        assert_eq!(
            identifier_name(
                parsed,
                declaration(parsed, 1).init_declarators[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    });
}

#[test]
fn array_designator_recovery_consumes_parentheses_not_owned_by_the_caller() {
    with_parse(
        "int f(){ int a[] = {[1 + )] = 2}; int after; return 0; }\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 1, "{:#?}", parsed.items);
            let items = block_items(function_definition(parsed, 0).body);
            assert_eq!(items.len(), 3, "{items:#?}");
            assert!(matches!(items[0], BlockItem::Declaration(_)));
            assert!(matches!(items[1], BlockItem::Declaration(_)));
            assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                index.kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            )));
        },
    );
}

#[test]
fn array_designator_recovery_preserves_a_for_header_parenthesis() {
    with_parse(
        "int f(void) { for (int a[] = {[1 + ) ; return; }\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 1, "{:#?}", parsed.items);
            let items = block_items(function_definition(parsed, 0).body);
            assert_eq!(items.len(), 2, "{items:#?}");
            let BlockItem::Statement(for_statement) = items[0] else {
                panic!("expected recovered for statement")
            };
            assert!(matches!(for_statement.kind, StatementType::For(_)));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                index.kind,
                StatementType::Return(None)
            )));
        },
    );
}

#[test]
fn initializer_recovery_consumes_unowned_closing_delimiters() {
    for closer in [")", "]"] {
        with_parse(
            &format!("int f(void) {{ int a[] = {{1 + {closer} }}; int after; return; }}\n"),
            |parsed| {
                assert_eq!(parsed.items.len(), 1, "{closer}: {:#?}", parsed.items);
                let items = block_items(function_definition(parsed, 0).body);
                assert_eq!(items.len(), 3, "{closer}: {items:#?}");
                assert!(matches!(items[0], BlockItem::Declaration(_)));
                assert!(matches!(items[1], BlockItem::Declaration(_)));
                assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
                    index.kind,
                    StatementType::Return(None)
                )));
            },
        );
    }
}

#[test]
fn initializer_recovery_preserves_an_enclosing_subscript_bracket() {
    with_parse(
        "int f(void) { return values[(int[]){1 + ]; return 0; }\n",
        |parsed| {
            assert_eq!(parsed.items.len(), 1, "{:#?}", parsed.items);
            let items = block_items(function_definition(parsed, 0).body);
            assert_eq!(items.len(), 2, "{items:#?}");
            assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
                index.kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            )));
            assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
                index.kind,
                StatementType::Return(Some(ExpressionSlot::Parsed(_)))
            )));
        },
    );
}

#[test]
fn imaginary_type_specifiers_are_diagnosed_in_type_names() {
    with_parse("int f(void) { return sizeof(_Imaginary); }\n", |parsed| {
        assert!(
            parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::UnsupportedImaginaryTypeSpecifier))
        );
        assert!(matches!(
            parsed.items[0],
            ExternalDeclaration::RecoveredFunctionDefinition(_)
        ));
        assert!(parsed.parser.syntax.nth::<TypeName<'_>>(0).recovered);
    });
}

#[test]
fn imaginary_type_specifier_does_not_cascade_into_missing_type() {
    with_parse("_Imaginary value; int after;\n", |parsed| {
        let errors = parser_errors(parsed).collect::<Vec<_>>();

        assert_eq!(
            errors
                .iter()
                .filter(|error| matches!(error, ParserErrorType::UnsupportedImaginaryTypeSpecifier))
                .count(),
            1
        );
        assert!(errors.iter().all(|error| !matches!(
            error,
            ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(_)
                | ParserErrorType::UnexpectedEndBeforeTypeSpecifier
        )));
        assert!(matches!(
            parsed.items.as_slice(),
            [
                ExternalDeclaration::RecoveredDeclaration(_),
                ExternalDeclaration::Declaration(_)
            ]
        ));
    });
}

#[test]
fn recovered_expression_children_mark_every_composite_parent() {
    with_parse("int f(void) { return foo(1 + ) + 2; }\n", |parsed| {
        let call = parsed
            .parser
            .syntax
            .iter::<Expression<'_>>()
            .find(|expression| matches!(expression.kind, ExpressionType::Call { .. }))
            .expect("expected the recovered call expression");
        assert!(call.recovered);
        assert!(
            parsed
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .filter(|expression| matches!(expression.kind, ExpressionType::Error))
                .all(|expression| expression.recovered)
        );
    });
}

#[test]
fn composite_expressions_retain_exact_operator_provenance() {
    with_parse(
        "typedef struct { int m; } T;\nint f(void) {\n(a); sizeof a; sizeof(int); (int)a; \
         (T){1}.m; p->m++; foo(a,b); a[0]; return -a+b ? c : d;\n}\n",
        |parsed| {
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            for expression in parsed.parser.syntax.iter::<Expression<'_>>() {
                if matches!(
                    expression.kind,
                    ExpressionType::Identifier(..)
                        | ExpressionType::Constant(..)
                        | ExpressionType::StringLiteral(..)
                        | ExpressionType::Error
                ) {
                    continue;
                }
                assert!(
                    expression.operator_source_vectors.is_some(),
                    "missing operator provenance for {expression:#?}"
                );
            }

            let addition = parsed
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .find(|expression| {
                    matches!(
                        expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Addition,
                            ..
                        }
                    )
                })
                .expect("expected addition expression");
            assert_eq!(
                sourced_text(
                    parsed,
                    addition
                        .operator_source_vectors
                        .expect("addition operator provenance")
                ),
                "+"
            );
            let conditional = parsed
                .parser
                .syntax
                .iter::<Expression<'_>>()
                .find(|expression| matches!(expression.kind, ExpressionType::Conditional(_)))
                .expect("expected conditional expression");
            assert_eq!(
                sourced_text(
                    parsed,
                    conditional
                        .operator_source_vectors
                        .expect("conditional operator provenance")
                ),
                "?:"
            );
        },
    );
}

#[test]
fn phase_05_syntax_facts_preserve_absence_and_identifier_provenance() {
    with_parse(
        "int implicit; auto int explicit; typedef int T; struct S { int member; }; enum E { VALUE \
         }; int f(void) { label: goto label; explicit.member; T designated = { .member = VALUE }; \
         return designated.member; }\n",
        |parsed| {
            assert_eq!(
                declaration(parsed, 0).declaration_specifiers.storage_class,
                None
            );
            assert_eq!(
                declaration(parsed, 1).declaration_specifiers.storage_class,
                Some(StorageClass::Auto)
            );

            let mut identifiers = Vec::new();
            for direct in parsed.parser.syntax.iter::<DirectDeclarator<'_>>() {
                if let DirectDeclarator::Identifier(identifier) = direct {
                    identifiers.push(*identifier);
                }
            }
            identifiers.extend(parsed.parser.syntax.iter::<Identifier>().copied());
            for specifier in parsed.parser.syntax.iter::<StructOrUnionSpecifier<'_>>() {
                identifiers.extend(specifier.identifier);
            }
            for specifier in parsed.parser.syntax.iter::<EnumSpecifier<'_>>() {
                identifiers.extend(specifier.name);
            }
            identifiers.extend(
                parsed
                    .parser
                    .syntax
                    .iter::<Enumerator<'_>>()
                    .map(|enumerator| enumerator.name),
            );
            for expression in parsed.parser.syntax.iter::<Expression<'_>>() {
                match expression.kind {
                    | ExpressionType::Identifier(identifier)
                    | ExpressionType::DirectMember {
                        member: identifier, ..
                    }
                    | ExpressionType::IndirectMember {
                        member: identifier, ..
                    } => identifiers.push(identifier),
                    | _ => {},
                }
            }
            for designator in parsed.parser.syntax.iter::<Designator<'_>>() {
                if let DesignatorType::Field(identifier) = designator.kind {
                    identifiers.push(identifier);
                }
            }
            for statement in parsed.parser.syntax.iter::<Statement<'_>>() {
                match statement.kind {
                    | StatementType::Goto(identifier) | StatementType::Label(identifier, _) => {
                        identifiers.push(identifier);
                    },
                    | _ => {},
                }
            }
            for specifiers in parsed
                .parser
                .syntax
                .iter::<Declaration<'_>>()
                .map(|declaration| declaration.declaration_specifiers.type_specifiers)
                .chain(
                    parsed
                        .parser
                        .syntax
                        .iter::<TypeName<'_>>()
                        .map(|type_name| type_name.declaration_specifiers.type_specifiers),
                )
                .chain(
                    parsed
                        .parser
                        .syntax
                        .iter::<ParameterDeclaration<'_>>()
                        .map(|parameter| parameter.declaration_specifiers.type_specifiers),
                )
                .chain(
                    parsed
                        .parser
                        .syntax
                        .iter::<StructDeclaration<'_>>()
                        .map(|declaration| declaration.type_specifiers),
                )
            {
                if let TypeSpecifiers::TypedefName(identifier) = specifiers {
                    identifiers.push(identifier);
                }
            }
            assert_ne!(identifiers, []);
            for identifier in identifiers {
                assert!(identifier.source_vectors.length > 0, "{identifier:?}");
                assert_eq!(
                    sourced_text(parsed, identifier.source_vectors),
                    parsed.context.string_cache.at(identifier.name)
                );
            }
        },
    );
}
