//! C99 macro replacement regressions.

use std::{
    fmt::Write,
    path::PathBuf,
};

use super::Preprocessor;
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        preprocessing::{
            PreprocessorError,
            PreprocessorErrorType,
            StringTokenType,
            TokenType,
        },
    },
    util::shared::SharedVec,
};

fn with_expansion<R>(source: &str, inspect: impl FnOnce(&str, &[TranslationError]) -> R) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let tok = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<macro regression>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let tokens = preprocessor.preprocess_all(&mut context, &tok);
    let spellings: Vec<_> = tokens
        .iter()
        .map(|token| match token.kind {
            | TokenType::String(StringTokenType::String(contents)) => format!(
                "{:?}",
                context
                    .literal_text(contents, false)
                    .as_deref()
                    .expect("UTF-8 test literal")
            ),
            | _ => context
                .string_cache
                .at(token.contents)
                .trim_end_matches('\0')
                .to_owned(),
        })
        .collect();
    let spelling = spellings.join(" ");
    let errors = context.take_pending_errors();
    inspect(&spelling, &errors)
}

#[track_caller]
fn assert_expansion(source: &str, expected: &str) {
    with_expansion(source, |actual, errors| {
        assert_eq!(actual, expected, "{source}");
        assert!(errors.is_empty(), "{source}: {errors:#?}");
    });
}

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
        with_expansion(source, |actual, errors| {
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
    with_expansion("#define CAT(a,b) a##b\nCAT(#,%:)\n", |_, errors| {
        assert!(
            errors.iter().any(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::TokenMergingError(lhs, rhs),
                    ..
                }) if lhs == "#" && rhs == "%:"
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
    with_expansion(source, |_, errors| {
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
    with_expansion(source, |tokens, errors| {
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(tokens, "F 1 after");
    });
}
