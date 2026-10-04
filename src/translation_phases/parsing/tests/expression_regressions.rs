//! Regression tests for verified expression bugs found by the overnight bug
//! hunt.

use super::with_parsed;
use crate::translation_phases::{
    TranslationError,
    parsing::{
        errors::ParserErrorType,
        inspection::InspectionOptions,
    },
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        TokenType,
    },
};

/// Selects the diagnostic a test case expects.
type ErrorPredicate = fn(&ParserErrorType<'_>) -> bool;

/// The syntax tree and every parser diagnostic of one translation unit.
struct Outcome<'tu> {
    tree:   String,
    /// Each diagnostic with the byte offset where its primary span starts.
    errors: Vec<(ParserErrorType<'tu>, usize)>,
}

impl Outcome<'_> {
    /// Distinct diagnostic locations, the way the CLI folds diagnostics that
    /// share one location into a single report.
    fn locations(&self) -> Vec<usize> {
        let mut locations = self
            .errors
            .iter()
            .map(|(_, offset)| *offset)
            .collect::<Vec<_>>();
        locations.sort_unstable();
        locations.dedup();
        locations
    }

    /// Number of top-level syntax-tree items.
    fn roots(&self) -> usize {
        self.tree
            .lines()
            .filter(|line| !line.starts_with(' '))
            .count()
    }

    fn has(&self, predicate: impl Fn(&ParserErrorType<'_>) -> bool) -> bool {
        self.errors.iter().any(|(error, _)| predicate(error))
    }

    fn reports_missing_operator(&self) -> bool {
        self.has(|error| {
            matches!(
                error,
                ParserErrorType::ExpectedStatementExpression("operator in expression", _)
            )
        })
    }
}

fn with_run(source: &str, f: impl FnOnce(&Outcome<'_>)) {
    with_parsed(source, |unit, context| {
        let tree = unit.inspect(context, InspectionOptions::default());
        let mut errors = Vec::new();
        while let Some(error) = context.pop_pending_error() {
            if let TranslationError::Parsing(error) = error {
                let offset = context
                    .get_source_vectors(error.source_vectors)
                    .first()
                    .map_or(usize::MAX, |vector| vector.range().start);
                errors.push((error.error_type, offset));
            }
        }
        f(&Outcome { tree, errors });
    });
}

fn offset_of(source: &str, needle: &str) -> usize {
    source.find(needle).expect("needle occurs in the source")
}

// corpus-triage-A:0: a stray operand inside a nested `(` must resync to the
// matching `)` instead of leaking the rest of the group into new statements.
#[test]
fn stray_operand_inside_parentheses_resyncs_to_the_matching_closer() {
    let source = "void f(int a, int b) { a = (a b); a = 1; }\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "b);")],
            "{:?}",
            outcome.errors
        );
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            2,
            "{}",
            outcome.tree
        );
        assert!(
            outcome.tree.contains("parenthesized recovered"),
            "{}",
            outcome.tree
        );
    });

    let source = "void f(int a, int b) { if ((a b)) a = 1; else b = 2; }\n";
    with_run(source, |outcome| {
        assert_eq!(outcome.locations().len(), 1, "{:?}", outcome.errors);
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            1,
            "{}",
            outcome.tree
        );
        assert!(
            outcome.tree.contains("then: expression"),
            "{}",
            outcome.tree
        );
        assert!(
            outcome.tree.contains("else: expression"),
            "{}",
            outcome.tree
        );
    });

    let source = "void f(int a, int b) { a = ((a b c) + 1) + 2; f((a b), 1); }\n";
    with_run(source, |outcome| {
        assert_eq!(outcome.locations().len(), 2, "{:?}", outcome.errors);
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            2,
            "{}",
            outcome.tree
        );
    });
}

// The resync must not swallow a following statement when the `)` is really
// missing: the closer's owner then names the missing `)` itself.
#[test]
fn missing_closer_still_stops_before_the_statement_terminator() {
    let source = "void f(int a, int b) { a = (a\n b; a = 1; }\n";
    with_run(source, |outcome| {
        // The first report at `b` is the one the CLI shows.
        assert!(
            matches!(
                outcome.errors.first(),
                Some((
                    ParserErrorType::ExpectedClosingParenthesisInStatement("grouped expression", _),
                    offset
                )) if *offset == offset_of(source, "b;")
            ),
            "{:?}",
            outcome.errors
        );
        assert_eq!(outcome.locations().len(), 1, "{:?}", outcome.errors);
        assert!(outcome.tree.contains("rhs: constant 1"), "{}", outcome.tree);
    });
}

// expressions:0: the same stray operand directly inside a statement header.
#[test]
fn stray_operand_in_a_condition_keeps_the_body_attached() {
    let source = "void g(int a, int b, int c) { while (a b) { c = 1; } if (a b c) c = 2; }\n";
    with_run(source, |outcome| {
        assert_eq!(outcome.locations().len(), 2, "{:?}", outcome.errors);
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            3,
            "{}",
            outcome.tree
        );
        assert!(outcome.tree.contains("body: compound"), "{}", outcome.tree);
        assert!(
            outcome.tree.contains("then: expression"),
            "{}",
            outcome.tree
        );
    });
}

// corpus-triage-A:2: a declaration starter inside an open call parenthesis
// must not be reparsed as a new declaration.
#[test]
fn declaration_starter_inside_a_call_stays_inside_the_call() {
    let source = "int f();\nstruct B { char a[f(int)]; int y; };\nint x;\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "int)")],
            "{:?}",
            outcome.errors
        );
        assert!(!outcome.tree.contains("root["), "{}", outcome.tree);
        assert!(outcome.tree.contains("identifier y"), "{}", outcome.tree);
        assert!(
            outcome.tree.contains(
                "\ndeclaration: declaration storage=none type=int qualifiers=none \
                 function-specifiers=none\n  declarator x"
            ),
            "{}",
            outcome.tree
        );
    });

    for source in [
        "int f();\nvoid g(void){ f(int); f(2); }\n",
        "int f();\nint a = f(int);\n",
        "int f();\nint a = f(1, int);\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(outcome.roots(), 2, "{source}: {}", outcome.tree);
            assert!(
                !outcome.tree.contains("block-item: declaration"),
                "{source}: {}",
                outcome.tree
            );
        });
    }
}

// declarations:7: storage classes and type specifiers inside a grouped
// expression or array bound build no phantom declarations.
#[test]
fn declaration_specifiers_inside_brackets_build_no_phantom_declaration() {
    for source in [
        "int x = (static int)1;\n",
        "int x = sizeof(static int);\n",
        "int x = (1 + struct s);\n",
        "int x = (typedef int)1;\n",
        "int x = (1 static);\n",
        "void g(int a[3 static]);\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(outcome.roots(), 1, "{source}: {}", outcome.tree);
            assert!(
                !outcome.tree.contains("no type specifier"),
                "{source}: {}",
                outcome.tree
            );
        });
    }
    with_run("void g(int a[3 static]);\n", |outcome| {
        assert!(
            !outcome.has(|error| matches!(
                error,
                ParserErrorType::ExpectedStatementExpression("operator in expression", _)
            )),
            "inside brackets the missing-`;` help does not apply: {:?}",
            outcome.errors
        );
    });
}

// statements:5: the case label diagnostics name the missing `:` and resync.
#[test]
fn malformed_case_labels_report_the_missing_colon_once() {
    for (source, found) in [
        ("void f(int x) { switch (x) { case 1; } }\n", "; }"),
        ("void f(int x) { switch (x) { case 4 x = 2; } }\n", "x = 2"),
    ] {
        with_run(source, |outcome| {
            assert!(
                outcome.has(|error| matches!(
                    error,
                    ParserErrorType::ExpectedColonInLabel("case label", _)
                )),
                "{source}: {:?}",
                outcome.errors
            );
            assert!(
                !outcome.reports_missing_operator(),
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(
                outcome.locations(),
                [offset_of(source, found)],
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
    for source in [
        "void f(int x) { switch (x) { case 1 2: break; } x = 3; }\n",
        "void f(int x) { switch (x) { case 2, 3: break; } x = 3; }\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert!(
                outcome.tree.contains("labeled: break"),
                "{source}: {}",
                outcome.tree
            );
        });
    }
}

// declarations:11, aggregates-initializers:6, expressions:3: a `;` before the
// closer is reported by the closer's owner, not as a missing operator.
#[test]
fn semicolon_before_a_closer_names_the_missing_closer() {
    let cases: [(&str, ErrorPredicate); 8] = [
        ("int a[1;\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(_)
            )
        }),
        ("struct S { int a[3; int b; };\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(_)
            )
        }),
        ("int b[] = { [1 ; int ok2;\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(_)
            )
        }),
        ("int x = (1;\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisInStatement("grouped expression", _)
            )
        }),
        ("void g(int a, int *p) { a = p[a; }\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketInSubscript(_)
            )
        }),
        ("void g(int a, int b) { a = b ? a; }\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedColonInLabel("conditional expression", _)
            )
        }),
        ("int f(); void g(int a) { a = f(a, a; }\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisInStatement("function call", _)
            )
        }),
        ("void g(int x) { if (x; }\n", |error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingParenthesisInStatement("if statement", _)
            )
        }),
    ];
    for (source, expected) in cases {
        with_run(source, |outcome| {
            assert!(outcome.has(expected), "{source}: {:?}", outcome.errors);
            assert!(
                !outcome.reports_missing_operator(),
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
}

// expressions:1, aggregates-initializers:4: a missing left operand keeps the
// stray binary operator, so its right operand parses normally.
#[test]
fn missing_left_operand_keeps_the_binary_operator() {
    let source = "int a, b; void g(void){ a = / b; }\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "/ b")],
            "{:?}",
            outcome.errors
        );
        assert!(outcome.tree.contains("binary /"), "{}", outcome.tree);
    });

    let source = "int a, b, c;\nvoid g(void){\n  if (a && (|| c)) a = b; else c = a;\n}\n";
    with_run(source, |outcome| {
        assert_eq!(outcome.locations().len(), 1, "{:?}", outcome.errors);
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            1,
            "{}",
            outcome.tree
        );
        assert!(
            outcome.tree.contains("else: expression"),
            "{}",
            outcome.tree
        );
    });

    for source in [
        "int x = = 2;\n",
        "int a[] = { = 2 };\n",
        "void f(void){ int x; x = (= 2); int ok; }\n",
        "int x; void f(void){ x = (/ 2); }\n",
        "int a, b, c; void g(void){ a = b , , c; }\n",
        "int a, b, c; void g(void){ a = b = = c; }\n",
        "int a, b, c; void g(void){ a = (a > ? b : c); }\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
}

// expressions:4: operand diagnostics do not read "an expression in expression".
#[test]
fn missing_operand_diagnostics_use_the_plain_wording() {
    with_run("int a, b, c; void g(void){ a = b , , c; }\n", |outcome| {
        assert!(
            !outcome.has(|error| matches!(
                error,
                ParserErrorType::ExpectedStatementExpression("expression", _)
            )),
            "{:?}",
            outcome.errors
        );
        let message = ParserErrorType::ExpectedStatementExpression(
            "expression operand",
            Some(TokenType::Operator(OperatorTokenType::Comma)),
        );
        assert!(
            outcome.has(|error| *error == message),
            "{:?}",
            outcome.errors
        );
    });
}

// expressions:5: a postfix operator after `sizeof(type)` is one error and the
// suffix stays inside the expression.
#[test]
fn postfix_after_a_non_postfix_expression_is_one_error() {
    for source in [
        "void g(void){ sizeof(int)[0]; }\n",
        "int a; void g(void){ a = sizeof(int)[0]; }\n",
        "void g(void){ sizeof(int)++; }\n",
        "void g(void){ sizeof(int)(0); }\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(
                outcome.tree.matches("block-item:").count(),
                1,
                "{source}: {}",
                outcome.tree
            );
        });
    }
}

// expressions:6: `++(int)a` is one error at the operand that should have been
// a `{`, not at the correct `)`.
#[test]
fn unary_operator_on_a_cast_reports_the_missing_brace_once() {
    let source = "int a;\nvoid g(void){\n  ++(int)a;\n}\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "a;\n}")],
            "{:?}",
            outcome.errors
        );
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            1,
            "{}",
            outcome.tree
        );
    });
}

// expressions:8: a brace list in operand position is one error and is
// skipped as a whole, unless it is a statement block.
#[test]
fn brace_list_in_operand_position_is_skipped_as_one_error_operand() {
    let source = "int a, b; void g(void){ a = {1, 2}; b = 2; }\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "{1")],
            "{:?}",
            outcome.errors
        );
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            2,
            "{}",
            outcome.tree
        );
    });

    for source in [
        "int a[2], b; void g(void){ a[{1}] = 2; b = 2; }\n",
        "int f(); void g(void){ f({1, 2}); }\n",
        "int a; void g(void){ a = (1 + {1, 2}); }\n",
        "int g(void){ return {1, 2}; }\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
        });
    }

    // A real block after a malformed condition stays the statement body.
    with_run("void g(int a){ if (a + { return; } a = 1; }\n", |outcome| {
        assert!(outcome.tree.contains("then: compound"), "{}", outcome.tree);
    });
}

// statements:4 (expression half): a statement keyword after an operand ends
// the expression without a "missing operator" diagnostic.
#[test]
fn statement_keyword_after_an_operand_ends_the_expression() {
    with_run("void f(int x) { x = 1 break; }\n", |outcome| {
        assert!(!outcome.reports_missing_operator(), "{:?}", outcome.errors);
        assert!(
            outcome.has(|error| matches!(
                error,
                ParserErrorType::ExpectedSemicolonInStatement(
                    "expression statement",
                    Some(TokenType::Keyword(KeywordTokenType::Break))
                )
            )),
            "{:?}",
            outcome.errors
        );
        assert!(
            outcome.tree.contains("block-item: break"),
            "{}",
            outcome.tree
        );
    });
}

// Round-2 regression from the corpus (gcc.c-torture/compile/20071107-1.c,
// pr33382.c, pr33173.c): a GNU statement expression `({ ... })` is not C99,
// but the brace group directly inside a `(` is one error operand. The
// enclosing call or parenthesis keeps its `)`, and the statements after the
// expression parse normally.
#[test]
fn brace_group_directly_inside_parentheses_is_one_error_operand() {
    let source =
        "void w(void);\nint e(int, int);\nvoid g(int r)\n{\n  if (e(__extension__ ({\n    int \
         v;\n    if (r == 1) v = 1;\n    else v = 0;\n    v;\n  }), 1)) {\n  }\n  else w();\n  r \
         = 2;\n}\n";
    with_run(source, |outcome| {
        assert_eq!(
            outcome.locations(),
            [offset_of(source, "{\n    int v")],
            "{:?}",
            outcome.errors
        );
        assert_eq!(
            outcome.tree.matches("block-item:").count(),
            2,
            "{}",
            outcome.tree
        );
        assert!(outcome.tree.contains("then: compound"), "{}", outcome.tree);
        assert!(
            outcome.tree.contains("else: expression"),
            "{}",
            outcome.tree
        );
    });

    for (source, block_items) in [
        ("int x = ({ 1; });\nint y;\n", 0),
        ("void g(int a) { a = ({ int t = a; t; }) + 1; a = 2; }\n", 2),
        (
            "int f(); void g(int a) { a = f(({ ({ 1; }); }), 2); a = 2; }\n",
            2,
        ),
        (
            "int f(); void g(int a) { while (f(({ for (;;) break; 0; }))) a = 1; a = 2; }\n",
            2,
        ),
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(
                outcome.tree.matches("block-item:").count(),
                block_items,
                "{source}: {}",
                outcome.tree
            );
        });
    }
    with_run("int x = ({ 1; });\nint y;\n", |outcome| {
        assert_eq!(outcome.roots(), 2);
    });
}

// recovery-fuzz:2 (remaining case): a `;` after an enumerator value is
// reported by the enumerator list, not as a missing operator whose headline
// would read "expected `;`, found `;`".
#[test]
fn semicolon_after_an_enumerator_value_is_reported_by_the_enumerator_list() {
    for source in ["enum e { A = 1\n; };\n", "enum e { A = 1 ; };\n"] {
        with_run(source, |outcome| {
            assert!(
                !outcome.reports_missing_operator(),
                "{source}: {:?}",
                outcome.errors
            );
            assert!(
                outcome.has(|error| matches!(
                    error,
                    ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                        TokenType::Operator(OperatorTokenType::Semicolon)
                    ))
                )),
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
}

// recovery-fuzz:2: an unclosed grouped expression before a `;` on a later
// line, or from a macro, names the missing `)` rather than a missing `;`.
#[test]
fn unclosed_group_before_a_later_semicolon_names_the_missing_parenthesis() {
    for source in [
        "int x = (1\n;\n",
        "void f(int i) {\n  i = (i\n  ;\n}\n",
        "#define SQ(x) ((x) * (x)\nvoid f(int i) {\n  i = SQ(i);\n}\n",
    ] {
        with_run(source, |outcome| {
            assert!(
                outcome.has(|error| matches!(
                    error,
                    ParserErrorType::ExpectedClosingParenthesisInStatement("grouped expression", _)
                )),
                "{source}: {:?}",
                outcome.errors
            );
            assert!(
                !outcome.reports_missing_operator(),
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
}

// recovery-fuzz:5: a misplaced binary operator in operand position is one
// error, and the operator keeps its right operand.
#[test]
fn misplaced_binary_operator_keeps_its_right_operand() {
    for source in [
        "int a = > 1;\n",
        "void f(int x){ x |= , x; }\n",
        "int a = 1 + * / 2;\n",
        "void f(int x){ x = (x |= , x); }\n",
        "int a = sizeof > 1;\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}: {:?}",
                outcome.errors
            );
            assert!(
                !outcome.reports_missing_operator(),
                "{source}: {:?}",
                outcome.errors
            );
        });
    }
    with_run("void f(int x){ if ( > 1) ; }\n", |outcome| {
        assert_eq!(outcome.locations().len(), 1, "{:?}", outcome.errors);
        assert!(
            outcome.tree.contains("condition: binary >"),
            "{}",
            outcome.tree
        );
        assert!(outcome.tree.contains("then: null"), "{}", outcome.tree);
    });
}

// Corpus (gcc.c-torture/execute/scal-to-vec1.c): a brace list after a
// parenthesized expression that is not a type name, as in a compound
// literal with an attribute in its type name, is skipped with the operand
// instead of leaking its `}` to the enclosing statement.
#[test]
fn brace_list_after_a_parenthesized_expression_is_skipped_with_it() {
    for (source, location) in [
        ("void g(int a, int b) { a = (a b){1, 2}; a = 2; }\n", "b){"),
        ("void g(int a) { a = (a){1, 2}; a = 2; }\n", "{1"),
        (
            "void g(int a) { a = (__attribute__((x)) float){2., 2.} + 1; a = 2; }\n",
            "float",
        ),
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations(),
                [offset_of(source, location)],
                "{source}: {:?}",
                outcome.errors
            );
            assert_eq!(
                outcome.tree.matches("block-item:").count(),
                2,
                "{source}: {}",
                outcome.tree
            );
        });
    }

    // A statement block after a call in a malformed condition stays the body.
    with_run(
        "int f(int); void g(int a) { while (f(a) { a = 1; } a = 2; }\n",
        |outcome| {
            assert!(outcome.tree.contains("body: compound"), "{}", outcome.tree);
            assert_eq!(outcome.locations().len(), 1, "{:?}", outcome.errors);
        },
    );
}

#[test]
fn brace_group_before_a_header_closer_keeps_the_statement_intact() {
    // Round-2 review: a block-shaped or empty brace group followed by the
    // header's `)` was taken as the statement body, detaching the real
    // body and `else`.
    for source in [
        "void f(int x){ if (x == {}) x = 1; else x = 2; }\n",
        "void f(int i) {\n  if (i + { i; }) i = 1;\n  i = 2;\n}\n",
        "void f(int x){ while (x == {}) x = 1; }\n",
    ] {
        with_run(source, |outcome| {
            assert_eq!(
                outcome.locations().len(),
                1,
                "{source}\n{:#?}",
                outcome.errors
            );
            assert!(
                !outcome.tree.contains("null recovered"),
                "{source}\n{}",
                outcome.tree
            );
        });
    }
    with_run(
        "void f(int x){ if (x == {}) x = 1; else x = 2; }\n",
        |outcome| {
            assert!(outcome.tree.contains("else"), "{}", outcome.tree);
        },
    );
}

#[test]
fn block_after_a_missing_header_closer_still_becomes_the_body() {
    with_run(
        "int f(int a) { if (a + { return 1; } return 0; }\n",
        |outcome| {
            assert!(outcome.tree.contains("then: compound"), "{}", outcome.tree);
        },
    );
}
