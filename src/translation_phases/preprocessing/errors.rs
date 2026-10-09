//! Preprocessor diagnostics and their rendering.
//!
//! These diagnose translation phases 4-7 as far as the preprocessor carries
//! them. C99 requires a diagnostic for every violated syntax rule or
//! constraint (§5.1.1.3 paragraph 1, p. 11; PDF p. 23); the other variants
//! report undefined behavior, recovery, or extensions that the extension
//! policy governs (§4 paragraph 6, p. 7; PDF p. 19). Each variant's comment
//! names the clause it comes from, and the rendered notes cite clauses in
//! the compact `C99 §6.10.3p2:` form.

use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
        Write as _,
    },
    path::Path,
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
        format_in,
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
    util::bump::{
        ArenaString,
        Bump,
    },
};

#[derive(Debug)]
pub(crate) struct PreprocessorError<'tu> {
    pub(crate) error_type:     PreprocessorErrorType<'tu>,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for PreprocessorError<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorError<'_> {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        self.error_type
            .explain_in(arena, context.source_spelling(source))
            .at(self.severity(), source)
    }
}

impl std::error::Error for PreprocessorError<'_> {}

impl GetPosition for PreprocessorError<'_> {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for PreprocessorError<'_> {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}

impl GetSeverity for PreprocessorError<'_> {
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
            | PreprocessorErrorType::ElifDirectiveWithoutIfDirective(_)
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
            | PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(..)
            | PreprocessorErrorType::VariadicMacroMustBeLastParameter(..)
            | PreprocessorErrorType::DuplicateMacroParameter(..)
            | PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(..)
            | PreprocessorErrorType::ExpectedIdentifierInUndefDirective(..)
            | PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(..)
            | PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(..)
            | PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(..)
            | PreprocessorErrorType::MissingRightHandSideOfHashHashOperator
            | PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator
            | PreprocessorErrorType::TokenMergingError(..)
            | PreprocessorErrorType::MissingNumberInLineDirective(..)
            | PreprocessorErrorType::MissingNewlineAfterLineDirective(..)
            | PreprocessorErrorType::WideStringInLineDirective
            | PreprocessorErrorType::EncodedStringInLineDirective(..)
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
            | PreprocessorErrorType::LanguageConstraint(..)
            | PreprocessorErrorType::EmbeddedResourceNotFound(..)
            | PreprocessorErrorType::EmbeddedResourceUnreadable { .. }
            | PreprocessorErrorType::EmbeddedResourceTooLarge(..)
            | PreprocessorErrorType::VaOptUnavailable
            | PreprocessorErrorType::MissingOpeningParenthesisAfterVaOpt
            | PreprocessorErrorType::NestedVaOpt
            | PreprocessorErrorType::UnterminatedVaOpt
            | PreprocessorErrorType::HashHashAtVaOptBoundary
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
            | PreprocessorErrorType::VaArgsOutsideVariadicMacro(policy)
            | PreprocessorErrorType::VaOptOutsideVariadicMacro(policy)
            | PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(_, policy)
            | PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(_, policy)
            | PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(_, policy) =>
                match policy {
                | ExtensionPolicy::Allow | ExtensionPolicy::Warn => ErrorSeverity::Warning,
                | ExtensionPolicy::Deny => ErrorSeverity::Error,
            },
            | PreprocessorErrorType::RedefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::UndefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::MissingWhitespaceAfterMacroName(..)
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
            | PreprocessorErrorType::LineDirectiveNumberZero(..)
            | PreprocessorErrorType::UnknownPragmaDirective
            | PreprocessorErrorType::ExtraTokensAfterPragmaOnce(..)
            | PreprocessorErrorType::ExtraTokensAfterPragmaOperator
            | PreprocessorErrorType::ExtraTokensAfterIncludeDirective
            | PreprocessorErrorType::ExtraTokensAfterConditionalDirective(_)
            | PreprocessorErrorType::ExtraTokensAfterIfdefDirective(_)
            | PreprocessorErrorType::ExtraTokensAfterIfndefDirective(_)
            | PreprocessorErrorType::WarningDirective(..)
            | PreprocessorErrorType::PragmaOnceInNonHeader
            | PreprocessorErrorType::IncludeNextInPrimarySource(..) => ErrorSeverity::Warning,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PreprocessorErrorType<'tu> {
    /// A violated lexical or directive constraint of a later standard, with
    /// its message.
    ///
    /// C99: diagnostics are required by §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    LanguageConstraint(&'tu str),
    /// An `#embed` resource that no search place holds.
    ///
    /// C23: §6.10.4.1 paragraph 3, p. 171; PDF p. 184.
    EmbeddedResourceNotFound(&'tu str),
    /// An `#embed` resource that was found but could not be opened, sized,
    /// or read, with the system's reason.
    ///
    /// C23: §6.10.4.1 paragraph 3, p. 171; PDF p. 184.
    EmbeddedResourceUnreadable {
        name:   &'tu str,
        reason: &'tu str,
    },
    /// An `#embed` resource larger than the host can address.
    ///
    /// C23: §6.10.4.1 paragraph 3, p. 171; PDF p. 184.
    EmbeddedResourceTooLarge(&'tu str),
    /// `__VA_OPT__` in a variadic macro's replacement list in a mode that
    /// lacks it. The definition is discarded.
    ///
    /// C23: §6.10.5.1 paragraph 1, p. 179; PDF p. 192.
    VaOptUnavailable,
    /// `__VA_OPT__` without its parenthesized replacement. The definition is
    /// discarded.
    ///
    /// C23: §6.10.5.1 paragraphs 1 and 3, p. 179; PDF p. 192.
    MissingOpeningParenthesisAfterVaOpt,
    /// `__VA_OPT__` within another's replacement. The definition is
    /// discarded.
    ///
    /// C23: §6.10.5.1 paragraph 3, p. 179; PDF p. 192.
    NestedVaOpt,
    /// A `__VA_OPT__` replacement without its closing parenthesis. The
    /// definition is discarded.
    ///
    /// C23: §6.10.5.1 paragraph 3, p. 179; PDF p. 192.
    UnterminatedVaOpt,
    /// `##` first or last in a `__VA_OPT__` replacement, which must form a
    /// valid replacement list. The definition is discarded.
    ///
    /// C23: §6.10.5.1 paragraph 3, p. 179; PDF p. 192, and §6.10.5.3
    /// paragraph 1, p. 181; PDF p. 194.
    HashHashAtVaOptBoundary,
    /// A `#warning` and its message, reported like `#error` but without
    /// failing translation. C23: §6.10.7 paragraph 1, p. 186; PDF p. 199;
    /// a GNU extension in earlier modes.
    WarningDirective(&'tu str),
    /// A pp-number with `0x` that is not a `hexadecimal-floating-constant`.
    ///
    /// C99: §6.4 paragraph 2, p. 49; PDF p. 61, and §6.4.4.2 paragraph 1,
    /// p. 57; PDF p. 69.
    InvalidHexadecimalFloatLiteral,
    /// A pp-number that is not a `decimal-floating-constant`.
    ///
    /// C99: §6.4 paragraph 2, p. 49; PDF p. 61, and §6.4.4.2 paragraphs 1-2,
    /// pp. 57-58; PDF pp. 69-70.
    InvalidDecimalFloatLiteral,
    /// A floating constant outside its type's range.
    ///
    /// C99: §6.4.4 paragraph 2, p. 54; PDF p. 66.
    FloatConstantOutOfRange {
        type_name: &'static str,
        error:     FloatRangeError,
    },
    // C99: a pp-number that is not an `integer-constant`, §6.4 paragraph 2,
    // p. 49; PDF p. 61, and §6.4.4.1 paragraph 1, pp. 54-55; PDF pp. 66-67.
    InvalidHexadecimalIntegerLiteral,
    /// A malformed binary constant. C99 has none; they are an extension.
    ///
    /// C99: extensions are permitted by §4 paragraph 6, p. 7; PDF p. 19.
    InvalidBinaryIntegerLiteral,
    InvalidOctalIntegerLiteral,
    InvalidDecimalIntegerLiteral,
    /// An integer constant beyond 64 bits, which no type can represent.
    ///
    /// C99: §6.4.4.1 paragraph 6, p. 56; PDF p. 68, and §6.4.4 paragraph 2,
    /// p. 54; PDF p. 66.
    IntegerLiteralOverflow,
    /// A decimal constant without `u` that no signed type in its list can
    /// represent. C99 gives it no type; it is given `to` instead, an
    /// implementation choice that matches Clang.
    ///
    /// C99: §6.4.4.1 paragraphs 5-6, pp. 55-56; PDF pp. 67-68.
    ForcedSignedToUnsignedConversion {
        to: UnsignedIntegerLiteralType,
    },
    // A constant that takes a later, wider type of its list: valid C99
    // (§6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68), warned about for
    // portability to a narrower `long`. Neither GCC nor Clang diagnoses it.
    ForcedUnsignedPromotion {
        from: UnsignedIntegerLiteralType,
        to:   UnsignedIntegerLiteralType,
    },
    ForcedSignedPromotion {
        from: SignedIntegerLiteralType,
        to:   SignedIntegerLiteralType,
    },
    /// A `#` that does not start a line.
    ///
    /// C99: §6.10 paragraph 2, p. 146; PDF p. 158.
    HashMustBeFirstCharacterOnLine,
    /// A `#` followed by neither a directive name nor a new-line: a
    /// `non-directive`, which C99 gives no meaning.
    ///
    /// C99: §6.10 paragraph 1, p. 146; PDF p. 158.
    HashMustBeFollowedByIdentifier,
    /// A `non-directive` whose name is not a directive name.
    ///
    /// C99: §6.10 paragraphs 1 and 3, pp. 146-147; PDF pp. 158-159.
    UnknownDirective,
    // C99: the `#if` expression is a `constant-expression`, §6.10.1
    // paragraph 1, p. 147; PDF p. 159, and §6.6 paragraph 1, p. 95; PDF
    // p. 107, whose operators take the operands that the syntax of §6.5,
    // pp. 67-94; PDF pp. 79-106, gives them.
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
    // C99: `conditional-expression`, §6.5.15 paragraph 1, p. 90; PDF p. 102.
    TernaryOperatorWithoutMhs,
    TernaryOperatorWithoutColon,
    TernaryOperatorWithoutRhs,
    ColonWithoutMatchingQuestionMark,
    /// An evaluated comma operator, an extension under the policy.
    ///
    /// C99: §6.6 paragraph 3, p. 95; PDF p. 107.
    CommaOperatorInPreprocessorExpression(ExtensionPolicy),
    BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(PreprocessorExpressionOperator),
    // C99: a zero divisor is undefined, §6.5.5 paragraph 5, p. 82; PDF p. 94.
    DivideByZero,
    ModuloByZero,
    // C99: a constant expression must stay in its type's range, §6.6
    // paragraph 4, p. 95; PDF p. 107, in `intmax_t` or `uintmax_t`, §6.10.1
    // paragraph 4, p. 148; PDF p. 160. Shift counts: §6.5.7 paragraph 3,
    // p. 84; PDF p. 96.
    UnaryMinusOverflow,
    BinaryPlusOverflow,
    BinaryMinusOverflow,
    MultiplyOverflow,
    DivideOverflow,
    ModuloOverflow,
    LeftShiftOverflow,
    RightShiftOverflow,
    /// C99: `( expression )`, §6.5.1 paragraph 1, p. 69; PDF p. 81.
    UnterminatedOpeningParenthesisInPreprocessorExpression,
    TildeInsteadOfBinaryOperatorInPreprocessorExpression,
    ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
    /// C99: a constant expression has no function call, §6.6 paragraph 3,
    /// p. 95; PDF p. 107.
    FunctionCallOperatorNotSupportedInPreprocessorExpression,
    DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
    // C99: the `#if` expression is an integer constant expression, §6.10.1
    // paragraph 1, p. 147; PDF p. 159, and §6.6 paragraph 6, p. 95; PDF
    // p. 107, with no addresses.
    AddressOfOperatorNotSupportedInPreprocessorExpression,
    DereferenceOperatorNotSupportedInPreprocessorExpression,
    ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(PreprocessorExpressionOperator),
    NumberInsteadOfBinaryOperatorInPreprocessorExpression,
    IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
    CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
    /// C99: §6.10.1 paragraph 1, p. 147; PDF p. 159.
    UnexpectedTokenInPreprocessorExpression(PreprocessorTokenType),
    /// A preprocessing token with no token's lexical form.
    ///
    /// C99: §6.4 paragraph 2, p. 49; PDF p. 61.
    UnexpectedTokenAtPhase7(PreprocessorTokenType),
    /// C99: floating constants appear in an integer constant expression only
    /// as cast operands, §6.6 paragraph 6, p. 95; PDF p. 107, and `#if` has
    /// no casts, §6.10.1 paragraph 1, p. 147; PDF p. 159.
    FloatInsteadOfIntegerInPreprocessorExpression,
    ExpectedBinaryOperatorInPreprocessorExpression,
    // C99: `defined identifier` or `defined ( identifier )`, §6.10.1
    // paragraph 1, p. 148; PDF p. 160.
    MissingOpeningParenthesisOrIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingClosingParenthesisInDefinedDirective(PreprocessorTokenType),
    // C99: `# if constant-expression` and `# elif constant-expression`,
    // §6.10 paragraph 1, p. 145; PDF p. 157.
    NoConditionInIfDirective,
    NoConditionInElifDirective,
    // C99: an `if-section` opens with an `if-group`, may hold one
    // `else-group` last, and closes with an `endif-line`, §6.10 paragraph 1,
    // p. 145; PDF p. 157.
    MoreIfDirectivesThanEndifDirectives,
    MoreEndifDirectivesThanIfDirectives,
    /// An `#elif`, or the named C23 `#elifdef` or `#elifndef`, outside a
    /// conditional.
    ElifDirectiveWithoutIfDirective(&'static str),
    ElseDirectiveWithoutIfDirective,
    ConditionalArmAfterElse(&'static str),
    /// C99: `# else new-line` and `# endif new-line`, §6.10 paragraph 1,
    /// p. 145; PDF p. 157, and footnote 147, p. 149; PDF p. 161.
    ExtraTokensAfterConditionalDirective(&'static str),
    // C99: `# ifdef identifier new-line`, §6.10 paragraph 1, p. 145; PDF
    // p. 157. The name is `ifdef` or `ifndef`, or C23's `elifdef` or
    // `elifndef`, §6.10.2 paragraph 16, p. 168; PDF p. 181.
    ExpectedIdentifierInIfdefDirective(&'static str, PreprocessorTokenType),
    ExpectedIdentifierInIfndefDirective(&'static str, PreprocessorTokenType),
    /// C99: `# define identifier`, §6.10 paragraph 1, p. 146; PDF p. 158.
    ExpectedIdentifierInDefineDirective(PreprocessorTokenType),
    /// C99: §6.10.8 paragraph 4, p. 161; PDF p. 173.
    RedefinitionOfBuiltInMacro(&'tu str),
    /// `#undef` of a predefined macro name, which stays defined.
    ///
    /// C99: §6.10.8 paragraph 4, p. 161; PDF p. 173. The rule is not a
    /// constraint, so this is a warning, like a redefinition.
    UndefinitionOfBuiltInMacro(&'tu str),
    /// An object-like macro whose replacement list follows its name with no
    /// whitespace between. The definition is kept, as GCC and Clang do.
    ///
    /// C99: §6.10.3 paragraph 3, p. 151; PDF p. 163.
    MissingWhitespaceAfterMacroName(&'tu str),
    /// `__VA_ARGS__` in a `#define` other than in the replacement list of a
    /// variadic macro. It is kept as an ordinary identifier, so this is a
    /// warning promoted to an error by pedantic-errors, as in GCC and Clang.
    ///
    /// C99: §6.10.3 paragraph 5, p. 151; PDF p. 163.
    VaArgsOutsideVariadicMacro(ExtensionPolicy),
    /// `__VA_OPT__` outside a variadic macro replacement list, retained
    /// as an identifier with a warning promoted to an error by pedantic-errors,
    /// like GCC and Clang.
    /// C23: §6.10.5p5, p. 178; PDF p. 191.
    VaOptOutsideVariadicMacro(ExtensionPolicy),
    /// A function-like macro that names one parameter twice. The definition
    /// is discarded.
    ///
    /// C99: §6.10.3 paragraph 6, p. 151; PDF p. 163.
    DuplicateMacroParameter(&'tu str),
    /// An identifier that `#if` replaces with 0, worth a warning.
    ///
    /// C99: §6.10.1 paragraph 4, p. 148; PDF p. 160.
    UndefinedIdentifierInPreprocessorExpression(&'tu str),
    /// C99: §6.10.2 paragraph 4, p. 150; PDF p. 162.
    ExpectedIncludeStringOrAngleBracketString(PreprocessorTokenType),
    /// A header name containing one of the sequences C99 §6.4.7p3 leaves
    /// undefined there: `'`, `\`, `"`, `//`, or `/*`.
    ///
    /// C99: §6.4.7 paragraph 3, pp. 64-65; PDF pp. 76-77.
    InvalidCharacterInHeaderName(&'static str),
    /// A header name without its closing `>` or `"`.
    ///
    /// C99: `header-name`, §6.4.7 paragraph 1, p. 64; PDF p. 76.
    UnterminatedHeaderName(char),
    /// A `\` in a `"…"` header name, accepted as a path character by the
    /// backslash extension.
    ///
    /// C99: undefined by §6.4.7 paragraph 3, pp. 64-65; PDF pp. 76-77; the
    /// extension is permitted by §4 paragraph 6, p. 7; PDF p. 19.
    BackslashInQuotedHeaderName(ExtensionPolicy),
    UnexpectedEndOfInput(&'static str),
    /// C99: §6.10.3 paragraph 4, p. 151; PDF p. 163.
    WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
        expected: usize,
        found:    usize,
    },
    /// A variadic macro invocation that supplies no argument for `...`.
    ///
    /// C99: §6.10.3 paragraph 4, p. 151; PDF p. 163, requires one; omitting
    /// it is an extension under the policy.
    MissingVariadicArgument(ExtensionPolicy),
    // C99: a `#include` must name a header or source file that can be
    // processed, §6.10.2 paragraph 1, p. 149; PDF p. 161.
    HeaderNotFound {
        name:             &'tu str,
        is_system_header: bool,
        searched:         &'tu [&'tu Path],
    },
    HeaderFileInaccessible(&'tu str),
    /// C99: the nesting limit is implementation-defined, §6.10.2 paragraph 6,
    /// p. 150; PDF p. 162, and at least 15, §5.2.4.1 paragraph 1, p. 21; PDF
    /// p. 33.
    IncludeNestingLimitExceeded(usize),
    // C99: `##` is a punctuator, §6.4.6 paragraph 1, p. 63; PDF p. 75, that
    // only a replacement list gives a meaning, §6.10.3.3, p. 154; PDF p. 166.
    HashHashUsedOutsideOfMacro,
    CannotUseHashHashAfterFunctionLikeMacroCall,
    /// A `\` followed by a character that begins no escape sequence.
    ///
    /// C99: §6.4.4.4 paragraph 1 and footnote 65, pp. 59-60; PDF pp. 71-72.
    InvalidEscapeSequence,
    /// A `#line` file name that is not a path; how names map to files is the
    /// implementation's.
    ///
    /// C99: §6.10.4 paragraph 4, p. 158; PDF p. 170.
    InvalidLineFilename,
    // C99: `escape-sequence`, §6.4.4.4 paragraph 1, p. 59; PDF p. 71.
    UnterminatedEscapeSequence,
    InvalidHexEscapeSequence,
    // C99: §6.4.4.4 paragraph 9, p. 61; PDF p. 73.
    HexEscapeSequenceTooLarge,
    OctalEscapeSequenceTooLarge,
    // C99: `universal-character-name` and its constraint, §6.4.3 paragraphs
    // 1-2, p. 53; PDF p. 65.
    InvalidSmallUnicodeEscapeSequence,
    SmallUnicodeEscapeSequenceTooShort,
    InvalidLargeUnicodeEscapeSequence,
    LargeUnicodeEscapeSequenceTooSmall,
    /// An empty character constant, which matches no `c-char-sequence`, or a
    /// wide one with several characters, whose implementation-defined value
    /// bcc does not assign.
    ///
    /// C99: §6.4.4.4 paragraphs 1 and 11, pp. 59-61; PDF pp. 71-73.
    MultiCharacterLiteralsUnsupported,
    /// A macro redefined with the other form. C99 §6.10.3 paragraph 2, p.
    /// 151; PDF p. 163 requires only a diagnostic; like GCC and Clang it is
    /// a warning, an error under `-pedantic-errors`, and the new definition
    /// replaces the old.
    RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(&'tu str, ExtensionPolicy),
    RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(&'tu str, ExtensionPolicy),
    // C99: the `# define` forms with `identifier-list` and `...`, §6.10
    // paragraph 1, p. 146; PDF p. 158.
    ExpectedIdentifierInMacroDefinition(PreprocessorTokenType),
    VariadicMacroMustBeLastParameter(&'tu str),
    ExpectedCommaOrClosingParenthesisInMacroDefinition(PreprocessorTokenType),
    /// A macro redefined with different parameters or replacement list; a
    /// warning, an error under `-pedantic-errors`, as for the variants above.
    ///
    /// C99: §6.10.3 paragraphs 1-2, p. 151; PDF p. 163.
    MacroRedefinedWithDifferentDefinition(&'tu str, ExtensionPolicy),
    // C99: `# undef identifier new-line`, §6.10 paragraph 1, p. 146; PDF
    // p. 158.
    ExpectedIdentifierInUndefDirective(PreprocessorTokenType),
    ExpectedNewlineAfterUndefDirective(PreprocessorTokenType),
    // C99: §6.10.3.2 paragraph 1, p. 153; PDF p. 165.
    HashOperatorMustBeFollowedByAMacroArgument(PreprocessorTokenType),
    IdentifierNotMacroArgumentAfterHashOperator(&'tu str),
    // C99: §6.10.3.3 paragraph 1, p. 154; PDF p. 166.
    MissingRightHandSideOfHashHashOperator,
    MissingLeftHandSideOfHashHashOperator,
    /// A paste whose result is not one preprocessing token, which is
    /// undefined.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166.
    TokenMergingError(&'tu str, &'tu str),
    // C99: `# line digit-sequence "s-char-sequence(opt)" new-line`, after
    // macro replacement, §6.10.4 paragraphs 3-5, p. 158; PDF p. 170.
    MissingNumberInLineDirective(PreprocessorTokenType),
    MissingNewlineAfterLineDirective(PreprocessorTokenType),
    LineDirectiveIsNotASimpleDigitSequence,
    LineDirectiveNumberTooLarge(&'tu str),
    /// A `#line` number that specifies zero, which is undefined; the line
    /// number is ignored.
    ///
    /// C99: §6.10.4 paragraph 3, p. 158; PDF p. 170.
    LineDirectiveNumberZero(&'tu str),
    /// A wide string literal as the `#line` file name; the name is ignored.
    ///
    /// C99: §6.10.4 paragraph 1, p. 158; PDF p. 170.
    WideStringInLineDirective,
    /// A `u`, `U` or `u8` string literal as the `#line` file name; the name
    /// is ignored. The field is the encoding prefix.
    ///
    /// C11: §6.10.4 paragraph 1, p. 173; PDF p. 191.
    EncodedStringInLineDirective(&'static str),
    // C99: `_Pragma ( string-literal )`, §6.10.9 paragraph 1, p. 161; PDF
    // p. 173.
    MissingOpeningParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingClosingParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingStringLiteralInPragmaOperator(PreprocessorTokenType),
    /// A pragma bcc does not recognize, which is ignored.
    ///
    /// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171.
    UnknownPragmaDirective,
    /// C99: §6.10.6 paragraph 2, p. 159; PDF p. 171.
    UnknownPragmaSTDCArgument(&'tu str),
    // C99: `#pragma once` is an implementation-defined pragma, §6.10.6
    // paragraph 1, p. 159; PDF p. 171.
    ExtraTokensAfterPragmaOnce(PreprocessorTokenType),
    /// C99: §6.10.9 paragraph 1, p. 161; PDF p. 173.
    ExtraTokensAfterPragmaOperator,
    /// C99: after replacement the directive must match `# include
    /// <h-char-sequence>` or `# include "q-char-sequence"`, §6.10.2
    /// paragraph 4, p. 150; PDF p. 162.
    ExtraTokensAfterIncludeDirective,
    // C99: `# ifdef identifier new-line`, §6.10 paragraph 1, p. 145; PDF
    // p. 157, named as for `ExpectedIdentifierInIfdefDirective`.
    ExtraTokensAfterIfdefDirective(&'static str),
    ExtraTokensAfterIfndefDirective(&'static str),
    // C99: `#pragma STDC` takes a pragma name and an `on-off-switch`,
    // §6.10.6 paragraph 2, p. 159; PDF p. 171.
    STDCPragmaDirectiveWithoutArgument,
    STDCPragmaDirectiveWithoutOnOffSwitch,
    MissingOnOffSwitchInSTDCPragma(&'tu str),
    PragmaOnceInNonHeader,
    /// `#include_next` or `__has_include_next`, named by the payload, in the
    /// primary source file, where there is no entry to continue after; the
    /// lookup searches as `#include` would, as GCC's and Clang's do. The
    /// directive is an
    /// extension (C99 §4p6, p. 7; PDF p. 19) over implementation-defined
    /// header places, §6.10.2 paragraphs 2-3, pp. 149-150; PDF pp. 161-162.
    IncludeNextInPrimarySource(&'static str),
    /// C99: §6.10.5 paragraph 1, p. 159; PDF p. 171; translation fails,
    /// §4 paragraph 4, p. 7; PDF p. 19.
    ErrorDirective(&'tu str),
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

impl PreprocessorErrorType<'_> {
    /// Describes the error; `spelling` is the source text it points at, used
    /// to name what was actually written.
    #[expect(
        clippy::too_many_lines,
        reason = "One exhaustive table keeps every preprocessor message reviewable in one place."
    )]
    fn explain_in<'d>(&self, arena: &'d Bump, spelling: Option<&str>) -> Explanation<'d> {
        let new = |message: &'d str| Explanation::new(arena, message);
        let titled = |title: &'static str| match spelling {
            | Some(spelling) => format_in!(arena, "{title} {}", quote_spelling(spelling)),
            | None => title,
        };
        let missing_operand = |operator: &str, side: &str| {
            new(format_in!(
                arena,
                "expected an expression {side} `{operator}`"
            ))
            .label(format_in!(arena, "`{operator}` needs an operand here"))
        };
        let overflow = |operation: &str| {
            new(format_in!(
                arena,
                "{operation} overflows in `#if` expression"
            ))
            .label("the result is out of range")
            .note(
                "C99 §6.10.1p4: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 \
                 requires constant expressions to stay in range",
            )
        };
        let instead_of_operator = |found: &dyn Display| {
            new(format_in!(arena, "expected an operator, found {found}"))
                .label("expected a binary operator before this")
                .note("operands in an `#if` expression must be joined by operators")
        };
        match self {
            | Self::InvalidHexadecimalFloatLiteral =>
                new(titled("invalid hexadecimal floating constant"))
                    .label("not a valid hexadecimal floating constant")
                    .note(
                        "C99 §6.4.4.2: a hexadecimal floating constant needs hexadecimal digits \
                         and a binary exponent, as in `0x1.8p3`",
                    ),
            | Self::InvalidDecimalFloatLiteral => new(titled("invalid floating constant"))
                .label("not a valid floating constant")
                .note(
                    "C99 §6.4.4.2: a floating constant is digits with a `.` or an exponent and an \
                     optional `f`, `F`, `l`, or `L` suffix, as in `1.5`, `2e10`, or `.5f`",
                ),
            | Self::FloatConstantOutOfRange { type_name, error } => match error {
                | FloatRangeError::Overflow => new(format_in!(
                    arena,
                    "floating constant is too large for `{type_name}`"
                ))
                .label("this value becomes infinity")
                .note("C99 §6.4.4p2: the value of a constant must be representable in its type"),
                | FloatRangeError::Underflow => new(format_in!(
                    arena,
                    "floating constant is too small for `{type_name}`"
                ))
                .label("this nonzero value becomes zero")
                .note("C99 §6.4.4p2: the value of a constant must be representable in its type"),
            },
            | Self::InvalidHexadecimalIntegerLiteral =>
                new(titled("invalid hexadecimal integer constant"))
                    .label("not a valid hexadecimal constant")
                    .note(
                        "C99 §6.4.4.1: hexadecimal digits are `0`-`9`, `a`-`f`, and `A`-`F`, \
                         optionally followed by an integer suffix such as `u`, `l`, or `ull`",
                    ),
            | Self::InvalidBinaryIntegerLiteral => new(titled("invalid binary integer constant"))
                .label("not a valid binary constant")
                .note("binary digits are `0` and `1`"),
            | Self::InvalidOctalIntegerLiteral => new(titled("invalid octal integer constant"))
                .label("not a valid octal constant")
                .note(
                    "C99 §6.4.4.1: a constant starting with `0` is octal, and octal digits are \
                     `0`-`7`",
                ),
            | Self::InvalidDecimalIntegerLiteral => new(titled("invalid integer constant"))
                .label("not a valid integer constant")
                .note(
                    "C99 §6.4.4.1: an integer constant is digits followed by an optional suffix \
                     such as `u`, `l`, `ll`, or `ull`",
                ),
            | Self::IntegerLiteralOverflow => new("integer constant is too large")
                .label("does not fit in `unsigned long long`")
                .note("the largest integer type, `unsigned long long`, has 64 bits"),
            | Self::ForcedSignedToUnsignedConversion { to } =>
                new("integer constant is so large that it is unsigned")
                    .label(format_in!(
                        arena,
                        "this constant has type `{}`",
                        to.spelling()
                    ))
                    .note(
                        "C99 §6.4.4.1p5-6: no signed type in a decimal constant's list can \
                         represent this value, so C99 gives it no type",
                    )
                    .help("add a `u` suffix to make the unsigned type explicit"),
            | Self::ForcedUnsignedPromotion { from, to } => new(format_in!(
                arena,
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format_in!(
                arena,
                "this constant has type `{}`",
                to.spelling()
            )),
            | Self::ForcedSignedPromotion { from, to } => new(format_in!(
                arena,
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format_in!(
                arena,
                "this constant has type `{}`",
                to.spelling()
            )),
            | Self::HashMustBeFirstCharacterOnLine => new("stray `#` in program")
                .label("a directive must start a line")
                .note(
                    "C99 §6.10p2: `#` begins a directive only as the first token on a line; \
                     elsewhere it is valid only inside a function-like macro definition",
                ),
            | Self::HashMustBeFollowedByIdentifier => new(match spelling {
                | Some(spelling) => format_in!(
                    arena,
                    "expected a directive name after `#`, found {}",
                    quote_spelling(spelling)
                ),
                | None => "expected a directive name after `#`, found a token",
            })
            .label("expected a directive name")
            .note(DIRECTIVE_LIST_NOTE),
            | Self::UnknownDirective => {
                let name = spelling.unwrap_or_default();
                let explanation = new(format_in!(
                    arena,
                    "unknown preprocessing directive `#{name}`"
                ))
                .label("not a C99 directive")
                .note(DIRECTIVE_LIST_NOTE);
                match closest_match(arena, name, &DIRECTIVE_NAMES) {
                    | Some(suggestion) =>
                        explanation.help(format_in!(arena, "did you mean `#{suggestion}`?")),
                    | None => explanation,
                }
            },
            | Self::EmptyParenthesesInPreprocessorExpression =>
                new("expected an expression inside `()`")
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
                new("expected `:` after `?` in preprocessor expression")
                    .label("this conditional operator has no colon"),
            | Self::TernaryOperatorWithoutMhs => missing_operand("?", "after"),
            | Self::TernaryOperatorWithoutRhs => missing_operand(":", "after"),
            | Self::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "after"),
            | Self::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "before"),
            | Self::ColonWithoutMatchingQuestionMark => new("`:` without a matching `?`")
                .label("no `?` precedes this `:` in the same parentheses"),
            | Self::CommaOperatorInPreprocessorExpression(_) =>
                new("comma operator in `#if` expression")
                    .label("evaluated comma operator")
                    .note(
                        "C99 §6.6p3: constant expressions shall not contain comma operators, \
                         except within an operand that is not evaluated",
                    ),
            | Self::DivideByZero => new("division by zero in `#if` expression")
                .label("the divisor is zero")
                .note("C99 §6.5.5p5: the result of `/` by zero is undefined"),
            | Self::ModuloByZero => new("remainder by zero in `#if` expression")
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
                new("unclosed `(` in `#if` expression").label("expected `)`"),
            | Self::TildeInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&"`~`"),
            | Self::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&"`!`"),
            | Self::NumberInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Number.found(spelling)),
            | Self::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Identifier.found(spelling)),
            | Self::CharacterInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Character.found(spelling)),
            | Self::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&"`defined`"),
            | Self::ExpectedBinaryOperatorInPreprocessorExpression =>
                new("expected an operator between the operands of `#if`")
                    .label("the expression ends with an operand still unjoined")
                    .note(IF_EXPRESSION_NOTE),
            | Self::AddressOfOperatorNotSupportedInPreprocessorExpression =>
                new("`&` cannot take an address in `#if` expression")
                    .label("addresses do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::DereferenceOperatorNotSupportedInPreprocessorExpression =>
                new("`*` cannot dereference in `#if` expression")
                    .label("pointers do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::FunctionCallOperatorNotSupportedInPreprocessorExpression =>
                new("function call in `#if` expression")
                    .label("functions cannot be called during preprocessing")
                    .note(
                        "C99 §6.10.1p4: identifiers that are not macros evaluate to `0`, so this \
                         looks like a call of an undefined function-like macro",
                    )
                    .help("define the function-like macro before this directive"),
            | Self::FloatInsteadOfIntegerInPreprocessorExpression =>
                new("floating constant in `#if` expression")
                    .label("not an integer")
                    .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenInPreprocessorExpression(kind) => new(format_in!(
                arena,
                "unexpected {} in `#if` expression",
                kind.found(spelling)
            ))
            .label("not valid in an integer constant expression")
            .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenAtPhase7(kind) => new(format_in!(
                arena,
                "stray {} in program",
                kind.found(spelling)
            ))
            .label("only meaningful inside a preprocessing directive"),
            | Self::MissingOpeningParenthesisOrIdentifierInDefinedDirective(kind) =>
                new(format_in!(
                    arena,
                    "expected a macro name after `defined`, found {}",
                    kind.found(spelling)
                ))
                .label("expected a macro name")
                .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingIdentifierInDefinedDirective(kind) => new(format_in!(
                arena,
                "expected a macro name inside `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingClosingParenthesisInDefinedDirective(kind) => new(format_in!(
                arena,
                "expected `)` to close `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::NoConditionInIfDirective => new("`#if` with no condition")
                .label("expected an expression")
                .help("write the condition to test, as in `#if VERSION >= 2`"),
            | Self::NoConditionInElifDirective => new("`#elif` with no condition")
                .label("expected an expression")
                .help("write the condition to test, or use `#else`"),
            | Self::MoreIfDirectivesThanEndifDirectives => new(format_in!(
                arena,
                "unterminated `#{}`",
                spelling.unwrap_or("if")
            ))
            .label("this conditional has no matching `#endif`")
            .help("add `#endif` where the conditional section should end"),
            | Self::MoreEndifDirectivesThanIfDirectives =>
                new("`#endif` without `#if`").label("no conditional directive is open here"),
            | Self::ElifDirectiveWithoutIfDirective(name) =>
                new(format_in!(arena, "`#{name}` without `#if`"))
                    .label("no conditional directive is open here"),
            | Self::ConditionalArmAfterElse(name) =>
                new(format_in!(arena, "`#{name}` after `#else`"))
                    .label("the final arm of this conditional has already begun")
                    .note("C99 §6.10.1: a conditional group permits one final #else arm"),
            | Self::ExtraTokensAfterConditionalDirective(name) =>
                new(format_in!(arena, "extra tokens after `#{name}`"))
                    .label("expected the end of the directive"),
            | Self::ElseDirectiveWithoutIfDirective =>
                new("`#else` without `#if`").label("no conditional directive is open here"),
            | Self::ExpectedIdentifierInIfdefDirective(name, kind)
            | Self::ExpectedIdentifierInIfndefDirective(name, kind) => new(format_in!(
                arena,
                "expected a macro name after `#{name}`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedIdentifierInDefineDirective(kind) => new(format_in!(
                arena,
                "expected a macro name after `#define`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.3: a macro name is an identifier"),
            | Self::RedefinitionOfBuiltInMacro(name)
                if super::language_features::overridable_gnu_builtin(name) =>
                new(format_in!(arena, "redefining builtin macro `{name}`"))
                    .label("replacement overrides the implementation definition"),
            | Self::RedefinitionOfBuiltInMacro(name) => new(format_in!(
                arena,
                "cannot redefine predefined macro `{name}`"
            ))
            .label("predefined by the implementation")
            .note("C99 §6.10.8p4: predefined macro names shall not be redefined"),
            | Self::UndefinitionOfBuiltInMacro(name)
                if super::language_features::overridable_gnu_builtin(name) =>
                new(format_in!(arena, "undefining builtin macro `{name}`"))
                    .label("implementation definition removed"),
            | Self::UndefinitionOfBuiltInMacro(name) => new(format_in!(
                arena,
                "cannot undefine predefined macro `{name}`"
            ))
            .label("predefined by the implementation")
            .note("C99 §6.10.8p4: predefined macro names shall not be undefined"),
            | Self::MissingWhitespaceAfterMacroName(name) => new(format_in!(
                arena,
                "missing whitespace after the macro name `{name}`"
            ))
            .label("the replacement list starts here")
            .note(
                "C99 §6.10.3p3: an object-like macro's name and replacement list are separated by \
                 whitespace",
            ),
            | Self::VaArgsOutsideVariadicMacro(_) =>
                new("`__VA_ARGS__` can only appear in the replacement list of a variadic macro")
                    .label("not in a variadic macro's replacement list")
                    .note(
                        "C99 §6.10.3p5: `__VA_ARGS__` is reserved for macros whose parameters end \
                         in `...`",
                    ),
            | Self::VaOptOutsideVariadicMacro(_) =>
                new("`__VA_OPT__` can only appear in the replacement list of a variadic macro")
                    .label("not in a variadic macro's replacement list")
                    .note(
                        "C23 §6.10.5p5: `__VA_OPT__` is reserved for macros whose parameters end \
                         in `...`",
                    ),
            | Self::DuplicateMacroParameter(name) =>
                new(format_in!(arena, "duplicate macro parameter `{name}`"))
                    .label("already a parameter of this macro")
                    .note("C99 §6.10.3p6: parameter names must be unique; the macro is not defined"),
            | Self::UndefinedIdentifierInPreprocessorExpression(name) => new(format_in!(
                arena,
                "`{name}` is not defined; it evaluates to 0"
            ))
            .label("not a macro")
            .note(
                "C99 §6.10.1p4: identifiers that are not macro names are replaced with `0` in \
                 `#if`",
            )
            .help(format_in!(
                arena,
                "use `defined({name})` to test whether it is defined"
            )),
            | Self::ExpectedIncludeStringOrAngleBracketString(kind) => new(format_in!(
                arena,
                "expected a header name after `#include`, found {}",
                kind.found(spelling)
            ))
            .label("expected `\"file.h\"` or `<file.h>`")
            .note(
                "C99 §6.10.2: `#include` takes `\"name\"`, `<name>`, or macros that expand to one \
                 of them",
            ),
            | Self::InvalidCharacterInHeaderName(sequence) => new(format_in!(
                arena,
                "header name in `#include` cannot contain `{sequence}`"
            ))
            .label("not allowed in a header name")
            .note(
                "C99 §6.4.7p3: the behavior is undefined if `'`, `\\`, `//`, or `/*` occurs in a \
                 header name, or `\"` occurs between `<` and `>`",
            ),
            | Self::UnterminatedHeaderName(delimiter) => new(format_in!(
                arena,
                "header name in `#include` is missing its closing `{delimiter}`"
            ))
            .label(format_in!(arena, "expected `{delimiter}` here"))
            .note("C99 §6.4.7p1: a header name is `<h-char-sequence>` or `\"q-char-sequence\"`")
            .help(format_in!(
                arena,
                "close the header name with `{delimiter}` on this line"
            )),
            | Self::BackslashInQuotedHeaderName(_) =>
                new("a backslash in a header name is an extension")
                    .label("read as a path character")
                    .note("C99 §6.4.7p3: the behavior is undefined if `\\` occurs in a header name"),
            | Self::UnexpectedEndOfInput(activity) => new(format_in!(
                arena,
                "unexpected end of file while {}",
                activity.trim_end_matches('.')
            ))
            .label("the file ends here"),
            | Self::WrongNumberOfArgumentsInFunctionLikeMacroInvocation { expected, found } =>
                new(format_in!(
                    arena,
                    "this macro takes {} but {} {} supplied",
                    count_of(*expected, "argument"),
                    count_of(*found, "argument"),
                    if *found == 1 { "was" } else { "were" }
                ))
                .label(format_in!(
                    arena,
                    "expected {}",
                    count_of(*expected, "argument")
                ))
                .note(
                    "C99 §6.10.3p4: an invocation must supply one argument per parameter, plus \
                     any for `...`",
                ),
            | Self::MissingVariadicArgument(_) =>
                new("this invocation supplies no argument for `...`")
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
                let mut explanation = new(format_in!(arena, "cannot find header `{name}`"))
                    .label("not found in any search directory");
                if searched.is_empty() {
                    explanation =
                        explanation.note("the path is absolute; no directory was searched");
                } else {
                    let mut note = ArenaString::new_in(arena);
                    note.push_str("searched these directories:");
                    for directory in *searched {
                        if directory.as_os_str().is_empty() {
                            note.push_str("\n  . (the working directory)");
                        } else {
                            let _ = write!(note, "\n  {}", directory.display());
                        }
                    }
                    explanation = explanation.note(note.into_str());
                }
                explanation.help(if *is_system_header {
                    "add the directory containing it with `--isystem <dir>`"
                } else {
                    "add the directory containing it with `--iquote <dir>` or `--isystem <dir>`"
                })
            },
            | Self::HeaderFileInaccessible(error) =>
                new(format_in!(arena, "cannot read included file: {error}")).label("included here"),
            | Self::IncludeNestingLimitExceeded(limit) => new(format_in!(
                arena,
                "include nesting exceeds the maximum of {limit}"
            ))
            .label("this include exceeds the implementation limit")
            .note("C99 §5.2.4.1: at least 15 nested included files are supported"),
            | Self::HashHashUsedOutsideOfMacro => new("`##` outside a macro definition")
                .label("token pasting only happens in replacement lists")
                .note("C99 §6.10.3.3: `##` is an operator of macro replacement lists"),
            | Self::CannotUseHashHashAfterFunctionLikeMacroCall =>
                new("`##` cannot follow a function-like macro invocation")
                    .label("pasting onto an invocation is not supported"),
            | Self::InvalidLineFilename => new("line filename is not representable as a path")
                .label("use a UTF-8 filename without NUL characters"),
            | Self::InvalidEscapeSequence => new("unknown escape sequence")
                .label("contains an escape C99 does not define")
                .note(ESCAPE_LIST_NOTE),
            | Self::UnterminatedEscapeSequence => new("incomplete escape sequence")
                .label("`\\` ends the literal")
                .help("write `\\\\` for a backslash character"),
            | Self::InvalidHexEscapeSequence =>
                new("hexadecimal escape sequence is not a valid character")
                    .label("contains an invalid `\\x` escape"),
            | Self::HexEscapeSequenceTooLarge => new("hexadecimal escape sequence is out of range")
                .label("contains an oversized `\\x` escape"),
            | Self::OctalEscapeSequenceTooLarge => new("octal escape sequence is out of range")
                .label("contains an oversized octal escape"),
            | Self::InvalidSmallUnicodeEscapeSequence =>
                new("`\\u` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note(
                        "C99 §6.4.3: values below U+00A0 are forbidden except U+0024, U+0040 and \
                         U+0060; surrogates are forbidden",
                    ),
            | Self::SmallUnicodeEscapeSequenceTooShort =>
                new("`\\u` escape needs exactly four hexadecimal digits")
                    .label("contains a short `\\u` escape"),
            | Self::InvalidLargeUnicodeEscapeSequence =>
                new("`\\U` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note(
                        "C99 §6.4.3: values below U+00A0 are forbidden except U+0024, U+0040 and \
                         U+0060; surrogates are forbidden",
                    ),
            | Self::LargeUnicodeEscapeSequenceTooSmall =>
                new("`\\U` escape needs exactly eight hexadecimal digits")
                    .label("contains a short `\\U` escape"),
            | Self::MultiCharacterLiteralsUnsupported => {
                if spelling.is_some_and(|spelling| spelling.ends_with("''")) {
                    new("empty character constant")
                        .label("contains no character")
                        .note("C99 §6.4.4.4: a character constant contains at least one character")
                } else {
                    new("wide character constant with more than one character")
                        .label("its value is implementation-defined")
                        .note(
                            "C99 §6.4.4.4p11: bcc does not assign a value to multi-character wide \
                             constants",
                        )
                }
            },
            | Self::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(name, _) => new(format_in!(
                arena,
                "function-like macro `{name}` redefined as an object-like macro"
            ))
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format_in!(
                arena,
                "add `#undef {name}` before this definition"
            )),
            | Self::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(name, _) => new(format_in!(
                arena,
                "object-like macro `{name}` redefined as a function-like macro"
            ))
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format_in!(
                arena,
                "add `#undef {name}` before this definition"
            )),
            | Self::ExpectedIdentifierInMacroDefinition(kind) => new(format_in!(
                arena,
                "expected a parameter name, found {}",
                kind.found(spelling)
            ))
            .label("expected an identifier or `...`"),
            | Self::VariadicMacroMustBeLastParameter(name) => new(format_in!(
                arena,
                "`...` must be the last parameter of `{name}`"
            ))
            .label("parameter after `...`")
            .note("C99 §6.10p1: the `# define` grammar puts `...` last in the parameter list"),
            | Self::ExpectedCommaOrClosingParenthesisInMacroDefinition(kind) => new(format_in!(
                arena,
                "expected `,` or `)` in macro parameter list, found {}",
                kind.found(spelling)
            ))
            .label("expected `,` or `)`"),
            | Self::MacroRedefinedWithDifferentDefinition(name, _) =>
                new(format_in!(arena, "macro `{name}` redefined differently"))
                    .label("this definition differs from the previous one")
                    .note(
                        "C99 §6.10.3p2: a redefinition must have the same parameters and an \
                         identical replacement list",
                    )
                    .help(format_in!(
                        arena,
                        "add `#undef {name}` before this definition"
                    )),
            | Self::ExpectedIdentifierInUndefDirective(kind) => new(format_in!(
                arena,
                "expected a macro name after `#undef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedNewlineAfterUndefDirective(kind) => new(format_in!(
                arena,
                "unexpected {} after the macro name in `#undef`",
                kind.found(spelling)
            ))
            .label("`#undef` takes one name"),
            | Self::HashOperatorMustBeFollowedByAMacroArgument(kind) => new(format_in!(
                arena,
                "expected a macro parameter after `#`, found {}",
                kind.found(spelling)
            ))
            .label("expected a parameter name")
            .note("C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a parameter"),
            | Self::IdentifierNotMacroArgumentAfterHashOperator(name) => new(format_in!(
                arena,
                "`{name}` is not a parameter of this macro"
            ))
            .label("`#` can only stringify a parameter")
            .note("C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a parameter"),
            | Self::MissingRightHandSideOfHashHashOperator =>
                new("`##` cannot end a replacement list")
                    .label("nothing follows this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::MissingLeftHandSideOfHashHashOperator =>
                new("`##` cannot start a replacement list")
                    .label("nothing precedes this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::TokenMergingError(lhs, rhs) => new(format_in!(
                arena,
                "pasting `{lhs}` and `{rhs}` does not give a valid token"
            ))
            .label("invalid token paste")
            .note("C99 §6.10.3.3p3: the result of `##` must be a single valid preprocessing token"),
            | Self::MissingNumberInLineDirective(kind) => new(format_in!(
                arena,
                "expected a line number after `#line`, found {}",
                kind.found(spelling)
            ))
            .label("expected a line number"),
            | Self::MissingNewlineAfterLineDirective(kind) => new(format_in!(
                arena,
                "unexpected {} after `#line`",
                kind.found(spelling)
            ))
            .label("`#line` takes a number and an optional file name"),
            | Self::LineDirectiveIsNotASimpleDigitSequence =>
                new("`#line` needs a plain decimal line number")
                    .label("not a digit sequence")
                    .note("C99 §6.10.4p3: the line number is a digit sequence, not any constant"),
            | Self::LineDirectiveNumberTooLarge(number) =>
                new(format_in!(arena, "line number {number} is out of range"))
                    .label("too large")
                    .note("C99 §6.10.4p3: the line number must be at most 2147483647"),
            | Self::LineDirectiveNumberZero(number) =>
                new(format_in!(arena, "line number {number} is out of range"))
                    .label("zero")
                    .note("C99 §6.10.4p3: the line number must be at least 1"),
            | Self::WideStringInLineDirective => new("`#line` file name is a wide string literal")
                .label("not a character string literal")
                .note("C99 §6.10.4p1: the file name of `#line` shall be a character string literal")
                .help("remove the `L` prefix"),
            | Self::EncodedStringInLineDirective(prefix) => new(format_in!(
                arena,
                "`#line` file name is a `{prefix}` string literal"
            ))
            .label("not a character string literal")
            .note("C11 §6.10.4p1: the file name of `#line` shall be a character string literal")
            .help(format_in!(arena, "remove the `{prefix}` prefix")),
            | Self::MissingOpeningParenthesisInPragmaOperator(kind) => new(format_in!(
                arena,
                "expected `(` after `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected `(`")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::MissingClosingParenthesisInPragmaOperator(kind) => new(format_in!(
                arena,
                "expected `)` to close `_Pragma(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::MissingStringLiteralInPragmaOperator(kind) => new(format_in!(
                arena,
                "expected a string literal in `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected a string literal")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::UnknownPragmaDirective => new("unknown pragma ignored")
                .label("not recognized")
                .note("bcc recognizes `#pragma once` and the `#pragma STDC` pragmas"),
            | Self::UnknownPragmaSTDCArgument(argument) =>
                new(format_in!(arena, "unknown `STDC` pragma `{argument}`"))
                    .label("not a standard pragma")
                    .note(
                        "C99 §6.10.6: the standard pragmas are `FP_CONTRACT`, `FENV_ACCESS`, and \
                         `CX_LIMITED_RANGE`",
                    ),
            | Self::ExtraTokensAfterPragmaOnce(kind) => new(format_in!(
                arena,
                "unexpected {} after `#pragma once`",
                kind.found(spelling)
            ))
            .label("`#pragma once` takes no arguments"),
            | Self::ExtraTokensAfterPragmaOperator =>
                new("extra tokens after the pragma in `_Pragma`").label("not part of the pragma"),
            | Self::ExtraTokensAfterIncludeDirective =>
                new("extra tokens at end of `#include` directive").label("ignored"),
            | Self::ExtraTokensAfterIfdefDirective(name)
            | Self::ExtraTokensAfterIfndefDirective(name) => new(format_in!(
                arena,
                "extra tokens at end of `#{name}` directive"
            ))
            .label("ignored"),
            | Self::STDCPragmaDirectiveWithoutArgument =>
                new("expected a pragma name after `#pragma STDC`")
                    .label("expected `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE`"),
            | Self::STDCPragmaDirectiveWithoutOnOffSwitch =>
                new("expected `ON`, `OFF`, or `DEFAULT` in `#pragma STDC`")
                    .label("the pragma ends here")
                    .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::MissingOnOffSwitchInSTDCPragma(argument) => new(format_in!(
                arena,
                "expected `ON`, `OFF`, or `DEFAULT` after `{argument}`"
            ))
            .label("expected an on-off switch")
            .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::PragmaOnceInNonHeader =>
                new("`#pragma once` in main file").label("only affects files that are included"),
            | Self::IncludeNextInPrimarySource(spelling) =>
                new(format_in!(arena, "`{spelling}` in the primary source file"))
                    .label("searches from the start of the include path")
                    .note(
                        "it continues after the search directory that provided the current \
                         header, and the primary source file came from none",
                    )
                    .help("use `#include` or `__has_include` outside headers"),
            | Self::LanguageConstraint(message) => new(format_in!(arena, "{message}")),
            | Self::EmbeddedResourceNotFound(name) =>
                new(format_in!(arena, "cannot find embedded resource `{name}`"))
                    .label("not found in any search directory")
                    .note("C23 §6.10.4.1p3: `#embed` must identify a resource it can process"),
            | Self::EmbeddedResourceUnreadable { name, reason } => new(format_in!(
                arena,
                "cannot read embedded resource `{name}`: {reason}"
            ))
            .label("embedded here")
            .note("C23 §6.10.4.1p3: `#embed` must identify a resource it can process"),
            | Self::EmbeddedResourceTooLarge(name) =>
                new(format_in!(arena, "embedded resource `{name}` is too large"))
                    .label("its size exceeds the host's address space")
                    .help("add a `limit` parameter to embed a prefix of the resource"),
            | Self::VaOptUnavailable => new("`__VA_OPT__` is not available in this mode")
                .label("the macro is not defined")
                .help("select C23 or a GNU mode for `__VA_OPT__`"),
            | Self::MissingOpeningParenthesisAfterVaOpt => new("expected `(` after `__VA_OPT__`")
                .label("the macro is not defined")
                .note("C23 §6.10.5.1p1: the form is `__VA_OPT__ ( pp-tokens(opt) )`"),
            | Self::NestedVaOpt => new("`__VA_OPT__` cannot be nested")
                .label("inside another `__VA_OPT__` replacement")
                .note(
                    "C23 §6.10.5.1p3: the pp-tokens of a `__VA_OPT__` replacement shall not \
                     contain `__VA_OPT__`; the macro is not defined",
                ),
            | Self::UnterminatedVaOpt => new("unterminated `__VA_OPT__` replacement")
                .label("no matching `)` before the end of the definition")
                .note("C23 §6.10.5.1p3: the macro is not defined"),
            | Self::HashHashAtVaOptBoundary =>
                new("`##` cannot begin or end a `__VA_OPT__` replacement")
                    .label("the macro is not defined")
                    .note(
                        "C23 §6.10.5.1p3: the replacement must form a valid replacement list, \
                         which `##` cannot begin or end (§6.10.5.3p1)",
                    ),
            | Self::WarningDirective(message) => {
                let message = message.trim();
                new(if message.is_empty() {
                    "#warning"
                } else {
                    format_in!(arena, "#warning {message}")
                })
                .label("`#warning` directive")
            },
            | Self::ErrorDirective(message) => {
                let message = message.trim();
                new(if message.is_empty() {
                    "#error"
                } else {
                    format_in!(arena, "#error {message}")
                })
                .label("`#error` directive")
            },
        }
    }
}

impl Display for PreprocessorErrorType<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let arena = Bump::new();
        f.write_str(self.explain_in(&arena, None).message)
    }
}
