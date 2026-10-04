//! Regression tests for verified statement bugs found by the overnight bug
//! hunt.

use super::{
    Parsed,
    block_items,
    expression_text,
    function_definition,
    parser_errors,
    with_parse,
};
use crate::translation_phases::{
    GetPosition,
    TranslationError,
    parsing::{
        errors::ParserErrorType,
        syntax::{
            BlockItem,
            ExpressionSlot,
            ForInitializer,
            StatementIndex,
            StatementType,
        },
    },
};

/// Returns the byte offsets of every parser error, in report order.
fn error_offsets(parsed: &Parsed<'_>) -> Vec<usize> {
    parsed
        .errors
        .iter()
        .filter_map(|error| match error {
            | TranslationError::Parsing(error) => Some(error.position(parsed.context).index),
            | _ => None,
        })
        .collect()
}

/// Returns the block items of the first function definition's body.
fn body_items<'a>(parsed: &'a Parsed<'_>) -> &'a [BlockItem] {
    block_items(parsed, function_definition(parsed, 0).body)
}

fn statement_item(items: &[BlockItem], index: usize) -> StatementIndex {
    let BlockItem::Statement(statement) = items[index] else {
        panic!("expected a statement block item: {items:?}");
    };
    statement
}

fn expression_statement_text(parsed: &Parsed<'_>, statement: StatementIndex) -> String {
    let StatementType::Expression(ExpressionSlot::Parsed(expression)) =
        parsed.parser.syntax[statement].kind
    else {
        panic!(
            "expected an expression statement: {:?}",
            parsed.parser.syntax[statement].kind
        );
    };
    expression_text(parsed, expression)
}

fn assert_errors_only_at(parsed: &Parsed<'_>, offset: usize) {
    let offsets = error_offsets(parsed);
    assert!(!offsets.is_empty(), "expected a diagnostic");
    assert!(
        offsets.iter().all(|&found| found == offset),
        "every diagnostic should be at offset {offset}: {offsets:?}\n{:#?}",
        parsed.errors
    );
}

// statements:0 -- a stray token in a statement header must resynchronize to
// the header's `)` so the real body stays attached to its statement.

#[test]
fn stray_operand_in_while_header_keeps_the_body_attached() {
    let source = "void f(int x) {\n  while (x y) { x--; }\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find(" y)").unwrap() + 1);
        let items = body_items(parsed);
        assert_eq!(items.len(), 1, "{items:?}");
        let StatementType::While { body_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a while statement");
        };
        assert_eq!(block_items(parsed, body_statement).len(), 1);
    });
}

#[test]
fn stray_operand_in_if_header_keeps_the_then_statement() {
    let source = "void f(int x) {\n  if (x 2) x = 4;\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find('2').unwrap());
        let items = body_items(parsed);
        assert_eq!(items.len(), 1, "{items:?}");
        let StatementType::If { then_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected an if statement");
        };
        assert_eq!(expression_statement_text(parsed, then_statement), "x=4");
    });
}

#[test]
fn stray_operand_in_switch_header_keeps_the_body_attached() {
    let source = "void f(int x) {\n  switch (x y) { default: x--; }\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find(" y)").unwrap() + 1);
        let items = body_items(parsed);
        assert_eq!(items.len(), 1, "{items:?}");
        let StatementType::Switch { body_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a switch statement");
        };
        assert_eq!(block_items(parsed, body_statement).len(), 1);
    });
}

#[test]
fn stray_operand_in_do_while_condition_does_not_leave_a_stray_statement() {
    let source = "void f(int x) {\n  do x--; while (x y);\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find(" y)").unwrap() + 1);
        let items = body_items(parsed);
        assert_eq!(items.len(), 2, "{items:?}");
        assert!(matches!(
            parsed.parser.syntax[statement_item(items, 0)].kind,
            StatementType::DoWhile { .. }
        ));
        assert_eq!(
            expression_statement_text(parsed, statement_item(items, 1)),
            "x=1"
        );
    });
}

#[test]
fn extra_clause_in_for_header_keeps_the_body_attached() {
    let source = "void f(int x) {\n  for (x; x; x; x) x--;\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find("; x)").unwrap());
        let items = body_items(parsed);
        assert_eq!(items.len(), 1, "{items:?}");
        let StatementType::For { body_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a for statement");
        };
        assert_eq!(expression_statement_text(parsed, body_statement), "x--");
    });
}

#[test]
fn stray_operand_in_for_iteration_keeps_the_body_attached() {
    let source = "void f(int x) {\n  for (;; x y(1)) x--;\n}\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find(" y(").unwrap() + 1);
        let items = body_items(parsed);
        assert_eq!(items.len(), 1, "{items:?}");
        let StatementType::For { body_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a for statement");
        };
        assert_eq!(expression_statement_text(parsed, body_statement), "x--");
    });
}

#[test]
fn header_resynchronization_never_skips_past_a_body_brace() {
    let source = "void f(int x) {\n  while (x y { x--; }\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        let items = body_items(parsed);
        assert_eq!(
            expression_statement_text(parsed, statement_item(items, items.len() - 1)),
            "x=1"
        );
    });
}

// statements:1 -- a missing `while` after a `do` body must not swallow the
// following statement as the loop condition.

#[test]
fn missing_while_after_do_body_keeps_the_next_statement() {
    let source = "void f(int x) {\n  do x--;\n  x = 2;\n}\n";
    with_parse(source, |parsed| {
        assert_eq!(
            parser_errors(parsed).collect::<Vec<_>>(),
            [&ParserErrorType::ExpectedWhileAfterDoBody(Some(
                crate::translation_phases::preprocessing::TokenType::Identifier
            ))],
        );
        let items = body_items(parsed);
        assert_eq!(items.len(), 2, "{items:?}");
        let StatementType::DoWhile {
            condition_expression,
            ..
        } = parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a do statement");
        };
        assert!(matches!(condition_expression, ExpressionSlot::Missing(_)));
        assert_eq!(
            expression_statement_text(parsed, statement_item(items, 1)),
            "x=2"
        );
    });
}

#[test]
fn missing_while_before_return_has_no_semicolon_hint() {
    with_parse("void f(int x) {\n  do x--;\n  return;\n}\n", |parsed| {
        assert_eq!(parser_errors(parsed).count(), 1, "{:#?}", parsed.errors);
        assert!(parsed.errors.iter().all(|error| !matches!(
            error,
            TranslationError::Parsing(error) if error.insertion_point.is_some()
        )));
        assert_eq!(body_items(parsed).len(), 2);
    });
}

// statements:4 -- a stray `:`, `)` or `]` at the start of a statement draws
// one diagnostic, and the statement after it parses cleanly.

#[test]
fn stray_colon_at_statement_start_is_one_diagnostic() {
    let source = "void f(void) { : break; }\n";
    with_parse(source, |parsed| {
        assert_errors_only_at(parsed, source.find(':').unwrap());
        let items = body_items(parsed);
        assert!(matches!(
            parsed.parser.syntax[statement_item(items, items.len() - 1)].kind,
            StatementType::Break
        ));
    });
}

#[test]
fn stray_closing_delimiters_at_statement_start_are_one_diagnostic() {
    for stray in [")", "]"] {
        let source = format!("void f(int x) {{ {stray} x = 1; }}\n");
        with_parse(&source, |parsed| {
            assert_errors_only_at(parsed, source.find("{ ").unwrap() + 2);
            assert_eq!(
                parser_errors(parsed).next(),
                Some(&ParserErrorType::ExpectedStatement(Some(
                    crate::translation_phases::preprocessing::TokenType::Operator(
                        if stray == ")" {
                            crate::translation_phases::preprocessing::OperatorTokenType::ClosingParenthesis
                        } else {
                            crate::translation_phases::preprocessing::OperatorTokenType::ClosingSquareBracket
                        }
                    )
                ))),
            );
            let items = body_items(parsed);
            assert_eq!(
                expression_statement_text(parsed, statement_item(items, items.len() - 1)),
                "x=1"
            );
        });
    }
}

// statements:6 -- a duplicate `default` is reported at its keyword.

#[test]
fn duplicate_default_points_at_the_second_keyword() {
    let source = "void f(int x) {\n  switch (x) default: default: ;\n}\n";
    let second = source.rfind("default").unwrap();
    with_parse(source, |parsed| {
        let duplicate = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(error.error_type, ParserErrorType::DuplicateDefaultLabel) =>
                    Some(error.position(parsed.context).index),
                | _ => None,
            })
            .expect("duplicate default is diagnosed");
        assert_eq!(duplicate, second);
    });
}

// statements:8 -- a declaration initializer missing its `;` names the
// initializer, not a condition the user never wrote.

#[test]
fn for_declaration_without_semicolon_names_the_initializer() {
    for source in [
        "void f(void) {\n  for (int i) ;\n}\n",
        "void f(void) {\n  for (int i = 0) ;\n}\n",
        "void f(void) {\n  for (int i = 0, j = 1) ;\n}\n",
    ] {
        with_parse(source, |parsed| {
            let first = parser_errors(parsed).next();
            assert!(
                matches!(
                    first,
                    Some(ParserErrorType::ExpectedSemicolonInStatement(
                        "for initializer",
                        _
                    ))
                ),
                "{source}: {:#?}",
                parsed.errors
            );
            assert_errors_only_at(parsed, source.find(") ;").unwrap());
            let items = body_items(parsed);
            assert!(matches!(
                parsed.parser.syntax[statement_item(items, 0)].kind,
                StatementType::For {
                    initializer: Some(ForInitializer::Declaration(_)),
                    ..
                }
            ));
        });
    }
}

#[test]
fn for_declaration_with_semicolon_still_reports_the_condition() {
    with_parse("void f(void) {\n  for (int i = 0; i) ;\n}\n", |parsed| {
        let first = parser_errors(parsed).next();
        assert!(
            matches!(
                first,
                Some(ParserErrorType::ExpectedSemicolonInStatement(
                    "for condition",
                    _
                ))
            ),
            "{:#?}",
            parsed.errors
        );
    });
}

// statements:9 -- a typedef-led declaration in substatement position is not
// a statement and must not be accepted as an expression.

#[test]
fn typedef_declaration_as_substatement_is_rejected() {
    let source = "typedef int T;\nvoid f(int x) {\n  while (x) T *p;\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        assert_eq!(
            parser_errors(parsed).collect::<Vec<_>>(),
            [&ParserErrorType::ExpectedStatement(Some(
                crate::translation_phases::preprocessing::TokenType::Identifier
            ))],
        );
        assert_errors_only_at(parsed, source.find("T *p").unwrap());
        let items = block_items(parsed, function_definition(parsed, 1).body);
        assert_eq!(items.len(), 2, "{items:?}");
    });
}

#[test]
fn typedef_declaration_after_label_or_if_is_one_diagnostic_each() {
    let source = "typedef int T;\nvoid f(int x) {\nL: T y;\n  if (x) T z;\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        assert_eq!(
            error_offsets(parsed),
            [source.find("T y").unwrap(), source.find("T z").unwrap()],
            "{:#?}",
            parsed.errors
        );
        let items = block_items(parsed, function_definition(parsed, 1).body);
        assert_eq!(items.len(), 3, "{items:?}");
    });
}

#[test]
fn shadowed_typedef_name_in_substatement_is_still_an_expression() {
    with_parse(
        "typedef int T;\nvoid f(int x) {\n  int T;\n  if (x) T * x;\n}\n",
        |parsed| {
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

#[test]
fn several_extra_for_clauses_with_nested_parentheses_are_skipped_together() {
    let source = "void f(int x) {\n  for (x; x; x; g(x, (x)); x) x--;\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        let items = body_items(parsed);
        let StatementType::For { body_statement, .. } =
            parsed.parser.syntax[statement_item(items, 0)].kind
        else {
            panic!("expected a for statement");
        };
        assert_ne!(
            parsed.parser.syntax[body_statement].kind,
            StatementType::Null,
            "{:#?}",
            parsed.errors
        );
        assert_eq!(
            expression_statement_text(parsed, statement_item(items, items.len() - 1)),
            "x=1"
        );
    });
}

#[test]
fn extra_for_clause_skipping_never_passes_a_body_brace() {
    let source = "void f(int x) {\n  for (x; x; x; x { x--; }\n  x = 1;\n}\n";
    with_parse(source, |parsed| {
        let items = body_items(parsed);
        assert_eq!(
            expression_statement_text(parsed, statement_item(items, items.len() - 1)),
            "x=1"
        );
    });
}

#[test]
fn for_declaration_continuation_does_not_offer_the_closing_parenthesis() {
    // Round-2 review: the message listed `)`, which the for statement then
    // rejected with "expected `;` after `for` initializer".
    with_parse("void f(void){ for (int i j) ; }\n", |parsed| {
        let messages: Vec<String> = parsed.errors.iter().map(ToString::to_string).collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("after the declarator")),
            "{messages:#?}"
        );
        assert!(
            messages
                .iter()
                .all(|message| !message.contains("`)` after the declarator")),
            "{messages:#?}"
        );
    });
}
