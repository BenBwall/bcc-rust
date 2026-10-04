//! Regression tests for verified declaration bugs found by the overnight bug
//! hunt.

use super::{
    block_items,
    declaration,
    function_definition,
    identifier_name,
    parser_errors,
    with_parse,
};
use crate::translation_phases::{
    TranslationError,
    parsing::{
        errors::ParserErrorType,
        syntax::ExternalDeclaration,
    },
};

/// Every distinct ordering of `words`, generated iteratively (Heap's
/// algorithm) so duplicate keywords such as `long long` are handled by
/// deduplicating the resulting spellings.
fn orderings(words: &[&'static str]) -> Vec<Vec<&'static str>> {
    let mut current = words.to_vec();
    let mut counters = vec![0; current.len()];
    let mut result = vec![current.clone()];
    let mut index = 1;
    while index < current.len() {
        if counters[index] < index {
            let swap_with = if index % 2 == 0 { 0 } else { counters[index] };
            current.swap(swap_with, index);
            if !result.contains(&current) {
                result.push(current.clone());
            }
            counters[index] += 1;
            index = 1;
        } else {
            counters[index] = 0;
            index += 1;
        }
    }
    result
}

/// C99 §6.7.2p2: the type specifiers of each listed multiset "may occur in
/// any order". Every ordering must be accepted and normalize to the same
/// canonical type.
#[test]
fn every_ordering_of_every_c99_type_specifier_multiset_is_accepted() {
    let multisets: &[&[&str]] = &[
        &["void"],
        &["char"],
        &["signed", "char"],
        &["unsigned", "char"],
        &["short"],
        &["signed", "short"],
        &["short", "int"],
        &["signed", "short", "int"],
        &["unsigned", "short"],
        &["unsigned", "short", "int"],
        &["int"],
        &["signed"],
        &["signed", "int"],
        &["unsigned"],
        &["unsigned", "int"],
        &["long"],
        &["signed", "long"],
        &["long", "int"],
        &["signed", "long", "int"],
        &["unsigned", "long"],
        &["unsigned", "long", "int"],
        &["long", "long"],
        &["signed", "long", "long"],
        &["long", "long", "int"],
        &["signed", "long", "long", "int"],
        &["unsigned", "long", "long"],
        &["unsigned", "long", "long", "int"],
        &["float"],
        &["double"],
        &["long", "double"],
        &["_Bool"],
        &["float", "_Complex"],
        &["double", "_Complex"],
        &["long", "double", "_Complex"],
    ];
    let mut failures = Vec::new();
    for multiset in multisets {
        // Each parse has its own tree, so the normalized specifiers are
        // compared by their (reference-free) debug form.
        let mut canonical: Option<String> = None;
        for ordering in orderings(multiset) {
            let source = format!("{} x;\n", ordering.join(" "));
            with_parse(&source, |parsed| {
                let errors: Vec<_> = parser_errors(parsed).collect();
                if !errors.is_empty() {
                    failures.push(format!("{source:?}: {errors:?}"));
                    return;
                }
                let type_specifiers = format!(
                    "{:?}",
                    declaration(parsed, 0)
                        .declaration_specifiers
                        .type_specifiers
                );
                match &canonical {
                    | None => canonical = Some(type_specifiers),
                    | Some(expected) if *expected != type_specifiers => failures.push(format!(
                        "{source:?}: normalized to {type_specifiers}, expected {expected}"
                    )),
                    | Some(_) => {},
                }
            });
        }
    }
    assert!(
        failures.is_empty(),
        "rejected or misnormalized:\n{}",
        failures.join("\n")
    );
}

#[test]
fn unsigned_short_int_is_accepted() {
    for source in [
        "unsigned short int x;",
        "signed short int x;",
        "short signed int x;",
        "unsigned int short x;",
        "int unsigned short x;",
        "signed int short x;",
        "unsigned int long x;",
        "signed int long x;",
    ] {
        with_parse(source, |parsed| {
            assert!(
                !parser_errors(parsed).any(|error| matches!(
                    error,
                    ParserErrorType::ConflictingTypeSpecifiers { .. }
                )),
                "{source:?} was rejected"
            );
        });
    }
}

#[test]
fn repeated_double_is_a_duplicate_not_a_conflict() {
    with_parse("double double x;\n", |parsed| {
        let errors: Vec<_> = parser_errors(parsed).collect();
        assert_eq!(errors.len(), 1, "{:#?}", parsed.errors);
        assert!(
            matches!(errors[0], ParserErrorType::TypeSpecifierSpecifiedTwice(_)),
            "{errors:?}"
        );
    });
}

/// C99 §6.7.2p2 lists `_Complex` only together with `float`, `double`, or
/// `long double`; plain `_Complex` and `long _Complex` are not valid
/// multisets in any specifier context.
#[test]
fn incomplete_complex_type_specifiers_are_diagnosed() {
    for source in [
        "_Complex c;\n",
        "long _Complex d;\n",
        "_Complex long d;\n",
        "struct s { _Complex m; };\n",
        "void f(_Complex p);\n",
        "int x = sizeof(_Complex);\n",
        "int y = (_Complex long)0;\n",
        "const _Complex;\n",
        "_Complex",
    ] {
        with_parse(source, |parsed| {
            let count = parser_errors(parsed)
                .filter(|error| matches!(error, ParserErrorType::IncompleteComplexTypeSpecifier))
                .count();
            assert_eq!(count, 1, "{source:?}: {:#?}", parsed.errors);
        });
    }
    with_parse(
        "float _Complex a; _Complex double b; long _Complex double c; double long _Complex d;\n",
        |parsed| {
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
        },
    );
}

/// C99 §6.9.1p1: an identifier-list head followed by a declaration-list is a
/// function definition even when the head itself needed recovery.
#[test]
fn old_style_head_with_recovered_error_keeps_its_declaration_list_and_body() {
    for (source, list_length) in [
        ("int f(a b) int a; int b; { return a; }\n", 2),
        ("f(a) int a; { return a; }\n", 1),
        ("static f(a) int a; { return a; }\n", 1),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                parser_errors(parsed).count(),
                1,
                "{source:?}: {:#?}",
                parsed.errors
            );
            assert_eq!(parsed.items.len(), 1, "{source:?}");
            assert!(
                matches!(
                    parsed.items[0],
                    ExternalDeclaration::RecoveredFunctionDefinition(_)
                ),
                "{source:?}"
            );
            let definition = function_definition(parsed, 0);
            assert_eq!(definition.declaration_list.len(), list_length, "{source:?}");
            assert_eq!(block_items(definition.body).len(), 1, "{source:?}");
        });
    }

    // A non-function declarator with an error still resynchronizes at the
    // next declaration instead of becoming a definition head.
    with_parse("int x[3 int y;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
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
            Some("y")
        );
    });
}

/// Once a declaration recovered from a bad continuation token, reaching the
/// synchronization point must not report the same mistake again.
#[test]
fn declaration_continuation_recovery_reports_once() {
    for (source, items) in [
        ("int x y\nint z;\n", 2),
        ("int x [ 1 ] 5", 1),
        ("int x, h(void) { return 0; }\n", 1),
        ("int x, h(void) { return 0; }\nint z;\n", 2),
        ("void f(void) { int x y\n int z; }\n", 1),
        ("void f(void) { int x 5 return; }\n", 1),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                parser_errors(parsed).count(),
                1,
                "{source:?}: {:#?}",
                parsed.errors
            );
            assert_eq!(parsed.items.len(), items, "{source:?}");
        });
    }
}

fn continuation_explanation(source: &str) -> (String, String) {
    with_parse(source, |parsed| {
        let explanation = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(..)
                    ) =>
                    Some(error.error_type.explain(None)),
                | _ => None,
            })
            .unwrap_or_else(|| panic!("{source:?}: no continuation error in {:#?}", parsed.errors));
        (explanation.message, explanation.label.unwrap_or_default())
    })
}

/// The expected continuation set depends on where the declaration is: a
/// diagnostic must never offer the very token it rejects.
#[test]
fn declaration_continuation_expectations_follow_the_context() {
    // External declarator without an initializer: the full set.
    let (message, label) = continuation_explanation("int a extra;\n");
    assert!(
        message.contains("`,`, `=`, `;`, or a function body"),
        "{message}"
    );
    assert_eq!(label, "expected one of `,`, `=`, `;`, or `{`");

    // `{` is the rejected token in each of these contexts.
    for source in [
        "void f(a) int a {}\n",
        "int f(a) int a {}\n",
        "int x\nint main(void) { return 0; }\n",
        "int x, h(void) { return 0; }\n",
        "void f(void) { int g(void) { return 1; } }\n",
        "void f(void) { for (int g(void) { return 1; } }\n",
    ] {
        let (message, label) = continuation_explanation(source);
        assert!(!message.contains("function body"), "{source:?}: {message}");
        assert!(!label.contains('{'), "{source:?}: {label}");
    }

    // After an initializer, neither `=` nor a function body can follow.
    for source in ["int x = 1 }\n", "void f(void) { int x = 1 }\n"] {
        let (message, label) = continuation_explanation(source);
        assert!(!message.contains('='), "{source:?}: {message}");
        assert!(!message.contains("function body"), "{source:?}: {message}");
        assert!(!label.contains('='), "{source:?}: {label}");
        assert!(!label.contains('{'), "{source:?}: {label}");
    }

    // A `for` initializer declaration still ends with `;` (C99 6.8.5p1);
    // `)` only ends recovery there, so it is not offered.
    let (message, label) = continuation_explanation("void f(void) { for (int i = 0 } }\n");
    assert!(!message.contains("`)`"), "{message}");
    assert!(!message.contains('='), "{message}");
    assert!(!label.contains("`)`"), "{label}");
}

/// An identifier that is not a visible typedef, followed by another
/// declarator start, is an unknown type name: report it once and keep
/// parsing the real declarator.
#[test]
fn unknown_type_name_is_reported_once() {
    for (source, name) in [
        ("size_t n;\n", Some("n")),
        ("size_t *p;\n", Some("p")),
        ("struct s { size_t n; int m; };\n", None),
        ("int f(int a, size_t n);\n", None),
    ] {
        with_parse(source, |parsed| {
            let errors: Vec<_> = parser_errors(parsed).collect();
            assert_eq!(errors.len(), 1, "{source:?}: {:#?}", parsed.errors);
            assert!(
                matches!(errors[0], ParserErrorType::UnknownTypeName),
                "{source:?}: {errors:?}"
            );
            if let Some(name) = name {
                assert_eq!(
                    identifier_name(
                        parsed,
                        declaration(parsed, 0).init_declarators[0].declarator
                    )
                    .as_deref(),
                    Some(name),
                    "{source:?}"
                );
            }
        });
    }

    // A lone identifier is still a declarator lacking its type.
    with_parse("x;\n", |parsed| {
        let errors: Vec<_> = parser_errors(parsed).collect();
        assert_eq!(errors.len(), 1, "{:#?}", parsed.errors);
        assert!(matches!(
            errors[0],
            ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(_)
        ));
    });
}

/// A non-typedef identifier in the empty type slot followed by another
/// declaration-specifier keyword cannot be a declarator either: it is an
/// unknown type name, and the rest of the specifier list still applies.
#[test]
fn unknown_type_name_before_specifier_keyword_is_reported_once() {
    for (source, name) in [
        ("int f(foo const);\n", None),
        ("int f(foo const char *p, int m, ...);\n", None),
        ("foo const x;\n", Some("x")),
        ("foo volatile int *x;\n", Some("x")),
        ("struct s { foo const m; int n; };\n", None),
        (
            "void g(void) { int y; { register foo unsigned z; } }\n",
            None,
        ),
    ] {
        with_parse(source, |parsed| {
            let errors: Vec<_> = parser_errors(parsed).collect();
            assert_eq!(errors.len(), 1, "{source:?}: {:#?}", parsed.errors);
            assert!(
                matches!(errors[0], ParserErrorType::UnknownTypeName),
                "{source:?}: {errors:?}"
            );
            if let Some(name) = name {
                assert_eq!(
                    identifier_name(
                        parsed,
                        declaration(parsed, 0).init_declarators[0].declarator
                    )
                    .as_deref(),
                    Some(name),
                    "{source:?}"
                );
            }
        });
    }
}

/// Declaration recovery that resumed at a declaration starter or at end of
/// file after skipping a balanced group must end the declaration quietly.
#[test]
fn declaration_recovery_over_a_brace_group_reports_once() {
    for (source, items) in [
        ("int a 3 {}\nint b;\n", 2),
        ("int a 3 { 1, 2 }\n", 1),
        ("int a 3 {}\n", 1),
        ("void f(int x)) { }\nint y;\n", 2),
        ("void f(int x)) {\n  x = 1;\n}\n", 1),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                parser_errors(parsed).count(),
                1,
                "{source:?}: {:#?}",
                parsed.errors
            );
            assert_eq!(parsed.items.len(), items, "{source:?}");
        });
    }
}

/// A declaration that is not a function only became a definition head
/// because a declaration followed it; when that following declaration then
/// fails, the diagnostic explains the probable missing `;`.
#[test]
fn missing_semicolon_before_function_definition_suggests_semicolon() {
    let source = "int x\nint f(void) {}\n";
    with_parse(source, |parsed| {
        let parsing_errors: Vec<_> = parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Parsing(error) => Some(error),
                | _ => None,
            })
            .collect();
        assert_eq!(parsing_errors.len(), 1, "{:#?}", parsed.errors);
        let error = parsing_errors[0];
        assert!(error.insertion_point.is_some(), "{error:#?}");
        assert_eq!(error.related.len(), 1, "{error:#?}");
        assert!(
            error.related[0].message.contains("not a function"),
            "{error:#?}"
        );
    });
}

/// The missing-`;` explanation for a non-function head never replaces an
/// insertion point that a declaration-list diagnostic already proposes for
/// its own missing `;`.
#[test]
fn head_semicolon_suggestion_keeps_an_existing_insertion_point() {
    with_parse("int z\nint q\nint g(void) { return 1; }\n", |parsed| {
        let parsing_errors: Vec<_> = parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Parsing(error) => Some(error),
                | _ => None,
            })
            .collect();
        assert_eq!(parsing_errors.len(), 2, "{:#?}", parsed.errors);
        let explains_head =
            |error: &crate::translation_phases::parsing::errors::ParserError<'_>| {
                error
                    .related
                    .iter()
                    .any(|related| related.message.contains("not a function"))
            };
        // `int q` lacks its own `;`: that diagnostic keeps its insertion point
        // after `q`, and the head explanation moves to the next diagnostic.
        assert!(parsing_errors[0].insertion_point.is_some());
        assert!(
            !explains_head(parsing_errors[0]),
            "{:#?}",
            parsing_errors[0]
        );
        assert!(explains_head(parsing_errors[1]), "{:#?}", parsing_errors[1]);
    });
}

/// An identifier-list head that already needed recovery and is followed by
/// a declaration-list but no body was most likely never a function (as with
/// an unknown `__declspec(dllimport)` prefix): the definition ends without a
/// body instead of swallowing the rest of the file as one.
#[test]
fn recovered_old_style_head_without_body_does_not_swallow_the_file() {
    let source = "__declspec(dllimport) int foo;\n__declspec(dllexport) int g(void) { return foo; \
                  }\nint h(void) { return 1; }\n";
    with_parse(source, |parsed| {
        assert!(
            !parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None)
            )),
            "{:#?}",
            parsed.errors
        );
        assert!(
            matches!(
                parsed.items.last(),
                Some(ExternalDeclaration::FunctionDefinition(_))
            ),
            "{:#?}",
            parsed.items
        );
    });

    // A recovered head followed directly by its body keeps that body.
    with_parse("f(a) int a; { return a; }\n", |parsed| {
        let definition = function_definition(parsed, 0);
        assert_eq!(block_items(definition.body).len(), 1);
    });
}

#[test]
fn recovered_old_style_head_does_not_swallow_unrelated_declarations() {
    // Round-2 review: after an error in an identifier-list head, every
    // following declaration up to the next `{` was read as its K&R
    // declaration list, absorbing a later function definition.
    with_parse(
        "int f(a, 1)\nint x;\nint y;\nint g(void) { return x; }\n",
        |parsed| {
            assert!(
                parsed
                    .items
                    .iter()
                    .any(|item| matches!(item, ExternalDeclaration::FunctionDefinition(_))),
                "{:#?}",
                parsed.items
            );
            assert!(parsed.items.len() >= 3, "{:#?}", parsed.items);
        },
    );

    // A recovered head whose following declarations name its parameters is
    // still a definition.
    with_parse("int f(a b) int a; int b; { return a; }\n", |parsed| {
        assert_eq!(parsed.items.len(), 1, "{:#?}", parsed.items);
    });
}

#[test]
fn complex_integer_type_is_diagnosed_once_in_any_order() {
    // Round-2 review: `_Complex int` drew the conflict and the
    // incomplete-complex error; `int _Complex` drew one.
    for source in [
        "_Complex char g;\n",
        "_Complex int i;\n",
        "int _Complex j;\n",
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                parser_errors(parsed).count(),
                1,
                "{source}: {:#?}",
                parsed.errors
            );
        });
    }
    with_parse("_Complex x;\n", |parsed| {
        assert!(
            parser_errors(parsed)
                .any(|error| matches!(error, ParserErrorType::IncompleteComplexTypeSpecifier)),
            "{:#?}",
            parsed.errors
        );
    });
}
