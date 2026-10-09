//! Regression cases for C99 controlling-expression evaluation.

use std::path::PathBuf;

use super::{
    Context,
    GetSourceVectors,
    Preprocessor,
    PreprocessorError,
    PreprocessorErrorType,
    SharedVec,
    TranslationError,
    preprocess,
};
use crate::translation_phases::SourceVector;

fn assert_true_expression(expression: &str) {
    let source =
        format!("#define NAME 1\n#if {expression}\nselected\n#else\nwrong\n#endif\nafter\n");
    preprocess(&source, |identifiers, errors| {
        assert_eq!(
            identifiers,
            ["selected", "after"],
            "{expression}: {errors:#?}"
        );
        assert!(errors.is_empty(), "{expression}: {errors:#?}");
    });
}

#[test]
fn unsigned_division_uses_unsigned_values() {
    assert_true_expression("18446744073709551615ULL / 2 == 9223372036854775807ULL");
    assert_true_expression("-1 / 2U == 9223372036854775807ULL");
}

#[test]
fn unsigned_remainder_uses_unsigned_values() {
    assert_true_expression("18446744073709551615ULL % 2 == 1");
    assert_true_expression("-1 % 2U == 1");
}

#[test]
fn unsigned_right_shift_is_logical() {
    assert_true_expression("18446744073709551615ULL >> 1 == 9223372036854775807ULL");
    assert_true_expression("9223372036854775808ULL >> 63 == 1");
}

#[test]
fn shift_result_signedness_comes_from_the_left_operand() {
    assert_true_expression("(-1 >> 1U) < 0");
    assert_true_expression("(1 << 0U) - 2 < 0");
}

#[test]
fn conditional_result_converts_both_branch_types() {
    assert_true_expression("(1 ? -1 : 0U) > 0");
    assert_true_expression("(0 ? 0U : -1) > 0");
    assert_true_expression("(1 ? 1 : 0U) - 2 > 0");
}

#[test]
fn single_operand_parentheses_do_not_produce_diagnostics() {
    for expression in ["(1)", "((1))", "(defined(NAME))", "((1 + 2) * 3) == 9"] {
        assert_true_expression(expression);
    }
}

#[test]
fn nested_groups_with_a_long_unary_prefix_reduce_without_recursion() {
    let expression = format!(
        "{}{}1{}",
        "+ ".repeat(5000),
        "(".repeat(5000),
        ")".repeat(5000)
    );
    assert_true_expression(&expression);
}

#[test]
fn empty_parentheses_recover_without_a_panic_or_spurious_unclosed_group() {
    for expression in ["()", "() + ()", "(())", "() + 1"] {
        let source = format!("#if {expression}\ninside\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(identifiers.last().map(String::as_str), Some("after"));
            assert!(!errors.is_empty(), "{expression} must be diagnosed");
            assert!(errors.iter().all(|error| !matches!(error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    ..
                })
            )), "{expression}: {errors:#?}");
        });
    }
}

#[test]
fn unmatched_closing_parenthesis_is_not_reported_as_an_empty_group() {
    preprocess("#if 1)\ninside\n#endif\nafter\n", |identifiers, errors| {
        assert_eq!(identifiers, ["inside", "after"]);
        assert!(
            matches!(
                errors,
                [TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(_),
                    ..
                })]
            ),
            "{errors:#?}"
        );
    });
}

#[test]
fn arithmetic_in_unevaluated_operands_does_not_produce_diagnostics() {
    for expression in [
        "!(0 && 1 / 0)",
        "1 || 1 / 0",
        "1 ? 1 : 1 / 0",
        "0 ? 1 / 0 : 1",
        "!(0 && 1 % 0)",
        "1 || (9223372036854775807LL + 1)",
        "1 || (-(-9223372036854775807LL - 1))",
        "1 || ((-9223372036854775807LL - 1) - 1)",
        "1 || (9223372036854775807LL * 2)",
        "1 || ((-9223372036854775807LL - 1) / -1)",
        "1 || ((-9223372036854775807LL - 1) % -1)",
        "1 || (1 << 64)",
        "1 || (1 >> 64)",
        "1 || ((1 / 0) + (2 / 0))",
        "1 ? (0 && 1 / 0) == 0 : 2 / 0",
        "(0 ? 1 / 0 : 1) + 0",
    ] {
        assert_true_expression(expression);
    }
}

#[test]
fn selected_arithmetic_still_reports_each_fault_once() {
    for expression in [
        "1 / 0 || 1",
        "1 && 1 / 0",
        "0 || 1 / 0",
        "1 ? 1 / 0 : 0",
        "0 ? 0 : 1 / 0",
        "-(1 / 0)",
        "(1 / 0) + 0",
        "1 / 0 ? 0 : 1",
    ] {
        let source = format!("#if {expression}\ninside\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(identifiers.last().map(String::as_str), Some("after"));
            assert!(
                matches!(
                    errors,
                    [TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideByZero,
                        ..
                    })]
                ),
                "{expression}: {errors:#?}"
            );
        });
    }
}

#[test]
fn each_evaluated_arithmetic_fault_keeps_its_diagnostic_kind() {
    for (expression, expected) in [
        (
            "-(-9223372036854775807LL - 1)",
            PreprocessorErrorType::UnaryMinusOverflow,
        ),
        (
            "9223372036854775807LL + 1",
            PreprocessorErrorType::BinaryPlusOverflow,
        ),
        (
            "(-9223372036854775807LL - 1) - 1",
            PreprocessorErrorType::BinaryMinusOverflow,
        ),
        (
            "9223372036854775807LL * 2",
            PreprocessorErrorType::MultiplyOverflow,
        ),
        ("1 / 0", PreprocessorErrorType::DivideByZero),
        (
            "(-9223372036854775807LL - 1) / -1",
            PreprocessorErrorType::DivideOverflow,
        ),
        ("1 % 0", PreprocessorErrorType::ModuloByZero),
        (
            "(-9223372036854775807LL - 1) % -1",
            PreprocessorErrorType::ModuloOverflow,
        ),
        ("1 << 64", PreprocessorErrorType::LeftShiftOverflow),
        ("1 >> 64", PreprocessorErrorType::RightShiftOverflow),
    ] {
        let source = format!("#if {expression}\ninside\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(identifiers.last().map(String::as_str), Some("after"));
            let [TranslationError::Preprocessing(PreprocessorError { error_type, .. })] = errors
            else {
                panic!("{expression}: {errors:#?}");
            };
            assert_eq!(
                std::mem::discriminant(error_type),
                std::mem::discriminant(&expected),
                "{expression}: {errors:#?}"
            );
        });
    }
}

fn divide_fault_sources(source: &str) -> Vec<SourceVector> {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<expression-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    preprocessor.for_each_iterator_item(&mut context, |_, _| {});
    context
        .take_pending_errors()
        .into_iter()
        .map(|error| {
            assert!(
                matches!(
                    &error,
                    TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideByZero,
                        ..
                    })
                ),
                "{error:#?}"
            );
            let sources = error.source_vectors(&mut context);
            let vectors = context.get_source_vectors(sources);
            assert_eq!(vectors.len(), 1, "{vectors:#?}");
            vectors[0].clone()
        })
        .collect()
}

#[test]
fn arithmetic_diagnostics_point_at_their_operators() {
    let source = "#if 1 / 0 + 2 / 0\ninside\n#endif\nafter\n";
    let actual: Vec<_> = divide_fault_sources(source)
        .into_iter()
        .map(|v| (v.index, v.line, v.column, v.length))
        .collect();
    assert_eq!(actual, [(6, 1, 7, 1), (14, 1, 15, 1)]);

    let source = "#if 1 \\\n / 0\ninside\n#endif\nafter\n";
    let actual: Vec<_> = divide_fault_sources(source)
        .into_iter()
        .map(|v| (v.index, v.line, v.column, v.length))
        .collect();
    assert_eq!(actual, [(9, 2, 2, 1)]);
}

#[test]
fn many_dead_arithmetic_faults_reduce_without_recursion() {
    let dead = vec!["(1 / 0)"; 1000].join(" + ");
    assert_true_expression(&format!("1 || ({dead})"));
}
#[test]
fn unsupported_function_calls_stop_recovery_at_the_directive_newline() {
    for expression in ["1(", "1((", "1(argument", "1((argument"] {
        let source = format!("#if {expression}\nselected\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(
                identifiers,
                ["selected", "after"],
                "{expression}: {errors:#?}"
            );
            assert!(matches!(errors, [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                ..
            })]), "{expression}: {errors:#?}");
        });
    }
}

#[test]
fn malformed_ternary_groups_keep_outer_operands_and_operator_locations() {
    for (expression, message, column) in [
        (
            "1+(2?:3)",
            "ternary operator `?:` is missing its middle operand",
            10,
        ),
        (
            "1+(2?3)",
            "expected `:` after `?` in preprocessor expression",
            9,
        ),
    ] {
        let source = format!("#if {expression}\nselected\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(identifiers, ["selected", "after"], "{errors:#?}");
            assert_eq!(errors.len(), 1, "{expression}: {errors:#?}");
        });
        // Pin the exact operator location, not the cursor after the directive.
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let preprocess_arena = crate::util::bump::Bump::new();
        let mut pp = Preprocessor::new(
            &preprocess_arena,
            &mut context,
            PathBuf::from("<test>").into_boxed_path(),
            &source,
            SharedVec::default(),
            SharedVec::default(),
        );
        pp.for_each_iterator_item(&mut context, |_, _| {});
        let error = context.take_pending_errors().remove(0);
        let sources = error.source_vectors(&mut context);
        assert_eq!(context.get_source_vectors(sources)[0].column, column);
        if expression == "1+(2?3)" {
            assert_eq!(error.to_string(), message);
        }
    }
}

#[test]
fn malformed_nested_expression_groups_preserve_following_source() {
    for expression in [
        "1+(2?3:)",
        "1+(()?:3)",
        "1+((2?:3))",
        "1+(2?:3)*4",
        "1+(-)",
        "1+(",
        "1+(2+",
        "1?(2:3):4",
    ] {
        let source = format!("#if {expression}\nselected\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(
                identifiers.last().map(String::as_str),
                Some("after"),
                "{expression}: {errors:#?}"
            );
            assert!(!errors.is_empty());
        });
    }
}

/// C99 §6.10.1p4 and footnote 145: `#if` types constants as though `int`
/// were `intmax_t`, so a hexadecimal or octal constant is unsigned only above
/// `INTMAX_MAX` or with `u`, though `0xFFFFFFFF` is `unsigned int` in phase 7.
/// Constants that need `long` in phase 7 draw no widening warning here.
#[test]
fn integer_constants_are_typed_as_intmax_in_controlling_expressions() {
    for expression in [
        "0xFFFFFFFF > -1",
        "037777777777 > -1",
        "0x80000000 > -1",
        "0x7FFFFFFFFFFFFFFF > -1",
        "0x7FFFFFFFFFFFFFFFL > -1",
        "0x7FFFFFFFFFFFFFFFLL > -1",
        "4294967295 > -1",
        "2147483648 > -1",
        "0x100000000 > -1",
        "0xFFFFFFFFu < -1",
        "0x100000000u < -1",
        "0x8000000000000000 < -1",
        "0xFFFFFFFFFFFFFFFEL < -1",
    ] {
        assert_true_expression(expression);
    }
}

/// A decimal constant above `INTMAX_MAX` has no type (C99 §6.4.4.1p6); in
/// `#if` it is still diagnosed and evaluated as `uintmax_t`.
#[test]
fn decimal_constants_above_intmax_warn_and_are_unsigned_in_controlling_expressions() {
    preprocess(
        "#if 18446744073709551615 < -1\nwrong\n#else\nselected\n#endif\nafter\n",
        |identifiers, errors| {
            assert_eq!(identifiers, ["selected", "after"], "{errors:#?}");
            assert!(
                matches!(
                    errors,
                    [TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. },
                        ..
                    })]
                ),
                "{errors:#?}"
            );
        },
    );
}

/// Each diagnostic of `#if {expression}` as its kind and `line:column`.
fn expression_diagnostics(expression: &str) -> Vec<String> {
    let source = format!("#if {expression}\n#endif\n");
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut pp = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        &source,
        SharedVec::default(),
        SharedVec::default(),
    );
    pp.for_each_iterator_item(&mut context, |_, _| {});
    context
        .take_pending_errors()
        .into_iter()
        .map(|error| {
            let TranslationError::Preprocessing(PreprocessorError { error_type, .. }) = &error
            else {
                panic!("{expression}: {error:#?}");
            };
            let kind = format!("{error_type:?}");
            let sources = error.source_vectors(&mut context);
            let vector = &context.get_source_vectors(sources)[0];
            format!("{kind}@{}:{}", vector.line, vector.column)
        })
        .collect()
}

/// A `:` where an operand belongs reports the operand that the operator
/// before it lacks, and only a `?` there lacks the middle operand (C99
/// §6.5.15p1). A placeholder takes the missing operand's place, so
/// reduction cannot take an operand from outside that operator.
#[test]
fn colon_in_operand_position_reports_the_operator_before_it() {
    for (expression, expected) in [
        ("1 ? : 2", &["TernaryOperatorWithoutMhs@1:9"][..]),
        (
            "1 + : 2",
            &[
                "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(BinaryPlus)@1:9",
                "ColonWithoutMatchingQuestionMark@1:9",
            ],
        ),
        (
            "! : 1",
            &[
                "LogicalNotWithoutOperand@1:7",
                "ColonWithoutMatchingQuestionMark@1:7",
            ],
        ),
        (
            "1 ? 1 : :",
            &[
                "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Conditional)@1:13",
                "ColonWithoutMatchingQuestionMark@1:13",
            ],
        ),
        (
            "1 ? 2 : 3 * :",
            &[
                "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Multiply)@1:17",
                "ColonWithoutMatchingQuestionMark@1:17",
            ],
        ),
        ("1 ? - : 2", &["UnaryMinusWithoutOperand@1:11"]),
        (
            "1 ? 1 / : 2",
            &["ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Divide)@1:13"],
        ),
        (
            "1 ? 1 % : 2",
            &["ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Modulo)@1:13"],
        ),
        (": 2", &["ColonWithoutMatchingQuestionMark@1:5"]),
        (
            "(1 + : 2)",
            &[
                "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(BinaryPlus)@1:10",
                "ColonWithoutMatchingQuestionMark@1:10",
            ],
        ),
        (
            "0 ? 1 / 0 : 1 + : 2",
            &[
                "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(BinaryPlus)@1:21",
                "ColonWithoutMatchingQuestionMark@1:21",
            ],
        ),
    ] {
        assert_eq!(expression_diagnostics(expression), expected, "{expression}");
    }
}

#[test]
fn missing_operands_at_group_end_do_not_create_arithmetic_faults() {
    for (expression, expected) in [
        (
            "(1 / )",
            "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Divide)@1:10",
        ),
        (
            "(1 % )",
            "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Modulo)@1:10",
        ),
        (
            "1 / -(1 / )",
            "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Divide)@1:15",
        ),
        (
            "1 / (1 ? (1 / ) : 2)",
            "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Divide)@1:19",
        ),
        (
            "1 / (1 && (1 / ))",
            "ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(Divide)@1:20",
        ),
    ] {
        assert_eq!(
            expression_diagnostics(expression),
            [expected],
            "{expression}"
        );
        let source = format!("#if {expression}\ninside\n#endif\nafter\n");
        preprocess(&source, |identifiers, errors| {
            assert_eq!(identifiers, ["after"], "{expression}: {errors:#?}");
        });
    }
}
