//! Regression tests for verified aggregate bugs found by the overnight bug
//! hunt.

use super::{
    Parsed,
    declaration,
    init_declarators,
    parser_errors,
    with_parse,
};
use crate::translation_phases::{
    parsing::{
        declaration_syntax::{
            DirectDeclarator,
            EnumSpecifier,
            InitializerType,
            StructOrUnionSpecifier,
        },
        errors::ParserErrorType,
        syntax::ExternalDeclaration,
    },
    preprocessing::{
        IntegerTokenType,
        KeywordTokenType,
        OperatorTokenType,
        TokenType,
    },
};

fn errors<'tu>(parsed: &Parsed<'_, 'tu>) -> Vec<ParserErrorType<'tu>> {
    parser_errors(parsed).cloned().collect()
}

/// Number of elements in the brace initializer of the first declarator of
/// external declaration `item`.
fn initializer_element_count(parsed: &Parsed<'_, '_>, item: usize) -> u32 {
    let declaration = declaration(parsed, item);
    let [init_declarator, ..] = init_declarators(parsed, declaration) else {
        panic!("expected an initialized declarator")
    };
    let initializer = init_declarator
        .initializer
        .expect("expected an initializer");
    let InitializerType::InitializerList(elements) = parsed.parser.syntax[initializer].kind else {
        panic!("expected an initializer list")
    };
    elements.length
}

fn designated_element_count(parsed: &Parsed<'_, '_>, item: usize) -> usize {
    let declaration = declaration(parsed, item);
    let [init_declarator, ..] = init_declarators(parsed, declaration) else {
        panic!("expected an initialized declarator")
    };
    let initializer = init_declarator
        .initializer
        .expect("expected an initializer");
    let InitializerType::InitializerList(elements) = parsed.parser.syntax[initializer].kind else {
        panic!("expected an initializer list")
    };
    parsed.parser.syntax[elements]
        .iter()
        .filter(|element| element.designation.is_some())
        .count()
}

/// aggregates-initializers:0. C99 §6.7.8p1: after the stray closer is
/// consumed as recovery, the following `,` is a valid separator.
#[test]
fn stray_closer_after_initializer_element_keeps_the_following_separator() {
    for source in [
        "int x[] = { 1 ), 2 }; int ok;",
        "int x[] = { 1 ], 2 }; int ok;",
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert_eq!(parsed.items.len(), 2, "{source}");
            assert_eq!(initializer_element_count(parsed, 0), 2, "{source}");
            assert!(matches!(
                parsed.items[1],
                ExternalDeclaration::Declaration(_)
            ));
        });
    }

    // A genuinely missing comma after the stray closer is still diagnosed.
    with_parse("int x[] = { 1 ) 2 };", |missing| {
        assert_eq!(errors(missing).len(), 2, "{:#?}", errors(missing));
        assert_eq!(initializer_element_count(missing, 0), 2);
    });
}

/// aggregates-initializers:2. A missing separator is not a missing
/// expression, and a field designator needs a member name.
#[test]
fn initializer_separator_and_field_designator_errors_name_what_is_expected() {
    with_parse("int a[2][1] = { {1} 2 };", |missing_comma| {
        assert_eq!(
            errors(missing_comma),
            [ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                Some(TokenType::Integer(IntegerTokenType::Int(2)))
            )]
        );
    });

    with_parse("int a[2][1] = { {1} ) };", |stray| {
        assert_eq!(
            errors(stray),
            [ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
            )]
        );
    });

    with_parse("struct { int a; } s = { .a. = 1 };", |field| {
        assert_eq!(
            errors(field),
            [ParserErrorType::ExpectedMemberIdentifier(Some(
                TokenType::Operator(OperatorTokenType::Equals)
            ))]
        );
    });
}

/// aggregates-initializers:3 and expressions:7. C99 §6.7.8p1: a designator
/// is `[ constant-expression ]`; a top-level `,` or `=` inside it is one
/// syntax error, after which recovery pairs the later `]` with the `[`.
#[test]
fn invalid_operator_inside_array_designator_recovers_at_the_closing_bracket() {
    for (source, found) in [
        ("int a[] = { [1, 2] = 2 };", OperatorTokenType::Comma),
        (
            "int a[] = { [1, 2] = 2, 3 }; int ok;",
            OperatorTokenType::Comma,
        ),
        ("int y[2] = { [1 = 2] = 3 };", OperatorTokenType::Equals),
        (
            "int a; int y[2] = { [a = 1] = 2 };",
            OperatorTokenType::Equals,
        ),
        ("int y[2] = { [1, 1] = 2 };", OperatorTokenType::Comma),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                errors(parsed),
                [
                    ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(Some(
                        TokenType::Operator(found)
                    ))
                ],
                "{source}"
            );
            let item = usize::from(source.starts_with("int a;"));
            assert_eq!(designated_element_count(parsed, item), 1, "{source}");
        });
    }

    with_parse("int a[] = { [1, 2] = 2, 3 }; int ok;", |trailing| {
        assert_eq!(initializer_element_count(trailing, 0), 2);
        assert_eq!(trailing.items.len(), 2);
    });

    // A designator whose `]` really is missing still resynchronizes at the
    // top-level `,` or `=`, where the missing bracket is reported.
    for (source, found) in [
        ("int a[] = { [1 = 2 };", OperatorTokenType::Equals),
        ("int a[] = { [1, 2, 3 };", OperatorTokenType::Comma),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                errors(parsed).first(),
                Some(
                    &ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(Some(
                        TokenType::Operator(found)
                    ))
                ),
                "{source}"
            );
            assert_eq!(designated_element_count(parsed, 0), 1, "{source}");
        });
    }
}

/// aggregates-initializers:5. C99 §6.7.2.2p1: a keyword in enumerator
/// position is one error; it does not end the enum body.
#[test]
fn keyword_in_enumerator_position_is_one_error() {
    for (source, items) in [
        ("enum { int };", 1),
        ("enum { int }; int ok;", 2),
        ("enum E { A, int, B }; int ok;", 2),
        ("struct S { int x; enum { int } e; }; int ok;", 2),
        ("enum E { A = 1, int = 2, B }; int ok;", 2),
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(
                errors,
                [
                    ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                        Some(TokenType::Keyword(KeywordTokenType::Int))
                    )
                ],
                "{source}"
            );
            assert_eq!(parsed.items.len(), items, "{source}");
        });
    }

    // The missing-`}` heuristic still stops before a following declaration
    // rather than consuming its specifier as a misplaced enumerator.
    with_parse("enum E { A,\nint x;", |missing_close| {
        assert_eq!(missing_close.items.len(), 2, "{:#?}", errors(missing_close));
    });
}

/// aggregates-initializers:9. The empty-list error must not repeat an
/// enumerator-list error already reported for the same body.
#[test]
fn empty_enumerator_list_error_is_not_repeated_after_recovery() {
    for source in ["enum E { , };", "enum E { 1, };"] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert!(
                matches!(
                    errors[0],
                    ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(_)
                ),
                "{source}: {errors:#?}"
            );
        });
    }

    with_parse("enum E { };", |empty| {
        assert_eq!(
            errors(empty),
            [ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace]
        );
    });
}

/// aggregates-initializers:7. One malformed member terminator is one error.
#[test]
fn malformed_member_terminator_is_reported_once() {
    for (source, found) in [
        (
            "struct S { int a 1 }; int ok;",
            TokenType::Integer(IntegerTokenType::Int(1)),
        ),
        (
            "struct S { int a = 3 }; int ok;",
            TokenType::Operator(OperatorTokenType::Equals),
        ),
        (
            "struct S { int a 1 int b; }; int ok;",
            TokenType::Integer(IntegerTokenType::Int(1)),
        ),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                errors(parsed),
                [ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(Some(found))],
                "{source}"
            );
            assert_eq!(parsed.items.len(), 2, "{source}");
        });
    }

    with_parse("struct S { int a };", |control| {
        assert_eq!(
            errors(control),
            [ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList]
        );
    });
}

/// declarations:9. C99 §6.7.6p1: `[ * ]` takes no qualifiers in any abstract
/// declarator, whatever precedes the suffix.
#[test]
fn qualified_variable_length_marker_is_rejected_in_every_abstract_suffix() {
    for source in [
        "void f(int [const *]);",
        "void h(int [3][const *]);",
        "void g(int (*)[const *]);",
        "int x = sizeof(int (*)[const *]);",
        "void k(int ([const *]));",
    ] {
        with_parse(source, |parsed| {
            assert!(
                parser_errors(parsed).any(|error| matches!(
                    error,
                    ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
                )),
                "{source}: {:#?}",
                errors(parsed)
            );
        });
    }

    for source in [
        "void k(int (*p)[const *]);",
        "void k(int p[3][const *]);",
        "void k(int (p)[const *]);",
    ] {
        with_parse(source, |parsed| {
            assert_eq!(errors(parsed), [], "{source}");
        });
    }
}

/// declarations:5. Deep function-suffix nesting stays correct: the named
/// innermost declarator still allows identifier lists at every level, and
/// an abstract one does not.
#[test]
fn deeply_nested_function_declarators_track_whether_they_are_named() {
    let depth = 2_000;
    let source = format!("int {}x{};", "(*".repeat(depth), ")()".repeat(depth));
    with_parse(&source, |parsed| {
        assert_eq!(errors(parsed), []);
        let declaration = declaration(parsed, 0);
        let [init_declarator] = init_declarators(parsed, declaration) else {
            panic!("expected one declarator")
        };
        let directs = &parsed.parser.syntax[init_declarator.declarator.kind];
        assert!(matches!(
            directs.last(),
            Some(DirectDeclarator::KAndRStyleFunction { .. })
        ));
    });

    // A named parenthesized declarator accepts an identifier list; an
    // abstract one parses its contents as a prototype.
    with_parse("int (*(*f)(a))(b);", |named| {
        assert_eq!(errors(named), []);
    });
    with_parse("void p(int (*)());", |abstract_parameter| {
        assert_eq!(errors(abstract_parameter), []);
    });
    with_parse("void p(int (*)(a));", |parameter| {
        assert!(
            !errors(parameter).is_empty(),
            "an abstract function suffix has no identifier list"
        );
    });
}

fn enumerator_counts(parsed: &Parsed<'_, '_>) -> Vec<u32> {
    parsed
        .parser
        .syntax
        .iter::<EnumSpecifier<'_>>()
        .filter_map(|specifier| specifier.enumeration_list)
        .map(|list| list.length)
        .collect()
}

fn member_counts(parsed: &Parsed<'_, '_>) -> Vec<u32> {
    parsed
        .parser
        .syntax
        .iter::<StructOrUnionSpecifier<'_>>()
        .filter_map(|specifier| specifier.struct_declaration_list)
        .map(|list| list.length)
        .collect()
}

/// recovery-fuzz:0. C99 §6.7.8p1: a declaration keyword in element position
/// is one bad element when this list's own `}` still follows; it does not
/// close the list and hand its `}` to the enclosing block.
#[test]
fn declaration_keyword_in_initializer_element_does_not_close_the_list() {
    for source in [
        "int a[] = { 1, int 0 }; int ok;",
        "void f(void) { int a[] = { 1, int 0 }; } int ok;",
        "void f(int x) { int a[] = { 1, int 0 }; x = 1; } int ok;",
        "void f(int x) { x = (int[]){ 1, typedef 0 }[0]; x = 1; } int ok;",
        "int a[] = { 1, int 0, 2 }; int ok;",
        "int a[] = { [0] = 1, int (0) }; int ok;",
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert!(
                matches!(
                    errors[0],
                    ParserErrorType::ExpectedStatementExpression(
                        _,
                        Some(TokenType::Keyword(
                            KeywordTokenType::Int | KeywordTokenType::Typedef
                        ))
                    )
                ),
                "{source}: {errors:#?}"
            );
            assert_eq!(parsed.items.len(), 2, "{source}");
            assert!(
                matches!(parsed.items[1], ExternalDeclaration::Declaration(_)),
                "{source}"
            );
        });
    }

    with_parse("int a[] = { 1, int 0, 2 };", |elements| {
        assert_eq!(initializer_element_count(elements, 0), 3);
    });

    // Each malformed element is diagnosed once, and the list still closes.
    with_parse("int a[] = { 1, int 0, int 1 }; int ok;", |repeated| {
        assert_eq!(errors(repeated).len(), 2, "{:#?}", errors(repeated));
        assert_eq!(initializer_element_count(repeated, 0), 3);
        assert_eq!(repeated.items.len(), 2);
    });

    // Without a following `}` for this list, the keyword still starts the
    // next statement or declaration after a missing `}`.
    for (source, found) in [
        (
            "void f(int x) { int a[] = { 1,\n if (x) { x = 1; } }",
            TokenType::Keyword(KeywordTokenType::If),
        ),
        (
            "void f(int x) { int a[] = { 1,\n int b; }",
            TokenType::Keyword(KeywordTokenType::Int),
        ),
    ] {
        with_parse(source, |parsed| {
            assert_eq!(
                errors(parsed).first(),
                Some(&ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(Some(found))),
                "{source}"
            );
            assert_eq!(parsed.items.len(), 1, "{source}: {:#?}", errors(parsed));
        });
    }
}

/// recovery-fuzz:6. A stray closer after a complete element is a missing
/// `,` or `}`, not a missing expression.
#[test]
fn stray_closer_after_initializer_element_names_the_separator() {
    for source in [
        "int a[] = { 1 ] };",
        "int a[] = { 1 ) };",
        "struct s { int x; } c = { .x = 1 ] };",
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert!(
                matches!(
                    errors[0],
                    ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(Some(_))
                ),
                "{source}: {errors:#?}"
            );
        });
    }
}

/// recovery-fuzz:7. C99 §6.7.2.2p1 and §6.7.2.1p1: `)` cannot end an
/// enumerator or member list. Outside an enclosing parenthesis it is one
/// stray token, and the list continues to its own `}`.
#[test]
fn stray_closing_parenthesis_does_not_end_an_enumerator_or_member_list() {
    for (source, enumerators) in [
        ("enum e { A ) }; int ok;", 1),
        ("enum e { A ), B }; int ok;", 2),
        ("enum e { A, ) B }; int ok;", 2),
        ("enum e { A = 1 + ), B }; int ok;", 2),
        ("enum e { A + ), B }; int ok;", 2),
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert_eq!(parsed.items.len(), 2, "{source}");
            assert_eq!(enumerator_counts(parsed), [enumerators], "{source}");
        });
    }

    for (source, members) in [
        ("struct s { int a ); int b; }; int ok;", 2),
        ("struct s { int a : 1 + ); int b; }; int ok;", 2),
        ("struct s { int a ), c; int b; }; int ok;", 2),
    ] {
        with_parse(source, |parsed| {
            let errors = errors(parsed);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert_eq!(parsed.items.len(), 2, "{source}");
            assert_eq!(member_counts(parsed), [members], "{source}");
        });
    }

    // A genuinely missing separator after the stray `)` is still reported.
    with_parse("enum e { A ) B }; int ok;", |missing| {
        assert_eq!(errors(missing).len(), 2, "{:#?}", errors(missing));
        assert_eq!(enumerator_counts(missing), [2]);
    });

    // Inside an enclosing parenthesis the `)` still closes it.
    for source in [
        "int f(enum E { A ) int after;",
        "int f(struct S { int x ) int after;",
    ] {
        with_parse(source, |parsed| {
            assert_eq!(parsed.items.len(), 2, "{source}: {:#?}", errors(parsed));
            assert!(
                matches!(parsed.items[1], ExternalDeclaration::Declaration(_)),
                "{source}"
            );
        });
    }
}
