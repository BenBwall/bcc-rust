//! Preprocessor diagnostics and their rendering.

use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    io::Error as IoError,
    path::PathBuf,
};

use super::{
    expression::PreprocessorExpressionOperator,
    token::{
        SignedIntegerLiteralType,
        UnsignedIntegerLiteralType,
    },
};
use crate::{
    configuration::ExtensionPolicy,
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        closest_match,
        count_of,
        quote_spelling,
    },
    float_parsing::FloatRangeError,
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceVectors,
        SourcePosition,
        SourceVectors,
        preprocessor_tokenizer::PreprocessorTokenType,
    },
};

#[derive(Debug)]
pub(crate) struct PreprocessorError {
    pub(crate) error_type:     PreprocessorErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for PreprocessorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorError {
    fn to_diagnostic(&self, context: &Context, source: SourceVectors) -> Diagnostic {
        self.error_type
            .explain(context.source_spelling(source))
            .at(self.severity(), source)
    }
}

impl std::error::Error for PreprocessorError {}

impl GetPosition for PreprocessorError {
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for PreprocessorError {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

impl GetSeverity for PreprocessorError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorErrorType::UnexpectedEndOfInput(_)
            | PreprocessorErrorType::InvalidHexadecimalFloatLiteral
            | PreprocessorErrorType::InvalidDecimalFloatLiteral
            | PreprocessorErrorType::InvalidHexadecimalIntegerLiteral
            | PreprocessorErrorType::InvalidBinaryIntegerLiteral
            | PreprocessorErrorType::InvalidOctalIntegerLiteral
            | PreprocessorErrorType::InvalidDecimalIntegerLiteral
            | PreprocessorErrorType::IntegerLiteralOverflow
            | PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression
            | PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                ..
            }
            | PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(
                ..,
            )
            | PreprocessorErrorType::MissingIdentifierInDefinedDirective(..)
            | PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(..)
            | PreprocessorErrorType::NoConditionInIfDirective
            | PreprocessorErrorType::NoConditionInElifDirective
            | PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives
            | PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives
            | PreprocessorErrorType::ElifDirectiveWithoutIfDirective
            | PreprocessorErrorType::ConditionalArmAfterElse(_)
            | PreprocessorErrorType::ElseDirectiveWithoutIfDirective
            | PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInDefineDirective(..)
            | PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(..)
            | PreprocessorErrorType::InvalidCharacterInHeaderName(..)
            | PreprocessorErrorType::UnterminatedHeaderName(..)
            | PreprocessorErrorType::HeaderNotFound { .. }
            | PreprocessorErrorType::HeaderFileInaccessible(..)
            | PreprocessorErrorType::IncludeNestingLimitExceeded(..)
            | PreprocessorErrorType::HashHashUsedOutsideOfMacro
            | PreprocessorErrorType::CannotUseHashHashAfterFunctionLikeMacroCall
            | PreprocessorErrorType::InvalidLineFilename
            | PreprocessorErrorType::InvalidEscapeSequence
            | PreprocessorErrorType::UnterminatedEscapeSequence
            | PreprocessorErrorType::InvalidHexEscapeSequence
            | PreprocessorErrorType::HexEscapeSequenceTooLarge
            | PreprocessorErrorType::OctalEscapeSequenceTooLarge
            | PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence
            | PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort
            | PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence
            | PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall
            | PreprocessorErrorType::MultiCharacterLiteralsUnsupported
            | PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(..)
            | PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(..)
            | PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(..)
            | PreprocessorErrorType::VariadicMacroMustBeLastParameter(..)
            | PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(..)
            | PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(..)
            | PreprocessorErrorType::ExpectedIdentifierInUndefDirective(..)
            | PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(..)
            | PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(..)
            | PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(..)
            | PreprocessorErrorType::MissingRightHandSideOfHashHashOperator
            | PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator
            | PreprocessorErrorType::TokenMergingError(..)
            | PreprocessorErrorType::MissingNumberInLineDirective(..)
            | PreprocessorErrorType::MissingNewlineAfterLineDirective(..)
            | PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(..)
            | PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(..)
            | PreprocessorErrorType::MissingStringLiteralInPragmaOperator(..)
            | PreprocessorErrorType::UnknownPragmaSTDCArgument(..)
            | PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument
            | PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch
            | PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(..)
            | PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::BinaryPlusOverflow
            | PreprocessorErrorType::BinaryMinusOverflow
            | PreprocessorErrorType::DivideOverflow
            | PreprocessorErrorType::DivideByZero
            | PreprocessorErrorType::ModuloOverflow
            | PreprocessorErrorType::ModuloByZero
            | PreprocessorErrorType::MultiplyOverflow
            | PreprocessorErrorType::UnaryMinusOverflow
            | PreprocessorErrorType::LeftShiftOverflow
            | PreprocessorErrorType::RightShiftOverflow
            | PreprocessorErrorType::BitwiseNotWithoutOperand
            | PreprocessorErrorType::LogicalNotWithoutOperand
            | PreprocessorErrorType::UnaryMinusWithoutOperand
            | PreprocessorErrorType::UnaryPlusWithoutOperand
            | PreprocessorErrorType::BinaryPlusWithoutRhs
            | PreprocessorErrorType::BinaryMinusWithoutRhs
            | PreprocessorErrorType::MultiplyWithoutRhs
            | PreprocessorErrorType::DivideWithoutRhs
            | PreprocessorErrorType::ModuloWithoutRhs
            | PreprocessorErrorType::BitwiseOrWithoutRhs
            | PreprocessorErrorType::BitwiseAndWithoutRhs
            | PreprocessorErrorType::BitwiseXorWithoutRhs
            | PreprocessorErrorType::LogicalAndWithoutRhs
            | PreprocessorErrorType::LogicalOrWithoutRhs
            | PreprocessorErrorType::LessThanWithoutRhs
            | PreprocessorErrorType::LessThanEqualsWithoutRhs
            | PreprocessorErrorType::GreaterThanWithoutRhs
            | PreprocessorErrorType::GreaterThanEqualsWithoutRhs
            | PreprocessorErrorType::EqualsWithoutRhs
            | PreprocessorErrorType::NotEqualsWithoutRhs
            | PreprocessorErrorType::LeftShiftWithoutRhs
            | PreprocessorErrorType::RightShiftWithoutRhs
            | PreprocessorErrorType::TernaryOperatorWithoutRhs
            | PreprocessorErrorType::TernaryOperatorWithoutColon
            | PreprocessorErrorType::TernaryOperatorWithoutMhs
            | PreprocessorErrorType::ColonWithoutMatchingQuestionMark
            | PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression
            | PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(_)
            | PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression
            | PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(_)
            | PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(..)
            | PreprocessorErrorType::UnexpectedTokenAtPhase7(..)
            | PreprocessorErrorType::ErrorDirective(..)
             => ErrorSeverity::Error,
            | PreprocessorErrorType::CommaOperatorInPreprocessorExpression(policy)
            | PreprocessorErrorType::MissingVariadicArgument(policy)
            | PreprocessorErrorType::BackslashInQuotedHeaderName(policy) =>
                match policy {
                    | ExtensionPolicy::Allow => unreachable!(
                        "allowed extensions produce no diagnostic"
                    ),
                    | ExtensionPolicy::Warn => ErrorSeverity::Warning,
                    | ExtensionPolicy::Deny => ErrorSeverity::Error,
                },
            | PreprocessorErrorType::RedefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(..)
            | PreprocessorErrorType::FloatConstantOutOfRange { .. }
            | PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. }
            | PreprocessorErrorType::ForcedUnsignedPromotion { .. }
            | PreprocessorErrorType::ForcedSignedPromotion { .. }
            | PreprocessorErrorType::HashMustBeFirstCharacterOnLine
            | PreprocessorErrorType::UnknownDirective
            | PreprocessorErrorType::HashMustBeFollowedByIdentifier
            | PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence
            | PreprocessorErrorType::LineDirectiveNumberTooLarge(..)
            | PreprocessorErrorType::UnknownPragmaDirective
            | PreprocessorErrorType::ExtraTokensAfterPragmaOnce(..)
            | PreprocessorErrorType::ExtraTokensAfterPragmaOperator
            | PreprocessorErrorType::ExtraTokensAfterIncludeDirective
            | PreprocessorErrorType::ExtraTokensAfterConditionalDirective(_)
            | PreprocessorErrorType::ExtraTokensAfterIfdefDirective
            | PreprocessorErrorType::ExtraTokensAfterIfndefDirective
            | PreprocessorErrorType::PragmaOnceInNonHeader => ErrorSeverity::Warning,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PreprocessorErrorType {
    InvalidHexadecimalFloatLiteral,
    InvalidDecimalFloatLiteral,
    FloatConstantOutOfRange {
        type_name: &'static str,
        error:     FloatRangeError,
    },
    InvalidHexadecimalIntegerLiteral,
    InvalidBinaryIntegerLiteral,
    InvalidOctalIntegerLiteral,
    InvalidDecimalIntegerLiteral,
    IntegerLiteralOverflow,
    ForcedSignedToUnsignedConversion {
        from: SignedIntegerLiteralType,
        to:   UnsignedIntegerLiteralType,
    },
    ForcedUnsignedPromotion {
        from: UnsignedIntegerLiteralType,
        to:   UnsignedIntegerLiteralType,
    },
    ForcedSignedPromotion {
        from: SignedIntegerLiteralType,
        to:   SignedIntegerLiteralType,
    },
    HashMustBeFirstCharacterOnLine,
    HashMustBeFollowedByIdentifier,
    UnknownDirective,
    EmptyParenthesesInPreprocessorExpression,
    UnaryPlusWithoutOperand,
    UnaryMinusWithoutOperand,
    BitwiseNotWithoutOperand,
    LogicalNotWithoutOperand,
    BinaryPlusWithoutRhs,
    BinaryMinusWithoutRhs,
    MultiplyWithoutRhs,
    DivideWithoutRhs,
    ModuloWithoutRhs,
    LessThanWithoutRhs,
    LessThanEqualsWithoutRhs,
    GreaterThanWithoutRhs,
    GreaterThanEqualsWithoutRhs,
    EqualsWithoutRhs,
    NotEqualsWithoutRhs,
    LeftShiftWithoutRhs,
    RightShiftWithoutRhs,
    BitwiseAndWithoutRhs,
    BitwiseXorWithoutRhs,
    BitwiseOrWithoutRhs,
    LogicalAndWithoutRhs,
    LogicalOrWithoutRhs,
    TernaryOperatorWithoutMhs,
    TernaryOperatorWithoutColon,
    TernaryOperatorWithoutRhs,
    ColonWithoutMatchingQuestionMark,
    CommaOperatorInPreprocessorExpression(ExtensionPolicy),
    BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(PreprocessorExpressionOperator),
    DivideByZero,
    ModuloByZero,
    UnaryMinusOverflow,
    BinaryPlusOverflow,
    BinaryMinusOverflow,
    MultiplyOverflow,
    DivideOverflow,
    ModuloOverflow,
    LeftShiftOverflow,
    RightShiftOverflow,
    UnterminatedOpeningParenthesisInPreprocessorExpression,
    TildeInsteadOfBinaryOperatorInPreprocessorExpression,
    ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
    FunctionCallOperatorNotSupportedInPreprocessorExpression,
    DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
    AddressOfOperatorNotSupportedInPreprocessorExpression,
    DereferenceOperatorNotSupportedInPreprocessorExpression,
    ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(PreprocessorExpressionOperator),
    NumberInsteadOfBinaryOperatorInPreprocessorExpression,
    IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
    CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
    UnexpectedTokenInPreprocessorExpression(PreprocessorTokenType),
    UnexpectedTokenAtPhase7(PreprocessorTokenType),
    FloatInsteadOfIntegerInPreprocessorExpression,
    ExpectedBinaryOperatorInPreprocessorExpression,
    MissingOpeningParenthesisOrIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingClosingParenthesisInDefinedDirective(PreprocessorTokenType),
    NoConditionInIfDirective,
    NoConditionInElifDirective,
    MoreIfDirectivesThanEndifDirectives,
    MoreEndifDirectivesThanIfDirectives,
    ElifDirectiveWithoutIfDirective,
    ElseDirectiveWithoutIfDirective,
    ConditionalArmAfterElse(&'static str),
    ExtraTokensAfterConditionalDirective(&'static str),
    ExpectedIdentifierInIfdefDirective(PreprocessorTokenType),
    ExpectedIdentifierInIfndefDirective(PreprocessorTokenType),
    ExpectedIdentifierInDefineDirective(PreprocessorTokenType),
    RedefinitionOfBuiltInMacro(String),
    UndefinedIdentifierInPreprocessorExpression(String),
    ExpectedIncludeStringOrAngleBracketString(PreprocessorTokenType),
    /// A header name containing one of the sequences C99 §6.4.7p3 leaves
    /// undefined there: `'`, `\`, `"`, `//`, or `/*`.
    InvalidCharacterInHeaderName(&'static str),
    /// A header name without its closing `>` or `"`.
    UnterminatedHeaderName(char),
    /// A `\` in a `"…"` header name, accepted as a path character by the
    /// backslash extension.
    BackslashInQuotedHeaderName(ExtensionPolicy),
    UnexpectedEndOfInput(&'static str),
    WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
        expected: usize,
        found:    usize,
    },
    /// A variadic macro invocation that supplies no argument for `...`.
    MissingVariadicArgument(ExtensionPolicy),
    HeaderNotFound {
        name:             String,
        is_system_header: bool,
        searched:         Vec<PathBuf>,
    },
    HeaderFileInaccessible(IoError),
    IncludeNestingLimitExceeded(usize),
    HashHashUsedOutsideOfMacro,
    CannotUseHashHashAfterFunctionLikeMacroCall,
    InvalidEscapeSequence,
    InvalidLineFilename,
    UnterminatedEscapeSequence,
    InvalidHexEscapeSequence,
    HexEscapeSequenceTooLarge,
    OctalEscapeSequenceTooLarge,
    InvalidSmallUnicodeEscapeSequence,
    SmallUnicodeEscapeSequenceTooShort,
    InvalidLargeUnicodeEscapeSequence,
    LargeUnicodeEscapeSequenceTooSmall,
    MultiCharacterLiteralsUnsupported,
    RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(String),
    RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(String),
    ExpectedIdentifierInMacroDefinition(PreprocessorTokenType),
    VariadicMacroMustBeLastParameter(String),
    ExpectedCommaOrClosingParenthesisInMacroDefinition(PreprocessorTokenType),
    MacroRedefinedWithDifferentDefinition(String),
    ExpectedIdentifierInUndefDirective(PreprocessorTokenType),
    ExpectedNewlineAfterUndefDirective(PreprocessorTokenType),
    HashOperatorMustBeFollowedByAMacroArgument(PreprocessorTokenType),
    IdentifierNotMacroArgumentAfterHashOperator(String),
    MissingRightHandSideOfHashHashOperator,
    MissingLeftHandSideOfHashHashOperator,
    TokenMergingError(String, String),
    MissingNumberInLineDirective(PreprocessorTokenType),
    MissingNewlineAfterLineDirective(PreprocessorTokenType),
    LineDirectiveIsNotASimpleDigitSequence,
    LineDirectiveNumberTooLarge(String),
    MissingOpeningParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingClosingParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingStringLiteralInPragmaOperator(PreprocessorTokenType),
    UnknownPragmaDirective,
    UnknownPragmaSTDCArgument(String),
    ExtraTokensAfterPragmaOnce(PreprocessorTokenType),
    ExtraTokensAfterPragmaOperator,
    ExtraTokensAfterIncludeDirective,
    ExtraTokensAfterIfdefDirective,
    ExtraTokensAfterIfndefDirective,
    STDCPragmaDirectiveWithoutArgument,
    STDCPragmaDirectiveWithoutOnOffSwitch,
    MissingOnOffSwitchInSTDCPragma(String),
    PragmaOnceInNonHeader,
    ErrorDirective(String),
}

/// The directives of C99 §6.10, for suggestions.
const DIRECTIVE_NAMES: [&str; 12] = [
    "if", "ifdef", "ifndef", "elif", "else", "endif", "include", "define", "undef", "line",
    "error", "pragma",
];

const DIRECTIVE_LIST_NOTE: &str = "C99 §6.10: the directives are `#if`, `#ifdef`, `#ifndef`, \
                                   `#elif`, `#else`, `#endif`, `#include`, `#define`, `#undef`, \
                                   `#line`, `#error`, and `#pragma`";

const IF_EXPRESSION_NOTE: &str =
    "C99 §6.10.1p1: the condition must be an integer constant expression";

const ESCAPE_LIST_NOTE: &str = "C99 §6.4.4.4: the escapes are `\\'`, `\\\"`, `\\?`, `\\\\`, \
                                `\\a`, `\\b`, `\\f`, `\\n`, `\\r`, `\\t`, `\\v`, octal `\\ooo`, \
                                hexadecimal `\\xhh`, and universal `\\uXXXX` or `\\UXXXXXXXX`";

impl PreprocessorErrorType {
    /// Describes the error; `spelling` is the source text it points at, used
    /// to name what was actually written.
    #[expect(
        clippy::too_many_lines,
        reason = "One exhaustive table keeps every preprocessor message reviewable in one place."
    )]
    pub(crate) fn explain(&self, spelling: Option<&str>) -> Explanation {
        let quoted = || spelling.map(quote_spelling);
        let titled = |title: &str| match quoted() {
            | Some(quoted) => format!("{title} {quoted}"),
            | None => title.to_owned(),
        };
        let missing_operand = |operator: &str, side: &str| {
            Explanation::new(format!("expected an expression {side} `{operator}`"))
                .label(format!("`{operator}` needs an operand here"))
        };
        let overflow = |operation: &str| {
            Explanation::new(format!("{operation} overflows in `#if` expression"))
                .label("the result is out of range")
                .note(
                    "C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 \
                     requires constant expressions to stay in range",
                )
        };
        let instead_of_operator = |found: &str| {
            Explanation::new(format!("expected an operator, found {found}"))
                .label("expected a binary operator before this")
                .note("operands in an `#if` expression must be joined by operators")
        };
        match self {
            | Self::InvalidHexadecimalFloatLiteral =>
                Explanation::new(titled("invalid hexadecimal floating constant"))
                    .label("not a valid hexadecimal floating constant")
                    .note(
                        "C99 §6.4.4.2: a hexadecimal floating constant needs hexadecimal digits \
                         and a binary exponent, as in `0x1.8p3`",
                    ),
            | Self::InvalidDecimalFloatLiteral =>
                Explanation::new(titled("invalid floating constant"))
                    .label("not a valid floating constant")
                    .note(
                        "C99 §6.4.4.2: a floating constant is digits with a `.` or an exponent \
                         and an optional `f`, `F`, `l`, or `L` suffix, as in `1.5`, `2e10`, or \
                         `.5f`",
                    ),
            | Self::FloatConstantOutOfRange { type_name, error } => match error {
                | FloatRangeError::Overflow =>
                    Explanation::new(format!("floating constant is too large for `{type_name}`"))
                        .label("this value becomes infinity")
                        .note(
                            "C99 §6.4.4p2: the value of a constant must be representable in its \
                             type",
                        ),
                | FloatRangeError::Underflow =>
                    Explanation::new(format!("floating constant is too small for `{type_name}`"))
                        .label("this nonzero value becomes zero")
                        .note(
                            "C99 §6.4.4p2: the value of a constant must be representable in its \
                             type",
                        ),
            },
            | Self::InvalidHexadecimalIntegerLiteral =>
                Explanation::new(titled("invalid hexadecimal integer constant"))
                    .label("not a valid hexadecimal constant")
                    .note(
                        "C99 §6.4.4.1: hexadecimal digits are `0`-`9`, `a`-`f`, and `A`-`F`, \
                         optionally followed by an integer suffix such as `u`, `l`, or `ull`",
                    ),
            | Self::InvalidBinaryIntegerLiteral =>
                Explanation::new(titled("invalid binary integer constant"))
                    .label("not a valid binary constant")
                    .note("binary digits are `0` and `1`"),
            | Self::InvalidOctalIntegerLiteral =>
                Explanation::new(titled("invalid octal integer constant"))
                    .label("not a valid octal constant")
                    .note(
                        "C99 §6.4.4.1: a constant starting with `0` is octal, and octal digits \
                         are `0`-`7`",
                    ),
            | Self::InvalidDecimalIntegerLiteral =>
                Explanation::new(titled("invalid integer constant"))
                    .label("not a valid integer constant")
                    .note(
                        "C99 §6.4.4.1: an integer constant is digits followed by an optional \
                         suffix such as `u`, `l`, `ll`, or `ull`",
                    ),
            | Self::IntegerLiteralOverflow => Explanation::new("integer constant is too large")
                .label("does not fit in `unsigned long long`")
                .note("the largest integer type, `unsigned long long`, has 64 bits"),
            | Self::ForcedSignedToUnsignedConversion { from, to } => match from {
                | SignedIntegerLiteralType::Int =>
                    Explanation::new("integer constant is so large that it is unsigned")
                        .label(format!("this constant has type `{}`", to.spelling()))
                        .note("C99 §6.4.4.1p5: no signed type can represent this decimal constant")
                        .help("add a `u` suffix to make the unsigned type explicit"),
                | SignedIntegerLiteralType::Long | SignedIntegerLiteralType::LongLong =>
                    Explanation::new(format!(
                        "integer constant is too large for `{}`",
                        from.spelling()
                    ))
                    .label(format!("this constant has type `{}`", to.spelling()))
                    .help("add a `u` suffix to make the unsigned type explicit"),
            },
            | Self::ForcedUnsignedPromotion { from, to } => Explanation::new(format!(
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format!("this constant has type `{}`", to.spelling())),
            | Self::ForcedSignedPromotion { from, to } => Explanation::new(format!(
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format!("this constant has type `{}`", to.spelling())),
            | Self::HashMustBeFirstCharacterOnLine => Explanation::new("stray `#` in program")
                .label("a directive must start a line")
                .note(
                    "C99 §6.10p2: `#` begins a directive only as the first token on a line; \
                     elsewhere it is valid only inside a function-like macro definition",
                ),
            | Self::HashMustBeFollowedByIdentifier => {
                let found = spelling.map_or_else(|| "a token".to_owned(), quote_spelling);
                Explanation::new(format!(
                    "expected a directive name after `#`, found {found}"
                ))
                .label("expected a directive name")
                .note(DIRECTIVE_LIST_NOTE)
            },
            | Self::UnknownDirective => {
                let name = spelling.unwrap_or_default();
                let explanation =
                    Explanation::new(format!("unknown preprocessing directive `#{name}`"))
                        .label("not a C99 directive")
                        .note(DIRECTIVE_LIST_NOTE);
                match closest_match(name, &DIRECTIVE_NAMES) {
                    | Some(suggestion) =>
                        explanation.help(format!("did you mean `#{suggestion}`?")),
                    | None => explanation,
                }
            },
            | Self::EmptyParenthesesInPreprocessorExpression =>
                Explanation::new("expected an expression inside `()`")
                    .label("empty parentheses")
                    .note(IF_EXPRESSION_NOTE),
            | Self::UnaryPlusWithoutOperand | Self::BinaryPlusWithoutRhs =>
                missing_operand("+", "after"),
            | Self::UnaryMinusWithoutOperand | Self::BinaryMinusWithoutRhs =>
                missing_operand("-", "after"),
            | Self::BitwiseNotWithoutOperand => missing_operand("~", "after"),
            | Self::LogicalNotWithoutOperand => missing_operand("!", "after"),
            | Self::MultiplyWithoutRhs => missing_operand("*", "after"),
            | Self::DivideWithoutRhs => missing_operand("/", "after"),
            | Self::ModuloWithoutRhs => missing_operand("%", "after"),
            | Self::LessThanWithoutRhs => missing_operand("<", "after"),
            | Self::LessThanEqualsWithoutRhs => missing_operand("<=", "after"),
            | Self::GreaterThanWithoutRhs => missing_operand(">", "after"),
            | Self::GreaterThanEqualsWithoutRhs => missing_operand(">=", "after"),
            | Self::EqualsWithoutRhs => missing_operand("==", "after"),
            | Self::NotEqualsWithoutRhs => missing_operand("!=", "after"),
            | Self::LeftShiftWithoutRhs => missing_operand("<<", "after"),
            | Self::RightShiftWithoutRhs => missing_operand(">>", "after"),
            | Self::BitwiseAndWithoutRhs => missing_operand("&", "after"),
            | Self::BitwiseXorWithoutRhs => missing_operand("^", "after"),
            | Self::BitwiseOrWithoutRhs => missing_operand("|", "after"),
            | Self::LogicalAndWithoutRhs => missing_operand("&&", "after"),
            | Self::LogicalOrWithoutRhs => missing_operand("||", "after"),
            | Self::TernaryOperatorWithoutColon =>
                Explanation::new("expected `:` after `?` in preprocessor expression")
                    .label("this conditional operator has no colon"),
            | Self::TernaryOperatorWithoutMhs => missing_operand("?", "after"),
            | Self::TernaryOperatorWithoutRhs => missing_operand(":", "after"),
            | Self::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "after"),
            | Self::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "before"),
            | Self::ColonWithoutMatchingQuestionMark =>
                Explanation::new("`:` without a matching `?`")
                    .label("no `?` precedes this `:` in the same parentheses"),
            | Self::CommaOperatorInPreprocessorExpression(_) =>
                Explanation::new("comma operator in `#if` expression")
                    .label("evaluated comma operator")
                    .note(
                        "C99 §6.6p3: constant expressions shall not contain comma operators, \
                         except within an operand that is not evaluated",
                    ),
            | Self::DivideByZero => Explanation::new("division by zero in `#if` expression")
                .label("the divisor is zero")
                .note("C99 §6.5.5p5: the result of `/` by zero is undefined"),
            | Self::ModuloByZero => Explanation::new("remainder by zero in `#if` expression")
                .label("the divisor is zero")
                .note("C99 §6.5.5p5: the result of `%` by zero is undefined"),
            | Self::UnaryMinusOverflow => overflow("negation"),
            | Self::BinaryPlusOverflow => overflow("addition"),
            | Self::BinaryMinusOverflow => overflow("subtraction"),
            | Self::MultiplyOverflow => overflow("multiplication"),
            | Self::DivideOverflow => overflow("division"),
            | Self::ModuloOverflow => overflow("remainder"),
            | Self::LeftShiftOverflow => overflow("left shift"),
            | Self::RightShiftOverflow => overflow("right shift"),
            | Self::UnterminatedOpeningParenthesisInPreprocessorExpression =>
                Explanation::new("unclosed `(` in `#if` expression").label("expected `)`"),
            | Self::TildeInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`~`"),
            | Self::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`!`"),
            | Self::NumberInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Number.found(spelling)),
            | Self::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Identifier.found(spelling)),
            | Self::CharacterInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Character.found(spelling)),
            | Self::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`defined`"),
            | Self::ExpectedBinaryOperatorInPreprocessorExpression =>
                Explanation::new("expected an operator between the operands of `#if`")
                    .label("the expression ends with an operand still unjoined")
                    .note(IF_EXPRESSION_NOTE),
            | Self::AddressOfOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("`&` cannot take an address in `#if` expression")
                    .label("addresses do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::DereferenceOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("`*` cannot dereference in `#if` expression")
                    .label("pointers do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::FunctionCallOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("function call in `#if` expression")
                    .label("functions cannot be called during preprocessing")
                    .note(
                        "C99 §6.10.1p3: identifiers that are not macros evaluate to `0`, so this \
                         looks like a call of an undefined function-like macro",
                    )
                    .help("define the function-like macro before this directive"),
            | Self::FloatInsteadOfIntegerInPreprocessorExpression =>
                Explanation::new("floating constant in `#if` expression")
                    .label("not an integer")
                    .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenInPreprocessorExpression(kind) => Explanation::new(format!(
                "unexpected {} in `#if` expression",
                kind.found(spelling)
            ))
            .label("not valid in an integer constant expression")
            .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenAtPhase7(kind) =>
                Explanation::new(format!("stray {} in program", kind.found(spelling)))
                    .label("only meaningful inside a preprocessing directive"),
            | Self::MissingOpeningParenthesisOrIdentifierInDefinedDirective(kind) =>
                Explanation::new(format!(
                    "expected a macro name after `defined`, found {}",
                    kind.found(spelling)
                ))
                .label("expected a macro name")
                .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingIdentifierInDefinedDirective(kind) => Explanation::new(format!(
                "expected a macro name inside `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingClosingParenthesisInDefinedDirective(kind) => Explanation::new(format!(
                "expected `)` to close `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::NoConditionInIfDirective => Explanation::new("`#if` with no condition")
                .label("expected an expression")
                .help("write the condition to test, as in `#if VERSION >= 2`"),
            | Self::NoConditionInElifDirective => Explanation::new("`#elif` with no condition")
                .label("expected an expression")
                .help("write the condition to test, or use `#else`"),
            | Self::MoreIfDirectivesThanEndifDirectives =>
                Explanation::new(format!("unterminated `#{}`", spelling.unwrap_or("if")))
                    .label("this conditional has no matching `#endif`")
                    .help("add `#endif` where the conditional section should end"),
            | Self::MoreEndifDirectivesThanIfDirectives =>
                Explanation::new("`#endif` without `#if`")
                    .label("no conditional directive is open here"),
            | Self::ElifDirectiveWithoutIfDirective => Explanation::new("`#elif` without `#if`")
                .label("no conditional directive is open here"),
            | Self::ConditionalArmAfterElse(name) =>
                Explanation::new(format!("`#{name}` after `#else`"))
                    .label("the final arm of this conditional has already begun")
                    .note("C99 §6.10.1: a conditional group permits one final #else arm"),
            | Self::ExtraTokensAfterConditionalDirective(name) =>
                Explanation::new(format!("extra tokens after `#{name}`"))
                    .label("expected the end of the directive"),
            | Self::ElseDirectiveWithoutIfDirective => Explanation::new("`#else` without `#if`")
                .label("no conditional directive is open here"),
            | Self::ExpectedIdentifierInIfdefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#ifdef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedIdentifierInIfndefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#ifndef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedIdentifierInDefineDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#define`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.3: a macro name is an identifier"),
            | Self::RedefinitionOfBuiltInMacro(name) =>
                Explanation::new(format!("cannot redefine predefined macro `{name}`"))
                    .label("predefined by the implementation")
                    .note("C99 §6.10.8p4: predefined macro names shall not be redefined"),
            | Self::UndefinedIdentifierInPreprocessorExpression(name) =>
                Explanation::new(format!("`{name}` is not defined; it evaluates to 0"))
                    .label("not a macro")
                    .note(
                        "C99 §6.10.1p3: identifiers that are not macro names are replaced with \
                         `0` in `#if`",
                    )
                    .help(format!(
                        "use `defined({name})` to test whether it is defined"
                    )),
            | Self::ExpectedIncludeStringOrAngleBracketString(kind) => Explanation::new(format!(
                "expected a header name after `#include`, found {}",
                kind.found(spelling)
            ))
            .label("expected `\"file.h\"` or `<file.h>`")
            .note(
                "C99 §6.10.2: `#include` takes `\"name\"`, `<name>`, or macros that expand to one \
                 of them",
            ),
            | Self::InvalidCharacterInHeaderName(sequence) => Explanation::new(format!(
                "header name in `#include` cannot contain `{sequence}`"
            ))
            .label("not allowed in a header name")
            .note(
                "C99 §6.4.7p3: the behavior is undefined if `'`, `\\`, `//`, or `/*` occurs in a \
                 header name, or `\"` occurs between `<` and `>`",
            ),
            | Self::UnterminatedHeaderName(delimiter) => Explanation::new(format!(
                "header name in `#include` is missing its closing `{delimiter}`"
            ))
            .label(format!("expected `{delimiter}` here"))
            .note("C99 §6.4.7p1: a header name is `<h-char-sequence>` or `\"q-char-sequence\"`")
            .help(format!(
                "close the header name with `{delimiter}` on this line"
            )),
            | Self::BackslashInQuotedHeaderName(_) =>
                Explanation::new("a backslash in a header name is an extension")
                    .label("read as a path character")
                    .note("C99 §6.4.7p3: the behavior is undefined if `\\` occurs in a header name"),
            | Self::UnexpectedEndOfInput(activity) => Explanation::new(format!(
                "unexpected end of file while {}",
                activity.trim_end_matches('.')
            ))
            .label("the file ends here"),
            | Self::WrongNumberOfArgumentsInFunctionLikeMacroInvocation { expected, found } =>
                Explanation::new(format!(
                    "this macro takes {} but {} {} supplied",
                    count_of(*expected, "argument"),
                    count_of(*found, "argument"),
                    if *found == 1 { "was" } else { "were" }
                ))
                .label(format!("expected {}", count_of(*expected, "argument")))
                .note(
                    "C99 §6.10.3p4: an invocation must supply one argument per parameter, plus \
                     any for `...`",
                ),
            | Self::MissingVariadicArgument(_) =>
                Explanation::new("this invocation supplies no argument for `...`")
                    .label("`__VA_ARGS__` is empty here")
                    .note(
                        "C99 §6.10.3p4: an invocation of a macro with `...` must supply more \
                         arguments than the macro has named parameters",
                    ),
            | Self::HeaderNotFound {
                name,
                is_system_header,
                searched,
            } => {
                let mut explanation = Explanation::new(format!("cannot find header `{name}`"))
                    .label("not found in any search directory");
                if searched.is_empty() {
                    explanation =
                        explanation.note("the path is absolute; no directory was searched");
                } else {
                    let list: Vec<String> = searched
                        .iter()
                        .map(|directory| {
                            if directory.as_os_str().is_empty() {
                                "  . (the working directory)".to_owned()
                            } else {
                                format!("  {}", directory.display())
                            }
                        })
                        .collect();
                    explanation = explanation
                        .note(format!("searched these directories:\n{}", list.join("\n")));
                }
                explanation.help(if *is_system_header {
                    "add the directory containing it with `--isystem <dir>`"
                } else {
                    "add the directory containing it with `--iquote <dir>` or `--isystem <dir>`"
                })
            },
            | Self::HeaderFileInaccessible(error) =>
                Explanation::new(format!("cannot read included file: {error}"))
                    .label("included here"),
            | Self::IncludeNestingLimitExceeded(limit) =>
                Explanation::new(format!("include nesting exceeds the maximum of {limit}"))
                    .label("this include exceeds the implementation limit")
                    .note("C99 §5.2.4.1: at least 15 nested included files are supported"),
            | Self::HashHashUsedOutsideOfMacro =>
                Explanation::new("`##` outside a macro definition")
                    .label("token pasting only happens in replacement lists")
                    .note("C99 §6.10.3.3: `##` is an operator of macro replacement lists"),
            | Self::CannotUseHashHashAfterFunctionLikeMacroCall =>
                Explanation::new("`##` cannot follow a function-like macro invocation")
                    .label("pasting onto an invocation is not supported"),
            | Self::InvalidLineFilename =>
                Explanation::new("line filename is not representable as a path")
                    .label("use a UTF-8 filename without NUL characters"),
            | Self::InvalidEscapeSequence => Explanation::new("unknown escape sequence")
                .label("contains an escape C99 does not define")
                .note(ESCAPE_LIST_NOTE),
            | Self::UnterminatedEscapeSequence => Explanation::new("incomplete escape sequence")
                .label("`\\` ends the literal")
                .help("write `\\\\` for a backslash character"),
            | Self::InvalidHexEscapeSequence =>
                Explanation::new("hexadecimal escape sequence is not a valid character")
                    .label("contains an invalid `\\x` escape"),
            | Self::HexEscapeSequenceTooLarge =>
                Explanation::new("hexadecimal escape sequence is out of range")
                    .label("contains an oversized `\\x` escape"),
            | Self::OctalEscapeSequenceTooLarge =>
                Explanation::new("octal escape sequence is out of range")
                    .label("contains an oversized octal escape"),
            | Self::InvalidSmallUnicodeEscapeSequence =>
                Explanation::new("`\\u` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note(
                        "C99 §6.4.3: values below U+00A0 are forbidden except U+0024, U+0040 and \
                         U+0060; surrogates are forbidden",
                    ),
            | Self::SmallUnicodeEscapeSequenceTooShort =>
                Explanation::new("`\\u` escape needs exactly four hexadecimal digits")
                    .label("contains a short `\\u` escape"),
            | Self::InvalidLargeUnicodeEscapeSequence =>
                Explanation::new("`\\U` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note(
                        "C99 §6.4.3: values below U+00A0 are forbidden except U+0024, U+0040 and \
                         U+0060; surrogates are forbidden",
                    ),
            | Self::LargeUnicodeEscapeSequenceTooSmall =>
                Explanation::new("`\\U` escape needs exactly eight hexadecimal digits")
                    .label("contains a short `\\U` escape"),
            | Self::MultiCharacterLiteralsUnsupported => {
                if spelling.is_some_and(|spelling| spelling.ends_with("''")) {
                    Explanation::new("empty character constant")
                        .label("contains no character")
                        .note("C99 §6.4.4.4: a character constant contains at least one character")
                } else {
                    Explanation::new("wide character constant with more than one character")
                        .label("its value is implementation-defined")
                        .note(
                            "C99 §6.4.4.4p11: bcc does not assign a value to multi-character wide \
                             constants",
                        )
                }
            },
            | Self::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(name) => Explanation::new(
                format!("function-like macro `{name}` redefined as an object-like macro"),
            )
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format!("add `#undef {name}` before this definition")),
            | Self::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(name) => Explanation::new(
                format!("object-like macro `{name}` redefined as a function-like macro"),
            )
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format!("add `#undef {name}` before this definition")),
            | Self::ExpectedIdentifierInMacroDefinition(kind) => Explanation::new(format!(
                "expected a parameter name, found {}",
                kind.found(spelling)
            ))
            .label("expected an identifier or `...`"),
            | Self::VariadicMacroMustBeLastParameter(name) =>
                Explanation::new(format!("`...` must be the last parameter of `{name}`"))
                    .label("parameter after `...`")
                    .note("C99 §6.10.3p12: `...` ends the parameter list"),
            | Self::ExpectedCommaOrClosingParenthesisInMacroDefinition(kind) =>
                Explanation::new(format!(
                    "expected `,` or `)` in macro parameter list, found {}",
                    kind.found(spelling)
                ))
                .label("expected `,` or `)`"),
            | Self::MacroRedefinedWithDifferentDefinition(name) =>
                Explanation::new(format!("macro `{name}` redefined differently"))
                    .label("this definition differs from the previous one")
                    .note(
                        "C99 §6.10.3p2: a redefinition must have the same parameters and an \
                         identical replacement list",
                    )
                    .help(format!("add `#undef {name}` before this definition")),
            | Self::ExpectedIdentifierInUndefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#undef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedNewlineAfterUndefDirective(kind) => Explanation::new(format!(
                "unexpected {} after the macro name in `#undef`",
                kind.found(spelling)
            ))
            .label("`#undef` takes one name"),
            | Self::HashOperatorMustBeFollowedByAMacroArgument(kind) => Explanation::new(format!(
                "expected a macro parameter after `#`, found {}",
                kind.found(spelling)
            ))
            .label("expected a parameter name")
            .note("C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a parameter"),
            | Self::IdentifierNotMacroArgumentAfterHashOperator(name) =>
                Explanation::new(format!("`{name}` is not a parameter of this macro"))
                    .label("`#` can only stringify a parameter")
                    .note(
                        "C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a \
                         parameter",
                    ),
            | Self::MissingRightHandSideOfHashHashOperator =>
                Explanation::new("`##` cannot end a replacement list")
                    .label("nothing follows this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::MissingLeftHandSideOfHashHashOperator =>
                Explanation::new("`##` cannot start a replacement list")
                    .label("nothing precedes this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::TokenMergingError(lhs, rhs) => Explanation::new(format!(
                "pasting `{lhs}` and `{rhs}` does not give a valid token"
            ))
            .label("invalid token paste")
            .note("C99 §6.10.3.3p3: the result of `##` must be a single valid preprocessing token"),
            | Self::MissingNumberInLineDirective(kind) => Explanation::new(format!(
                "expected a line number after `#line`, found {}",
                kind.found(spelling)
            ))
            .label("expected a line number"),
            | Self::MissingNewlineAfterLineDirective(kind) =>
                Explanation::new(format!("unexpected {} after `#line`", kind.found(spelling)))
                    .label("`#line` takes a number and an optional file name"),
            | Self::LineDirectiveIsNotASimpleDigitSequence =>
                Explanation::new("`#line` needs a plain decimal line number")
                    .label("not a digit sequence")
                    .note("C99 §6.10.4p3: the line number is a digit sequence, not any constant"),
            | Self::LineDirectiveNumberTooLarge(number) =>
                Explanation::new(format!("line number {number} is out of range"))
                    .label("too large")
                    .note("C99 §6.10.4p3: the line number must be at most 2147483647"),
            | Self::MissingOpeningParenthesisInPragmaOperator(kind) => Explanation::new(format!(
                "expected `(` after `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected `(`")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::MissingClosingParenthesisInPragmaOperator(kind) => Explanation::new(format!(
                "expected `)` to close `_Pragma(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::MissingStringLiteralInPragmaOperator(kind) => Explanation::new(format!(
                "expected a string literal in `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected a string literal")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::UnknownPragmaDirective => Explanation::new("unknown pragma ignored")
                .label("not recognized")
                .note("bcc recognizes `#pragma once` and the `#pragma STDC` pragmas"),
            | Self::UnknownPragmaSTDCArgument(argument) =>
                Explanation::new(format!("unknown `STDC` pragma `{argument}`"))
                    .label("not a standard pragma")
                    .note(
                        "C99 §6.10.6: the standard pragmas are `FP_CONTRACT`, `FENV_ACCESS`, and \
                         `CX_LIMITED_RANGE`",
                    ),
            | Self::ExtraTokensAfterPragmaOnce(kind) => Explanation::new(format!(
                "unexpected {} after `#pragma once`",
                kind.found(spelling)
            ))
            .label("`#pragma once` takes no arguments"),
            | Self::ExtraTokensAfterPragmaOperator =>
                Explanation::new("extra tokens after the pragma in `_Pragma`")
                    .label("not part of the pragma"),
            | Self::ExtraTokensAfterIncludeDirective =>
                Explanation::new("extra tokens at end of `#include` directive").label("ignored"),
            | Self::ExtraTokensAfterIfdefDirective =>
                Explanation::new("extra tokens at end of `#ifdef` directive").label("ignored"),
            | Self::ExtraTokensAfterIfndefDirective =>
                Explanation::new("extra tokens at end of `#ifndef` directive").label("ignored"),
            | Self::STDCPragmaDirectiveWithoutArgument =>
                Explanation::new("expected a pragma name after `#pragma STDC`")
                    .label("expected `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE`"),
            | Self::STDCPragmaDirectiveWithoutOnOffSwitch =>
                Explanation::new("expected `ON`, `OFF`, or `DEFAULT` in `#pragma STDC`")
                    .label("the pragma ends here")
                    .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::MissingOnOffSwitchInSTDCPragma(argument) => Explanation::new(format!(
                "expected `ON`, `OFF`, or `DEFAULT` after `{argument}`"
            ))
            .label("expected an on-off switch")
            .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::PragmaOnceInNonHeader => Explanation::new("`#pragma once` in main file")
                .label("only affects files that are included"),
            | Self::ErrorDirective(message) => {
                let message = message.trim();
                Explanation::new(if message.is_empty() {
                    "#error".to_owned()
                } else {
                    format!("#error {message}")
                })
                .label("`#error` directive")
            },
        }
    }
}

impl Display for PreprocessorErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(&self.explain(None).message)
    }
}
