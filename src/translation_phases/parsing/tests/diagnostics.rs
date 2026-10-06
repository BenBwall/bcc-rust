//! Structured parser diagnostics and warning policy.

use super::{
    function_definition,
    parser_errors,
    sourced_text,
    with_parse,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        ErrorSeverity,
        GetPosition,
        TranslationError,
        parsing::{
            declaration_syntax::{
                ParameterDeclaration,
                TypeSpecifiers,
            },
            errors::{
                ExpectedSyntax,
                ParserDiagnosticCode,
                ParserError,
                ParserErrorType,
                ParserWarningGroup,
            },
            machine::ParseFrameKind,
            syntax::{
                BinaryOperator,
                Expression,
                ExpressionType,
                ExternalDeclaration,
                StatementType,
            },
        },
        preprocessing::{
            OperatorTokenType,
            TokenType,
        },
    },
};

#[test]
fn repeated_specifier_warning_group_is_named_and_suppressible() {
    let strict = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny);
    for specifier in ["const", "volatile", "restrict", "inline"] {
        let source = format!("{specifier} {specifier} int f(void) {{ return 0; }}\n");
        with_parse_configuration(&source, strict, |warned| {
            let warning = warned.errors.iter().find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if error.warning_group == Some(ParserWarningGroup::RepeatedSpecifiers) =>
                    Some(error),
                | _ => None,
            });
            let warning = warning.expect("expected the named quality warning");
            assert_eq!(warning.severity, ErrorSeverity::Warning, "{specifier}");
            assert_eq!(warning.code, ParserDiagnosticCode::Quality, "{specifier}");
            assert!(matches!(
                warned.items[0],
                ExternalDeclaration::FunctionDefinition(_)
            ));
        });

        with_parse_configuration(
            &source,
            strict.with_repeated_specifier_warnings(false),
            |suppressed| {
                assert!(
                    suppressed.errors.iter().all(|error| !matches!(
                        error,
                        TranslationError::Parsing(ParserError {
                            warning_group: Some(ParserWarningGroup::RepeatedSpecifiers),
                            ..
                        })
                    )),
                    "{specifier}"
                );
                assert!(matches!(
                    suppressed.items[0],
                    ExternalDeclaration::FunctionDefinition(_)
                ));
            },
        );
    }
}

#[test]
fn strict_c99_compatibility_corpus_matches_reviewed_parser_boundaries() {
    let strict = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny);
    let fixtures = [
        (
            "repeated-specifiers.c",
            include_str!("../../../../tests/fixtures/parser/compatibility/repeated-specifiers.c"),
            false,
        ),
        (
            "imaginary-type.c",
            include_str!("../../../../tests/fixtures/parser/compatibility/imaginary-type.c"),
            true,
        ),
        (
            "gnu-statement-expression.c",
            include_str!(
                "../../../../tests/fixtures/parser/compatibility/gnu-statement-expression.c"
            ),
            true,
        ),
        (
            "constraint-invalid-lvalue.c",
            include_str!(
                "../../../../tests/fixtures/parser/compatibility/constraint-invalid-lvalue.c"
            ),
            false,
        ),
        (
            "typedef-parameter-preference.c",
            include_str!(
                "../../../../tests/fixtures/parser/compatibility/typedef-parameter-preference.c"
            ),
            false,
        ),
    ];

    for (name, source, expect_hard_parser_diagnostic) in fixtures {
        with_parse_configuration(source, strict, |parsed| {
            let has_hard_parser_diagnostic = parsed.errors.iter().any(|error| {
                matches!(
                    error,
                    TranslationError::Parsing(error) if error.severity == ErrorSeverity::Error
                )
            });
            assert_eq!(
                has_hard_parser_diagnostic, expect_hard_parser_diagnostic,
                "compatibility classification drifted for {name}: {:?}",
                parsed.errors
            );
            assert!(parsed.parser.frames.is_empty(), "{name}");
            assert!(parsed.parser.returned.is_none(), "{name}");
            assert_eq!(parsed.parser.scopes.depth(), 0, "{name}");
            match name {
                | "repeated-specifiers.c" => {
                    assert_eq!(parsed.items.len(), 2);
                    assert!(parsed.items.iter().all(|item| matches!(
                        item,
                        ExternalDeclaration::Declaration(_)
                            | ExternalDeclaration::FunctionDefinition(_)
                    )));
                },
                | "imaginary-type.c" => {
                    assert!(parser_errors(parsed).any(|error| matches!(
                        error,
                        ParserErrorType::UnsupportedImaginaryTypeSpecifier
                    )));
                    assert!(matches!(
                        parsed.items.as_slice(),
                        [ExternalDeclaration::RecoveredDeclaration(_)]
                    ));
                },
                | "gnu-statement-expression.c" => {
                    assert!(matches!(
                        parsed.items.as_slice(),
                        [ExternalDeclaration::RecoveredFunctionDefinition(_)]
                    ));
                    assert!(
                        parsed
                            .parser
                            .syntax
                            .iter::<Expression<'_>>()
                            .any(|expression| {
                                matches!(expression.kind, ExpressionType::Error)
                                    && expression.recovered
                            })
                    );
                    assert!(
                        parsed
                            .parser
                            .syntax
                            .iter::<Expression<'_>>()
                            .all(|expression| {
                                !matches!(expression.kind, ExpressionType::CompoundLiteral { .. })
                            })
                    );
                },
                | "constraint-invalid-lvalue.c" => {
                    assert!(parsed.parser.syntax.iter::<Expression<'_>>().any(
                        |expression| matches!(
                            expression.kind,
                            ExpressionType::Binary {
                                operator: BinaryOperator::Assignment,
                                ..
                            }
                        )
                    ));
                    assert!(matches!(
                        parsed.items.as_slice(),
                        [ExternalDeclaration::FunctionDefinition(_)]
                    ));
                },
                | "typedef-parameter-preference.c" => {
                    assert!(parsed.parser.syntax.iter::<ParameterDeclaration<'_>>().any(
                        |parameter| {
                            matches!(
                                parameter.declaration_specifiers.type_specifiers,
                                TypeSpecifiers::TypedefName(_)
                            )
                        }
                    ));
                    assert!(parsed.items.iter().all(|item| matches!(
                        item,
                        ExternalDeclaration::Declaration(_)
                            | ExternalDeclaration::FunctionDefinition(_)
                    )));
                },
                | _ => unreachable!("the fixture table is exhaustive"),
            }
        });
    }
}

#[test]
fn parser_diagnostics_expose_structured_context_and_fifo_recovery() {
    with_parse("}\nconst const inline inline int value;\n", |parsed| {
        let diagnostics = parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Parsing(error) => Some(error),
                | _ => None,
            })
            .collect::<Vec<_>>();

        let first = diagnostics[0];
        assert_eq!(first.code, ParserDiagnosticCode::Syntax);
        assert_eq!(first.severity, ErrorSeverity::Error);
        assert_eq!(first.frame, ParseFrameKind::DeclarationSpecifiers);
        assert_eq!(first.expected, ExpectedSyntax::DeclarationSpecifier);
        assert_eq!(
            first.found,
            Some(TokenType::Operator(OperatorTokenType::ClosingCurlyBrace))
        );
        assert!(first.source_vectors.length() > 0);
        assert!(first.recovery.is_some());
        assert_eq!(first.ranges, []);
        assert_eq!(first.related.len(), 1);
        assert_eq!(first.related[0].message, "parsing resumes here");

        let warnings = diagnostics
            .iter()
            .filter(|error| error.warning_group.is_some())
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings[0].position(parsed.parser.context).index
                < warnings[1].position(parsed.parser.context).index
        );
    });
}

#[test]
fn discarded_recovery_exposes_its_complete_summary() {
    with_parse("int first extra junk; int after;\n", |parsed| {
        let diagnostic = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if error
                        .recovery
                        .is_some_and(|recovery| recovery.discarded_tokens > 0) =>
                    Some(error),
                | _ => None,
            })
            .expect("expected a diagnostic with discarded input");
        let recovery = diagnostic.recovery.expect("recovery summary");
        let discarded = recovery.discarded.expect("discarded provenance");

        assert_eq!(recovery.owner, ParseFrameKind::Declaration);
        assert_eq!(recovery.discarded_tokens, 2);
        assert_eq!(
            recovery.stopped_at,
            Some(TokenType::Operator(OperatorTokenType::Semicolon))
        );
        assert_eq!(sourced_text(parsed, discarded), "extrajunk");
        assert_eq!(diagnostic.ranges, [discarded]);
        assert_eq!(diagnostic.related.len(), 1);
        assert_eq!(diagnostic.related[0].message, "parsing resumes here");
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
fn prototype_declaration_list_is_retained_in_one_recovered_function() {
    with_parse("int f(int x) int y; { return x; }\n", |parsed| {
        assert!(matches!(
            parsed.items.as_slice(),
            [ExternalDeclaration::RecoveredFunctionDefinition(_)]
        ));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::DeclarationListAfterParameterTypeList
        )));
        let definition = function_definition(parsed, 0);
        assert_eq!(definition.declaration_list.len(), 1);
        assert!(matches!(
            definition.body.kind,
            StatementType::Compound { .. }
        ));
    });
}

#[test]
fn declaration_continuation_diagnostic_has_structured_expectation() {
    with_parse("int first extra; int after;\n", |parsed| {
        let diagnostic = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(..)
                    ) =>
                    Some(error),
                | _ => None,
            })
            .expect("expected declaration-continuation diagnostic");

        assert_eq!(diagnostic.expected, ExpectedSyntax::DeclarationContinuation);
    });
}

#[test]
fn eof_delimiter_diagnostics_have_structured_expectations() {
    for source in ["int f(\n", "int f(int value, ...\n"] {
        with_parse(source, |parsed| {
            let diagnostic = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::UnexpectedEndOfFunctionDeclaratorParameterList
                            | ParserErrorType::UnexpectedEndOfVariadicFunctionDeclaratorParameterList
                    ) =>
                    Some(error),
                | _ => None,
            })
            .expect("expected an EOF delimiter diagnostic");

            assert_eq!(diagnostic.expected, ExpectedSyntax::OwnedDelimiter);
        });
    }
    assert_eq!(
        ParserErrorType::UnexpectedEndOfArrayDeclaratorAfterPointer.expected_syntax(),
        ExpectedSyntax::OwnedDelimiter
    );
}
