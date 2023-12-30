use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    mem::take,
    ops::ControlFlow,
    path::{
        Path,
        PathBuf,
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
    util::{
        read_to_string_lossy,
        string_cache::{
            Id as StringCacheId,
            StringCache,
        },
    },
    HashMap,
};

pub(crate) trait FromInput: ISavePoint {
    fn from_input(input: Arc<str>, source_file: StringCacheId) -> Self;
}

impl FromInput for Pptsp {
    fn from_input(input: Arc<str>, source_file: StringCacheId) -> Self {
        let inner = RemoveEscapedNewlinesSavePoint {
            inner:            MapCharacterSetsSavePoint {
                inner: NewlineTrackingSavePoint {
                    source: input,
                    position: Position {
                        index: 0,
                        line: 1,
                        column: 1,
                        source_file,
                    },
                    is_middle_of_windows_newline: false,
                },
            },
            last_was_newline: false,
            state:            RemoveEscapedNewlinesState::Default,
        };
        Self {
            current_token_start: inner.clone(),
            inner,
            state: PreprocessorTokenizerState::Default,
            is_tokenizing_include_string: false,
        }
    }
}

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

    /// Returns the string cloned into a [`TokenString`].
    fn to_token_string(&self) -> TokenString
    where
        for<'a> &'a Self: Into<TokenString>,
    {
        self.into()
    }
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

use super::{
    phase_0_newline_tracking::{
        NewlineTracking,
        SavePoint as NewlineTrackingSavePoint,
    },
    phase_1_map_character_sets::{
        MapCharacterSets,
        MapCharacterSetsError,
        SavePoint as MapCharacterSetsSavePoint,
    },
    phase_2_remove_escaped_newlines::{
        RemoveEscapedNewlines,
        RemoveEscapedNewlinesError,
        SavePoint as RemoveEscapedNewlinesSavePoint,
        State as RemoveEscapedNewlinesState,
    },
    phase_3_preprocessor_tokenizer::{
        IsTokenizingIncludeString,
        PreprocessorToken,
        PreprocessorTokenType,
        PreprocessorTokenizer,
        PreprocessorTokenizerError,
        SavePoint as PreprocessorTokenizerSavePoint,
        State as PreprocessorTokenizerState,
    },
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    Position,
    SavePoint as ISavePoint,
    TranslationPhase,
};

pub(crate) type TokenString = SmallString<[u8; 1024]>;

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
        | PreprocessorTokenType::PlusPlus => construct_error(ForbiddenPrefixOperator::PreIncrement),
        | PreprocessorTokenType::MinusMinus =>
            construct_error(ForbiddenPrefixOperator::PreDecrement),
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
        | PreprocessorTokenType::Percent => Ok(Some((4, 3))),

        | PreprocessorTokenType::Plus | PreprocessorTokenType::Minus => Ok(Some((6, 5))),

        | PreprocessorTokenType::LessThanLessThan
        | PreprocessorTokenType::GreaterThanGreaterThan => Ok(Some((8, 7))),

        | PreprocessorTokenType::LessThan
        | PreprocessorTokenType::LessThanEquals
        | PreprocessorTokenType::GreaterThan
        | PreprocessorTokenType::GreaterThanEquals => Ok(Some((10, 9))),

        | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals =>
            Ok(Some((11, 12))),

        | PreprocessorTokenType::Ampersand => Ok(Some((14, 13))),

        | PreprocessorTokenType::Caret => Ok(Some((16, 15))),

        | PreprocessorTokenType::Pipe => Ok(Some((18, 17))),

        | PreprocessorTokenType::AmpersandAmpersand => Ok(Some((20, 19))),

        | PreprocessorTokenType::PipePipe => Ok(Some((22, 21))),

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
        | PreprocessorTokenType::OpeningParenthesis =>
            construct_error(ForbiddenPostfixOperator::OpeningParenthesis),
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

#[allow(variant_size_differences)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum PreprocessorAtomKind {
    Number(i128),
    Character(char),
    Identifier(StringCacheId),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorAtom {
    kind:     PreprocessorAtomKind,
    position: Position,
    length:   usize,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum PreprocessorSubExpressionKind {
    Atom(PreprocessorAtom),
    BinaryOperator {
        operator: PreprocessorTokenType,
        left:     usize,
        right:    usize,
    },
    Defined {
        name: StringCacheId,
    },
    UnaryOperator {
        operator: PreprocessorTokenType,
        operand:  usize,
    },
    TernaryOperator {
        condition: usize,
        if_true:   usize,
        if_false:  usize,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorSubExpression {
    kind:     PreprocessorSubExpressionKind,
    position: Position,
    length:   usize,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorExpression {
    position:        Position,
    length:          usize,
    sub_expressions: Vec<PreprocessorSubExpression>,
}

const PREDEFINED_MACRO_NAMES: [&str; 4] = ["__LINE__", "__FILE__", "__DATE__", "__TIME__"];

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType<PrevSavePoint> {
    SourceFile,
    ObjectLikeMacroInvocation,
    FunctionLikeMacroInvocation {
        arguments:   HashMap<StringCacheId, FunctionLikeMacroArgument<PrevSavePoint>>,
        is_variadic: bool,
    },
    FunctionLikeMacroArgument {
        argument:    FunctionLikeMacroArgument<PrevSavePoint>,
        paren_depth: usize,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition<PrevSavePoint> {
    ObjectLike {
        start_save_point: PrevSavePoint,
    },
    FunctionLike {
        argument_names:   Arc<[StringCacheId]>,
        start_save_point: PrevSavePoint,
        is_variadic:      bool,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame<PrevSavePoint> {
    frame_type: TokenizerFrameType<PrevSavePoint>,
    save_point: PrevSavePoint,
    name:       StringCacheId,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Preprocessor<Prev, PrevError, PrevSavePoint> {
    pub(crate) previous_phase: Prev,
    tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    macro_definitions: HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
    pending_results: VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    state: State,
    last_preprocessor_token: Option<PreprocessorToken>,
    current_preprocessor_token: Option<PreprocessorToken>,
    if_directive_balance: isize,
    should_tokenize_whitespace: bool,
    quote_include_directories: Arc<Vec<PathBuf>>,
    system_include_directories: Arc<Vec<PathBuf>>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SavePoint<PrevSavePoint, PrevError> {
    pub(crate) inner: PrevSavePoint,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    pub(crate) pending_results: VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    pub(crate) state: State,
    pub(crate) last_preprocessor_token: Option<PreprocessorToken>,
    pub(crate) current_preprocessor_token: Option<PreprocessorToken>,
    pub(crate) macro_definitions: HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
    pub(crate) if_directive_balance: isize,
    pub(crate) should_tokenize_whitespace: bool,
    pub(crate) quote_include_directories: Arc<Vec<PathBuf>>,
    pub(crate) system_include_directories: Arc<Vec<PathBuf>>,
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
#[allow(clippy::enum_variant_names)]
pub(crate) enum UnsignedIntegerLiteralType {
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    Default,
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
    Int(i32),
    Long(i64),
    LongLong(i64),
    UnsignedInt(u32),
    UnsignedLong(u64),
    UnsignedLongLong(u64),
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
            | Self::Float(v) => write!(f, "{v}"),
            | Self::Double(v) => write!(f, "{v}"),
            | Self::LongDouble(v) => write!(f, "{v}"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum KeywordTokenType {
    Auto,
    Break,
    Case,
    Char,
    Const,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extern,
    Float,
    For,
    Goto,
    If,
    Inline,
    Int,
    Long,
    Register,
    Restrict,
    Return,
    Short,
    Signed,
    Sizeof,
    Static,
    Struct,
    Switch,
    Typedef,
    Union,
    Unsigned,
    Void,
    Volatile,
    While,
    Bool,
    Complex,
    Imaginary,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum OperatorTokenType {
    Plus,
    Minus,
    Asterisk,
    ForwardSlash,
    Percent,
    LessThanLessThan,
    GreaterThanGreaterThan,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    EqualsEquals,
    ExclamationMarkEquals,
    Ampersand,
    Caret,
    Pipe,
    AmpersandAmpersand,
    PipePipe,
    QuestionMark,
    Colon,
    SemiColon,
    OpeningParenthesis,
    ClosingParenthesis,
    OpeningSquareBracket,
    ClosingSquareBracket,
    OpeningCurlyBrace,
    ClosingCurlyBrace,
    Period,
    Arrow,
    PlusPlus,
    MinusMinus,
    Comma,
    Tilde,
    ExclamationMark,
    Equals,
    PlusEquals,
    MinusEquals,
    AsteriskEquals,
    ForwardSlashEquals,
    PercentEquals,
    LessThanLessThanEquals,
    GreaterThanGreaterThanEquals,
    AmpersandEquals,
    CaretEquals,
    PipeEquals,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringLikeTokenType {
    Char(char),
    String(StringCacheId),
    WideChar(char),
    WideString(StringCacheId),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Identifier,
    Keyword(KeywordTokenType),
    Operator(OperatorTokenType),
    StringLike(StringLikeTokenType),
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
            | PreprocessorErrorType::NoExpressionAfterQuestionMarkInConditionalExpression
            | PreprocessorErrorType::NoColonAfterQuestionMarkInConditionalExpression(..)
            | PreprocessorErrorType::NoExpressionAfterColonInConditionalExpression
            | PreprocessorErrorType::NoExpressionAfterInfixOperatorInPreprocessorExpression(..)
            | PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                ..
            }
            | PreprocessorErrorType::ExpectedAtomInPreprocessorExpression(..)
            | PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(
                ..,
            )
            | PreprocessorErrorType::MissingIdentifierInDefinedDirective(..)
            | PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(..)
            | PreprocessorErrorType::NoConditionInIfDirective
            | PreprocessorErrorType::NoConditionInElifDirective
            | PreprocessorErrorType::ExpectedIdentifierInPreprocessorDirective(..)
            | PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives
            | PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives
            | PreprocessorErrorType::ElifDirectiveWithoutIfDirective
            | PreprocessorErrorType::ElseDirectiveWithoutIfDirective
            | PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInDefineDirective(..)
            | PreprocessorErrorType::RedefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(..)
            | PreprocessorErrorType::HeaderNotFound
            | PreprocessorErrorType::CurrentWorkingDirectoryInaccessible
            | PreprocessorErrorType::HeaderFileInaccessible
            | PreprocessorErrorType::HashHashUsedOutsideOfMacro
            | PreprocessorErrorType::InvalidEscapeSequence
            | PreprocessorErrorType::UnterminatedEscapeSequence
            | PreprocessorErrorType::InvalidHexEscapeSequence
            | PreprocessorErrorType::HexEscapeSequenceTooLarge
            | PreprocessorErrorType::InvalidOctalEscapeSequence
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
            | PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(..) =>
                ErrorSeverity::Error,
            | PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression
            | PreprocessorErrorType::FloatLiteralOverflow(..)
            | PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. }
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
    OpeningParenthesis,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum PreprocessorErrorType {
    InvalidHexadecimalFloatLiteral,
    InvalidDecimalFloatLiteral,
    FloatLiteralOverflow(FloatTokenType),
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
    MissingOpeningParenthesisInFunctionLikeMacroInvocation,
    PrefixOperatorNotAllowedInPreprocessorExpression(ForbiddenPrefixOperator),
    InfixOperatorNotAllowedInPreprocessorExpression(ForbiddenInfixOperator),
    PostfixOperatorNotAllowedInPreprocessorExpression(ForbiddenPostfixOperator),
    EmptyParenthesesInPreprocessorExpression,
    UnterminatedParenthesesInPreprocessorExpression,
    NoExpressionAfterPrefixOperatorInPreprocessorExpression(PreprocessorTokenType),
    NoExpressionAfterQuestionMarkInConditionalExpression,
    NoColonAfterQuestionMarkInConditionalExpression(PreprocessorTokenType),
    NoExpressionAfterColonInConditionalExpression,
    NoExpressionAfterInfixOperatorInPreprocessorExpression(PreprocessorTokenType),
    ExpectedAtomInPreprocessorExpression(PreprocessorTokenType),
    MissingOpeningParenthesisOrIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingClosingParenthesisInDefinedDirective(PreprocessorTokenType),
    NoConditionInIfDirective,
    NoConditionInElifDirective,
    ExpectedIdentifierInPreprocessorDirective(PreprocessorTokenType),
    MoreIfDirectivesThanEndifDirectives,
    MoreEndifDirectivesThanIfDirectives,
    ElifDirectiveWithoutIfDirective,
    ElseDirectiveWithoutIfDirective,
    ExpectedIdentifierInIfdefDirective(PreprocessorTokenType),
    ExpectedIdentifierInIfndefDirective(PreprocessorTokenType),
    ExpectedIdentifierInDefineDirective(PreprocessorTokenType),
    RedefinitionOfBuiltInMacro(String),
    UndefinedIdentifierInPreprocessorExpression,
    ExpectedIncludeStringOrAngleBracketString(PreprocessorTokenType),
    UnexpectedEndOfInput(&'static str),
    WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
        expected: usize,
        found:    usize,
    },
    HeaderNotFound,
    CurrentWorkingDirectoryInaccessible,
    HeaderFileInaccessible,
    HashHashUsedOutsideOfMacro,
    InvalidEscapeSequence,
    UnterminatedEscapeSequence,
    InvalidHexEscapeSequence,
    HexEscapeSequenceTooLarge,
    InvalidOctalEscapeSequence,
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
}

impl Display for PreprocessorErrorType {
    #[allow(clippy::too_many_lines)]
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
                        | ForbiddenPostfixOperator::OpeningParenthesis => "(",
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

            | Self::NoExpressionAfterQuestionMarkInConditionalExpression => {
                write!(
                    f,
                    "No expression after '?' in conditional expression in preprocessor constant \
                     expression!"
                )
            },

            | Self::NoColonAfterQuestionMarkInConditionalExpression(kind) => {
                write!(
                    f,
                    "No ':' after '?' in conditional expression in preprocessor constant \
                     expression! Found instead {kind:#?}"
                )
            },

            | Self::NoExpressionAfterColonInConditionalExpression => {
                write!(
                    f,
                    "No expression after ':' in conditional expression in preprocessor constant \
                     expression!"
                )
            },

            | Self::NoExpressionAfterInfixOperatorInPreprocessorExpression(operator) => {
                write!(
                    f,
                    "No expression after infix operator '{}' in preprocessor constant expression!",
                    match operator {
                        | PreprocessorTokenType::HashHash => "##",
                        | PreprocessorTokenType::Period => ".",
                        | PreprocessorTokenType::Arrow => "->",
                        | PreprocessorTokenType::Comma => ",",
                        | PreprocessorTokenType::Equals => "=",
                        | PreprocessorTokenType::PlusEquals => "+=",
                        | PreprocessorTokenType::MinusEquals => "-=",
                        | PreprocessorTokenType::AsteriskEquals => "*=",
                        | PreprocessorTokenType::ForwardSlashEquals => "/=",
                        | PreprocessorTokenType::PercentEquals => "%=",
                        | PreprocessorTokenType::LessThanLessThanEquals => "<<=",
                        | PreprocessorTokenType::GreaterThanGreaterThanEquals => ">>=",
                        | PreprocessorTokenType::AmpersandEquals => "&=",
                        | PreprocessorTokenType::CaretEquals => "^=",
                        | PreprocessorTokenType::PipeEquals => "|=",
                        | PreprocessorTokenType::Asterisk => "*",
                        | PreprocessorTokenType::ForwardSlash => "/",
                        | PreprocessorTokenType::Percent => "%",
                        | PreprocessorTokenType::Plus => "+",
                        | PreprocessorTokenType::Minus => "-",
                        | PreprocessorTokenType::LessThanLessThan => "<<",
                        | PreprocessorTokenType::GreaterThanGreaterThan => ">>",
                        | PreprocessorTokenType::LessThan => "<",
                        | PreprocessorTokenType::LessThanEquals => "<=",
                        | PreprocessorTokenType::GreaterThan => ">",
                        | PreprocessorTokenType::GreaterThanEquals => ">=",
                        | PreprocessorTokenType::EqualsEquals => "==",
                        | PreprocessorTokenType::ExclamationMarkEquals => "!=",
                        | PreprocessorTokenType::Ampersand => "&",
                        | PreprocessorTokenType::Caret => "^",
                        | PreprocessorTokenType::Pipe => "|",
                        | PreprocessorTokenType::AmpersandAmpersand => "&&",
                        | PreprocessorTokenType::PipePipe => "||",
                        | PreprocessorTokenType::QuestionMark => "?",
                        | _ => unreachable!(),
                    }
                )
            },
            | Self::ExpectedAtomInPreprocessorExpression(tt) => {
                write!(
                    f,
                    "Expected atom in preprocessor constant expression! An atom is either an \
                     integer literal, or a character literal. Found instead {tt:#?}"
                )
            },
            | Self::MissingOpeningParenthesisOrIdentifierInDefinedDirective(tt) => {
                write!(
                    f,
                    "Missing opening parenthesis or identifier in 'defined' directive! Found \
                     instead {tt:#?}"
                )
            },
            | Self::MissingIdentifierInDefinedDirective(tt) => {
                write!(
                    f,
                    "Missing identifier in 'defined' directive! Found instead {tt:#?}"
                )
            },
            | Self::MissingClosingParenthesisInDefinedDirective(tt) => {
                write!(
                    f,
                    "Missing closing parenthesis in 'defined' directive! Found instead {tt:#?}"
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
            | Self::ExpectedIdentifierInPreprocessorDirective(tt) => {
                write!(
                    f,
                    "Expected identifier in preprocessor directive! Found instead {tt:#?}"
                )
            },
            | Self::MoreIfDirectivesThanEndifDirectives => {
                write!(
                    f,
                    "More 'if', 'ifdef' and 'ifndef' directives than 'endif' directives! Every \
                     'if', 'ifdef' and 'ifndef' directive must be followed by an 'endif' \
                     directive."
                )
            },
            | Self::MoreEndifDirectivesThanIfDirectives => {
                write!(
                    f,
                    "More 'endif' directives than 'if', 'ifdef' and 'ifndef' directives! Every \
                     'endif' directive must be preceded by an 'if', 'ifdef' or 'ifndef' directive."
                )
            },
            | Self::ElifDirectiveWithoutIfDirective => {
                write!(
                    f,
                    "'elif' directive without preceding 'if', 'ifdef' or 'ifndef' directive!"
                )
            },
            | Self::ElseDirectiveWithoutIfDirective => {
                write!(
                    f,
                    "'else' directive without preceding 'if', 'ifdef' or 'ifndef' directive!"
                )
            },
            | Self::ExpectedIdentifierInIfdefDirective(tt) => {
                write!(
                    f,
                    "Expected identifier in 'ifdef' directive! Found instead {tt:#?}"
                )
            },
            | Self::ExpectedIdentifierInIfndefDirective(tt) => {
                write!(
                    f,
                    "Expected identifier in 'ifndef' directive! Found instead {tt:#?}"
                )
            },
            | Self::ExpectedIdentifierInDefineDirective(tt) => {
                write!(
                    f,
                    "Expected identifier in 'define' directive! Found instead {tt:#?}"
                )
            },
            | Self::RedefinitionOfBuiltInMacro(name) => {
                write!(
                    f,
                    "Redefinition of built-in macro '{name}'! Built-in macros cannot be redefined."
                )
            },
            | Self::ExpectedIncludeStringOrAngleBracketString(tt) => {
                write!(
                    f,
                    "Expected include string or angle bracket string in 'include' directive! \
                     Found instead {tt:#?}"
                )
            },
            | Self::UnexpectedEndOfInput(message) => {
                write!(f, "Unexpected end of input while {message}!")
            },
            | Self::UndefinedIdentifierInPreprocessorExpression => {
                write!(
                    f,
                    "Undefined identifier in preprocessor constant expression! Will be treated as \
                     if it had a value of 0."
                )
            },
            | Self::HeaderNotFound => {
                write!(f, "Header not found!")
            },
            | Self::CurrentWorkingDirectoryInaccessible => {
                write!(
                    f,
                    "Current working directory inaccessible! The operating system returned an \
                     error when trying to get the current working directory."
                )
            },
            | Self::HeaderFileInaccessible => {
                write!(
                    f,
                    "Header file inaccessible! The header file was deleted or moved while the \
                     preprocessor was trying to read it."
                )
            },
            | Self::HashHashUsedOutsideOfMacro => {
                write!(
                    f,
                    "'##' operator used outside of macro! The '##' operator can only be used \
                     inside a macro definition."
                )
            },
            | Self::InvalidEscapeSequence => {
                write!(
                    f,
                    "Invalid escape sequence! Expected one of 0..7, x, u or U"
                )
            },
            | Self::UnterminatedEscapeSequence => {
                write!(
                    f,
                    "Lonely backslash detected! Backslash must be followed by an escape sequence"
                )
            },
            | Self::InvalidHexEscapeSequence => {
                write!(f, "Hexadecimal escape sequence was not a valid codepoint!")
            },
            | Self::HexEscapeSequenceTooLarge => {
                write!(
                    f,
                    "Overflow while trying to parse hexadecimal escape sequence!"
                )
            },
            | Self::InvalidOctalEscapeSequence => {
                write!(f, "Octal escape sequence was not a valid codepoint!")
            },
            | Self::OctalEscapeSequenceTooLarge => {
                write!(f, "Overflow while trying to parse octal escape sequence!")
            },
            | Self::InvalidSmallUnicodeEscapeSequence => {
                write!(
                    f,
                    "Small unicode escape sequence was not a valid codepoint!"
                )
            },
            | Self::SmallUnicodeEscapeSequenceTooShort => {
                write!(
                    f,
                    "Overflow while trying to parse small unicode escape sequence!"
                )
            },
            | Self::InvalidLargeUnicodeEscapeSequence => {
                write!(
                    f,
                    "Large unicode escape sequence was not a valid codepoint!"
                )
            },
            | Self::LargeUnicodeEscapeSequenceTooSmall => {
                write!(
                    f,
                    "Overflow while trying to parse large unicode escape sequence!"
                )
            },
            | Self::MultiCharacterLiteralsUnsupported => {
                write!(f, "Multi-character literals are not supported by BCC!")
            },
            | Self::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(name) => {
                write!(
                    f,
                    "Redefinition of function-like macro '{name}' as object-like macro! \
                     Function-like macros cannot be redefined as object-like macros without \
                     undefining them first."
                )
            },
            | Self::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(name) => {
                write!(
                    f,
                    "Redefinition of object-like macro '{name}' as function-like macro! \
                     Object-like macros cannot be redefined as function-like macros without \
                     undefining them first."
                )
            },
            | Self::ExpectedIdentifierInMacroDefinition(tt) => {
                write!(
                    f,
                    "Expected identifier in macro definition! Found instead {tt:#?}"
                )
            },
            | Self::VariadicMacroMustBeLastParameter(name) => {
                write!(
                    f,
                    "Variadic macro '{name}' must be the last parameter in the macro definition!"
                )
            },
            | Self::ExpectedCommaOrClosingParenthesisInMacroDefinition(tt) => {
                write!(
                    f,
                    "Expected ',' or ')' in macro definition! Found instead {tt:#?}"
                )
            },
            | Self::MacroRedefinedWithDifferentDefinition(name) => {
                write!(
                    f,
                    "Macro '{name}' redefined with different definition! All definitions of a \
                     macro must be identical. If you want to change the definition of a macro, \
                     you must first undefine it first."
                )
            },
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct FunctionLikeMacroArgument<PrevSavePoint> {
    name:             StringCacheId,
    start_save_point: PrevSavePoint,
}

impl<Prev: TranslationPhase> Preprocessor<Prev, Prev::Error, Prev::SavePoint> {
    pub(crate) fn with_previous_phase(
        source_file: StringCacheId,
        previous_phase: Prev,
        string_cache: &mut StringCache,
        quote_include_directories: Arc<Vec<PathBuf>>,
        system_include_directories: Arc<Vec<PathBuf>>,
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
            current_preprocessor_token: None,
            if_directive_balance: 0,
            should_tokenize_whitespace: false,
            quote_include_directories,
            system_include_directories,
        }
    }
}

macro_rules! get_from_cache {
    ($self:expr, $id:expr) => {
        ($self)
            .previous_phase
            .as_ref()
            .get($id)
            .unwrap_or_else(|| panic!("Compiler bug: StringCacheId is out of bounds: {:#?}", $id))
    };
}

type Pptsp = PreprocessorTokenizerSavePoint<
    RemoveEscapedNewlinesSavePoint<MapCharacterSetsSavePoint<NewlineTrackingSavePoint>>,
>;
type Ppt = PreprocessorTokenizer<
    RemoveEscapedNewlines<MapCharacterSets<NewlineTracking>>,
    RemoveEscapedNewlinesSavePoint<MapCharacterSetsSavePoint<NewlineTrackingSavePoint>>,
>;
type Ppte =
    PreprocessorTokenizerError<RemoveEscapedNewlinesError<MapCharacterSetsError<Infallible>>>;
impl Preprocessor<Ppt, Ppte, Pptsp> {
    pub(crate) fn new(
        source_string: Arc<str>,
        source_file: StringCacheId,
        mut string_cache: StringCache,
        quote_include_directories: Arc<Vec<PathBuf>>,
        system_include_directories: Arc<Vec<PathBuf>>,
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
            current_preprocessor_token: None,
            if_directive_balance: 0,
            should_tokenize_whitespace: false,
            quote_include_directories,
            system_include_directories,
        }
    }
}

impl<PrevPrevError, Prev> Preprocessor<Prev, Prev::Error, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>
        + IsTokenizingIncludeString,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone + PartialEq,
    Prev::SavePoint: FromInput,
{
    fn expect_token_no_expand(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        mut on_previous_phase_error: impl FnMut(
            &mut Self,
            Prev::Error,
        )
            -> ControlFlow<<Self as TranslationPhase>::Error>,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<<Self as TranslationPhase>::Error>,
        eof_message: &'static str,
    ) -> Result<PreprocessorToken, <Self as TranslationPhase>::Error> {
        loop {
            match self.next_preprocessor_token_no_expand() {
                | Some(Ok(token)) => {
                    if is_correct_token(self, token) {
                        return Ok(token);
                    }
                    match on_wrong_token_type(self, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => break Err(e),
                    }
                },
                | Some(Err(e)) => match on_previous_phase_error(self, e) {
                    | ControlFlow::Continue(()) => continue,
                    | ControlFlow::Break(e) => break Err(e),
                },
                | None => {
                    let contents = self.insert_into_cache("EOF");
                    break Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                            start_position: self.previous_phase.current_position(),
                            contents,
                        },
                    ));
                },
            }
        }
    }

    #[allow(dead_code)]
    fn expect_token(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        mut on_error: impl FnMut(
            &mut Self,
            <Self as TranslationPhase>::Error,
        ) -> ControlFlow<<Self as TranslationPhase>::Error>,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<<Self as TranslationPhase>::Error>,
        eof_message: &'static str,
    ) -> Result<PreprocessorToken, <Self as TranslationPhase>::Error> {
        loop {
            match self.next_preprocessor_token() {
                | Some(Ok(token)) => {
                    if is_correct_token(self, token) {
                        return Ok(token);
                    }
                    match on_wrong_token_type(self, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => break Err(e),
                    }
                },
                | Some(Err(e)) => match on_error(self, e) {
                    | ControlFlow::Continue(()) => continue,
                    | ControlFlow::Break(e) => break Err(e),
                },
                | None => {
                    let contents = self.insert_into_cache("EOF");
                    break Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                            start_position: self.previous_phase.current_position(),
                            contents,
                        },
                    ));
                },
            }
        }
    }

    fn next_preprocessor_token_no_expand(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorTokenizerError<PrevPrevError>>> {
        let ret = loop {
            self.last_preprocessor_token = self.current_preprocessor_token;
            if self.tokenizer_stack.last().is_some() {
                match self.previous_phase.next() {
                    | Some(Err(e)) => {
                        break Some(Err(e));
                    },
                    | Some(Ok(token))
                        if token.kind == PreprocessorTokenType::Whitespace
                            && (!self.should_tokenize_whitespace
                                || self.last_preprocessor_token.as_ref().map(|t| t.kind)
                                    == Some(PreprocessorTokenType::Whitespace)) =>
                    {
                        continue;
                    },
                    | Some(Ok(token)) => {
                        match self.tokenizer_stack.last_mut().unwrap() {
                            | TokenizerFrame {
                                frame_type:
                                    TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                                    | TokenizerFrameType::ObjectLikeMacroInvocation,
                                ..
                            } =>
                                if token.kind == PreprocessorTokenType::Newline {
                                    drop(self.tokenizer_stack.pop());
                                    if let Some(last) = self.tokenizer_stack.last() {
                                        self.previous_phase.restore(last.save_point.clone());
                                    }
                                    continue;
                                },
                            | TokenizerFrame {
                                frame_type:
                                    TokenizerFrameType::FunctionLikeMacroArgument {
                                        paren_depth,
                                        argument,
                                    },
                                ..
                            } => {
                                let mut paren_depth = *paren_depth;
                                let argument = argument.clone();
                                if (token.kind == PreprocessorTokenType::Comma
                                    && argument.name != self.insert_into_cache("__VA_ARGS__"))
                                    || (token.kind == PreprocessorTokenType::ClosingParenthesis
                                        && paren_depth == 1)
                                {
                                    drop(self.tokenizer_stack.pop());
                                    if let Some(last) = self.tokenizer_stack.last() {
                                        self.previous_phase.restore(last.save_point.clone());
                                    }
                                    continue;
                                }
                                if token.kind == PreprocessorTokenType::OpeningParenthesis {
                                    paren_depth += 1;
                                }
                                if token.kind == PreprocessorTokenType::ClosingParenthesis {
                                    paren_depth -= 1;
                                }
                                let TokenizerFrame {
                                    frame_type:
                                        TokenizerFrameType::FunctionLikeMacroArgument {
                                            paren_depth: p,
                                            ..
                                        },
                                    ..
                                } = self.tokenizer_stack.last_mut().unwrap()
                                else {
                                    unreachable!();
                                };
                                *p = paren_depth;
                            },
                            | TokenizerFrame {
                                frame_type: TokenizerFrameType::SourceFile,
                                ..
                            } => (),
                        }
                        break Some(Ok(token));
                    },
                    | None => {
                        drop(self.tokenizer_stack.pop());
                        if let Some(last) = self.tokenizer_stack.last() {
                            self.previous_phase.restore(last.save_point.clone());
                        }
                        continue;
                    },
                }
            }
            break None;
        }?;
        match ret {
            | Err(e) => Some(Err(e)),
            | Ok(t) => {
                self.current_preprocessor_token = Some(t);
                Some(Ok(t))
            },
        }
    }

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
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation,
                            save_point: start_save_point.clone(),
                            name:       token.contents,
                        };
                        self.tokenizer_stack.push(frame);
                        self.previous_phase.restore(start_save_point);
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
                                    if token.kind == PreprocessorTokenType::Whitespace =>
                                    continue,
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
                        let mut arguments = HashMap::default();
                        let mut paren_depth = 1;
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let start_save_point = self.previous_phase.save();
                            loop {
                                match self.next_preprocessor_token() {
                                    | Some(Ok(token))
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            drop(arguments.insert(
                                                argument_names[i],
                                                FunctionLikeMacroArgument {
                                                    name:             argument_names[i],
                                                    start_save_point: start_save_point.clone(),
                                                },
                                            ));

                                            break 'outer;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(Ok(token))
                                        if token.kind == PreprocessorTokenType::Comma =>
                                    {
                                        drop(arguments.insert(
                                            argument_names[i],
                                            FunctionLikeMacroArgument {
                                                name:             argument_names[i],
                                                start_save_point: start_save_point.clone(),
                                            },
                                        ));
                                        i += 1;
                                        continue 'outer;
                                    },
                                    | Some(Ok(token))
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(Ok(_)) => {
                                        continue;
                                    },
                                    | Some(Err(e)) => {
                                        self.pending_results.push_back(Err(e));
                                        continue;
                                    },
                                    | None => {
                                        return Some(Err(
                                            PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:
                                                        PreprocessorErrorType::UnexpectedEndOfInput(
                                                            "parsing function-like macro \
                                                             invocation",
                                                        ),
                                                    start_position: token.start_position,
                                                    contents:       token.contents,
                                                },
                                            ),
                                        ));
                                    },
                                }
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
                        if is_variadic {
                            drop(arguments.insert(
                                self.insert_into_cache("__VA_ARGS__"),
                                FunctionLikeMacroArgument {
                                    name:             self.insert_into_cache("__VA_ARGS__"),
                                    start_save_point: self.previous_phase.save(),
                                },
                            ));
                            let mut paren_depth = 1;

                            loop {
                                match self.next_preprocessor_token() {
                                    | Some(Ok(token))
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            break;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(Ok(token))
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(Ok(_)) => {
                                        continue;
                                    },
                                    | Some(Err(e)) => {
                                        self.pending_results.push_back(Err(e));
                                        continue;
                                    },
                                    | None => {
                                        return Some(Err(
                                            PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:
                                                        PreprocessorErrorType::UnexpectedEndOfInput(
                                                            "parsing function-like macro \
                                                             invocation",
                                                        ),
                                                    start_position: token.start_position,
                                                    contents:       token.contents,
                                                },
                                            ),
                                        ));
                                    },
                                }
                            }
                        }
                        self.tokenizer_stack.last_mut().unwrap().save_point =
                            self.previous_phase.save();
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                arguments,
                                is_variadic,
                            },
                            save_point: start_save_point.clone(),
                            name:       token.contents,
                        };
                        self.tokenizer_stack.push(frame);
                        self.previous_phase.restore(start_save_point);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match get_from_cache!(self, token.contents) {
                        | "__FILE__" => {
                            let length =
                                get_from_cache!(self, token.start_position.source_file).len();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            return Some(Ok(PreprocessorToken {
                                kind: PreprocessorTokenType::String,
                                contents: token.start_position.source_file,
                                start_position: Position {
                                    index:       0,
                                    line:        1,
                                    column:      1,
                                    source_file: file_name,
                                },
                                length,
                            }));
                        },
                        | "__LINE__" => {
                            let string = token.start_position.line.to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       self.insert_into_cache(&string),
                                start_position: Position {
                                    index:       0,
                                    line:        1,
                                    column:      1,
                                    source_file: file_name,
                                },
                                length:         string.len(),
                            }));
                        },
                        | "__TIME__" => {
                            let now = Local::now();
                            let string = now.format("%H:%M:%S").to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                start_position: Position {
                                    index:       0,
                                    line:        1,
                                    column:      1,
                                    source_file: file_name,
                                },
                                length:         string.len(),
                            }));
                        },
                        | "__DATE__" => {
                            let now = Local::now();
                            let string = now.format("%b %e %Y").to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            return Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                start_position: Position {
                                    index:       0,
                                    line:        1,
                                    column:      1,
                                    source_file: file_name,
                                },
                                length:         string.len(),
                            }));
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            } else {
                if let Some(tokenizer_frame) = self.tokenizer_stack.last() {
                    if let TokenizerFrameType::FunctionLikeMacroInvocation {
                        ref arguments,
                        is_variadic: _,
                    } = tokenizer_frame.frame_type
                    {
                        if let Some(arg) = arguments.get(&token.contents) {
                            let frame = TokenizerFrame {
                                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                                    argument:    arg.clone(),
                                    paren_depth: 1,
                                },
                                save_point: arg.start_save_point.clone(),
                                name:       token.contents,
                            };
                            let start_save_point = arg.start_save_point.clone();
                            self.tokenizer_stack.last_mut().unwrap().save_point =
                                self.previous_phase.save();
                            self.tokenizer_stack.push(frame);
                            self.previous_phase.restore(start_save_point);
                            continue;
                        }
                    }
                }
                return Some(Ok(token));
            }
        }
    }

    fn eval_escape_sequences(
        &mut self,
        token: PreprocessorToken,
    ) -> Result<String, <Self as TranslationPhase>::Error> {
        let mut ret = String::new();
        let mut index = 0;
        let mut pending_results = guard(take(&mut self.pending_results), |pending_results| {
            self.pending_results = pending_results;
        });
        let string = get_from_cache!(self, token.contents);
        while let Some(c) = string.char_at(index) {
            if c == '\\' {
                let Some(c) = string.char_at(index + 1) else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnterminatedEscapeSequence,
                            start_position: token.start_position,
                            contents:       token.contents,
                        },
                    ));
                };
                index += 2;
                ret.push(match c {
                    | 'a' => '\x07',
                    | 'b' => '\x08',
                    | 'f' => '\x0C',
                    | 'n' => '\n',
                    | 'r' => '\r',
                    | 't' => '\t',
                    | 'v' => '\x0B',
                    | '\'' => '\'',
                    | '"' => '"',
                    | '?' => '?',
                    | '\\' => '\\',
                    | 'x' => {
                        let code_point = (|| {
                            let mut code_point = 0u32;
                            while let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) {
                                index += 1;
                                code_point = code_point.checked_mul(16)?;
                                code_point = code_point.checked_add(d)?;
                            }
                            Some(code_point)
                        })();
                        let Some(code_point) = code_point else {
                            pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::HexEscapeSequenceTooLarge,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            )));
                            continue;
                        };
                        let Ok(c) = char::try_from(code_point) else {
                            pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidHexEscapeSequence,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            )));
                            continue;
                        };
                        c
                    },
                    | '0'..='7' => {
                        #[allow(clippy::cast_possible_truncation)]
                        let code_point = (|| {
                            let mut code_point = c as u16 - '0' as u16;
                            for _ in 0..2 {
                                let Some(d) = string.char_at(index).and_then(|c| c.to_digit(8))
                                else {
                                    break;
                                };
                                index += 1;
                                code_point = code_point.checked_mul(8)?;

                                code_point = code_point.checked_add(d as u16)?;
                            }
                            Some(code_point)
                        })();

                        let Some(code_point) = code_point else {
                            pending_results.push_back(Err(
                                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::OctalEscapeSequenceTooLarge,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                }),
                            ));
                            continue;
                        };
                        let Ok(c) = char::try_from(u32::from(code_point)) else {
                            pending_results.push_back(Err(
                                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidOctalEscapeSequence,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                }),
                            ));
                            continue;
                        };
                        c
                    },
                    | 'u' => {
                        let mut code_point = 0u32;
                        for _ in 0..4 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16))
                            else {
                                pending_results.push_back(
                                    Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort,
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    )),
                                );
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            )));
                            continue;
                        };
                        c
                    },
                    | 'U' => {
                        let mut code_point = 0u32;
                        for _ in 0..8 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16))
                            else {
                                pending_results.push_back(
                                    Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall,
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    )),
                                );
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            )));
                            continue;
                        };
                        c
                    }
                    | _ => {
                        pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidEscapeSequence,
                                start_position: token.start_position,
                                contents:       token.contents,
                            },
                        )));
                        continue;
                    }
                });
            } else {
                ret.push(c);
                index += c.len_utf8();
            }
        }
        Ok(ret)
    }

    fn insert_into_cache(&mut self, string: &str) -> StringCacheId {
        self.previous_phase.as_mut().intern(string)
    }

    fn build_token(token: PreprocessorToken, kind: TokenType) -> Token {
        Token {
            kind,
            contents: token.contents,
            start_position: token.start_position,
        }
    }

    fn build_operator_token(token: PreprocessorToken, kind: OperatorTokenType) -> Token {
        Self::build_token(token, TokenType::Operator(kind))
    }

    fn map_preprocessor_token(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<Result<Token, PreprocessorError<Prev::Error>>> {
        let mut contents = get_from_cache!(self, token.contents).to_token_string();

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
            | PreprocessorTokenType::Hash => match self.parse_directive(token, &contents) {
                | Ok(()) => return None,
                | Err(e) => Err(e),
            },
            | PreprocessorTokenType::String => self.eval_escape_sequences(token).map(|contents| {
                let cached_contents = self.insert_into_cache(&contents);
                let string_like_token_type =
                    if get_from_cache!(self, token.contents).starts_with('L') {
                        StringLikeTokenType::WideString(cached_contents)
                    } else {
                        StringLikeTokenType::String(cached_contents)
                    };
                Token {
                    kind:           TokenType::StringLike(string_like_token_type),
                    contents:       token.contents,
                    start_position: token.start_position,
                }
            }),
            | PreprocessorTokenType::Character =>
                self.eval_escape_sequences(token).map(|contents| {
                    if contents.chars().take(2).count() != 1 {
                        self.pending_results.push_back(Err(
                            PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                                start_position: token.start_position,
                                contents:       token.contents,
                            }),
                        ));
                    }
                    let char = contents.chars().next().unwrap();
                    let string_like_token_type =
                        if get_from_cache!(self, token.contents).starts_with('L') {
                            StringLikeTokenType::WideChar(char)
                        } else {
                            StringLikeTokenType::Char(char)
                        };
                    Token {
                        kind:           TokenType::StringLike(string_like_token_type),
                        contents:       token.contents,
                        start_position: token.start_position,
                    }
                }),
            | PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined =>
                Ok(Self::build_token(
                    token,
                    match get_from_cache!(self, token.contents) {
                        | "auto" => TokenType::Keyword(KeywordTokenType::Auto),
                        | "break" => TokenType::Keyword(KeywordTokenType::Break),
                        | "case" => TokenType::Keyword(KeywordTokenType::Case),
                        | "char" => TokenType::Keyword(KeywordTokenType::Char),
                        | "const" => TokenType::Keyword(KeywordTokenType::Const),
                        | "continue" => TokenType::Keyword(KeywordTokenType::Continue),
                        | "default" => TokenType::Keyword(KeywordTokenType::Default),
                        | "do" => TokenType::Keyword(KeywordTokenType::Do),
                        | "double" => TokenType::Keyword(KeywordTokenType::Double),
                        | "else" => TokenType::Keyword(KeywordTokenType::Else),
                        | "enum" => TokenType::Keyword(KeywordTokenType::Enum),
                        | "extern" => TokenType::Keyword(KeywordTokenType::Extern),
                        | "float" => TokenType::Keyword(KeywordTokenType::Float),
                        | "for" => TokenType::Keyword(KeywordTokenType::For),
                        | "goto" => TokenType::Keyword(KeywordTokenType::Goto),
                        | "if" => TokenType::Keyword(KeywordTokenType::If),
                        | "inline" => TokenType::Keyword(KeywordTokenType::Inline),
                        | "int" => TokenType::Keyword(KeywordTokenType::Int),
                        | "long" => TokenType::Keyword(KeywordTokenType::Long),
                        | "register" => TokenType::Keyword(KeywordTokenType::Register),
                        | "restrict" => TokenType::Keyword(KeywordTokenType::Restrict),
                        | "return" => TokenType::Keyword(KeywordTokenType::Return),
                        | "short" => TokenType::Keyword(KeywordTokenType::Short),
                        | "signed" => TokenType::Keyword(KeywordTokenType::Signed),
                        | "sizeof" => TokenType::Keyword(KeywordTokenType::Sizeof),
                        | "static" => TokenType::Keyword(KeywordTokenType::Static),
                        | "struct" => TokenType::Keyword(KeywordTokenType::Struct),
                        | "switch" => TokenType::Keyword(KeywordTokenType::Switch),
                        | "typedef" => TokenType::Keyword(KeywordTokenType::Typedef),
                        | "union" => TokenType::Keyword(KeywordTokenType::Union),
                        | "unsigned" => TokenType::Keyword(KeywordTokenType::Unsigned),
                        | "void" => TokenType::Keyword(KeywordTokenType::Void),
                        | "volatile" => TokenType::Keyword(KeywordTokenType::Volatile),
                        | "while" => TokenType::Keyword(KeywordTokenType::While),
                        | "_Bool" => TokenType::Keyword(KeywordTokenType::Bool),
                        | "_Complex" => TokenType::Keyword(KeywordTokenType::Complex),
                        | "_Imaginary" => TokenType::Keyword(KeywordTokenType::Imaginary),
                        | _ => TokenType::Identifier,
                    },
                )),
            | PreprocessorTokenType::Plus =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Plus)),
            | PreprocessorTokenType::Minus =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Minus)),
            | PreprocessorTokenType::Asterisk => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::Asterisk,
            )),
            | PreprocessorTokenType::ForwardSlash => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ForwardSlash,
            )),
            | PreprocessorTokenType::Percent => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::Percent,
            )),
            | PreprocessorTokenType::LessThanLessThan => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::LessThanLessThan,
            )),
            | PreprocessorTokenType::GreaterThanGreaterThan => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::GreaterThanGreaterThan,
            )),
            | PreprocessorTokenType::LessThan => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::LessThan,
            )),
            | PreprocessorTokenType::LessThanEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::LessThanEquals,
            )),
            | PreprocessorTokenType::GreaterThan => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::GreaterThan,
            )),
            | PreprocessorTokenType::GreaterThanEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::GreaterThanEquals,
            )),
            | PreprocessorTokenType::EqualsEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::EqualsEquals,
            )),
            | PreprocessorTokenType::ExclamationMarkEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ExclamationMarkEquals,
            )),
            | PreprocessorTokenType::Ampersand => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::Ampersand,
            )),
            | PreprocessorTokenType::Caret =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Caret)),
            | PreprocessorTokenType::Pipe =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Pipe)),
            | PreprocessorTokenType::AmpersandAmpersand => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::AmpersandAmpersand,
            )),
            | PreprocessorTokenType::PipePipe => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::PipePipe,
            )),
            | PreprocessorTokenType::QuestionMark => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::QuestionMark,
            )),
            | PreprocessorTokenType::Colon =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Colon)),
            | PreprocessorTokenType::SemiColon => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::SemiColon,
            )),
            | PreprocessorTokenType::OpeningParenthesis => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::OpeningParenthesis,
            )),
            | PreprocessorTokenType::ClosingParenthesis => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ClosingParenthesis,
            )),
            | PreprocessorTokenType::OpeningSquareBracket => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::OpeningSquareBracket,
            )),
            | PreprocessorTokenType::ClosingSquareBracket => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ClosingSquareBracket,
            )),
            | PreprocessorTokenType::OpeningCurlyBrace => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::OpeningCurlyBrace,
            )),
            | PreprocessorTokenType::ClosingCurlyBrace => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ClosingCurlyBrace,
            )),
            | PreprocessorTokenType::Period =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Period)),
            | PreprocessorTokenType::Arrow =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Arrow)),
            | PreprocessorTokenType::PlusPlus => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::PlusPlus,
            )),
            | PreprocessorTokenType::MinusMinus => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::MinusMinus,
            )),
            | PreprocessorTokenType::AsteriskEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::AsteriskEquals,
            )),
            | PreprocessorTokenType::ForwardSlashEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ForwardSlashEquals,
            )),
            | PreprocessorTokenType::PercentEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::PercentEquals,
            )),
            | PreprocessorTokenType::PlusEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::PlusEquals,
            )),
            | PreprocessorTokenType::MinusEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::MinusEquals,
            )),
            | PreprocessorTokenType::LessThanLessThanEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::LessThanLessThanEquals,
            )),
            | PreprocessorTokenType::GreaterThanGreaterThanEquals => Ok(
                Self::build_operator_token(token, OperatorTokenType::GreaterThanGreaterThanEquals),
            ),
            | PreprocessorTokenType::AmpersandEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::AmpersandEquals,
            )),
            | PreprocessorTokenType::CaretEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::CaretEquals,
            )),
            | PreprocessorTokenType::PipeEquals => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::PipeEquals,
            )),
            | PreprocessorTokenType::Equals =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Equals)),
            | PreprocessorTokenType::Comma =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Comma)),
            | PreprocessorTokenType::Tilde =>
                Ok(Self::build_operator_token(token, OperatorTokenType::Tilde)),
            | PreprocessorTokenType::ExclamationMark => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::ExclamationMark,
            )),
            | PreprocessorTokenType::HashHash => {
                return Some(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HashHashUsedOutsideOfMacro,
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                )));
            },

            | x => todo!("Not implemented: {x:#?}"),
        })
    }

    fn parse_directive(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if !matches!(
            self.last_preprocessor_token.map(|p| p.kind),
            None | Some(PreprocessorTokenType::Newline)
        ) {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            ));
        }
        let directive = loop {
            match self.next_preprocessor_token_no_expand() {
                | Some(Err(e)) => {
                    self.pending_results
                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                    continue;
                },
                | Some(Ok(d)) => break d,
                | None => return Ok(()),
            }
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => return Ok(()),
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined | PreprocessorTokenType::Identifier => (),
            | _ =>
                return Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                        start_position: directive.start_position,
                        contents:       directive.contents,
                    },
                )),
        }
        match get_from_cache!(self, directive.contents) {
            | "if" => self.parse_if_directive(directive),
            | "ifdef" => self.parse_ifdef_directive(directive),
            | "ifndef" => self.parse_ifndef_directive(directive),
            | "elif" => self.parse_elif_directive(directive),
            | "else" => self.parse_else_directive(directive),
            | "endif" => self.parse_endif_directive(directive),
            | "include" => self.parse_include_directive(directive),
            | "define" => self.parse_define_directive(directive),
            | "undef" => self.parse_undef_directive(directive),
            | "line" => self.parse_line_directive(directive),
            | "error" => self.parse_error_directive(directive),
            | "pragma" => self.parse_pragma_directive(directive),
            | _ => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    start_position: directive.start_position,
                    contents:       directive.contents,
                },
            )),
        }
    }

    fn eval_preprocessor_expression(&mut self, expression: &PreprocessorExpression) -> bool {
        let result = self.eval_preprocessor_sub_expression(
            &expression.sub_expressions,
            expression.sub_expressions.len() - 1,
        );
        println!("Main sub expression returned {result}");
        result != 0
    }

    fn eval_preprocessor_sub_expression(
        &mut self,
        sub_expressions: &[PreprocessorSubExpression],
        current_index: usize,
    ) -> i128 {
        match &sub_expressions[current_index].kind {
            | PreprocessorSubExpressionKind::Atom(atom) => match atom.kind {
                | PreprocessorAtomKind::Character(c) => c as i128,
                | PreprocessorAtomKind::Number(i) => i,
                | PreprocessorAtomKind::Identifier(_) => 0,
            },
            | PreprocessorSubExpressionKind::Defined { name } =>
                i128::from(self.macro_definitions.contains_key(name)),
            | PreprocessorSubExpressionKind::UnaryOperator { operator, operand } => {
                let operand = self.eval_preprocessor_sub_expression(sub_expressions, *operand);
                match operator {
                    | PreprocessorTokenType::Minus => -operand,
                    | PreprocessorTokenType::Plus => operand,
                    | PreprocessorTokenType::ExclamationMark => i128::from(operand == 0),
                    | PreprocessorTokenType::Tilde => !operand,
                    | _ => unreachable!(),
                }
            },
            | PreprocessorSubExpressionKind::BinaryOperator {
                operator,
                left,
                right,
            } => {
                let left = self.eval_preprocessor_sub_expression(sub_expressions, *left);
                let right = self.eval_preprocessor_sub_expression(sub_expressions, *right);
                match operator {
                    | PreprocessorTokenType::Asterisk => left * right,
                    | PreprocessorTokenType::ForwardSlash => left / right,
                    | PreprocessorTokenType::Percent => left % right,
                    | PreprocessorTokenType::Plus => left + right,
                    | PreprocessorTokenType::Minus => left - right,
                    | PreprocessorTokenType::LessThanLessThan => left << right,
                    | PreprocessorTokenType::GreaterThanGreaterThan => left >> right,
                    | PreprocessorTokenType::LessThan => i128::from(left < right),
                    | PreprocessorTokenType::LessThanEquals => i128::from(left <= right),
                    | PreprocessorTokenType::GreaterThan => i128::from(left > right),
                    | PreprocessorTokenType::GreaterThanEquals => i128::from(left >= right),
                    | PreprocessorTokenType::EqualsEquals => i128::from(left == right),
                    | PreprocessorTokenType::ExclamationMarkEquals => i128::from(left != right),
                    | PreprocessorTokenType::Ampersand => left & right,
                    | PreprocessorTokenType::Caret => left ^ right,
                    | PreprocessorTokenType::Pipe => left | right,
                    | PreprocessorTokenType::AmpersandAmpersand =>
                        i128::from(left != 0 && right != 0),
                    | PreprocessorTokenType::PipePipe => i128::from(left != 0 || right != 0),
                    | _ => unreachable!(),
                }
            },
            | PreprocessorSubExpressionKind::TernaryOperator {
                condition,
                if_true,
                if_false,
            } => {
                let condition = self.eval_preprocessor_sub_expression(sub_expressions, *condition);
                if condition != 0 {
                    self.eval_preprocessor_sub_expression(sub_expressions, *if_true)
                } else {
                    self.eval_preprocessor_sub_expression(sub_expressions, *if_false)
                }
            },
        }
    }

    fn parse_preprocessor_expression(
        &mut self,
        directive: PreprocessorToken,
        no_condition_error_type: PreprocessorErrorType,
    ) -> Result<PreprocessorExpression, PreprocessorError<Prev::Error>> {
        let current_position = self.current_position();
        let mut expression = PreprocessorExpression {
            sub_expressions: Vec::new(),
            position:        current_position,
            length:          0,
        };
        match self.parse_preprocessor_sub_expression(u32::MAX, &mut expression.sub_expressions) {
            | Ok(Some(sub_expression)) => expression.sub_expressions.push(sub_expression),
            | Ok(None) =>
                return Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     no_condition_error_type,
                        start_position: directive.start_position,
                        contents:       directive.contents,
                    },
                )),
            | Err(e) => return Err(e),
        }
        expression.length = self
            .current_position()
            .index
            .saturating_sub(current_position.index);
        Ok(expression)
    }

    #[allow(clippy::similar_names)]
    fn parse_preprocessor_sub_expression(
        &mut self,
        max_binding_power: u32,
        sub_expressions: &mut Vec<PreprocessorSubExpression>,
    ) -> Result<Option<PreprocessorSubExpression>, PreprocessorError<Prev::Error>> {
        let lhs = 'lhs: loop {
            match self.next_preprocessor_token() {
                | Some(Err(e)) => self.pending_results.push_back(Err(e)),
                | Some(Ok(token)) => match token.kind {
                    | PreprocessorTokenType::OpeningParenthesis => {
                        let Some(lhs) =
                            self.parse_preprocessor_sub_expression(u32::MAX, sub_expressions)?
                        else {
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
                    | PreprocessorTokenType::Newline => return Ok(None),
                    | _ => {
                        let Some(((), r_bp)) = prefix_binding_power(token)? else {
                            let atom = self.parse_preprocessor_atom(token)?;

                            break PreprocessorSubExpression {
                                position: atom.position,
                                length:   atom.length,
                                kind:     PreprocessorSubExpressionKind::Atom(atom),
                            };
                        };
                        if token.kind == PreprocessorTokenType::Defined {
                            let position = self.current_position();
                            let name_or_paren = loop {
                                match self.next_preprocessor_token_no_expand() {
                                | Some(Err(e)) => {self.pending_results.push_back(Err(PreprocessorError::PreviousPhaseError(e))); continue},
                                | Some(Ok(token @ PreprocessorToken {kind: PreprocessorTokenType::OpeningParenthesis |PreprocessorTokenType::Identifier, ..})) =>
                                    break token,
                                | Some(Ok(token)) => {
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(token.kind),
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    ))
                                },
                                | None => {
                                    let contents = self.insert_into_cache("EOF");
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput("parsing defined directive"),
                                            start_position: position,
                                            contents,
                                        },
                                    ));
                                },
                            }
                            };
                            if name_or_paren.kind == PreprocessorTokenType::Identifier {
                                let name = name_or_paren.contents;
                                let length = name_or_paren.length;
                                let position = name_or_paren.start_position;
                                let kind = PreprocessorSubExpressionKind::Defined { name };
                                return Ok(Some(PreprocessorSubExpression {
                                    kind,
                                    position,
                                    length,
                                }));
                            }
                            let position = self.current_position();
                            let name = loop {
                                match self.next_preprocessor_token_no_expand() {
                                | Some(Err(e)) => {self.pending_results.push_back(Err(PreprocessorError::PreviousPhaseError(e))); continue},
                                | Some(Ok(token @ PreprocessorToken {kind: PreprocessorTokenType::Identifier, ..})) =>
                                    break token,
                                | Some(Ok(token)) => {
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                                                    token.kind,
                                                ),
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    ))
                                },
                                | None => {
                                    let contents = self.insert_into_cache("EOF");
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput("parsing defined directive"),
                                            start_position: position,
                                            contents,
                                        },
                                    ));
                                },
                            }
                            };
                            let closing_paren = loop {
                                match self.next_preprocessor_token_no_expand() {
                                | Some(Err(e)) => {self.pending_results.push_back(Err(PreprocessorError::PreviousPhaseError(e))); continue},
                                | Some(Ok(token @ PreprocessorToken {kind: PreprocessorTokenType::ClosingParenthesis, ..})) =>
                                    break token,
                                | Some(Ok(token)) => {
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(
                                                    token.kind,
                                                ),
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    ))
                                },
                                | None => {
                                    let contents = self.insert_into_cache("EOF");
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput("parsing defined directive"),
                                            start_position: position,
                                            contents,
                                        },
                                    ));
                                },
                            }
                            };
                            let length = closing_paren
                                .start_position
                                .index
                                .saturating_sub(position.index);
                            let kind = PreprocessorSubExpressionKind::Defined {
                                name: name.contents,
                            };
                            return Ok(Some(PreprocessorSubExpression {
                                kind,
                                position,
                                length,
                            }));
                        }
                        let Some(rhs) =
                            self.parse_preprocessor_sub_expression(r_bp, sub_expressions)?
                        else {
                            return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::NoExpressionAfterPrefixOperatorInPreprocessorExpression(
                                                token.kind,
                                            ),
                                        start_position: token.start_position,
                                        contents:       token.contents,
                                    },
                                ));
                        };
                        sub_expressions.push(rhs);
                        return Ok(Some(PreprocessorSubExpression {
                            position: token.start_position,
                            length:   self
                                .current_position()
                                .index
                                .saturating_sub(token.start_position.index),
                            kind:     PreprocessorSubExpressionKind::UnaryOperator {
                                operator: token.kind,
                                operand:  sub_expressions.len() - 1,
                            },
                        }));
                    },
                },
                | None => return Ok(None),
            }
        };
        let save_point = self.save();
        'outer: {
            let op = loop {
                match self.next_preprocessor_token() {
                    | Some(Err(e)) => {
                        self.pending_results.push_back(Err(e));
                        continue;
                    },
                    | Some(Ok(token)) => break token,
                    | None => return Ok(Some(lhs)),
                }
            };
            if let Some((l_bp, ())) = postfix_binding_power(op)? {
                if l_bp > max_binding_power {
                    break 'outer;
                }
                let lhs_index = sub_expressions.len();
                sub_expressions.push(lhs);
                return Ok(Some(PreprocessorSubExpression {
                    position: op.start_position,
                    length:   self
                        .current_position()
                        .index
                        .saturating_sub(op.start_position.index),
                    kind:     PreprocessorSubExpressionKind::UnaryOperator {
                        operator: op.kind,
                        operand:  lhs_index,
                    },
                }));
            }
            self.restore(save_point);
        }
        let save_point = self.save();
        'outer: {
            let op = loop {
                match self.next_preprocessor_token() {
                    | Some(Err(e)) => {
                        self.pending_results.push_back(Err(e));
                        continue;
                    },
                    | Some(Ok(token)) => break token,
                    | None => return Ok(Some(lhs)),
                }
            };

            if let Some((l_bp, r_bp)) = infix_binding_power(op)? {
                if l_bp > max_binding_power {
                    break 'outer;
                }
                let lhs_index = sub_expressions.len();
                sub_expressions.push(lhs);
                #[allow(clippy::redundant_else)]
                if op.kind == PreprocessorTokenType::QuestionMark {
                    let Some(mhs) =
                        self.parse_preprocessor_sub_expression(u32::MAX, sub_expressions)?
                    else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::NoExpressionAfterQuestionMarkInConditionalExpression,
                                    start_position: op.start_position,
                                    contents:       op.contents,
                                },
                            ));
                    };
                    match self.next_preprocessor_token() {
                        | Some(Err(e)) => self.pending_results.push_back(Err(e)),
                        | Some(Ok(token))
                            if token.kind == PreprocessorTokenType::Colon => (),
                        | Some(Ok(token)) => {
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::NoColonAfterQuestionMarkInConditionalExpression(token.kind),
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            ))
                        },
                        | None => {
                            let contents = self.insert_into_cache("EOF");
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::UnexpectedEndOfInput("parsing ternary expression"),
                                    start_position: mhs.position,
                                    contents,
                                },
                            ));
                        },
                    };

                    let mhs_index = sub_expressions.len();
                    sub_expressions.push(mhs);
                    let Some(rhs) =
                        self.parse_preprocessor_sub_expression(r_bp, sub_expressions)?
                    else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::NoExpressionAfterColonInConditionalExpression,
                                    start_position: op.start_position,
                                    contents:       op.contents,
                                },
                            ));
                    };
                    let rhs_index = sub_expressions.len();
                    sub_expressions.push(rhs);
                    return Ok(Some(PreprocessorSubExpression {
                        position: op.start_position,
                        length:   self
                            .current_position()
                            .index
                            .saturating_sub(op.start_position.index),
                        kind:     PreprocessorSubExpressionKind::TernaryOperator {
                            condition: lhs_index,
                            if_true:   mhs_index,
                            if_false:  rhs_index,
                        },
                    }));
                } else {
                    let Some(rhs) =
                        self.parse_preprocessor_sub_expression(r_bp, sub_expressions)?
                    else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::NoExpressionAfterInfixOperatorInPreprocessorExpression(
                                            op.kind,
                                        ),
                                    start_position: op.start_position,
                                    contents:       op.contents,
                                },
                            ));
                    };
                    let rhs_index = sub_expressions.len();
                    sub_expressions.push(rhs);
                    return Ok(Some(PreprocessorSubExpression {
                        position: op.start_position,
                        length:   self
                            .current_position()
                            .index
                            .saturating_sub(op.start_position.index),
                        kind:     PreprocessorSubExpressionKind::BinaryOperator {
                            operator: op.kind,
                            left:     lhs_index,
                            right:    rhs_index,
                        },
                    }));
                }
            }
            self.restore(save_point);
        }
        Ok(Some(lhs))
    }

    fn parse_preprocessor_atom(
        &mut self,
        token: PreprocessorToken,
    ) -> Result<PreprocessorAtom, PreprocessorError<Prev::Error>> {
        match token.kind {
            | PreprocessorTokenType::Identifier => {
                self.pending_results
                    .push_back(Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:
                                PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression,
                            start_position: token.start_position,
                            contents:       token.contents,
                        },
                    )));
                Ok(PreprocessorAtom {
                    position: token.start_position,
                    length:   self
                        .current_position()
                        .index
                        .saturating_sub(token.start_position.index),
                    kind:     PreprocessorAtomKind::Identifier(token.contents),
                })
            },
            | PreprocessorTokenType::Character => Ok(PreprocessorAtom {
                position: token.start_position,
                length:   self
                    .current_position()
                    .index
                    .saturating_sub(token.start_position.index),
                kind:     PreprocessorAtomKind::Character(
                    get_from_cache!(self, token.contents).char_at(1).unwrap(),
                ),
            }),
            | PreprocessorTokenType::Number => {
                let contents = get_from_cache!(self, token.contents).to_token_string();
                let token = match if contents.starts_with("0x") || contents.starts_with("0X") {
                    self.parse_hexadecimal_integer(token, &contents)
                } else if contents.starts_with("0b") || contents.starts_with("0B") {
                    self.parse_binary_integer(token, &contents)
                } else if contents.starts_with('0') {
                    self.parse_octal_integer(token, &contents)
                } else {
                    self.parse_decimal_integer(token, &contents)
                } {
                    | Ok(token) => token,
                    | Err(e) => return Err(e),
                };
                let Token {
                    kind: TokenType::Integer(i),
                    ..
                } = token
                else {
                    unreachable!()
                };

                let number = match i {
                    | IntegerTokenType::UnsignedLongLong(i) | IntegerTokenType::UnsignedLong(i) =>
                        i128::from(i),
                    | IntegerTokenType::LongLong(i) | IntegerTokenType::Long(i) => i128::from(i),
                    | IntegerTokenType::UnsignedInt(i) => i128::from(i),
                    | IntegerTokenType::Int(i) => i128::from(i),
                };
                Ok(PreprocessorAtom {
                    position: token.start_position,
                    length:   self
                        .current_position()
                        .index
                        .saturating_sub(token.start_position.index),
                    kind:     PreprocessorAtomKind::Number(number),
                })
            },
            | _ => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedAtomInPreprocessorExpression(
                        token.kind,
                    ),
                    start_position: token.start_position,
                    contents:       token.contents,
                },
            )),
        }
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

    fn skip_over_dead_code(&mut self) -> Result<(), PreprocessorError<Prev::Error>> {
        let start_balance = self.if_directive_balance;
        'outer: while start_balance <= self.if_directive_balance {
            println!(
                "Start balance: {start_balance}, current balance: {}",
                self.if_directive_balance
            );
            match self.next_preprocessor_token_no_expand() {
                // We don't care about syntax errors in code that's not being compiled.
                | Some(Err(_)) => continue,
                | Some(Ok(token)) => match token.kind {
                    | PreprocessorTokenType::Newline => {
                        loop {
                            match self.next_preprocessor_token_no_expand() {
                                | Some(Err(_)) => continue,
                                | Some(Ok(token)) if token.kind != PreprocessorTokenType::Hash =>
                                    continue 'outer,
                                | Some(Ok(_)) => break,
                                | None =>
                                    return Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing dead code. Expected #endif instead",
                                                ),
                                            start_position: token.start_position,
                                            contents:       token.contents,
                                        },
                                    )),
                            }
                        }
                        let directive_name = loop {
                            match self.next_preprocessor_token_no_expand() {
                                | Some(Err(_)) => continue,
                                | Some(Ok(token)) if token.kind == PreprocessorTokenType::Identifier => break token,
                                | Some(Ok(token)) => return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type: PreprocessorErrorType::ExpectedIdentifierInPreprocessorDirective(token.kind),
                                        start_position: token.start_position,
                                        contents: token.contents,
                                    },
                                )),
                                | None => return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing dead code. Expected #endif instead."),
                                        start_position: token.start_position,
                                        contents: token.contents,
                                    },
                                ))
                            }
                        };
                        match get_from_cache!(self, directive_name.contents) {
                            | "endif" => {
                                self.if_directive_balance -= 1;
                            },
                            | "if" | "ifndef" | "ifdef" => {
                                self.if_directive_balance += 1;
                            },
                            | "elif" if self.if_directive_balance == start_balance => {
                                let expression = self.parse_preprocessor_expression(
                                    directive_name,
                                    PreprocessorErrorType::NoConditionInElifDirective,
                                )?;
                                let result = self.eval_preprocessor_expression(&expression);
                                if result {
                                    break;
                                }
                            },
                            | "else" if self.if_directive_balance == start_balance => break,
                            | _ => (),
                        }
                    },
                    | _ => continue,
                },
                | None => return Ok(()),
            }
        }
        Ok(())
    }

    fn parse_if_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance += 1;
        let expression = self.parse_preprocessor_expression(
            directive,
            PreprocessorErrorType::NoConditionInIfDirective,
        )?;
        let result = self.eval_preprocessor_expression(&expression);
        if !result {
            self.skip_over_dead_code()?;
        }
        Ok(())
    }

    fn parse_elif_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if self.if_directive_balance <= 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ElifDirectiveWithoutIfDirective,
                    start_position: directive.start_position,
                    contents:       directive.contents,
                },
            ));
        }
        // Elif directives only matter if we are currently skipping over dead code.
        Ok(())
    }

    fn parse_else_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if self.if_directive_balance <= 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
                    start_position: directive.start_position,
                    contents:       directive.contents,
                },
            ));
        }
        // Else directives only matter if we are currently skipping over dead code.
        Ok(())
    }

    fn parse_endif_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance -= 1;
        if self.if_directive_balance < 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                    start_position: directive.start_position,
                    contents:       directive.contents,
                },
            ));
        }
        Ok(())
    }

    fn parse_ifdef_directive(
        &mut self,
        _directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance += 1;
        let name = self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Identifier,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, token| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(
                            token.kind,
                        ),
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            "parsing ifdef directive",
        )?;
        if !self.macro_definitions.contains_key(&name.contents) {
            self.skip_over_dead_code()?;
        }
        Ok(())
    }

    fn parse_ifndef_directive(
        &mut self,
        _directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance += 1;
        let name = self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Identifier,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, token| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(
                            token.kind,
                        ),
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            "parsing ifndef directive",
        )?;
        if self.macro_definitions.contains_key(&name.contents) {
            self.skip_over_dead_code()?;
        }
        Ok(())
    }

    fn search_for_header_in(path: &Path, dirs: &[PathBuf]) -> Option<PathBuf> {
        for dir in dirs {
            println!("Searching for header in {dir:#?}");
            let mut header_path = dir.clone();
            header_path.push(path);
            println!("Synthesized path: {header_path:#?}");
            if header_path.exists() {
                return Some(header_path);
            }
        }
        None
    }

    fn find_header_from_path(
        &mut self,
        include_token: PreprocessorToken,
        path: &Path,
        is_system_header: bool,
    ) -> Result<PathBuf, <Self as TranslationPhase>::Error> {
        println!(
            "Including {}header from path: {:#?}",
            if is_system_header { "system " } else { "" },
            path
        );
        if path.is_absolute() {
            if path.exists() {
                return Ok(path.to_owned());
            }
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::HeaderNotFound,
                    start_position: include_token.start_position,
                    contents:       include_token.contents,
                },
            ));
        }

        if is_system_header {
            if let Some(header) = Self::search_for_header_in(path, &self.system_include_directories)
            {
                return Ok(header);
            }
        }
        if let Some(header) = Self::search_for_header_in(path, &self.quote_include_directories) {
            return Ok(header);
        }

        let cwd = std::env::current_dir().map_err(|_| {
            PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                error_type:     PreprocessorErrorType::CurrentWorkingDirectoryInaccessible,
                start_position: include_token.start_position,
                contents:       include_token.contents,
            })
        })?;
        if let Some(header) = Self::search_for_header_in(path, &[cwd]) {
            return Ok(header);
        }

        Err(PreprocessorError::InnerPreprocessorError(
            InnerPreprocessorError {
                error_type:     PreprocessorErrorType::HeaderNotFound,
                start_position: include_token.start_position,
                contents:       include_token.contents,
            },
        ))
    }

    fn parse_include_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.previous_phase.set_is_tokenizing_include_string(true);
        let include_string = self.expect_token(
            |this, token| match token.kind {
                | PreprocessorTokenType::AngleBracketString
                | PreprocessorTokenType::IncludeString => true,
                | _ if get_from_cache!(this, token.contents).starts_with('<') => true,
                | _ => false,
            },
            |_, e| ControlFlow::Break(e),
            |_, token| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:
                            PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(
                                token.kind,
                            ),
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            "parsing include directive",
        )?;
        self.previous_phase.set_is_tokenizing_include_string(false);
        let header_path = match include_string.kind {
            | PreprocessorTokenType::IncludeString => {
                let contents = get_from_cache!(self, include_string.contents);
                let contents = &contents[1..contents.len() - 1].to_token_string();
                let path = Path::new(contents.as_str());
                self.find_header_from_path(include_string, path, false)
            },
            | PreprocessorTokenType::AngleBracketString => {
                let contents = get_from_cache!(self, include_string.contents);
                let contents = &contents[1..contents.len() - 1].to_token_string();
                let path = Path::new(contents.as_str());
                self.find_header_from_path(include_string, path, true)
            },
            | _ => {
                self.should_tokenize_whitespace = true;
                let mut contents = TokenString::new();
                let start_index = include_string.start_position.index;
                contents.push_str(&get_from_cache!(self, include_string.contents)[1..]);
                loop {
                    match self.next_preprocessor_token() {
                        | Some(Err(e)) => {
                            self.pending_results.push_back(Err(e));
                            continue;
                        },
                        | Some(Ok(token)) => {
                            if token.kind == PreprocessorTokenType::Newline {
                                break;
                            }
                            let token_contents = get_from_cache!(self, token.contents);
                            if let Some(idx) = token_contents.find('>') {
                                contents.push_str(&token_contents[..idx]);
                                break;
                            }
                            contents.push_str(get_from_cache!(self, token.contents));
                        },
                        | None => {
                            let contents = self.insert_into_cache("EOF");
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                        "parsing include directive",
                                    ),
                                    start_position: directive.start_position,
                                    contents,
                                },
                            ));
                        },
                    }
                }
                self.should_tokenize_whitespace = false;
                let path = Path::new(contents.as_str());
                let synthetic_token = PreprocessorToken {
                    start_position: include_string.start_position,
                    contents:       self.insert_into_cache(&contents),
                    kind:           PreprocessorTokenType::AngleBracketString,
                    length:         self.current_position().index.saturating_sub(start_index),
                };
                self.find_header_from_path(synthetic_token, path, true)
            },
        }?;
        let header_string = read_to_string_lossy(&header_path).map_err(|_| {
            PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                error_type:     PreprocessorErrorType::HeaderFileInaccessible,
                start_position: directive.start_position,
                contents:       directive.contents,
            })
        })?;
        let name = self.insert_into_cache(&header_path.to_string_lossy());
        self.tokenizer_stack.last_mut().unwrap().save_point = self.previous_phase.save();
        let save_point = Prev::SavePoint::from_input(Arc::from(header_string), name);
        let frame = TokenizerFrame {
            save_point: save_point.clone(),
            name,
            frame_type: TokenizerFrameType::SourceFile,
        };
        self.tokenizer_stack.push(frame);
        self.previous_phase.restore(save_point);
        Ok(())
    }

    fn parse_define_directive(
        &mut self,
        _directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        let name = self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Identifier,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, token| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                            token.kind,
                        ),
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ))
            },
            "parsing define directive",
        )?;
        let old_definition = self.macro_definitions.get(&name.contents).cloned();
        let save_point = self.previous_phase.save();
        let old_save_point = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike {
                    start_save_point, ..
                }
                | MacroDefinition::ObjectLike { start_save_point } =>
                    Some(start_save_point.clone()),
                | MacroDefinition::BuiltIn => {
                    self.pending_results.push_back(Err(
                        PreprocessorError::<Prev::Error>::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                                    get_from_cache!(self, name.contents).to_owned(),
                                ),
                                start_position: name.start_position,
                                contents:       name.contents,
                            },
                        ),
                    ));
                    None
                },
            },
        };
        let opening_paren = loop {
            match self.next_preprocessor_token_no_expand() {
                | Some(Err(e)) => {
                    self.pending_results
                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                    continue;
                },
                | Some(Ok(
                    token @ PreprocessorToken {
                        kind: PreprocessorTokenType::OpeningParenthesis,
                        ..
                    },
                )) => break Some(token),
                | Some(Ok(_)) => {
                    break None;
                },
                | None => {
                    let contents = self.insert_into_cache("EOF");
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                "parsing macro definition",
                            ),
                            start_position: name.start_position,
                            contents,
                        },
                    ));
                },
            }
        };
        if opening_paren.is_some() {
            if old_definition.is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                | MacroDefinition::FunctionLike { .. } => false,
                | MacroDefinition::BuiltIn =>
                    unreachable!("The case where name is a built-in macro is handled above"),
            }) {
                self.pending_results
                    .push_back(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:
                            PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                                get_from_cache!(self, name.contents).to_owned(),
                            ),
                        start_position: name.start_position,
                        contents:       name.contents,
                    },
                )));
            }
            let mut argument_names = Vec::new();
            let mut is_variadic = false;
            loop {
                let name_or_ellipsis = self.expect_token_no_expand(
                    |_, t| {
                        t.kind == PreprocessorTokenType::Identifier
                            || t.kind == PreprocessorTokenType::Ellipsis
                    },
                    |this, e| {
                        this.pending_results
                            .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                        ControlFlow::Continue(())
                    },
                    |_, token| {
                        ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(
                                        token.kind,
                                    ),
                                start_position: token.start_position,
                                contents:       token.contents,
                            },
                        ))
                    },
                    "parsing macro definition",
                )?;
                if is_variadic {
                    self.pending_results
                        .push_back(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                                        get_from_cache!(self, name.contents).to_owned(),
                                    ),
                                start_position: name.start_position,
                                contents:       name.contents,
                            },
                        )));
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else {
                    argument_names.push(name_or_ellipsis.contents);
                }

                let comma_or_closing_parent = self.expect_token_no_expand(
                    |_, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis,
                    |this, e| {
                        this.pending_results
                            .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                        ControlFlow::Continue(())
                    },
                    |_, token| {
                        ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                start_position: token.start_position,
                                contents:       token.contents,
                            },
                        ))
                    },
                    "parsing macro definition",
                )?;
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            let save_point = self.previous_phase.save();
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::FunctionLike {
                    start_save_point: save_point,
                    argument_names: argument_names.into(),
                    is_variadic,
                },
            ));
        } else {
            if old_definition.is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => false,
                | MacroDefinition::FunctionLike { .. } => true,
                | MacroDefinition::BuiltIn =>
                    unreachable!("The case where name is a built-in macro is handled above"),
            }) {
                self.pending_results
                    .push_back(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:
                            PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                                get_from_cache!(self, name.contents).to_owned(),
                            ),
                        start_position: name.start_position,
                        contents:       name.contents,
                    },
                )));
            }
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::ObjectLike {
                    start_save_point: save_point.clone(),
                },
            ));
        }
        if let Some(mut old_save_point) = old_save_point {
            let mut new_save_point = save_point;
            loop {
                self.previous_phase.restore(old_save_point);
                let old_next = loop {
                    match self.next_preprocessor_token_no_expand() {
                        | Some(Err(_)) => {
                            // Don't generate errors while validating macro definitions.
                            continue;
                        },
                        | Some(Ok(token)) => break Some(token),
                        | None => break None,
                    }
                };
                old_save_point = self.previous_phase.save();
                self.previous_phase.restore(new_save_point);
                let new_next = loop {
                    match self.next_preprocessor_token_no_expand() {
                        | Some(Err(_)) => {
                            // Don't generate errors while validating macro definitions.
                            continue;
                        },
                        | Some(Ok(token)) => break Some(token),
                        | None => break None,
                    }
                };
                new_save_point = self.previous_phase.save();
                if old_next != new_next {
                    self.pending_results
                        .push_back(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                        get_from_cache!(self, name.contents).to_owned(),
                                    ),
                                start_position: name.start_position,
                                contents:       name.contents,
                            },
                        )));
                }
                if !new_next.is_some_and(|t| t.kind != PreprocessorTokenType::Newline) {
                    break;
                }
            }
        } else {
            loop {
                match self.next_preprocessor_token_no_expand() {
                    | Some(Err(_)) => {
                        // Don't generate errors while validating macro definitions.
                        continue;
                    },
                    | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(Ok(_)) => continue,
                }
            }
        }
        Ok(())
    }

    fn parse_undef_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_line_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_error_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        todo!()
    }

    fn parse_pragma_directive(
        &mut self,
        directive: PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        todo!()
    }

    #[allow(clippy::inline_always)]
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
                        error_type:     PreprocessorErrorType::ForcedUnsignedPromotion {
                            from: UnsignedIntegerLiteralType::UnsignedInt,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
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
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn parse_float(
        &mut self,
        invalid_float_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
        contents: &mut TokenString,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        _ = self;
        contents.push('\0');
        let res = match contents.char_at(contents.len() - 2) {
            | Some('f' | 'F') => string_to_float(contents.as_str()).map(FloatTokenType::Float),
            | Some('l' | 'L') =>
                string_to_long_double(contents.as_str()).map(FloatTokenType::LongDouble),
            | _ => string_to_double(contents.as_str()).map(FloatTokenType::Double),
        };
        match res {
            | Ok(kind) => Ok(Token {
                kind:           TokenType::Float(kind),
                start_position: token.start_position,
                contents:       token.contents,
            }),
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
        contents: &mut TokenString,
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
        contents: &mut TokenString,
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
        + AsRef<StringCache>
        + IsTokenizingIncludeString,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone + PartialEq,
    Prev::SavePoint: FromInput,
{
    type Item = Result<Token, PreprocessorError<Prev::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(result) = self.pending_results.pop_front() {
                return Some(result);
            }
            if self.state == State::Done {
                return None;
            }
            let token = match self.next_preprocessor_token() {
                | Some(Ok(token)) => token,
                | Some(Err(e)) => return Some(Err(e)),
                | None => {
                    self.state = State::Done;
                    if self.if_directive_balance != 0 {
                        return Some(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                                start_position: self.current_position(),
                                contents:       self.insert_into_cache("EOF"),
                            },
                        )));
                    }
                    return None;
                },
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
        + AsRef<StringCache>
        + IsTokenizingIncludeString,
    PrevPrevError: GetPosition + GetSeverity + std::error::Error + Clone + PartialEq,
    Prev::SavePoint: FromInput,
{
    type Error = PreprocessorError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint, Prev::Error>;
    type Yield = Token;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
            tokenizer_stack: self.tokenizer_stack.clone(),
            pending_results: self.pending_results.clone(),
            state: self.state,
            last_preprocessor_token: self.last_preprocessor_token,
            current_preprocessor_token: self.current_preprocessor_token,
            macro_definitions: self.macro_definitions.clone(),
            if_directive_balance: self.if_directive_balance,
            should_tokenize_whitespace: self.should_tokenize_whitespace,
            quote_include_directories: self.quote_include_directories.clone(),
            system_include_directories: self.system_include_directories.clone(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.tokenizer_stack = save_point.tokenizer_stack;
        self.pending_results = save_point.pending_results;
        self.state = save_point.state;
        self.last_preprocessor_token = save_point.last_preprocessor_token;
        self.current_preprocessor_token = save_point.current_preprocessor_token;
        self.macro_definitions = save_point.macro_definitions;
        self.if_directive_balance = save_point.if_directive_balance;
        self.should_tokenize_whitespace = save_point.should_tokenize_whitespace;
        self.quote_include_directories = save_point.quote_include_directories;
        self.system_include_directories = save_point.system_include_directories;
    }

    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}
