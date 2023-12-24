use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    sync::Arc,
};

use chrono::Local;
use scopeguard::guard;
use smallstr::SmallString;
use thiserror::Error;

use crate::{
    float_parsing::{
        string_to_double,
        string_to_float,
        string_to_long_double,
        LongDouble,
        ParseFloatError,
    },
    util::string_cache::{
        Id as StringCacheId,
        StringCache,
    },
    HashMap,
};

trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or index is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;

    /// Returns the character at the given index but in lowercase,
    /// See [`StrExt::char_at`] for more information.
    fn char_at_case_insensitive(&self, index: usize) -> Option<char> {
        self.char_at(index).map(|c| c.to_ascii_lowercase())
    }
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

use super::{
    phase_0_newline_tracking::NewlineTracking,
    phase_1_map_character_sets::{
        MapCharacterSets,
        MapCharacterSetsError,
        SavePoint as MapCharacterSetsSavePoint,
    },
    phase_2_remove_escaped_newlines::{
        RemoveEscapedNewlines,
        RemoveEscapedNewlinesError,
        SavePoint as RemoveEscapedNewlinesSavePoint,
    },
    phase_3_preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
        PreprocessorTokenizer,
        PreprocessorTokenizerError,
        SavePoint as PreprocessorTokenizerSavePoint,
    },
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    Position,
    TranslationPhase,
};

// Lowest value has highest precedence.
pub(crate) fn prefix_binding_power<PrevError>(
    op: PreprocessorToken,
) -> Result<Option<((), u32)>, PreprocessorError<PrevError>> {
    let construct_error = |error_type| {
        Err(PreprocessorError::InnerPreprocessorError(
            InnerPreprocessorError {
                error_type:
                    PreprocessorErrorType::PrefixOperatorNotAllowedInPreprocessorExpression(
                        error_type,
                    ),
                start_position: op.start_position,
                contents:       op.contents,
            },
        ))
    };
    match op.kind {
        | PreprocessorTokenType::Defined => Ok(Some(((), 0))),
        | PreprocessorTokenType::Minus
        | PreprocessorTokenType::Plus
        | PreprocessorTokenType::ExclamationMark
        | PreprocessorTokenType::Tilde => Ok(Some(((), 2))),
        | PreprocessorTokenType::Asterisk => construct_error(ForbiddenPrefixOperator::Dereference),
        | PreprocessorTokenType::Ampersand => construct_error(ForbiddenPrefixOperator::AddressOf),
        | PreprocessorTokenType::Hash =>
            if op.length == 1 {
                construct_error(ForbiddenPrefixOperator::Hash)
            } else {
                construct_error(ForbiddenPrefixOperator::DigraphHash)
            },
        | _ => Ok(None),
    }
}

pub(crate) fn infix_binding_power<PrevError>(
    op: PreprocessorToken,
) -> Result<Option<(u32, u32)>, PreprocessorError<PrevError>> {
    let construct_error = |error_type| {
        Err(PreprocessorError::InnerPreprocessorError(
            InnerPreprocessorError {
                error_type:
                    PreprocessorErrorType::InfixOperatorNotAllowedInPreprocessorExpression(
                        error_type,
                    ),
                start_position: op.start_position,
                contents:       op.contents,
            },
        ))
    };
    match op.kind {
        | PreprocessorTokenType::HashHash =>
            if op.length == 2 {
                construct_error(ForbiddenInfixOperator::HashHash)
            } else {
                construct_error(ForbiddenInfixOperator::DigraphHashHash)
            },
        | PreprocessorTokenType::Period => construct_error(ForbiddenInfixOperator::MemberAccess),
        | PreprocessorTokenType::Arrow =>
            construct_error(ForbiddenInfixOperator::MemberAccessByPointer),
        | PreprocessorTokenType::Comma => construct_error(ForbiddenInfixOperator::Comma),
        | PreprocessorTokenType::Equals => construct_error(ForbiddenInfixOperator::Assignment),
        | PreprocessorTokenType::PlusEquals =>
            construct_error(ForbiddenInfixOperator::AdditionAssignment),
        | PreprocessorTokenType::MinusEquals =>
            construct_error(ForbiddenInfixOperator::SubtractionAssignment),
        | PreprocessorTokenType::AsteriskEquals =>
            construct_error(ForbiddenInfixOperator::MultiplicationAssignment),
        | PreprocessorTokenType::ForwardSlashEquals =>
            construct_error(ForbiddenInfixOperator::DivisionAssignment),
        | PreprocessorTokenType::PercentEquals =>
            construct_error(ForbiddenInfixOperator::RemainderAssignment),
        | PreprocessorTokenType::LessThanLessThanEquals =>
            construct_error(ForbiddenInfixOperator::LeftShiftAssignment),
        | PreprocessorTokenType::GreaterThanGreaterThanEquals =>
            construct_error(ForbiddenInfixOperator::RightShiftAssignment),
        | PreprocessorTokenType::AmpersandEquals =>
            construct_error(ForbiddenInfixOperator::BitwiseAndAssignment),
        | PreprocessorTokenType::CaretEquals =>
            construct_error(ForbiddenInfixOperator::BitwiseXorAssignment),
        | PreprocessorTokenType::PipeEquals =>
            construct_error(ForbiddenInfixOperator::BitwiseOrAssignment),

        | PreprocessorTokenType::Asterisk
        | PreprocessorTokenType::ForwardSlash
        | PreprocessorTokenType::Percent => Ok(Some((3, 4))),

        | PreprocessorTokenType::Plus | PreprocessorTokenType::Minus => Ok(Some((5, 6))),

        | PreprocessorTokenType::LessThanLessThan
        | PreprocessorTokenType::GreaterThanGreaterThan => Ok(Some((7, 8))),

        | PreprocessorTokenType::LessThan
        | PreprocessorTokenType::LessThanEquals
        | PreprocessorTokenType::GreaterThan
        | PreprocessorTokenType::GreaterThanEquals => Ok(Some((9, 10))),

        | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals =>
            Ok(Some((11, 12))),

        | PreprocessorTokenType::Ampersand => Ok(Some((13, 14))),

        | PreprocessorTokenType::Caret => Ok(Some((15, 16))),

        | PreprocessorTokenType::Pipe => Ok(Some((17, 18))),

        | PreprocessorTokenType::AmpersandAmpersand => Ok(Some((19, 20))),

        | PreprocessorTokenType::PipePipe => Ok(Some((21, 22))),

        | PreprocessorTokenType::QuestionMark => Ok(Some((23, 24))),

        | _ => Ok(None),
    }
}

fn postfix_binding_power<PrevError>(
    op: PreprocessorToken,
) -> Result<Option<(u32, ())>, PreprocessorError<PrevError>> {
    let construct_error = |error_type| {
        Err(PreprocessorError::InnerPreprocessorError(
            InnerPreprocessorError {
                error_type:
                    PreprocessorErrorType::PostfixOperatorNotAllowedInPreprocessorExpression(
                        error_type,
                    ),
                start_position: op.start_position,
                contents:       op.contents,
            },
        ))
    };
    match op.kind {
        | PreprocessorTokenType::OpeningParenthesis => Ok(Some((1, ()))),
        | PreprocessorTokenType::PlusPlus =>
            construct_error(ForbiddenPostfixOperator::PostIncrement),
        | PreprocessorTokenType::MinusMinus =>
            construct_error(ForbiddenPostfixOperator::PostDecrement),
        | PreprocessorTokenType::OpeningSquareBracket =>
            if op.length == 1 {
                construct_error(ForbiddenPostfixOperator::ArraySubscript)
            } else {
                construct_error(ForbiddenPostfixOperator::DigraphArraySubscript)
            },
        | PreprocessorTokenType::ClosingSquareBracket =>
            if op.length == 1 {
                construct_error(ForbiddenPostfixOperator::ClosingArraySubscript)
            } else {
                construct_error(ForbiddenPostfixOperator::DigraphClosingArraySubscript)
            },
        | PreprocessorTokenType::OpeningCurlyBrace =>
            if op.length == 1 {
                construct_error(ForbiddenPostfixOperator::CurlyBrace)
            } else {
                construct_error(ForbiddenPostfixOperator::DigraphCurlyBrace)
            },
        | PreprocessorTokenType::ClosingCurlyBrace =>
            if op.length == 1 {
                construct_error(ForbiddenPostfixOperator::ClosingCurlyBrace)
            } else {
                construct_error(ForbiddenPostfixOperator::DigraphClosingCurlyBrace)
            },
        | _ => Ok(None),
    }
}

pub(crate) enum PreprocessorAtomKind {
    Number(u128),
    Character(char),
    ObjectLikeMacro(StringCacheId),
    FunctionLikeMacro {
        name:      StringCacheId,
        arguments: Vec<PreprocessorExpression>,
    },
}

pub(crate) struct PreprocessorAtom {
    kind:     PreprocessorAtomKind,
    position: Position,
    length:   usize,
}

pub(crate) enum PreprocessorSubExpressionKind {
    Atom(PreprocessorAtom),
    BinaryOperator {
        operator: PreprocessorTokenType,
        left:     usize,
        right:    usize,
    },
    UnaryOperator {
        operator: PreprocessorTokenType,
        operand:  usize,
    },
}

pub(crate) struct PreprocessorSubExpression {
    kind:     PreprocessorSubExpressionKind,
    position: Position,
    length:   usize,
}

pub(crate) struct PreprocessorExpression {
    position:        Position,
    length:          usize,
    sub_expressions: Vec<PreprocessorSubExpression>,
}

const PREDEFINED_MACRO_NAMES: [&str; 4] = ["__LINE__", "__FILE__", "__DATE__", "__TIME__"];

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType {
    SourceFile,
    ObjectLikeMacroInvocation,
    FunctionLikeMacroInvocation {
        arguments: Vec<FunctionLikeMacroArgument>,
    },
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum MacroDefinition<PrevSavePoint> {
    ObjectLike {
        start_save_point: PrevSavePoint,
    },
    FunctionLike {
        argument_names:   Arc<Vec<StringCacheId>>,
        start_save_point: PrevSavePoint,
        is_variadic:      bool,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame<InnerSavePoint> {
    frame_type: TokenizerFrameType,
    save_point: InnerSavePoint,
    name:       StringCacheId,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SavePoint<PrevSavePoint, PrevError> {
    pub(crate) inner:                   PrevSavePoint,
    pub(crate) tokenizer_stack:         Vec<TokenizerFrame<PrevSavePoint>>,
    pub(crate) pending_results:         VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    pub(crate) state:                   State,
    pub(crate) last_preprocessor_token: Option<PreprocessorToken>,
    pub(crate) macro_definitions:       HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
}

impl<PrevSavePoint, PrevError> super::SavePoint for SavePoint<PrevSavePoint, PrevError>
where
    PrevSavePoint: super::SavePoint,
    PrevError: Debug + Clone,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum SignedIntegerLiteralType {
    Int,
    Long,
    LongLong,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum UnsignedIntegerLiteralType {
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    Default,
    MiddleOfDirective,
    Done,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum IntegerSuffix {
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Token {
    pub(crate) kind:           TokenType,
    pub(crate) start_position: Position,
    pub(crate) contents:       StringCacheId,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum IntegerTokenType {
    Short(i16),
    Int(i32),
    Long(i64),
    LongLong(i64),
    UnsignedShort(u16),
    UnsignedInt(u32),
    UnsignedLong(u64),
    UnsignedLongLong(u64),
    Int128(i128),
    UInt128(u128),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum FloatTokenType {
    Float(f32),
    Double(f64),
    LongDouble(LongDouble),
}

impl Display for FloatTokenType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Float(v) => write!(f, "{}", v),
            | Self::Double(v) => write!(f, "{}", v),
            | Self::LongDouble(v) => write!(f, "{}", v),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct InnerPreprocessorError {
    pub(crate) error_type:     PreprocessorErrorType,
    pub(crate) start_position: Position,
    pub(crate) contents:       StringCacheId,
}

impl Display for InnerPreprocessorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl std::error::Error for InnerPreprocessorError {}

impl GetSeverity for InnerPreprocessorError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorErrorType::UnexpectedEndOfInput(_)
            | PreprocessorErrorType::MissingOpeningParenthesisInFunctionLikeMacroInvocation
            | PreprocessorErrorType::InvalidHexadecimalFloatLiteral
            | PreprocessorErrorType::InvalidDecimalFloatLiteral
            | PreprocessorErrorType::InvalidHexadecimalIntegerLiteral
            | PreprocessorErrorType::InvalidBinaryIntegerLiteral
            | PreprocessorErrorType::InvalidOctalIntegerLiteral
            | PreprocessorErrorType::InvalidDecimalIntegerLiteral
            | PreprocessorErrorType::IntegerLiteralOverflow
            | PreprocessorErrorType::PrefixOperatorNotAllowedInPreprocessorExpression(..)
            | PreprocessorErrorType::InfixOperatorNotAllowedInPreprocessorExpression(..)
            | PreprocessorErrorType::PostfixOperatorNotAllowedInPreprocessorExpression(..)
            | PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression
            | PreprocessorErrorType::UnterminatedParenthesesInPreprocessorExpression
            | PreprocessorErrorType::NoExpressionAfterPrefixOperatorInPreprocessorExpression(
                ..,
            )
            | PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                ..
            }
            | PreprocessorErrorType::NoConditionInIfDirective
            | PreprocessorErrorType::NoConditionInElifDirective => ErrorSeverity::Error,
            | PreprocessorErrorType::FloatLiteralOverflow(..)
            | PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. }
            | PreprocessorErrorType::UnsignedIntLiteralOverflow
            | PreprocessorErrorType::ForcedUnsignedPromotion { .. }
            | PreprocessorErrorType::ForcedSignedPromotion { .. }
            | PreprocessorErrorType::HashMustBeFirstCharacterOnLine
            | PreprocessorErrorType::UnknownDirective
            | PreprocessorErrorType::HashMustBeFollowedByIdentifier => ErrorSeverity::Warning,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Error)]
pub(crate) enum PreprocessorError<PrevError> {
    #[error(transparent)]
    PreviousPhaseError(PrevError),
    #[error(transparent)]
    InnerPreprocessorError(InnerPreprocessorError),
}

impl<PrevError> GetSeverity for PreprocessorError<PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::PreviousPhaseError(e) => e.severity(),
            | Self::InnerPreprocessorError(e) => e.severity(),
        }
    }
}

impl<PrevError> GetPosition for PreprocessorError<PrevError>
where
    PrevError: GetPosition,
{
    fn position(&self) -> Position {
        match self {
            | Self::PreviousPhaseError(e) => e.position(),
            | Self::InnerPreprocessorError(e) => e.start_position,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ForbiddenPrefixOperator {
    PreIncrement,
    PreDecrement,
    Dereference,
    AddressOf,
    Hash,
    DigraphHash,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ForbiddenInfixOperator {
    MemberAccess,
    MemberAccessByPointer,
    Comma,
    Assignment,
    AdditionAssignment,
    SubtractionAssignment,
    MultiplicationAssignment,
    DivisionAssignment,
    RemainderAssignment,
    LeftShiftAssignment,
    RightShiftAssignment,
    BitwiseAndAssignment,
    BitwiseXorAssignment,
    BitwiseOrAssignment,
    HashHash,
    DigraphHashHash,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ForbiddenPostfixOperator {
    PostIncrement,
    PostDecrement,
    ArraySubscript,
    DigraphArraySubscript,
    ClosingArraySubscript,
    DigraphClosingArraySubscript,
    CurlyBrace,
    DigraphCurlyBrace,
    ClosingCurlyBrace,
    DigraphClosingCurlyBrace,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum PreprocessorErrorType {
    InvalidHexadecimalFloatLiteral,
    InvalidDecimalFloatLiteral,
    FloatLiteralOverflow(FloatTokenType),
    InvalidHexadecimalIntegerLiteral,
    InvalidBinaryIntegerLiteral,
    InvalidOctalIntegerLiteral,
    InvalidDecimalIntegerLiteral,
    IntegerLiteralOverflow,
    UnsignedIntLiteralOverflow,
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
    MissingOpeningParenthesisInFunctionLikeMacroInvocation,
    PrefixOperatorNotAllowedInPreprocessorExpression(ForbiddenPrefixOperator),
    InfixOperatorNotAllowedInPreprocessorExpression(ForbiddenInfixOperator),
    PostfixOperatorNotAllowedInPreprocessorExpression(ForbiddenPostfixOperator),
    EmptyParenthesesInPreprocessorExpression,
    UnterminatedParenthesesInPreprocessorExpression,
    NoExpressionAfterPrefixOperatorInPreprocessorExpression(PreprocessorTokenType),
    NoConditionInIfDirective,
    NoConditionInElifDirective,
    UnexpectedEndOfInput(&'static str),
    WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
        expected: usize,
        found:    usize,
    },
}

#[allow(clippy::too_many_lines)]
impl Display for PreprocessorErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::WrongNumberOfArgumentsInFunctionLikeMacroInvocation { expected, found } => {
                write!(
                    f,
                    "Wrong number of arguments in function-like macro invocation! Expected \
                     {expected} arguments, found {found} arguments",
                )
            },
            | Self::MissingOpeningParenthesisInFunctionLikeMacroInvocation => {
                write!(
                    f,
                    "Missing opening parenthesis in function-like macro invocation!"
                )
            },
            | Self::InvalidHexadecimalFloatLiteral => {
                write!(f, "Invalid hexadecimal floating point literal")
            },
            | Self::InvalidDecimalFloatLiteral => {
                write!(f, "Invalid decimal floating point literal")
            },
            | Self::FloatLiteralOverflow(float_token_type) => {
                write!(
                    f,
                    "Overflow while parsing floating point literal. Value of {float_token_type} \
                     literal was too big to be accurately represented. Some precision may have \
                     been lost.",
                )
            },
            | Self::InvalidHexadecimalIntegerLiteral => {
                write!(f, "Invalid hexadecimal integer literal")
            },
            | Self::InvalidBinaryIntegerLiteral => {
                write!(f, "Invalid binary integer literal")
            },
            | Self::InvalidOctalIntegerLiteral => {
                write!(f, "Invalid octal integer literal")
            },
            | Self::InvalidDecimalIntegerLiteral => {
                write!(f, "Invalid decimal integer literal")
            },
            | Self::IntegerLiteralOverflow => {
                write!(
                    f,
                    "Overflow while parsing integer literal. Must be smaller than UINT64_MAX"
                )
            },
            | Self::UnsignedIntLiteralOverflow => {
                write!(
                    f,
                    "Overflow while parsing unsigned int literal. Must be smaller than UINT32_MAX"
                )
            },
            | Self::ForcedSignedToUnsignedConversion { from, to } => {
                write!(
                    f,
                    "Integer literal was too big to fit into a signed value. Forced conversion \
                     from {} to {}",
                    match from {
                        | SignedIntegerLiteralType::Int => "int",
                        | SignedIntegerLiteralType::Long => "long",
                        | SignedIntegerLiteralType::LongLong => "long long",
                    },
                    match to {
                        | UnsignedIntegerLiteralType::UnsignedInt => "unsigned int",
                        | UnsignedIntegerLiteralType::UnsignedLong => "unsigned long",
                        | UnsignedIntegerLiteralType::UnsignedLongLong => "unsigned long long",
                    },
                )
            },
            | Self::ForcedUnsignedPromotion { from, to } => {
                write!(
                    f,
                    "Integer literal was too big to fit into an {0}. Forced conversion from {0} \
                     to {1}",
                    match from {
                        | UnsignedIntegerLiteralType::UnsignedInt => "unsigned int",
                        | UnsignedIntegerLiteralType::UnsignedLong => "unsigned long",
                        | UnsignedIntegerLiteralType::UnsignedLongLong => "unsigned long long",
                    },
                    match to {
                        | UnsignedIntegerLiteralType::UnsignedInt => "unsigned int",
                        | UnsignedIntegerLiteralType::UnsignedLong => "unsigned long",
                        | UnsignedIntegerLiteralType::UnsignedLongLong => "unsigned long long",
                    },
                )
            },
            | Self::ForcedSignedPromotion { from, to } => {
                write!(
                    f,
                    "Integer literal was too big to fit into an {0}. Forced conversion from {0} \
                     to {1}",
                    match from {
                        | SignedIntegerLiteralType::Int => "int",
                        | SignedIntegerLiteralType::Long => "long",
                        | SignedIntegerLiteralType::LongLong => "long long",
                    },
                    match to {
                        | SignedIntegerLiteralType::Int => "int",
                        | SignedIntegerLiteralType::Long => "long",
                        | SignedIntegerLiteralType::LongLong => "long long",
                    },
                )
            },
            | Self::HashMustBeFirstCharacterOnLine => {
                write!(
                    f,
                    "'#' operator used outside macro! If you were trying to define a preprocessor \
                     directive, it must the first token on the current line."
                )
            },
            | Self::HashMustBeFollowedByIdentifier => {
                write!(
                    f,
                    "'#' operator at the start of a line must be followed by an identifier! If \
                     you were trying to define a preprocessor directive, you probably forgot the \
                     directive name."
                )
            },
            | Self::UnknownDirective => {
                write!(
                    f,
                    "Unknown preprocessor directive. Must be one of: 'if' | 'ifdef' | 'ifndef' | \
                     'elif' | 'else' | 'endif' | 'include' | 'define' | 'undef' | 'line' | \
                     'error' | 'pragma'"
                )
            },
            | Self::PrefixOperatorNotAllowedInPreprocessorExpression(forbidden_unary_operator) => {
                write!(
                    f,
                    "Prefix operator '{}' is not allowed in preprocessor constant expressions!",
                    match forbidden_unary_operator {
                        | ForbiddenPrefixOperator::PreIncrement => "++",
                        | ForbiddenPrefixOperator::PreDecrement => "--",
                        | ForbiddenPrefixOperator::Dereference => "*",
                        | ForbiddenPrefixOperator::AddressOf => "&",
                        | ForbiddenPrefixOperator::Hash => "#",
                        | ForbiddenPrefixOperator::DigraphHash => "%:",
                    }
                )
            },
            | Self::InfixOperatorNotAllowedInPreprocessorExpression(forbidden_binary_operator) => {
                write!(
                    f,
                    "Infix operator '{}' is not allowed in preprocessor constant expressions!",
                    match forbidden_binary_operator {
                        | ForbiddenInfixOperator::MemberAccess => ".",
                        | ForbiddenInfixOperator::MemberAccessByPointer => "->",
                        | ForbiddenInfixOperator::Comma => ",",
                        | ForbiddenInfixOperator::Assignment => "=",
                        | ForbiddenInfixOperator::AdditionAssignment => "+=",
                        | ForbiddenInfixOperator::SubtractionAssignment => "-=",
                        | ForbiddenInfixOperator::MultiplicationAssignment => "*=",
                        | ForbiddenInfixOperator::DivisionAssignment => "/=",
                        | ForbiddenInfixOperator::RemainderAssignment => "%=",
                        | ForbiddenInfixOperator::LeftShiftAssignment => "<<=",
                        | ForbiddenInfixOperator::RightShiftAssignment => ">>=",
                        | ForbiddenInfixOperator::BitwiseAndAssignment => "&=",
                        | ForbiddenInfixOperator::BitwiseXorAssignment => "^=",
                        | ForbiddenInfixOperator::BitwiseOrAssignment => "|=",
                        | ForbiddenInfixOperator::HashHash => "##",
                        | ForbiddenInfixOperator::DigraphHashHash => "%:%:",
                    }
                )
            },
            | Self::PostfixOperatorNotAllowedInPreprocessorExpression(
                forbidden_postfix_operator,
            ) => {
                write!(
                    f,
                    "Postfix operator '{}' is not allowed in preprocessor constant expressions!",
                    match forbidden_postfix_operator {
                        | ForbiddenPostfixOperator::PostIncrement => "++",
                        | ForbiddenPostfixOperator::PostDecrement => "--",
                        | ForbiddenPostfixOperator::ArraySubscript => "[",
                        | ForbiddenPostfixOperator::DigraphArraySubscript => "<:",
                        | ForbiddenPostfixOperator::ClosingArraySubscript => "]",
                        | ForbiddenPostfixOperator::DigraphClosingArraySubscript => ":>",
                        | ForbiddenPostfixOperator::CurlyBrace => "{",
                        | ForbiddenPostfixOperator::DigraphCurlyBrace => "<%",
                        | ForbiddenPostfixOperator::ClosingCurlyBrace => "}",
                        | ForbiddenPostfixOperator::DigraphClosingCurlyBrace => "%>",
                    }
                )
            },
            | Self::EmptyParenthesesInPreprocessorExpression => {
                write!(
                    f,
                    "Parentheses containing no expression that are not part of a macro invocation \
                     are not allowed in preprocessor constant expressions!"
                )
            },
            | Self::UnterminatedParenthesesInPreprocessorExpression => {
                write!(
                    f,
                    "Unterminated parentheses in preprocessor constant expression!"
                )
            },
            | Self::NoExpressionAfterPrefixOperatorInPreprocessorExpression(operator) => {
                write!(
                    f,
                    "No expression after prefix operator '{}' in preprocessor constant expression!",
                    match operator {
                        | PreprocessorTokenType::Minus => "-",
                        | PreprocessorTokenType::Plus => "+",
                        | PreprocessorTokenType::ExclamationMark => "!",
                        | PreprocessorTokenType::Tilde => "~",
                        | _ => unreachable!(),
                    }
                )
            },

            | Self::NoConditionInIfDirective => {
                write!(
                    f,
                    "No condition in 'if' directive! If preprocessor directives must contain a \
                     condition."
                )
            },
            | Self::NoConditionInElifDirective => {
                write!(
                    f,
                    "No condition in 'elif' directive! Elif preprocessor directives must contain \
                     a condition."
                )
            },
            | Self::UnexpectedEndOfInput(message) => {
                write!(f, "Unexpected end of input while {message}!")
            },
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Preprocessor<Prev, PrevError, PrevSavePoint> {
    pub(crate) previous_phase: Prev,
    tokenizer_stack:           Vec<TokenizerFrame<PrevSavePoint>>,
    macro_definitions:         HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
    pending_results:           VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    state:                     State,
    last_preprocessor_token:   Option<PreprocessorToken>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct FunctionLikeMacroArgument {
    name:   StringCacheId,
    tokens: Vec<PreprocessorToken>,
}

impl<Prev: TranslationPhase> Preprocessor<Prev, Prev::Error, Prev::SavePoint> {
    pub(crate) fn with_previous_phase(
        source_file: StringCacheId,
        previous_phase: Prev,
        string_cache: &mut StringCache,
    ) -> Self {
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                save_point: previous_phase.save(),
                name:       source_file,
            }],
            previous_phase,
            macro_definitions: PREDEFINED_MACRO_NAMES
                .into_iter()
                .map(|s| -> (StringCacheId, MacroDefinition<Prev::SavePoint>) {
                    (string_cache.intern(s), MacroDefinition::BuiltIn)
                })
                .collect(),
            pending_results: VecDeque::new(),
            state: State::Default,
            last_preprocessor_token: None,
        }
    }
}

type Pptsp = PreprocessorTokenizerSavePoint<
    RemoveEscapedNewlinesSavePoint<MapCharacterSetsSavePoint<Position>>,
>;
type Ppt<'input> = PreprocessorTokenizer<
    RemoveEscapedNewlines<MapCharacterSets<NewlineTracking<'input>>>,
    RemoveEscapedNewlinesSavePoint<MapCharacterSetsSavePoint<Position>>,
>;
type Ppte =
    PreprocessorTokenizerError<RemoveEscapedNewlinesError<MapCharacterSetsError<Infallible>>>;
impl<'input> Preprocessor<Ppt<'input>, Ppte, Pptsp> {
    pub(crate) fn new(
        source_string: &'input str,
        source_file: StringCacheId,
        mut string_cache: StringCache,
    ) -> Self {
        let phase0 = NewlineTracking::new(source_string, source_file);
        let phase1 = MapCharacterSets::new(phase0);
        let phase2 = RemoveEscapedNewlines::new(phase1);
        let macro_definitions = PREDEFINED_MACRO_NAMES
            .into_iter()
            .map(|s| -> (StringCacheId, MacroDefinition<Pptsp>) {
                (string_cache.intern(s), MacroDefinition::BuiltIn)
            })
            .collect();
        let phase3 = PreprocessorTokenizer::new(phase2, string_cache);
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                save_point: phase3.save(),
                name:       source_file,
            }],
            previous_phase: phase3,
            macro_definitions,
            pending_results: VecDeque::new(),
            state: State::Default,
            last_preprocessor_token: None,
        }
    }
}

impl<PrevPrevError, Prev> Preprocessor<Prev, Prev::Error, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone,
{
    fn next_preprocessor_token_no_expand(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorTokenizerError<PrevPrevError>>> {
        loop {
            if let Some(tokenizer_frame) = self.tokenizer_stack.last_mut() {
                match tokenizer_frame.frame_type {
                    | TokenizerFrameType::SourceFile => match self.previous_phase.next() {
                        | Some(Err(e)) => {
                            self.pending_results
                                .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                            continue;
                        },
                        | Some(Ok(token)) => {
                            return Some(Ok(token));
                        },
                        | None => {
                            self.previous_phase
                                .restore(tokenizer_frame.save_point.clone());
                            drop(self.tokenizer_stack.pop());
                            continue;
                        },
                    },
                    | TokenizerFrameType::ObjectLikeMacroInvocation => {
                        match self.previous_phase.next() {
                            | Some(Err(e)) => {
                                self.pending_results
                                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                continue;
                            },
                            | Some(Ok(token)) => {
                                if token.kind == PreprocessorTokenType::Newline {
                                    drop(self.tokenizer_stack.pop());
                                    continue;
                                }
                                return Some(Ok(token));
                            },
                            | None => {
                                self.previous_phase
                                    .restore(tokenizer_frame.save_point.clone());
                                drop(self.tokenizer_stack.pop());
                                continue;
                            },
                        }
                    },
                    | TokenizerFrameType::FunctionLikeMacroInvocation { .. } => {
                        match self.previous_phase.next() {
                            | Some(Err(e)) => {
                                self.pending_results
                                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                continue;
                            },
                            | Some(Ok(token)) => {
                                if token.kind == PreprocessorTokenType::Newline {
                                    drop(self.tokenizer_stack.pop());
                                    continue;
                                }
                                return Some(Ok(token));
                            },
                            | None => {
                                self.previous_phase
                                    .restore(tokenizer_frame.save_point.clone());
                                drop(self.tokenizer_stack.pop());
                                continue;
                            },
                        }
                    },
                }
            }
            return None;
        }
    }

    #[allow(clippy::too_many_lines)]
    fn next_preprocessor_token(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        loop {
            let token = match self.next_preprocessor_token_no_expand() {
                | None => return None,
                | Some(Err(e)) => return Some(Err(PreprocessorError::PreviousPhaseError(e))),
                | Some(Ok(token)) => token,
            };
            if token.kind != PreprocessorTokenType::Identifier {
                return Some(Ok(token));
            }
            if let Some(md) = self.macro_definitions.get(&token.contents).cloned() {
                match md {
                    | MacroDefinition::ObjectLike { start_save_point } => {
                        let save_point = self.previous_phase.save();
                        self.tokenizer_stack.last_mut().unwrap().save_point = save_point;
                        self.tokenizer_stack.push(TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation,
                            save_point: start_save_point.clone(),
                            name:       token.contents,
                        });
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        start_save_point,
                        is_variadic,
                    } => {
                        let save_point = self.save();
                        loop {
                            match self.next_preprocessor_token_no_expand() {
                                | Some(Ok(token))
                                    if token.kind == PreprocessorTokenType::OpeningParenthesis =>
                                    break,
                                | Some(Ok(token)) => {
                                    self.pending_results.push_back(Err(
                                    PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingOpeningParenthesisInFunctionLikeMacroInvocation,
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    ),
                                ));
                                    self.restore(save_point);
                                    return Some(Ok(token));
                                },
                                | Some(Err(e)) => {
                                    self.pending_results
                                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                    continue;
                                },
                                | None => {
                                    return None;
                                },
                            }
                        }
                        let mut i = 0;
                        let mut arguments = Vec::new();
                        loop {
                            let mut argument_tokens = Vec::new();
                            match self.next_preprocessor_token() {
                                | Some(Ok(token))
                                    if token.kind == PreprocessorTokenType::ClosingParenthesis =>
                                {
                                    arguments.push(FunctionLikeMacroArgument {
                                        name:   argument_names[i],
                                        tokens: argument_tokens,
                                    });

                                    break;
                                },
                                | Some(Ok(token)) if token.kind == PreprocessorTokenType::Comma => {
                                    arguments.push(FunctionLikeMacroArgument {
                                        name:   argument_names[i],
                                        tokens: argument_tokens,
                                    });
                                    i += 1;
                                    continue;
                                },
                                | Some(Ok(token)) => {
                                    argument_tokens.push(token);
                                    continue;
                                },
                                | Some(Err(e)) => {
                                    self.pending_results.push_back(Err(e));
                                    continue;
                                },
                                | None => {
                                    return Some(Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    )));
                                },
                            }
                        }

                        if arguments.len() != argument_names.len() && !is_variadic {
                            self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      arguments.len(),
                                    },
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            )));
                        }
                        self.tokenizer_stack.push(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                arguments,
                            },
                            save_point: start_save_point.clone(),
                            name:       token.contents,
                        });
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match self.get_from_cache(token.contents) {
                        | "__FILE__" => {
                            let length =
                                self.get_from_cache(token.start_position.source_file).len();
                            return Some(Ok(PreprocessorToken {
                                kind: PreprocessorTokenType::String,
                                contents: token.start_position.source_file,
                                start_position: token.start_position,
                                length,
                            }));
                        },
                        | "__LINE__" => {
                            let string = token.start_position.line.to_string();
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       self.insert_into_cache(&string),
                                start_position: token.start_position,
                                length:         string.len(),
                            }));
                        },
                        | "__TIME__" => {
                            let now = Local::now();
                            let string = now.format("%H:%M:%S").to_string();
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                start_position: token.start_position,
                                length:         string.len(),
                            }));
                        },
                        | "__DATE__" => {
                            let now = Local::now();
                            let string = now.format("%b %e %Y").to_string();
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                start_position: token.start_position,
                                length:         string.len(),
                            }));
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            } else {
                return Some(Ok(token));
            }
        }
    }

    fn get_from_cache(&self, id: StringCacheId) -> &str {
        self.previous_phase
            .as_ref()
            .get(id)
            .unwrap_or_else(|| panic!("Compiler bug: StringCacheId is out of bounds: {id:?}"))
    }

    fn insert_into_cache(&mut self, string: &str) -> StringCacheId {
        self.previous_phase.as_mut().intern(string)
    }

    fn map_preprocessor_token(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<Result<Token, PreprocessorError<Prev::Error>>> {
        let mut contents: SmallString<[u8; 1024]> = self.get_from_cache(token.contents).into();

        Some(match token.kind {
            | PreprocessorTokenType::Number => {
                let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
                let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
                let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
                if is_hex {
                    if contents.contains(|c| c == '.' || c == 'p' || c == 'P') {
                        self.parse_hexadecimal_float(token, &mut contents)
                    } else {
                        self.parse_hexadecimal_integer(token, &contents)
                    }
                } else if is_binary {
                    self.parse_binary_integer(token, &contents)
                } else if contents.contains(|c| c == '.' || c == 'e' || c == 'E') {
                    self.parse_decimal_float(token, &mut contents)
                } else if is_octal {
                    self.parse_octal_integer(token, &contents)
                } else {
                    self.parse_decimal_integer(token, &contents)
                }
            },
            | PreprocessorTokenType::Newline => return None,
            | PreprocessorTokenType::Hash => return self.parse_directive(token, &contents),
            | x => todo!("Not implemented: {x:?}"),
        })
    }

    fn parse_directive(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Option<Result<Token, PreprocessorError<Prev::Error>>> {
        if !matches!(
            self.last_preprocessor_token.map(|p| p.kind),
            None | Some(PreprocessorTokenType::Newline)
        ) {
            self.state = State::MiddleOfDirective;
            self.last_preprocessor_token = Some(token);
            return Some(Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            )));
        }
        self.last_preprocessor_token = Some(token);
        let directive = match self.next_preprocessor_token_no_expand() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfDirective;
                return Some(Err(PreprocessorError::PreviousPhaseError(e)));
            },
            | Some(Ok(d)) => d,
            | None => return None,
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => return None,
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined | PreprocessorTokenType::Identifier => (),
            | _ =>
                return Some(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                        start_position: directive.start_position,
                        contents:       directive.contents,
                    },
                ))),
        }
        Some(match self.get_from_cache(directive.contents) {
            | "if" => self.parse_if_directive(),
            | "ifdef" => self.parse_ifdef_directive(),
            | "ifndef" => self.parse_ifndef_directive(),
            | "elif" => self.parse_elif_directive(),
            | "else" => self.parse_else_directive(),
            | "endif" => self.parse_endif_directive(),
            | "include" => self.parse_include_directive(),
            | "define" => self.parse_define_directive(),
            | "undef" => self.parse_undef_directive(),
            | "line" => self.parse_line_directive(),
            | "error" => self.parse_error_directive(),
            | "pragma" => self.parse_pragma_directive(),
            | _ => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    start_position: directive.start_position,
                    contents:       directive.contents,
                },
            )),
        })
    }

    fn const_eval_until_newline(&mut self) -> bool {
        todo!()
    }

    fn parse_preprocessor_expression(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<PreprocessorExpression, PreprocessorError<Prev::Error>> {
        let current_position = self.current_position();
        let mut expression = PreprocessorExpression {
            sub_expressions: Vec::new(),
            position:        current_position,
            length:          0,
        };
        match self.parse_preprocessor_sub_expression(u32::MAX) {
            | Ok(Some(sub_expression)) => expression.sub_expressions.push(sub_expression),
            | Ok(None) =>
                return Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::NoConditionInIfDirective,
                        start_position: directive.start_position,
                        contents:       directive.contents,
                    },
                )),
            | Err(e) => return Err(e),
        }
        expression.length = self.current_position().index - current_position.index;
        Ok(expression)
    }

    fn parse_preprocessor_sub_expression(
        &mut self,
        max_binding_power: u32,
    ) -> Result<Option<PreprocessorSubExpression>, PreprocessorError<Prev::Error>> {
        let mut lhs = 'lhs: loop {
            match self.next_preprocessor_token() {
                | Some(Err(e)) => self.pending_results.push_back(Err(e)),
                | Some(Ok(token)) => match token.kind {
                    | PreprocessorTokenType::OpeningParenthesis => {
                        let Some(lhs) = self.parse_preprocessor_sub_expression(u32::MAX)? else {
                            return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                        start_position: token.start_position,
                                        contents:       token.contents,
                                    },
                                ));
                        };
                        let save_point = self.previous_phase.save();
                        let position = self.current_position();
                        loop {
                            match self.next_preprocessor_token() {
                                | Some(Err(e)) => self.pending_results.push_back(Err(e)),
                                | Some(Ok(token))
                                    if token.kind == PreprocessorTokenType::ClosingParenthesis =>
                                    break 'lhs lhs,
                                | Some(Ok(token)) =>
                                    return Err(self.handle_unterminated_parens(
                                        position,
                                        save_point,
                                        token.contents,
                                    )),
                                | None => {
                                    let contents = self.insert_into_cache("EOF");
                                    return Err(self.handle_unterminated_parens(
                                        position, save_point, contents,
                                    ));
                                },
                            }
                        }
                    },
                    | _ => {
                        let Some(((), r_bp)) = prefix_binding_power(token)? else {
                            let atom = self.parse_preprocessor_atom()?;

                            break PreprocessorSubExpression {
                                position: atom.position,
                                length:   atom.length,
                                kind:     PreprocessorSubExpressionKind::Atom(atom),
                            };
                        };
                        let rhs = match self.parse_preprocessor_sub_expression(r_bp)? {
                            | Some(rhs) => rhs,
                            | None => {
                                return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::NoExpressionAfterPrefixOperatorInPreprocessorExpression(
                                                token.kind,
                                            ),
                                        start_position: token.start_position,
                                        contents:       token.contents,
                                    },
                                ))
                            },
                        };
                    },
                },
                | None => return Ok(None),
            }
        };
        loop {
            let op = loop {
                match self.next_preprocessor_token() {
                    | Some(Err(e)) => self.pending_results.push_back(Err(e)),
                    | Some(Ok(token)) => break token,
                    | None => return Ok(Some(lhs)),
                }
            };
            if let Some((l_bp, ())) = postfix_binding_power(op)? {
                if l_bp > max_binding_power {
                    break;
                }
                if op.kind == PreprocessorTokenType::OpeningParenthesis {}
                lhs = match self.parse_preprocessor_sub_expression(u32::MAX)? {
                    | Some(rhs) => rhs,
                    | None => {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::NoExpressionAfterPrefixOperatorInPreprocessorExpression(
                                        op.kind,
                                    ),
                                start_position: op.start_position,
                                contents:       op.contents,
                            },
                        ))
                    },
                };
            };
        }
        todo!()
    }

    fn parse_preprocessor_atom(
        &mut self,
    ) -> Result<PreprocessorAtom, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn handle_unterminated_parens(
        &mut self,
        start_position: Position,
        save_point: Prev::SavePoint,
        contents: StringCacheId,
    ) -> PreprocessorError<Prev::Error> {
        self.previous_phase.restore(save_point);
        PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
            error_type: PreprocessorErrorType::UnterminatedParenthesesInPreprocessorExpression,
            start_position,
            contents,
        })
    }

    fn parse_if_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_elif_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_else_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_endif_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_ifdef_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_ifndef_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_include_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_define_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_undef_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_line_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_error_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_pragma_directive(&mut self) -> Result<Token, PreprocessorError<Prev::Error>> {
        todo!()
    }

    #[allow(clippy::inline_always)]
    #[allow(clippy::too_many_lines)]
    #[inline(always)]
    fn parse_integer_radix(
        &mut self,
        radix: u32,
        start_index: usize,
        invalid_integer_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        let mut index = start_index;
        let result = (|| {
            let mut result = 0u64;
            while let Some(digit) = contents.char_at(index).and_then(|c| c.to_digit(radix)) {
                result = result.checked_mul(u64::from(radix))?;
                result = result.checked_add(u64::from(digit))?;
                index += 1;
            }
            Some(result)
        })();
        let Some(result) = result else {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::IntegerLiteralOverflow,
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            ));
        };
        let suffix_type = match (
            contents.char_at(index),
            contents.char_at(index + 1),
            contents.char_at(index + 2),
        ) {
            | (Some('u'), Some('l'), Some('l'))
            | (Some('U'), Some('L'), Some('L'))
            | (Some('l'), Some('l'), Some('u'))
            | (Some('L'), Some('L'), Some('U')) => {
                index += 3;
                Some(IntegerSuffix::UnsignedLongLong)
            },
            | (Some('u'), Some('l'), _)
            | (Some('U'), Some('L'), _)
            | (Some('l'), Some('u'), _)
            | (Some('L'), Some('U'), _) => {
                index += 2;
                Some(IntegerSuffix::UnsignedLong)
            },
            | (Some('u' | 'U'), _, _) => {
                index += 1;
                Some(IntegerSuffix::Unsigned)
            },
            | (Some('l'), Some('l'), _) | (Some('L'), Some('L'), _) => {
                index += 2;
                Some(IntegerSuffix::LongLong)
            },
            | (Some('l' | 'L'), _, _) => {
                index += 1;
                Some(IntegerSuffix::Long)
            },
            | _ => None,
        };
        if index != contents.len() {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     invalid_integer_literal_error,
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            ));
        }
        match suffix_type {
            | Some(IntegerSuffix::UnsignedLongLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::LongLong) if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    start_position: token.start_position,
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::LongLong,
                            to:   UnsignedIntegerLiteralType::UnsignedLongLong,
                        },
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            | Some(IntegerSuffix::LongLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::LongLong(result as _)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::UnsignedLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::Long) if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    start_position: token.start_position,
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::Long,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            | Some(IntegerSuffix::Long) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::Long(result as _)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::Unsigned) if result > u64::from(u32::MAX) => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    start_position: token.start_position,
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::IntegerLiteralOverflow,
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            #[allow(clippy::cast_possible_truncation)]
            | Some(IntegerSuffix::Unsigned) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedInt(result as u32)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
            | None if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    start_position: token.start_position,
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::Int,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            | None if result > i32::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::Long(result as i64)),
                    start_position: token.start_position,
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedPromotion {
                            from: SignedIntegerLiteralType::Int,
                            to:   SignedIntegerLiteralType::Long,
                        },
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            #[allow(clippy::cast_possible_truncation)]
            | None => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::Int(result as i32)),
                start_position: token.start_position,
                contents:       token.contents,
            }),
        }
    }

    fn parse_hexadecimal_integer(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_integer_radix(
            16,
            2,
            PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            token,
            contents,
        )
    }

    fn parse_binary_integer(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_integer_radix(
            2,
            2,
            PreprocessorErrorType::InvalidBinaryIntegerLiteral,
            token,
            contents,
        )
    }

    fn parse_octal_integer(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_integer_radix(
            8,
            1,
            PreprocessorErrorType::InvalidOctalIntegerLiteral,
            token,
            contents,
        )
    }

    fn parse_decimal_integer(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_integer_radix(
            10,
            0,
            PreprocessorErrorType::InvalidDecimalIntegerLiteral,
            token,
            contents,
        )
    }

    #[allow(clippy::inline_always)]
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn parse_float(
        &mut self,
        invalid_float_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
        contents: &mut SmallString<[u8; 1024]>,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        contents.push('\0');
        let contents = guard(contents, |contents| {
            let _ = contents.pop();
        });
        let res = match contents.char_at(contents.len() - 2) {
            | Some('f' | 'F') => string_to_float(contents.as_str()).map(FloatTokenType::Float),
            | Some('l' | 'L') =>
                string_to_long_double(contents.as_str()).map(FloatTokenType::LongDouble),
            | _ => string_to_double(contents.as_str()).map(FloatTokenType::Double),
        };
        println!("res: {res:?}");
        match res {
            | Ok(kind) => {
                println!("Float: {kind}");
                Ok(Token {
                    kind:           TokenType::Float(kind),
                    start_position: token.start_position,
                    contents:       token.contents,
                })
            },
            | Err(ParseFloatError::Invalid) => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     invalid_float_literal_error,
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            )),
            | Err(ParseFloatError::Overflow(kind)) => Err(
                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::FloatLiteralOverflow(kind),
                    start_position: token.start_position,
                    contents:       token.contents,
                }),
            ),
        }
    }

    fn parse_hexadecimal_float(
        &mut self,
        token: PreprocessorToken,
        contents: &mut SmallString<[u8; 1024]>,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_float(
            PreprocessorErrorType::InvalidHexadecimalFloatLiteral,
            token,
            contents,
        )
    }

    fn parse_decimal_float(
        &mut self,
        token: PreprocessorToken,
        contents: &mut SmallString<[u8; 1024]>,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        self.parse_float(
            PreprocessorErrorType::InvalidDecimalFloatLiteral,
            token,
            contents,
        )
    }
}

impl<PrevPrevError, Prev> Iterator for Preprocessor<Prev, Prev::Error, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone,
{
    type Item = Result<Token, PreprocessorError<Prev::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(result) = self.pending_results.pop_front() {
                return Some(result);
            }
            let token = match self.next_preprocessor_token() {
                | Some(Ok(token)) => token,
                | Some(Err(e)) => return Some(Err(e)),
                | None => return None,
            };

            if let Some(result) = self.map_preprocessor_token(token) {
                return Some(result);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

impl<PrevPrevError, Prev> TranslationPhase for Preprocessor<Prev, Prev::Error, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone,
{
    type Error = PreprocessorError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint, Prev::Error>;
    type Yield = Token;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner:                   self.previous_phase.save(),
            tokenizer_stack:         self.tokenizer_stack.clone(),
            pending_results:         self.pending_results.clone(),
            state:                   self.state,
            last_preprocessor_token: self.last_preprocessor_token,
            macro_definitions:       self.macro_definitions.clone(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.tokenizer_stack = save_point.tokenizer_stack;
        self.pending_results = save_point.pending_results;
        self.state = save_point.state;
        self.last_preprocessor_token = save_point.last_preprocessor_token;
        self.macro_definitions = save_point.macro_definitions;
    }

    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}
