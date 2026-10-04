//! Regression tests for verified parameter bugs found by the overnight bug
//! hunt.

use super::{
    Parsed,
    block_items,
    declaration,
    function_definition,
    init_declarators,
    parser_errors,
    with_parse,
};
use crate::translation_phases::parsing::{
    declaration_syntax::DirectDeclarator,
    errors::ParserErrorType,
    syntax::{
        BlockItem,
        StatementType,
    },
};

fn errors<'a, 'tu>(parsed: &'a Parsed<'_, 'tu>) -> Vec<&'a ParserErrorType<'tu>> {
    parser_errors(parsed).collect()
}

/// The function suffix of the first declarator of the declaration at `item`.
fn declared_suffix(parsed: &Parsed<'_, '_>, item: usize) -> DirectDeclarator {
    let declaration = declaration(parsed, item);
    let declarator = init_declarators(parsed, declaration)[0].declarator;
    parsed
        .parser
        .function_suffix(declarator)
        .expect("the declarator has a function suffix")
}

/// Whether the body item at `index` of the function definition at `item` is
/// an expression statement (as opposed to a declaration).
fn body_item_is_expression(parsed: &Parsed<'_, '_>, item: usize, index: usize) -> bool {
    let body = function_definition(parsed, item).body;
    match block_items(parsed, body)[index] {
        | BlockItem::Statement(statement) => matches!(
            parsed.parser.syntax[statement].kind,
            StatementType::Expression(_)
        ),
        | BlockItem::Declaration(_) => false,
    }
}

// --- statements:2 / declarations:1 -------------------------------------------

/// C99 §6.2.1p4: an enumerator declared anywhere in a function definition's
/// parameter declarations, including inside an array-bound `sizeof`, has block
/// scope that extends to the end of the body and hides a file-scope typedef.
#[test]
fn enumerators_declared_in_parameter_array_bounds_hide_typedefs_in_the_body() {
    for source in [
        "typedef int T;\nvoid k(int a[sizeof(enum {T})]) { T * 4; }\n",
        "typedef int T;\nint h(int (*p)[sizeof(enum {T})]) { T * 2; return 0; }\n",
        "typedef int a;\nvoid f(int p[sizeof(enum { a })]) { a * p; }\n",
        "typedef int T;\nvoid k(int a[sizeof(struct S { int q[sizeof(enum {T})]; })]) { T * 4; }\n",
        "typedef int T;\nvoid k(int a[sizeof(enum {X = sizeof(enum {T})})]) { T * 4; }\n",
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                errors(parsed),
                Vec::<&ParserErrorType<'_>>::new(),
                "{source}"
            );
            assert!(body_item_is_expression(parsed, 1, 0), "{source}");
        });
    }
}

/// The enumerator from the bound remains visible only for that definition:
/// the typedef is restored after the body.
#[test]
fn parameter_bound_enumerators_end_with_the_function_body() {
    with_parse(
        "typedef int T;\nvoid k(int a[sizeof(enum {T})]) { T * 4; }\nvoid m(void) { T * p; }\n",
        |parsed| {
            assert_eq!(errors(parsed), Vec::<&ParserErrorType<'_>>::new());
            assert!(body_item_is_expression(parsed, 1, 0));
            assert!(!body_item_is_expression(parsed, 2, 0));
        },
    );
}

/// Enumerators in a nested function declarator's parameters have their own
/// prototype scope (§6.2.1p4) and must not leak into the outer body.
#[test]
fn nested_prototype_enumerators_do_not_reach_the_function_body() {
    with_parse(
        "typedef int T;\nvoid k(int (*g)(int a[sizeof(enum {T})])) { T * p; }\n",
        |parsed| {
            assert_eq!(errors(parsed), Vec::<&ParserErrorType<'_>>::new());
            assert!(!body_item_is_expression(parsed, 1, 0));
        },
    );
}

/// The body sees the definition's own parameter list, not a later function
/// suffix of the same declarator.
#[test]
fn only_the_definitions_own_parameter_list_reaches_the_body() {
    with_parse(
        "typedef int T, U;\nint (*f(int a[sizeof(enum {T})]))(int b[sizeof(enum {U})]) { T * 1; U \
         * p; return 0; }\n",
        |parsed| {
            assert_eq!(errors(parsed), Vec::<&ParserErrorType<'_>>::new());
            assert!(body_item_is_expression(parsed, 1, 0));
            assert!(!body_item_is_expression(parsed, 1, 1));
        },
    );
}

// --- corpus-triage-A:1 / aggregates-initializers:8 ---------------------------

/// A prototype parameter after a K&R name gets one "mixed" diagnostic; the
/// whole offending declaration is skipped, so its own declarator name is not
/// reported again as a missing comma.
#[test]
fn mixed_prototype_parameter_after_k_and_r_name_is_diagnosed_once() {
    for (source, expected) in [
        ("int g(a, int b);\n", 1),
        ("void f(a, int ok, int z);\n", 1),
        ("void f(a, int *ok, b);\n", 1),
        ("void f(a, const char *name);\n", 1),
        ("void f(a, int ok, ...);\n", 1),
        ("void f(a, struct { int q; } s, b);\n", 1),
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), expected, "{source}: {errors:?}");
            assert!(
                matches!(
                    errors.first(),
                    Some(ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator)
                ),
                "{source}: {errors:?}"
            );
            assert!(
                !errors.iter().skip(1).any(|error| matches!(
                    error,
                    ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator
                        | ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            ..
                        )
                )),
                "{source}: {errors:?}"
            );
        });
    }
}

/// `...` in an identifier list without a prior prototype parameter is still
/// diagnosed.
#[test]
fn ellipsis_in_a_plain_identifier_list_is_still_diagnosed() {
    with_parse("void f(a, ...);\n", |parsed| {
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(..)
        )));
    });
}

/// The K&R identifiers on either side of a mixed prototype parameter are kept.
#[test]
fn k_and_r_names_survive_a_mixed_prototype_parameter() {
    with_parse("void f(a, int *ok, b);\nint after;\n", |parsed| {
        assert_eq!(parsed.items.len(), 2);
        let DirectDeclarator::KAndRStyleFunction { parameters } = declared_suffix(parsed, 0) else {
            panic!("expected an identifier list");
        };
        let names = parsed.parser.syntax[parameters]
            .iter()
            .map(|identifier| parsed.context.string_cache.at(identifier.name).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["a", "b"]);
    });
}

/// An identifier followed by `*` or a declaration-specifier keyword cannot
/// start an identifier list (C99 §6.7.5), so the list is parsed as a
/// prototype: later valid parameters and `...` produce no extra errors.
#[test]
fn unknown_type_name_before_a_declarator_selects_prototype_syntax() {
    for source in [
        "int f(size_t *p);\n",
        "int f(foo const char *p, int m, ...);\n",
        "int f(size_t n, int m);\n",
        "int f(size_t n, int m, ...);\n",
    ] {
        with_parse(source, |parsed| {
            assert!(
                matches!(
                    declared_suffix(parsed, 0),
                    DirectDeclarator::Function { .. }
                ),
                "{source}"
            );
            let errors = errors(parsed);
            assert!(
                !errors.iter().any(|error| matches!(
                    error,
                    ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator
                        | ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(..)
                        | ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(..)
                )),
                "{source}: {errors:?}"
            );
            // At most the unknown type name and its displaced declarator name.
            assert!(errors.len() <= 2, "{source}: {errors:?}");
        });
    }
}

/// A list of bare identifiers with an omitted comma stays an identifier list.
#[test]
fn adjacent_identifiers_without_declaration_syntax_stay_an_identifier_list() {
    for source in ["int f(a b, c);\n", "int f(a b, (c));\n", "int f(x y z);\n"] {
        with_parse(source, |parsed| {
            assert!(
                matches!(
                    declared_suffix(parsed, 0),
                    DirectDeclarator::KAndRStyleFunction { .. }
                ),
                "{source}"
            );
        });
    }
}

// --- declarations:10
// ----------------------------------------------------------

/// The identifier-list diagnostic must not claim the token was a typedef name.
#[test]
fn identifier_list_diagnostic_has_no_typedef_note() {
    for source in [
        "int g(a, 1);
",
        "int h(a, );
",
        "int k(a, *);
",
    ] {
        with_parse(source, |parsed| {
            let error = parser_errors(parsed)
                .find(|error| {
                    matches!(
                        error,
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            ..
                        )
                    )
                })
                .unwrap_or_else(|| panic!("{source}: no identifier-list diagnostic"));
            let notes = error.explain(None).notes;
            assert!(
                notes.iter().all(|note| !note.contains("typedef")),
                "{source}: {notes:?}"
            );
        });
    }
}

#[test]
fn identifier_list_diagnostic_at_end_of_input_has_no_typedef_note() {
    let notes = ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(None)
        .explain(None)
        .notes;
    assert!(
        notes.iter().all(|note| !note.contains("typedef")),
        "{notes:?}"
    );
}

// --- declarations:4
// -----------------------------------------------------------

/// Name lookup must not scan every open scope: `T` bound at file scope and
/// looked up from inside `n` nested blocks would otherwise cost O(n) hash
/// probes per lookup and O(n^2) for the translation unit.
#[test]
fn typedef_lookups_in_deep_block_nesting_use_a_bounded_number_of_probes() {
    let depth = 2_000;
    let source = format!(
        "typedef int T;\nvoid g(void) {{{}{}}}\n",
        "{ T x;".repeat(depth),
        "}".repeat(depth)
    );
    with_parse(&source, |parsed| {
        assert_eq!(errors(parsed), Vec::<&ParserErrorType<'_>>::new());
        let probes = parsed.parser.scopes.lookup_probes.get();
        assert!(
            probes <= 64 * depth,
            "{probes} scope probes for {depth} nested lookups"
        );
    });
}

#[test]
fn declarator_in_an_identifier_list_is_one_error() {
    // Round-2 review: `T *r` in an identifier list reported the `*` and then
    // the `r` the recovery stopped at.
    with_parse("int j(U, T *r) { return 0; }\nint after;\n", |parsed| {
        assert_eq!(parser_errors(parsed).count(), 1, "{:#?}", parsed.errors);
        assert_eq!(parsed.items.len(), 2, "{:#?}", parsed.items);
    });
}
