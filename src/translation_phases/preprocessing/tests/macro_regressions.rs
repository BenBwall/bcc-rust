//! C99 macro replacement regressions.

use std::fmt::Write;

use super::{
    assert_expansion,
    expansion_of,
    with_tokens_of,
};
use crate::translation_phases::{
    TranslationError,
    preprocessing::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};

#[test]
fn stringification_discards_trailing_argument_whitespace() {
    for (argument, expected) in [
        ("a   b   ", "\"a b\""),
        ("\t a\t b\t", "\"a b\""),
        ("a\n b\n", "\"a b\""),
        ("a /* trailing */ ", "\"a\""),
        (" /* empty */ \n\t", "\"\""),
    ] {
        assert_expansion(&format!("#define S(x) #x\nS({argument})\n"), expected);
    }
    assert_expansion("#define S(x,y) #x ; #y\nS(a , b )\n", "\"a\" ; \"b\"");
    assert_expansion("#define S(x) #x\nS(\"a \" )\n", &format!("{:?}", "\"a \""));
}

#[test]
fn empty_zero_parameter_invocations_have_zero_arguments() {
    assert_expansion(
        "#define F() marker\nF() F( ) F(\t) F(/*empty*/) F(\n)\n",
        "marker marker marker marker marker",
    );
    // An invocation of a one-parameter macro still supplies one empty argument.
    assert_expansion("#define S(x) #x\nS() ; S( )\n", "\"\" ; \"\"");
}

#[test]
fn arity_diagnostics_count_every_supplied_argument() {
    for (source, expected, found) in [
        ("#define F() marker\nF(a,b,c)\n", 0, 3),
        ("#define F() marker\nF(,)\n", 0, 2),
        ("#define F(x) marker\nF(a,b,c)\n", 1, 3),
        ("#define F(x,y) marker\nF(a,b,c,d,e)\n", 2, 5),
    ] {
        expansion_of(source, |actual, errors| {
            assert_eq!(actual, "marker", "{source}");
            assert!(
                matches!(
                    errors,
                    [TranslationError::Preprocessing(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                expected: actual_expected,
                                found: actual_found,
                            },
                        ..
                    })] if *actual_expected == expected && *actual_found == found
                ),
                "{source}: {errors:#?}",
            );
        });
    }
}

#[test]
fn source_file_macro_invocations_cross_newlines() {
    assert_expansion(
        "#define F(x) [x]\nF\n(value) F /*comment*/\n\n (other)\n",
        "[ value ] [ other ]",
    );
    assert_expansion("#define F(x) x\nF\n; after\n", "F ; after");
}

#[test]
fn newline_lookahead_never_reads_past_a_replacement_list() {
    // The source following G's definition is not part of its replacement
    // list, even if it starts with a parenthesis and was already emitted.
    assert_expansion("#define F(x) x\n#define G F\nG\n; after\n", "F ; after");
    assert_expansion(
        "#define F(x) x\n#define G F\n(earlier)\nG\n; after\n",
        "( earlier ) F ; after",
    );
}

#[test]
fn punctuation_pastes_accept_arrows_and_digraphs() {
    assert_expansion(
        "#define CAT(a,b) a##b\nCAT(-,>) CAT(<,:) CAT(:,>) CAT(<,%) CAT(%,>)\n",
        "-> <: :> <% %>",
    );
    assert_expansion(
        "#define CAT(a,b) a##b\n#define S(x) #x\n#define XS(x) S(x)\nXS(CAT(%,:))\n",
        "\"%:\"",
    );
}

#[test]
fn pasted_hash_hash_is_a_token_in_later_stringification() {
    for (hashes, expected) in [("# ## #", "\"x ## y\""), ("%: ## %:", "\"x %:%: y\"")] {
        // C99 6.10.3.3 example: the generated token is not a ## operator.
        assert_expansion(
            &format!(
                "#define hash_hash {hashes}\n#define mkstr(a) #a\n#define between(a) \
                 mkstr(a)\n#define join(c,d) between(c hash_hash d)\njoin(x,y)\n",
            ),
            expected,
        );
    }
}

#[test]
fn mixed_hash_spellings_do_not_form_a_single_pasted_token() {
    expansion_of("#define CAT(a,b) a##b\nCAT(#,%:)\n", |_, errors| {
        assert!(
            errors.iter().any(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::TokenMergingError(lhs, rhs),
                    ..
                }) if *lhs == "#" && *rhs == "%:"
            )),
            "{errors:#?}"
        );
    });
}

#[test]
fn compound_shift_pastes_accept_either_partition() {
    assert_expansion(
        "#define CAT(a,b) a ## b\n#define S(x) #x\n#define XS(x) S(x)\nXS(CAT(<,<=)) ; \
         XS(CAT(>,>=)) ; XS(CAT(<<,=)) ; XS(CAT(>>,=))\n",
        "\"<<=\" ; \">>=\" ; \"<<=\" ; \">>=\"",
    );
}

#[test]
fn chained_pp_number_pastes_preserve_the_terminal_period() {
    // A second paste strips DOT's internal NUL, not its final source period.
    assert_expansion(
        "#define CAT(a,b) a ## b\n#define XCAT(a,b) CAT(a,b)\n#define S(x) #x\n#define XS(x) \
         S(x)\n#define DOT 1 ## .\nXS(XCAT(DOT,2)) ; XS(XCAT(DOT,e2))\n",
        "\"1.2\" ; \"1.e2\"",
    );
}

#[test]
fn pasted_float_ending_in_a_period_converts_without_panicking() {
    assert_expansion("#define CAT(a,b) a ## b\nCAT(1,.)\n", "1.");
}

#[test]
fn generated_wide_literal_pastes_accept_values_starting_with_l() {
    assert_expansion(
        "#define W(x) L ## x\n#define XW(x) W(x)\n#define S(x) #x\n#define XS(x) \
         S(x)\nXS(XW(S(Long))) ; XS(XW(S(foo)))\n",
        "\"L\\\"Long\\\"\" ; \"L\\\"foo\\\"\"",
    );
}

// C99 §6.10.3.3p3 leaves the order of `##` evaluation unspecified, but each
// operator pastes the operands written beside it: a parameter stands for its
// argument as written and an empty argument for a placemarker. Expected
// spellings come from `clang -E -P`.

#[test]
fn paste_chains_replace_every_parameter_operand() {
    for (source, expected) in [
        ("#define P(a,b) a##c##b\nP(x,y)\n", "xcy"),
        ("#define P(a,b) a##_##b\nP(x,y)\n", "x_y"),
        ("#define P(a,b) a##b##c\nP(x,y)\n", "xyc"),
        ("#define P(N,I,J) N##I##J\nP(d,0,0)\n", "d00"),
        (
            "#define F(type,n) type type##_##n\nF(float,min1)\n",
            "float float_min1",
        ),
        // Three and four operators.
        ("#define C(a,b,c,d) a##b##c##d\nC(w,x,y,z)\n", "wxyz"),
        ("#define C(a,b) a##_##b##_##a\nC(p,q)\n", "p_q_p"),
        (
            "#define C(a,b,c,d,e) a ## b ## c ## d ## e\nC(v,w,x,y,z) C(1,2,3,4,5)\n",
            "vwxyz 12345",
        ),
        // Only the last token of a left argument and the first of a right one
        // take part.
        ("#define J(a,b,c) a##b##c\nJ(p q, r s, t u)\n", "p qr st u"),
        // Results that are not identifiers.
        (
            "#define J(a,b,c) a##b##c\n#define S(x) #x\n#define XS(x) S(x)\nXS(J(1,.,5)) ; \
             XS(J(<,<,=)) ; XS(J(1,e,+))\n",
            "\"1.5\" ; \"<<=\" ; \"1e+\"",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn paste_chains_treat_empty_arguments_as_placemarkers() {
    // C99 §6.10.3.3p2-3: an empty argument operand is a placemarker, which
    // pastes to the other operand, and two placemarkers paste to one.
    for (invocations, expected) in [
        ("J(,,) end", "end"),
        ("J(x,,) J(,y,) J(,,z) J(x,,z)", "x y z xz"),
        ("J(, y z ,) J( ,  , ) end", "y z end"),
        ("U(,) U(x,) U(,y)", "_ x_ _y"),
        ("M(,) M(x,) M(,y)", "c xc cy"),
        ("T(,) end", "__ end"),
        ("Q(,b,,) end", "b end"),
    ] {
        assert_expansion(
            &format!(
                "#define J(a,b,c) a##b##c\n#define U(a,b) a##_##b\n#define M(a,b) \
                 a##c##b\n#define T(a,b) a##_##b##_##a\n#define Q(a,b,c,d) \
                 a##b##c##d\n{invocations}\n"
            ),
            expected,
        );
    }
}

#[test]
fn paste_chains_take_stringified_operands() {
    // C99 §6.10.3.2p2: here `#` applies before an adjacent `##`.
    assert_expansion(
        "#define W(p,a,s) p ## #a ## s\n#define H(a,b) #a ## b ## b\nW(, x y, ) ; W( ,, ) ; H(x \
         y, ) ; H(, )\n",
        "\"x y\" ; \"\" ; \"x y\" ; \"\"",
    );
    assert_expansion("#define W(p,a,s) p ## #a ## s\nW(L, x y, )\n", "L\"x y\"");
}

#[test]
fn paste_chains_take_variadic_arguments() {
    for (source, expected) in [
        (
            "#define V(a, ...) a##_##__VA_ARGS__\nV(p, q) V(p) V(p,) V(p, q r)\n",
            "p_q p_ p_ p_q r",
        ),
        (
            "#define V(...) pre##__VA_ARGS__##post\nV(x) V() V(a,b)\n",
            "prexpost prepost prea , bpost",
        ),
        (
            "#define V(a, ...) a##__VA_ARGS__##a\nV(m,n) V(m)\n",
            "mnm mm",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn paste_chain_operands_are_not_macro_replaced() {
    // C99 §6.10.3.1p1: a `##` operand is its argument as written. The
    // result is rescanned with the rest of the replacement list
    // (§6.10.3.4p1).
    for (invocations, expected) in [
        ("P(x,y) J(x,y,_)", "done xy_"),
        ("J(_,FOO,_) J(F,O,O)", "_FOO_ bar"),
        ("U(FOO,x)", "FOO_x"),
    ] {
        assert_expansion(
            &format!(
                "#define x 1\n#define y 2\n#define FOO bar\n#define xcy done\n#define P(a,b) \
                 a##c##b\n#define J(a,b,c) a##b##c\n#define U(a,b) a##_##b\n{invocations}\n"
            ),
            expected,
        );
    }
    // A chain written in an argument of a nested invocation pastes the
    // enclosing macro's arguments.
    for (source, expected) in [
        (
            "#define CAT(a,b) a##b\n#define Q(a) CAT(a##b##c, a)\nQ(k)\n",
            "kbck",
        ),
        (
            "#define J(a,b,c) a##b##c\n#define R(a,b) J(a,b,z)\nR(m n, o)\n",
            "m noz",
        ),
        // The result is a `#` operand as written, but rescanned elsewhere.
        (
            "#define S(x) #x\n#define I(x) x\n#define kbc oops\n#define Q(a) S(a##b##c) \
             I(a##b##c)\nQ(k)\n",
            "\"kbc\" oops",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn paste_chains_report_each_invalid_paste() {
    // C99 §6.10.3.3p3: each paste that forms no valid preprocessing token is
    // diagnosed, left to right.
    expansion_of("#define J(a,b,c) a##b##c\nJ(.,.,.)\n", |_, errors| {
        let pastes: Vec<_> = errors
            .iter()
            .map(|error| match error {
                | TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::TokenMergingError(lhs, rhs),
                    ..
                }) => (*lhs, *rhs),
                | other => panic!("unexpected diagnostic: {other:#?}"),
            })
            .collect();
        assert_eq!(pastes, [(".", "."), (".", ".")]);
    });
    expansion_of("#define J(a,b,c) a##b##c\nJ(x,+,y)\n", |_, errors| {
        assert!(
            matches!(
                errors,
                [TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::TokenMergingError("x", "+"),
                    ..
                })]
            ),
            "{errors:#?}"
        );
    });
}

#[test]
fn paste_chain_results_locate_every_operand() {
    // The result spans its operands in order: `x` and `y` in the invocation,
    // `c` in the definition.
    with_tokens_of(
        "#define P(a,b) a##c##b\nP(x,y)\n",
        "<test>",
        |tokens, context| {
            assert!(context.take_pending_errors().is_empty());
            let [token] = tokens else {
                panic!("expected one token: {tokens:#?}");
            };
            assert_eq!(context.string_cache.at(token.contents), "xcy");
            let locations: Vec<_> = context
                .get_source_vectors(token.source_vectors)
                .iter()
                .map(|vector| (vector.line, vector.column, vector.length))
                .collect();
            assert_eq!(locations, [(2, 3, 1), (1, 19, 1), (2, 5, 1)]);
        },
    );
    // The tokens after a chain's first output token keep their locations
    // when provenance is compacted between output tokens.
    with_tokens_of(
        "#define J(a,b,c) a##b##c\nJ(m n, o, p q)\n",
        "<test>",
        |tokens, context| {
            assert!(context.take_pending_errors().is_empty());
            let tokens: Vec<_> = tokens
                .iter()
                .map(|token| {
                    let locations: Vec<_> = context
                        .get_source_vectors(token.source_vectors)
                        .iter()
                        .map(|vector| (vector.line, vector.column))
                        .collect();
                    (context.string_cache.at(token.contents), locations)
                })
                .collect();
            assert_eq!(
                tokens,
                [
                    ("m", vec![(2, 3)]),
                    ("nop", vec![(2, 5), (2, 8), (2, 11)]),
                    ("q", vec![(2, 13)]),
                ]
            );
        },
    );
}

#[test]
fn replacement_aliases_call_functions_from_parent_input_once() {
    let source = "#define g(x) [x]\n#define G g\n#define H G\nG(0) between G\n(1) H(2) after\n";
    assert_expansion(source, "[ 0 ] between [ 1 ] [ 2 ] after");
}

#[test]
fn alias_call_does_not_read_following_macro_definition_lines() {
    let source = "#define g(x) [x]\n#define G g\n(earlier)\nG(0) G; after\n";
    assert_expansion(source, "( earlier ) [ 0 ] g ; after");
}

#[test]
fn alias_call_arguments_use_the_cursor_owners_parameter_environment() {
    let source = "#define g(x) [x]\n#define G g\n#define C(v) G(v)\nC(foo) C(bar) after\n";
    assert_expansion(source, "[ foo ] [ bar ] after");
}

#[test]
fn alias_call_remains_inside_its_current_argument() {
    let source = "#define g(x) [x]\n#define G g\n#define P(v) v\nP(G(3)) P(G((a,b))) after\n";
    assert_expansion(source, "[ 3 ] [ ( a , b ) ] after");
}

#[test]
fn alias_call_keeps_raw_hash_and_paste_operands() {
    let source = "#define g(x) [x]\n#define G g\n#define S(v) #v\n#define XS(v) S(v)\n#define \
                  CAT(a,b) a##b\n#define ALIAS CAT\n#define C(v) ALIAS(v,end)\nXS(G)(0) S(G(4)) \
                  C(front) after\n";
    assert_expansion(source, "\"g\" ( 0 ) \"G(4)\" frontend after");
}

#[test]
fn replacement_aliases_keep_noncall_parent_input_untouched() {
    for suffix in ["; after", " + after", "\n; after"] {
        let source = format!("#define g(x) [x]\n#define G g\nG{suffix}\n");
        let expected = if suffix.contains('+') {
            "g + after"
        } else {
            "g ; after"
        };
        assert_expansion(&source, expected);
    }
}

#[test]
fn repeated_alias_calls_preserve_locations_through_compaction() {
    let mut source = String::from("#define g(x) [x]\n#define G g\n");
    let mut expected = String::new();
    for value in 0..2000 {
        writeln!(source, "G({value}) marker").unwrap();
        if value != 0 {
            expected.push(' ');
        }
        write!(expected, "[ {value} ] marker").unwrap();
    }
    // The differential helper owns locations at each iterator boundary, so
    // this compares diagnostics/provenance in addition to the token sequence.
    assert_expansion(&source, &expected);
}

#[test]
fn unterminated_alias_call_reports_a_clean_diagnostic() {
    let source = "#define g(x) [x]\n#define G g\nG(0";
    expansion_of(source, |_, errors| {
        assert!(
            errors.iter().any(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing function-like macro invocation"
                    ),
                    ..
                })
            )),
            "{errors:#?}"
        );
    });
}

#[test]
fn macro_invocations_can_span_replacement_and_argument_frames() {
    for (source, expected) in [
        (
            "#define g(x) [x]\n#define t(a) a\nt(g)(0) after\n",
            "[ 0 ] after",
        ),
        (
            "#define g(x) [x]\n#define Open g(\nOpen 0) after\n",
            "[ 0 ] after",
        ),
        (
            "#define g(x,y) [x][y]\n#define Open g(1,\nOpen (2,3)) after\n",
            "[ 1 ] [ ( 2 , 3 ) ] after",
        ),
        (
            "#define S(x) #x\n#define Open S(\nOpen a b) after\n",
            "\"a b\" after",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn stringified_other_tokens_are_interpreted_as_c_literal_spelling() {
    for (argument, value) in [
        (r"\n", "\n"),
        (r"\x41", "A"),
        (r": @\n", ": @\n"),
        (r#""\n""#, r#""\n""#),
        (r"'\\'", r"'\\'"),
    ] {
        let source = format!("#define S(x) #x\nS({argument})\n");
        assert_expansion(&source, &format!("{value:?}"));
    }
}

#[test]
fn stringification_supplies_line_and_pragma_operands() {
    assert_expansion(
        "#define S(x) #x\n#line 20 S(logical.c)\n__LINE__; __FILE__\n",
        "20 ; \"logical.c\"",
    );
    assert_expansion(
        "#define P(x) _Pragma(#x)\nP(STDC FP_CONTRACT ON)\nafter\n",
        "after",
    );
}

#[test]
fn builtin_location_is_the_macro_invocation_not_its_definition() {
    assert_expansion(
        "#define LINE __LINE__\n#define OUTER LINE\nOUTER;\n#line 40 \"logical.c\"\nOUTER;\n",
        "3 ; 40 ;",
    );
    assert_expansion(
        "#line 2 \"definition.h\"\n#define FILE __FILE__\n#line 10 \"use.c\"\nFILE\n",
        "\"use.c\"",
    );
}
// C99 6.10.3.5 EXAMPLE 3, N1256 p167, unchanged source and expected output.
#[test]
fn c99_reexamination_example_three_matches_normative_output() {
    let source = r"#define x 3
#define f(a) f(x * (a))
#undef x
#define x 2
#define g f
#define z z[0]
#define h g(~
#define m(a) a(w)
#define w 0,1
#define t(a) a
#define p() int
#define q(x) x
#define r(x,y) x ## y
#define str(x) # x
f(y+1) + f(f(z)) % t(t(g)(0) + t)(1);
g(x+(3,4)-w) | h 5) & m
(f)^m(m);
p() i[q()] = { q(1), r(2,3), r(4,), r(,5), r(,) };
char c[2][6] = { str(hello), str() };
";
    let expected = concat!(
        "f ( 2 * ( y + 1 ) ) + f ( 2 * ( f ( 2 * ( z [ 0 ] ) ) ) ) % ",
        "f ( 2 * ( 0 ) ) + t ( 1 ) ; ",
        "f ( 2 * ( 2 + ( 3 , 4 ) - 0 , 1 ) ) | f ( 2 * ( ~ 5 ) ) & ",
        "f ( 2 * ( 0 , 1 ) ) ^ m ( 0 , 1 ) ; ",
        "int i [ ] = { 1 , 23 , 4 , 5 , } ; ",
        "char c [ 2 ] [ 6 ] = { \"hello\" , \"\" } ;",
    );
    assert_expansion(source, expected);
}
#[test]
fn nested_calls_receive_stringified_parent_arguments() {
    for source in [
        "#define G(x) x\n#define H(x) G(#x)\nH(a) after\n",
        "#define G(x) x\n#define Alias G\n#define H(x) Alias(#x)\nH(a) after\n",
    ] {
        assert_expansion(source, "\"a\" after");
    }
}

#[test]
fn failed_cross_frame_lookahead_prescans_each_argument_once() {
    let source = "#define F(x) x\n#define BAD(a,b) a\n#define H(x) F x\nH(BAD(1)) after\n";
    expansion_of(source, |tokens, errors| {
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(tokens, "F 1 after");
    });
}

#[track_caller]
fn assert_only_error(
    source: &str,
    expected: &str,
    is_expected: impl Fn(&PreprocessorErrorType<'_>) -> bool,
) {
    expansion_of(source, |actual, errors| {
        assert_eq!(actual, expected, "{source:?}");
        assert!(
            matches!(
                errors,
                [TranslationError::Preprocessing(PreprocessorError { error_type, .. })]
                    if is_expected(error_type)
            ),
            "{source:?}: {errors:#?}"
        );
    });
}

#[test]
fn object_like_macro_name_needs_following_whitespace() {
    // C99 §6.10.3p3. The definition is kept, as in GCC and Clang.
    for (source, expected, name) in [
        ("#define A+1\nA\n", "+ 1", "A"),
        ("#define S\"s\"\nS\n", "\"s\"", "S"),
    ] {
        assert_only_error(source, expected, |error| {
            matches!(
                error,
                PreprocessorErrorType::MissingWhitespaceAfterMacroName(found) if *found == name
            )
        });
    }
    // A comment is whitespace, and an empty list needs none.
    assert_expansion("#define B/**/+1\nB\n", "+ 1");
    assert_expansion("#define E\n#define T\t1\nE T\n", "1");
}

#[test]
fn va_args_outside_a_variadic_replacement_list_is_diagnosed() {
    // C99 §6.10.3p5. The identifier stays ordinary and the macro defined.
    for (source, expected) in [
        ("#define F(x) x __VA_ARGS__\nF(1)\n", "1 __VA_ARGS__"),
        ("#define G __VA_ARGS__\nG\n", "__VA_ARGS__"),
        ("#define __VA_ARGS__ 1\n__VA_ARGS__\n", "1"),
        ("#define H(__VA_ARGS__) 2\nH(0)\n", "2"),
    ] {
        assert_only_error(source, expected, |error| {
            matches!(error, PreprocessorErrorType::VaArgsOutsideVariadicMacro(_))
        });
    }
    assert_expansion("#define V(x, ...) x __VA_ARGS__\nV(1, 2, 3)\n", "1 2 , 3");
}

#[test]
fn duplicate_macro_parameter_discards_the_definition() {
    // C99 §6.10.3p6.
    assert_only_error(
        "#define PAIR(a, b, a) a\nPAIR(1, 2, 3)\n",
        "PAIR ( 1 , 2 , 3 )",
        |error| matches!(error, PreprocessorErrorType::DuplicateMacroParameter("a")),
    );
}

#[test]
fn hash_hash_at_either_end_of_a_new_definition_discards_it() {
    // C99 §6.10.3.3p1, for first definitions as well as redefinitions. A
    // discarded redefinition leaves the old definition in effect.
    let left = |error: &PreprocessorErrorType<'_>| {
        matches!(
            error,
            PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator
        )
    };
    let right = |error: &PreprocessorErrorType<'_>| {
        matches!(
            error,
            PreprocessorErrorType::MissingRightHandSideOfHashHashOperator
        )
    };
    assert_only_error("#define P ## x\nP\n", "P", left);
    assert_only_error("#define P  ##  x\nP\n", "P", left);
    assert_only_error("#define P ##\nP\n", "P", left);
    assert_only_error("#define P(a) ## a\nP(1)\n", "P ( 1 )", left);
    assert_only_error("#define P x\n#define P ## x\nP\n", "x", left);
    assert_only_error("#define P x ##\nP\n", "P", right);
    assert_only_error("#define P(a) a ##   \nP(1)\n", "P ( 1 )", right);
    assert_expansion("#define P(a, b) a ## b\nP(x, y)\n", "xy");
}

#[test]
fn definition_on_an_unterminated_last_line_reports_the_missing_newline_once() {
    // Each `#define` reads its line once, so the end-of-file diagnostic is
    // not repeated by its checks or by a redefinition's comparison.
    for source in [
        "#define A 1",
        "#define A 1\n#define A 1",
        "#define F(x) x",
        "#define F(x) x\n#define F(x) x",
        "#define A ##",
    ] {
        expansion_of(source, |_, errors| {
            let missing_newlines = errors
                .iter()
                .filter(|error| matches!(error, TranslationError::InitialProcessing(_)))
                .count();
            assert_eq!(missing_newlines, 1, "{source:?}: {errors:#?}");
        });
    }
}
