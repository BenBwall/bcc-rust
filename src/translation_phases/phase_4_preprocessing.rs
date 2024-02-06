use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    mem::replace,
    ops::ControlFlow,
    path::{
        Path,
        PathBuf,
    },
    sync::Arc,
};

use chrono::Local;
use smallstr::SmallString;
use smallvec::SmallVec;
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
        shared::{
            SharedString,
            SharedVec,
        },
        string_cache::{
            Id as StringCacheId,
            StringCache,
        },
        unlikely,
    },
    HashMap,
    HashSet,
};

pub(crate) trait FromInput: ISavePoint {
    fn from_input(input: SharedString, source_file: StringCacheId) -> Self;
}

impl FromInput for Pptsp {
    fn from_input(input: SharedString, source_file: StringCacheId) -> Self {
        let inner = RemoveEscapedNewlinesSavePoint {
            inner:            MapCharacterSetsSavePoint {
                inner: NewlineTrackingSavePoint {
                    source: input,
                    position: SourcePosition {
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
    SavePoint as ISavePoint,
    SourcePosition,
    SourceVector,
    SourceVectors,
    TranslationPhase,
};

pub(crate) type TokenString = SmallString<[u8; 1024]>;

const PREDEFINED_MACRO_NAMES: [&str; 5] =
    ["__LINE__", "__FILE__", "__DATE__", "__TIME__", "_Pragma"];

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType<PrevSavePoint> {
    SourceFile,
    ObjectLikeMacroInvocation {
        hash_hash_positions: Arc<HashSet<SourcePosition>>,
    },
    FunctionLikeMacroInvocation {
        arguments:           Arc<HashMap<StringCacheId, FunctionLikeMacroArgument<PrevSavePoint>>>,
        hash_hash_positions: Arc<HashSet<SourcePosition>>,
        is_variadic:         bool,
    },
    FunctionLikeMacroArgument {
        argument:            FunctionLikeMacroArgument<PrevSavePoint>,
        paren_depth:         usize,
        has_generated_token: bool,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition<PrevSavePoint> {
    ObjectLike {
        start_save_point:    PrevSavePoint,
        hash_hash_positions: Arc<HashSet<SourcePosition>>,
    },
    FunctionLike {
        argument_names:      Arc<[StringCacheId]>,
        start_save_point:    PrevSavePoint,
        is_variadic:         bool,
        hash_hash_positions: Arc<HashSet<SourcePosition>>,
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
pub(crate) enum HashHash {
    Lhs(PreprocessorToken),
    Rhs(PreprocessorToken),
    Empty,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Preprocessor<Prev, PrevError, PrevSavePoint> {
    pub(crate) previous_phase: Prev,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    once_set: HashSet<StringCacheId>,
    macro_definitions: HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
    pending_results: VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    state: State,
    last_preprocessor_token: Option<PreprocessorToken>,
    current_preprocessor_token: Option<PreprocessorToken>,
    if_directive_balance: isize,
    should_tokenize_whitespace: bool,
    generate_placeholders: bool,
    quote_include_directories: SharedVec<PathBuf>,
    system_include_directories: SharedVec<PathBuf>,
    expression_parser: PreprocessorExpressionParser,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SavePoint<PrevSavePoint, PrevError> {
    pub(crate) inner: PrevSavePoint,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    pub(crate) once_set: HashSet<StringCacheId>,
    pub(crate) pending_results: VecDeque<Result<Token, PreprocessorError<PrevError>>>,
    pub(crate) state: State,
    pub(crate) last_preprocessor_token: Option<PreprocessorToken>,
    pub(crate) current_preprocessor_token: Option<PreprocessorToken>,
    pub(crate) macro_definitions: HashMap<StringCacheId, MacroDefinition<PrevSavePoint>>,
    pub(crate) if_directive_balance: isize,
    pub(crate) should_tokenize_whitespace: bool,
    pub(crate) generate_placeholders: bool,
    pub(crate) quote_include_directories: SharedVec<PathBuf>,
    pub(crate) system_include_directories: SharedVec<PathBuf>,
    pub(crate) expression_parser: PreprocessorExpressionParser,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionParserState {
    Unary,
    Binary,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperator {
    // Unary
    UnaryPlus,
    UnaryMinus,
    BitwiseNot,
    LogicalNot,

    // Binary
    BinaryPlus,
    BinaryMinus,
    Multiply,
    Divide,
    Modulo,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    Equals,
    NotEquals,
    LeftShift,
    RightShift,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    Ternary,
    Else,

    // Grouping
    OpeningParenthesis,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionAssociativity {
    Left,
    Right,
}

impl PreprocessorExpressionOperator {
    fn precedence(self) -> u32 {
        match self {
            // Based on https://en.cppreference.com/w/c/language/operator_precedence.
            | Self::UnaryPlus | Self::UnaryMinus | Self::BitwiseNot | Self::LogicalNot => 2,
            | Self::Multiply | Self::Divide | Self::Modulo => 3,
            | Self::BinaryPlus | Self::BinaryMinus => 4,
            | Self::LeftShift | Self::RightShift => 5,
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals => 6,
            | Self::Equals | Self::NotEquals => 7,
            | Self::BitwiseAnd => 8,
            | Self::BitwiseXor => 9,
            | Self::BitwiseOr => 10,
            | Self::LogicalAnd => 11,
            | Self::LogicalOr => 12,
            | Self::Ternary | Self::Else => 13,
            // OpeningParenthesis is not a normal operator. We only pop it off the stack when we
            // encounter a closing parenthesis.
            | Self::OpeningParenthesis => u32::MAX,
        }
    }

    fn associativity(self) -> PreprocessorExpressionAssociativity {
        match self {
            | Self::Ternary
            | Self::Else
            | Self::UnaryPlus
            | Self::UnaryMinus
            | Self::BitwiseNot
            | Self::LogicalNot => PreprocessorExpressionAssociativity::Right,
            | Self::BinaryMinus
            | Self::BinaryPlus
            | Self::Multiply
            | Self::Divide
            | Self::Modulo
            | Self::LeftShift
            | Self::RightShift
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals
            | Self::Equals
            | Self::NotEquals
            | Self::BitwiseAnd
            | Self::BitwiseOr
            | Self::BitwiseXor
            | Self::LogicalAnd
            | Self::LogicalOr
            | Self::OpeningParenthesis => PreprocessorExpressionAssociativity::Left,
        }
    }

    fn has_precedence_over(self, other: Self) -> bool {
        match self.associativity() {
            | PreprocessorExpressionAssociativity::Left => self.precedence() <= other.precedence(),
            | PreprocessorExpressionAssociativity::Right => self.precedence() < other.precedence(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorExpressionParser {
    operator_stack: Vec<PreprocessorExpressionOperator>,
    operand_stack:  Vec<i128>,
    state:          PreprocessorExpressionParserState,
}

impl PreprocessorExpressionParser {
    fn new() -> Self {
        Self {
            operator_stack: Vec::new(),
            operand_stack:  Vec::new(),
            state:          PreprocessorExpressionParserState::Unary,
        }
    }

    fn reset(&mut self) {
        self.operator_stack.clear();
        self.operand_stack.clear();
        self.state = PreprocessorExpressionParserState::Unary;
    }
}

impl<PrevSavePoint, PrevError> super::SavePoint for SavePoint<PrevSavePoint, PrevError>
where
    PrevSavePoint: super::SavePoint,
    PrevError: Debug + Clone,
{
    fn current_position(&self) -> SourcePosition {
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
    pub(crate) source_vectors: SourceVectors,
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

impl From<IntegerTokenType> for i128 {
    fn from(v: IntegerTokenType) -> Self {
        match v {
            | IntegerTokenType::UnsignedLong(v) | IntegerTokenType::UnsignedLongLong(v) =>
                i128::from(v),
            | IntegerTokenType::Long(v) | IntegerTokenType::LongLong(v) => i128::from(v),
            | IntegerTokenType::UnsignedInt(v) => i128::from(v),
            | IntegerTokenType::Int(v) => i128::from(v),
        }
    }
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
    Ellipsis,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringTokenType {
    String(StringCacheId),
    WideString(StringCacheId),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum CharacterTokenType {
    Char(char),
    WideChar(char),
}

impl From<CharacterTokenType> for char {
    fn from(v: CharacterTokenType) -> Self {
        match v {
            | CharacterTokenType::Char(c) | CharacterTokenType::WideChar(c) => c,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Identifier,
    Keyword(KeywordTokenType),
    Operator(OperatorTokenType),
    String(StringTokenType),
    Character(CharacterTokenType),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct InnerPreprocessorError {
    pub(crate) error_type:     PreprocessorErrorType,
    pub(crate) source_vectors: SourceVectors,
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
            | PreprocessorErrorType::ExpectedIdentifierInPreprocessorDirective(..)
            | PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives
            | PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives
            | PreprocessorErrorType::ElifDirectiveWithoutIfDirective
            | PreprocessorErrorType::ElseDirectiveWithoutIfDirective
            | PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInDefineDirective(..)
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
            | PreprocessorErrorType::FloatingPointNumberInLineDirective(..)
            | PreprocessorErrorType::NegativeNumberInLineDirective(..)
            | PreprocessorErrorType::UnsupportedLineDirectiveValue(..)
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
            | PreprocessorErrorType::TernaryOperatorWithoutMhs
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
            | PreprocessorErrorType::ErrorDirective(..)
             => ErrorSeverity::Error,
            | PreprocessorErrorType::RedefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(..)
            | PreprocessorErrorType::FloatLiteralOverflow(..)
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
            | PreprocessorErrorType::PragmaOnceInNonHeader => ErrorSeverity::Warning,
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
    fn position(&self) -> SourcePosition {
        match self {
            | Self::PreviousPhaseError(e) => e.position(),
            | Self::InnerPreprocessorError(e) => e.source_vectors[0].position,
        }
    }
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
    TernaryOperatorWithoutRhs,
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
    FloatInsteadOfIntegerInPreprocessorExpression,
    ExpectedBinaryOperatorInPreprocessorExpression,
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
    UndefinedIdentifierInPreprocessorExpression(String),
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
    ExpectedIdentifierInUndefDirective(PreprocessorTokenType),
    ExpectedNewlineAfterUndefDirective(PreprocessorTokenType),
    HashOperatorMustBeFollowedByAMacroArgument(PreprocessorTokenType),
    IdentifierNotMacroArgumentAfterHashOperator(String),
    MissingRightHandSideOfHashHashOperator,
    MissingLeftHandSideOfHashHashOperator,
    TokenMergingError(String, String),
    MissingNumberInLineDirective(PreprocessorTokenType),
    MissingNewlineAfterLineDirective(PreprocessorTokenType),
    FloatingPointNumberInLineDirective(FloatTokenType),
    LineDirectiveIsNotASimpleDigitSequence,
    UnsupportedLineDirectiveValue(i128),
    NegativeNumberInLineDirective(i128),
    LineDirectiveNumberTooLarge(i128),
    MissingOpeningParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingClosingParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingStringLiteralInPragmaOperator(PreprocessorTokenType),
    UnknownPragmaDirective,
    UnknownPragmaSTDCArgument(String),
    ExtraTokensAfterPragmaOnce(PreprocessorTokenType),
    ExtraTokensAfterPragmaOperator,
    STDCPragmaDirectiveWithoutArgument,
    STDCPragmaDirectiveWithoutOnOffSwitch,
    MissingOnOffSwitchInSTDCPragma(String),
    PragmaOnceInNonHeader,
    ErrorDirective(String),
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
            | Self::EmptyParenthesesInPreprocessorExpression => {
                write!(
                    f,
                    "Parentheses containing no expression that are not part of a macro invocation \
                     are not allowed in preprocessor constant expressions!"
                )
            },
            | Self::MissingOpeningParenthesisOrIdentifierInDefinedDirective(tt) => {
                write!(
                    f,
                    "Missing opening parenthesis or identifier in 'defined' directive! Found \
                     instead {tt:#?}"
                )
            },
            | Self::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(operator) => {
                write!(
                    f,
                    "Expected right-hand side of binary operator '{}' in preprocessor constant \
                     expression!",
                    match operator {
                        | PreprocessorExpressionOperator::BinaryPlus => "+",
                        | PreprocessorExpressionOperator::BinaryMinus => "-",
                        | PreprocessorExpressionOperator::Multiply => "*",
                        | PreprocessorExpressionOperator::Divide => "/",
                        | PreprocessorExpressionOperator::Modulo => "%",
                        | PreprocessorExpressionOperator::LeftShift => "<<",
                        | PreprocessorExpressionOperator::RightShift => ">>",
                        | PreprocessorExpressionOperator::LessThan => "<",
                        | PreprocessorExpressionOperator::LessThanEquals => "<=",
                        | PreprocessorExpressionOperator::GreaterThan => ">",
                        | PreprocessorExpressionOperator::GreaterThanEquals => ">=",
                        | PreprocessorExpressionOperator::Equals => "==",
                        | PreprocessorExpressionOperator::NotEquals => "!=",
                        | PreprocessorExpressionOperator::BitwiseAnd => "&",
                        | PreprocessorExpressionOperator::BitwiseXor => "^",
                        | PreprocessorExpressionOperator::BitwiseOr => "|",
                        | PreprocessorExpressionOperator::LogicalAnd => "&&",
                        | PreprocessorExpressionOperator::LogicalOr => "||",
                        | PreprocessorExpressionOperator::Ternary => "?",
                        | PreprocessorExpressionOperator::Else => ":",
                        | _ => unreachable!(),
                    }
                )
            },
            | Self::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Exclamation mark '!' instead of binary operator in preprocessor constant \
                     expression!"
                )
            },
            | Self::TildeInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Tilde '~' instead of binary operator in preprocessor constant expression!"
                )
            },
            | Self::NumberInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Number instead of binary operator in preprocessor constant expression!"
                )
            },
            | Self::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Identifier instead of binary operator in preprocessor constant expression!"
                )
            },
            | Self::CharacterInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Character instead of binary operator in preprocessor constant expression!"
                )
            },
            | Self::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "'defined' operator instead of binary operator in preprocessor constant \
                     expression!"
                )
            },
            | Self::ExpectedBinaryOperatorInPreprocessorExpression => {
                write!(
                    f,
                    "Expected binary operator in preprocessor constant expression!"
                )
            },
            | Self::UnterminatedOpeningParenthesisInPreprocessorExpression => {
                write!(
                    f,
                    "Unterminated opening parenthesis in preprocessor constant expression!"
                )
            },
            | Self::AddressOfOperatorNotSupportedInPreprocessorExpression => {
                write!(
                    f,
                    "Address-of operator '&' is not supported in preprocessor constant \
                     expressions!"
                )
            },
            | Self::DereferenceOperatorNotSupportedInPreprocessorExpression => {
                write!(
                    f,
                    "Dereference operator '*' is not supported in preprocessor constant \
                     expressions!"
                )
            },
            | Self::FunctionCallOperatorNotSupportedInPreprocessorExpression => {
                write!(
                    f,
                    "Function call operator '()' is not supported in preprocessor constant \
                     expressions!"
                )
            },
            | Self::BinaryPlusOverflow => {
                write!(
                    f,
                    "Overflow while trying to add two operands in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::BinaryMinusOverflow => {
                write!(
                    f,
                    "Overflow while trying to subtract two operands in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::DivideOverflow => {
                write!(
                    f,
                    "Overflow while trying to divide two operands in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::DivideByZero => {
                write!(
                    f,
                    "Division by zero in preprocessor constant expression! Division by zero is \
                     not allowed in preprocessor constant expressions."
                )
            },
            | Self::ModuloOverflow => {
                write!(
                    f,
                    "Overflow while trying to calculate the remainder of two operands in \
                     preprocessor constant expression! BCC uses 128-bit integers for evaluating \
                     preprocessor expressions."
                )
            },
            | Self::ModuloByZero => {
                write!(
                    f,
                    "Modulo by zero in preprocessor constant expression! Modulo by zero is not \
                     allowed in preprocessor constant expressions."
                )
            },
            | Self::MultiplyOverflow => {
                write!(
                    f,
                    "Overflow while trying to multiply two operands in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::UnaryMinusOverflow => {
                write!(
                    f,
                    "Overflow while trying to negate an operand in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::LeftShiftOverflow => {
                write!(
                    f,
                    "Overflow while trying to left shift an operand in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::RightShiftOverflow => {
                write!(
                    f,
                    "Overflow while trying to right shift an operand in preprocessor constant \
                     expression! BCC uses 128-bit integers for evaluating preprocessor constant \
                     expressions."
                )
            },
            | Self::BitwiseNotWithoutOperand => {
                write!(
                    f,
                    "Bitwise NOT operator '~' without operand in preprocessor constant expression!"
                )
            },
            | Self::LogicalNotWithoutOperand => {
                write!(
                    f,
                    "Logical NOT operator '!' without operand in preprocessor constant expression!"
                )
            },
            | Self::UnaryMinusWithoutOperand => {
                write!(
                    f,
                    "Unary minus operator '-' without operand in preprocessor constant expression!"
                )
            },
            | Self::UnaryPlusWithoutOperand => {
                write!(
                    f,
                    "Unary plus operator '+' without operand in preprocessor constant expression!"
                )
            },
            | Self::BinaryPlusWithoutRhs => {
                write!(
                    f,
                    "Binary plus operator '+' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::BinaryMinusWithoutRhs => {
                write!(
                    f,
                    "Binary minus operator '-' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::MultiplyWithoutRhs => {
                write!(
                    f,
                    "Multiply operator '*' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::DivideWithoutRhs => {
                write!(
                    f,
                    "Divide operator '/' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::ModuloWithoutRhs => {
                write!(
                    f,
                    "Modulo operator '%' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::LessThanWithoutRhs => {
                write!(
                    f,
                    "Less-than operator '<' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::LessThanEqualsWithoutRhs => {
                write!(
                    f,
                    "Less-than-or-equals operator '<=' without right-hand side in preprocessor \
                     constant expression!"
                )
            },
            | Self::GreaterThanWithoutRhs => {
                write!(
                    f,
                    "Greater-than operator '>' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::GreaterThanEqualsWithoutRhs => {
                write!(
                    f,
                    "Greater-than-or-equals operator '>=' without right-hand side in preprocessor \
                     constant expression!"
                )
            },
            | Self::EqualsWithoutRhs => {
                write!(
                    f,
                    "Equals operator '==' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::NotEqualsWithoutRhs => {
                write!(
                    f,
                    "Not-equals operator '!=' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::LeftShiftWithoutRhs => {
                write!(
                    f,
                    "Left-shift operator '<<' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::RightShiftWithoutRhs => {
                write!(
                    f,
                    "Right-shift operator '>>' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::BitwiseAndWithoutRhs => {
                write!(
                    f,
                    "Bitwise AND operator '&' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::BitwiseXorWithoutRhs => {
                write!(
                    f,
                    "Bitwise XOR operator '^' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::BitwiseOrWithoutRhs => {
                write!(
                    f,
                    "Bitwise OR operator '|' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::LogicalAndWithoutRhs => {
                write!(
                    f,
                    "Logical AND operator '&&' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::LogicalOrWithoutRhs => {
                write!(
                    f,
                    "Logical OR operator '||' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::TernaryOperatorWithoutMhs => {
                write!(
                    f,
                    "Ternary operator '?' without middle-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::TernaryOperatorWithoutRhs => {
                write!(
                    f,
                    "Ternary operator ':' without right-hand side in preprocessor constant \
                     expression!"
                )
            },
            | Self::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(operator) => {
                write!(
                    f,
                    "Binary operator '{}' instead of unary expression in preprocessor constant \
                     expression!",
                    match operator {
                        | PreprocessorExpressionOperator::Multiply => "*",
                        | PreprocessorExpressionOperator::Divide => "/",
                        | PreprocessorExpressionOperator::Modulo => "%",
                        | PreprocessorExpressionOperator::LessThan => "<",
                        | PreprocessorExpressionOperator::LessThanEquals => "<=",
                        | PreprocessorExpressionOperator::GreaterThan => ">",
                        | PreprocessorExpressionOperator::GreaterThanEquals => ">=",
                        | PreprocessorExpressionOperator::Equals => "==",
                        | PreprocessorExpressionOperator::NotEquals => "!=",
                        | PreprocessorExpressionOperator::LeftShift => "<<",
                        | PreprocessorExpressionOperator::RightShift => ">>",
                        | PreprocessorExpressionOperator::BitwiseAnd => "&",
                        | PreprocessorExpressionOperator::BitwiseXor => "^",
                        | PreprocessorExpressionOperator::BitwiseOr => "|",
                        | PreprocessorExpressionOperator::LogicalAnd => "&&",
                        | PreprocessorExpressionOperator::LogicalOr => "||",
                        | PreprocessorExpressionOperator::Ternary => "?",
                        | _ => unreachable!(),
                    }
                )
            },
            | Self::FloatInsteadOfIntegerInPreprocessorExpression => {
                write!(
                    f,
                    "Floating point literal instead of integer literal in preprocessor constant \
                     expression! Floating point literals are not allowed in preprocessor constant \
                     expressions."
                )
            },
            | Self::UnexpectedTokenInPreprocessorExpression(tt) => {
                write!(
                    f,
                    "Unexpected token {tt:#?} in preprocessor constant expression!",
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
            | Self::UndefinedIdentifierInPreprocessorExpression(ident) => {
                write!(
                    f,
                    "Undefined identifier '{ident}' in preprocessor constant expression! Will be \
                     treated as if it had a value of 0."
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
            | Self::ExpectedIdentifierInUndefDirective(tt) => {
                write!(
                    f,
                    "Expected identifier in 'undef' directive! Found instead {tt:#?}"
                )
            },
            | Self::ExpectedNewlineAfterUndefDirective(tt) => {
                write!(
                    f,
                    "Expected newline after 'undef' directive! Found instead {tt:#?}"
                )
            },
            | Self::HashOperatorMustBeFollowedByAMacroArgument(tt) => {
                write!(
                    f,
                    "'#' operator, when used in a function-like macro body, must be followed by a \
                     macro argument! Found instead {tt:#?}"
                )
            },
            | Self::IdentifierNotMacroArgumentAfterHashOperator(name) => {
                write!(
                    f,
                    "Identifier '{name}' not a macro argument after '#' operator! The '#' \
                     operator, when used in a function-like macro body, must be followed by a \
                     macro argument."
                )
            },
            | Self::MissingRightHandSideOfHashHashOperator => {
                write!(
                    f,
                    "Missing right hand side of '##' operator! The '##' operator must be followed \
                     by a macro argument or a token inside of the macro body."
                )
            },
            | Self::MissingLeftHandSideOfHashHashOperator => {
                write!(
                    f,
                    "Missing left hand side of '##' operator! The '##' operator must be preceded \
                     by a macro argument or a token inside of the macro body."
                )
            },
            | Self::TokenMergingError(lhs, rhs) => {
                write!(
                    f,
                    "Error while merging tokens! Could not merge '{lhs}' and '{rhs}'"
                )
            },
            | Self::MissingNumberInLineDirective(tt) => {
                write!(
                    f,
                    "Missing number in 'line' directive! Found instead {tt:#?}"
                )
            },
            | Self::MissingNewlineAfterLineDirective(tt) => {
                write!(
                    f,
                    "Expected newline after 'line' directive! Found instead {tt:#?}"
                )
            },
            | Self::FloatingPointNumberInLineDirective(float_token_type) => {
                write!(
                    f,
                    "Expected an integer in 'line' directive! Found instead {} {}",
                    match float_token_type {
                        | FloatTokenType::Float(_) => "floating point number",
                        | FloatTokenType::Double(_) => "double precision floating point number",
                        | FloatTokenType::LongDouble(_) =>
                            "long double precision floating point number",
                    },
                    match float_token_type {
                        | FloatTokenType::Float(f) => f.to_string(),
                        | FloatTokenType::Double(f) => f.to_string(),
                        | FloatTokenType::LongDouble(f) => f.to_string(),
                    }
                )
            },
            | Self::LineDirectiveIsNotASimpleDigitSequence => {
                write!(
                    f,
                    "The 'line' directive must be followed by a simple digit sequence according \
                     to the C standard!"
                )
            },
            | Self::UnsupportedLineDirectiveValue(i) => {
                write!(
                    f,
                    "Unsupported value in 'line' directive! BCC only supports line numbers up to \
                     SIZE_MAX. Value was '{i}'"
                )
            },
            | Self::NegativeNumberInLineDirective(i) => {
                write!(
                    f,
                    "Negative number in 'line' directive! The 'line' directive must be followed \
                     by a positive integer! Value was '{i}'"
                )
            },
            | Self::LineDirectiveNumberTooLarge(i) => {
                write!(
                    f,
                    "Number in 'line' directive was larger than INT_MAX! The C standard only \
                     supports line numbers up to INT_MAX. Value was '{i}'"
                )
            },
            | Self::MissingOpeningParenthesisInPragmaOperator(tt) => {
                write!(
                    f,
                    "Missing opening parenthesis in 'pragma' operator! Found instead {tt:#?}"
                )
            },
            | Self::MissingClosingParenthesisInPragmaOperator(tt) => {
                write!(
                    f,
                    "Missing closing parenthesis in 'pragma' operator! Found instead {tt:#?}"
                )
            },
            | Self::MissingStringLiteralInPragmaOperator(tt) => {
                write!(
                    f,
                    "Missing string literal in 'pragma' operator! Found instead {tt:#?}"
                )
            },
            | Self::UnknownPragmaDirective => {
                write!(
                    f,
                    "Unknown 'pragma' directive! BCC only supports the 'once' and 'STDC' \
                     directives."
                )
            },
            | Self::UnknownPragmaSTDCArgument(arg) => {
                write!(
                    f,
                    "Unknown argument to 'STDC' 'pragma' directive! BCC only supports the \
                     'FP_CONTRACT', 'FENV_ACCESS' and 'CX_LIMITED_RANGE' arguments. Found instead \
                     '{arg}'"
                )
            },
            | Self::ExtraTokensAfterPragmaOnce(tt) => {
                write!(
                    f,
                    "Extra tokens after 'once' 'pragma' directive! A once directive should \
                     consist of only the identifier 'once' and nothing else. Found {tt:#?}"
                )
            },
            | Self::ExtraTokensAfterPragmaOperator => {
                write!(
                    f,
                    "Extra tokens after 'pragma' operator! Not all tokens were consumed within \
                     the 'pragma' operator."
                )
            },
            | Self::STDCPragmaDirectiveWithoutArgument => {
                write!(
                    f,
                    "STDC 'pragma' directive without argument! The 'STDC' directive must be \
                     followed by an argument."
                )
            },
            | Self::STDCPragmaDirectiveWithoutOnOffSwitch => {
                write!(
                    f,
                    "STDC 'pragma' directive without on-off switch! Pragma directive ended before \
                     the on-off switch was found."
                )
            },
            | Self::MissingOnOffSwitchInSTDCPragma(arg) => {
                write!(
                    f,
                    "Missing on-off switch in STDC 'pragma' directive! 'FP_CONTRACT', \
                     'FENV_ACCESS' and 'CX_LIMITED_RANGE' must be followed by an on-off switch. \
                     Found instead '{arg}'"
                )
            },
            | Self::PragmaOnceInNonHeader => {
                write!(
                    f,
                    "Pragma 'once' directive used in non-header file! The 'once' directive can \
                     only be used in header files."
                )
            },
            | Self::ErrorDirective(s) => {
                write!(f, "Error directive: {s}")
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct FunctionLikeMacroArgument<PrevSavePoint> {
    name:             StringCacheId,
    start_save_point: PrevSavePoint,
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
        source_string: SharedString,
        source_file: StringCacheId,
        mut string_cache: StringCache,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
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
            hash_hash_stack: Vec::new(),
            once_set: HashSet::default(),
            previous_phase: phase3,
            macro_definitions,
            pending_results: VecDeque::new(),
            state: State::Default,
            last_preprocessor_token: None,
            current_preprocessor_token: None,
            if_directive_balance: 0,
            should_tokenize_whitespace: false,
            generate_placeholders: false,
            quote_include_directories,
            system_include_directories,
            expression_parser: PreprocessorExpressionParser::new(),
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
    Prev::SavePoint: FromInput + PartialEq + Eq + Clone + Debug,
{
    fn push_tokenizer_frame(&mut self, frame: TokenizerFrame<Prev::SavePoint>) {
        // eprintln!("pushing frame: {frame:#?}");
        // eprintln!(
        // "Called push_tokenizer_frame with length: {} and frame: {frame:?}",
        // self.tokenizer_stack.len()
        // );
        self.tokenizer_stack.last_mut().unwrap().save_point = self.previous_phase.save();
        let save_point = frame.save_point.clone();
        self.previous_phase.restore(save_point);
        self.tokenizer_stack.push(frame);
    }

    fn pop_tokenizer_frame(&mut self) {
        let f = self.tokenizer_stack.pop();
        drop(f);
        if let Some(last) = self.tokenizer_stack.last() {
            self.previous_phase.restore(last.save_point.clone());
        }
    }

    fn expect_token_no_expand(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, &PreprocessorToken) -> bool,
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
                    if is_correct_token(self, &token) {
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
                    break Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                eof_message,
                            ),
                            source_vectors: SourceVectors::from(SourceVector {
                                position: self.previous_phase.current_position(),
                                length:   0,
                            }),
                        },
                    ));
                },
            }
        }
    }

    #[allow(dead_code)]
    fn expect_token(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, &PreprocessorToken) -> bool,
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
                    if is_correct_token(self, &token) {
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
                    break Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                eof_message,
                            ),
                            source_vectors: SourceVectors::from(SourceVector {
                                position: self.previous_phase.current_position(),
                                length:   0,
                            }),
                        },
                    ));
                },
            }
        }
    }

    fn merge_token_contents(
        &mut self,
        lhs: &PreprocessorToken,
        rhs: &PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        let mut new_contents = TokenString::new();
        new_contents.push_str(get_from_cache!(self, lhs.contents));
        new_contents.push_str(get_from_cache!(self, rhs.contents));
        let mut new_source_vector = SmallVec::<[SourceVector; 1024]>::new();
        new_source_vector.extend_from_slice(&lhs.source_vectors);
        new_source_vector.extend_from_slice(&rhs.source_vectors);
        PreprocessorToken {
            kind:           result_token_type,
            contents:       self.insert_into_cache(&new_contents),
            source_vectors: new_source_vector.as_ref().into(),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn create_merge_error(
        &mut self,
        lhs: &PreprocessorToken,
        rhs: &PreprocessorToken,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        let lhs_contents = get_from_cache!(self, lhs.contents).to_string();
        let rhs_contents = get_from_cache!(self, rhs.contents).to_string();
        let source_vectors = &lhs.source_vectors + &rhs.source_vectors;
        self.pending_results
            .push_back(Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type: PreprocessorErrorType::TokenMergingError(
                        lhs_contents,
                        rhs_contents,
                    ),
                    source_vectors,
                },
            )));
        Some(Ok(lhs.clone()))
    }

    fn merge_tokens(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        // eprintln!(
        // "Called merge tokens with lhs: {:#?} and rhs: {:#?}",
        // lhs.kind, rhs.kind
        // );

        match (lhs.kind, rhs.kind) {
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(Ok(rhs)),
            | (_, PreprocessorTokenType::Placeholder) => Some(Ok(lhs)),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                if get_from_cache!(self, rhs.contents).contains('.') {
                    return self.create_merge_error(&lhs, &rhs);
                }
                let new = self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::Identifier);
                let kind = if new.contents == self.insert_into_cache("defined") {
                    PreprocessorTokenType::Defined
                } else {
                    PreprocessorTokenType::Identifier
                };
                Some(Ok(PreprocessorToken {
                    kind,
                    contents: new.contents,
                    source_vectors: new.source_vectors,
                }))
            },
            | (
                PreprocessorTokenType::Identifier,
                PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString,
            ) =>
                if get_from_cache!(self, lhs.contents) == "L"
                    && !get_from_cache!(self, rhs.contents).starts_with('L')
                {
                    let lhs = lhs.clone();
                    Some(Ok(self.merge_token_contents(
                        &lhs,
                        &rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    )))
                } else {
                    self.create_merge_error(&lhs, &rhs)
                },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            )
            | (PreprocessorTokenType::Number, PreprocessorTokenType::Period) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::Number)
            )),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::PlusPlus)
            )),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::MinusMinus)
            )),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::PlusEquals)
            )),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::MinusEquals)
            )),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::AsteriskEquals)
            )),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::ForwardSlashEquals),
            )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::PercentEquals)
            )),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::LessThanLessThan)
            )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) =>
                Some(Ok(self.merge_token_contents(
                    &lhs,
                    &rhs,
                    PreprocessorTokenType::GreaterThanGreaterThan,
                ))),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::LessThanEquals)
            )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::GreaterThanEquals)
            )),
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) =>
                Some(Ok(self.merge_token_contents(
                    &lhs,
                    &rhs,
                    PreprocessorTokenType::LessThanLessThanEquals,
                ))),
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(Ok(self.merge_token_contents(
                    &lhs,
                    &rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                ))),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::AmpersandEquals)
            )),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::CaretEquals)
            )),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::PipeEquals)
            )),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::ExclamationMarkEquals),
            )),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::EqualsEquals)
            )),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::PipePipe)
            )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) => Some(Ok(
                self.merge_token_contents(&lhs, &rhs, PreprocessorTokenType::AmpersandAmpersand),
            )),

            | _ => self.create_merge_error(&lhs, &rhs),
        }
    }

    fn should_ignore_whitespace(&self) -> bool {
        !self.should_tokenize_whitespace
            || self.last_preprocessor_token.as_ref().map(|t| t.kind)
                == Some(PreprocessorTokenType::Whitespace)
    }

    fn next_preprocessor_token_no_expand(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorTokenizerError<PrevPrevError>>> {
        let last = self.current_preprocessor_token.clone();
        let ret = 'base: loop {
            if unlikely(self.tokenizer_stack.is_empty()) {
                break 'base None;
            }
            match self.previous_phase.next() {
                | Some(Err(e)) => {
                    break 'base Some(Err(e));
                },
                | Some(Ok(token))
                    if token.kind == PreprocessorTokenType::Whitespace
                        && self.should_ignore_whitespace() =>
                {
                    continue 'base;
                },
                | Some(Ok(mut token)) => {
                    match self.tokenizer_stack.last_mut().unwrap() {
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                                | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                            ..
                        } =>
                            if token.kind == PreprocessorTokenType::Newline {
                                self.pop_tokenizer_frame();
                                continue 'base;
                            },
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroArgument {
                                    paren_depth,
                                    argument,
                                    has_generated_token,
                                },
                            ..
                        } => {
                            let paren_depth = *paren_depth;
                            let argument = argument.clone();
                            let has_generated_token = *has_generated_token;
                            if let Some(paren_depth) = self.update_macro_argument_paren_depth(
                                &token,
                                argument.name,
                                paren_depth,
                            ) {
                                let TokenizerFrame {
                                    frame_type:
                                        TokenizerFrameType::FunctionLikeMacroArgument {
                                            paren_depth: p,
                                            has_generated_token,
                                            ..
                                        },
                                    ..
                                } = self.tokenizer_stack.last_mut().unwrap()
                                else {
                                    unreachable!();
                                };
                                *p = paren_depth;
                                *has_generated_token = true;
                            } else {
                                self.pop_tokenizer_frame();
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(Ok(PreprocessorToken {
                                        kind:           PreprocessorTokenType::Placeholder,
                                        contents:       self.insert_into_cache(""),
                                        source_vectors: SourceVectors::new(),
                                    }));
                                }
                                continue 'base;
                            }
                            if token.kind == PreprocessorTokenType::Newline {
                                if self.should_ignore_whitespace() {
                                    continue 'base;
                                }
                                token.kind = PreprocessorTokenType::Whitespace;
                                token.contents = self.insert_into_cache(" ");
                            }
                        },
                        | TokenizerFrame {
                            frame_type: TokenizerFrameType::SourceFile,
                            ..
                        } => (),
                    }
                    break Some(Ok(token));
                },
                | None => {
                    self.pop_tokenizer_frame();
                    continue;
                },
            }
        };
        // eprintln!(
        // "Ret in next_preprocessor_token_no_expand_no_hash_hash: {:#?}",
        // ret.as_ref().map(|r| r.as_ref().map(|t| t.kind))
        // );
        // eprintln!("Ret in next_preprocessor_token_no_expand_no_hash_hash: {ret:#?}");
        match ret? {
            | Err(e) => Some(Err(e)),
            | Ok(t) => {
                self.last_preprocessor_token = last;
                self.current_preprocessor_token = Some(t.clone());
                Some(Ok(t))
            },
        }
    }

    fn update_macro_argument_paren_depth(
        &mut self,
        token: &PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        // eprintln!("Updating macro argument paren depth: {:#?}", token.kind);
        if (token.kind == PreprocessorTokenType::Comma
            && argument_name != self.insert_into_cache("__VA_ARGS__"))
            || (token.kind == PreprocessorTokenType::ClosingParenthesis && paren_depth == 1)
        {
            return None;
        }
        if token.kind == PreprocessorTokenType::OpeningParenthesis {
            return Some(paren_depth + 1);
        }
        if token.kind == PreprocessorTokenType::ClosingParenthesis {
            return Some(paren_depth - 1);
        }
        Some(paren_depth)
    }

    fn current_macro(&self) -> Option<(StringCacheId, SourcePosition)> {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                    | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                name,
                save_point,
                ..
            }) => Some((*name, save_point.current_position())),
            | _ => None,
        }
    }

    #[allow(dead_code)]
    fn current_is_macro(&self) -> bool {
        self.current_macro().is_some()
    }

    fn current_is_header(&self) -> bool {
        for frame in self.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile => return true,
                | _ => (),
            }
        }
        false
    }

    #[allow(dead_code)]
    #[allow(clippy::type_complexity)]
    fn current_function_like_macro(
        &self,
    ) -> Option<(
        StringCacheId,
        Prev::SavePoint,
        Arc<HashMap<StringCacheId, FunctionLikeMacroArgument<Prev::SavePoint>>>,
        Arc<HashSet<SourcePosition>>,
        bool,
    )> {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation {
                        arguments,
                        is_variadic,
                        hash_hash_positions,
                    },
                name,
                save_point,
                ..
            }) => Some((
                *name,
                save_point.clone(),
                arguments.clone(),
                hash_hash_positions.clone(),
                *is_variadic,
            )),
            | _ => None,
        }
    }

    #[allow(dead_code)]
    fn current_is_function_like_macro(&self) -> bool {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                ..
            }) => true,
            | _ => false,
        }
    }

    fn handle_hash_operator(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        let token = match self.next_preprocessor_token_no_expand() {
            | None => return None,
            | Some(Err(e)) => return Some(Err(PreprocessorError::PreviousPhaseError(e))),
            | Some(Ok(token)) => token,
        };

        match token.kind {
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    Some(self.parse_hash_operator(token))
                } else {
                    Some(Ok(token))
                }
            },
            | _ => Some(Ok(token)),
        }
    }

    fn handle_hash_hash_operator(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        loop {
            let token = match self.handle_hash_operator() {
                | None => return None,
                | Some(Err(e)) => return Some(Err(e)),
                | Some(Ok(token)) => token,
            };
            let hash_hash = if let Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation {
                        hash_hash_positions,
                        ..
                    }
                    | TokenizerFrameType::ObjectLikeMacroInvocation {
                        hash_hash_positions,
                    },
                ..
            }) = self.tokenizer_stack.last()
            {
                let mut save_point = self.previous_phase.save();
                while let Some(res) = self.previous_phase.next() {
                    match res {
                        | Ok(token) => {
                            if token.kind == PreprocessorTokenType::Whitespace {
                                save_point = self.previous_phase.save();
                                continue;
                            }
                            self.previous_phase.restore(save_point);
                            break;
                        },
                        | Err(_) => {
                            continue;
                        },
                    }
                }
                if hash_hash_positions.contains(&self.current_position()) {
                    loop {
                        match self.previous_phase.next() {
                            | Some(Err(e)) => {
                                self.pending_results
                                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                continue;
                            },
                            | Some(Ok(token)) if token.kind == PreprocessorTokenType::HashHash =>
                                break Some(token),
                            | Some(Ok(_)) | None => {
                                unreachable!();
                            },
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(h) = hash_hash {
                match self.tokenizer_stack.last() {
                    | Some(TokenizerFrame {
                        frame_type:
                            TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                            | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                        ..
                    }) => (),
                    | _ => {
                        return Some(Ok(token));
                    },
                }
                let rhs = match self.handle_hash_hash_operator() {
                    | Some(Err(e)) => return Some(Err(e)),
                    | Some(Ok(token)) => token,
                    | None =>
                        return Some(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                    "parsing hash-hash operator. Hash hash operator must be \
                                     followed by a token on the same line.",
                                ),
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: self.previous_phase.current_position(),
                                    length:   0,
                                }),
                            },
                        ))),
                };
                if let Some(r) = self.parse_hash_hash_operator(&token, &h, &rhs) {
                    return Some(r);
                };
            } else {
                return Some(Ok(token));
            }
        }
    }

    fn next_preprocessor_token(
        &mut self,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        // eprintln!("Tokenizer stack: {:#?}", self.tokenizer_stack);
        // eprintln!("Hash hash stack: {:#?}", self.hash_hash_stack);
        let last = self.current_preprocessor_token.clone();
        let ret = 'base: loop {
            self.generate_placeholders = true;
            let mut token = match self.handle_hash_hash_operator() {
                | None => break 'base None,
                | Some(Err(e)) => break 'base Some(Err(e)),
                | Some(Ok(token)) => token,
            };
            self.generate_placeholders = false;
            if !self.hash_hash_stack.is_empty() {
                'merge: loop {
                    // eprintln!("Hash hash stack: {:#?}", self.hash_hash_stack);
                    let result = match self.hash_hash_stack.last() {
                        | None | Some(HashHash::Empty) => None,
                        | Some(HashHash::Lhs(lhs)) => {
                            let lhs = lhs.clone();
                            drop(self.hash_hash_stack.pop());
                            self.merge_tokens(lhs.clone(), token.clone())
                        },
                        | Some(HashHash::Rhs(rhs)) => {
                            let rhs = rhs.clone();
                            drop(self.hash_hash_stack.pop());
                            self.merge_tokens(token.clone(), rhs.clone())
                        },
                    };
                    let new = match result {
                        | None => None,
                        | Some(Err(e)) => break 'base Some(Err(e)),
                        | Some(Ok(token)) => Some(token),
                    };

                    let next_is_end = if let Some(TokenizerFrame {
                        frame_type:
                            TokenizerFrameType::FunctionLikeMacroArgument {
                                argument,
                                paren_depth,
                                has_generated_token: _,
                            },
                        ..
                    }) = self.tokenizer_stack.last()
                    {
                        let paren_depth = *paren_depth;
                        let argument = argument.clone();
                        let save_point = self.previous_phase.save();
                        let next_is_end = loop {
                            match self.previous_phase.next() {
                                | Some(Ok(token)) =>
                                    break self
                                        .update_macro_argument_paren_depth(
                                            &token,
                                            argument.name,
                                            paren_depth,
                                        )
                                        .is_none(),
                                | Some(Err(_)) => continue,
                                | None => break true,
                            }
                        };
                        self.previous_phase.restore(save_point);
                        next_is_end
                    } else {
                        false
                    };
                    if next_is_end || token.kind == PreprocessorTokenType::Placeholder {
                        if let Some(x @ HashHash::Empty) = self.hash_hash_stack.last_mut() {
                            *x = HashHash::Lhs(if let Some(new) = new.clone() {
                                new
                            } else {
                                token.clone()
                            });
                            continue 'base;
                        }
                    }
                    if let Some(new) = new {
                        token = new;
                    } else {
                        break 'merge;
                    }
                }
            }
            if token.kind == PreprocessorTokenType::Placeholder {
                continue 'base;
            }
            if token.kind != PreprocessorTokenType::Identifier {
                break 'base Some(Ok(token));
            }

            if let Some(md) = self.macro_definitions.get(&token.contents).cloned() {
                match md {
                    | MacroDefinition::ObjectLike {
                        start_save_point,
                        hash_hash_positions,
                    } => {
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation {
                                hash_hash_positions,
                            },
                            save_point: start_save_point,
                            name:       token.contents,
                        };
                        self.push_tokenizer_frame(frame);
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        start_save_point,
                        is_variadic,
                        hash_hash_positions,
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
                                            source_vectors: token.source_vectors.clone(),
                                        },
                                    ),
                                ));
                                    self.restore(save_point);
                                    break 'base Some(Ok(token));
                                },
                                | Some(Err(e)) => {
                                    self.pending_results
                                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                    continue;
                                },
                                | None => {
                                    break 'base None;
                                },
                            }
                        }
                        let mut i = 0;
                        let mut arguments = HashMap::default();
                        let mut paren_depth = 1isize;
                        macro_rules! at {
                            () => {
                                argument_names
                                    .get(i)
                                    .copied()
                                    .unwrap_or(self.insert_into_cache("<undefined>"))
                            };
                        }
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
                                                at!(),
                                                FunctionLikeMacroArgument {
                                                    name: at!(),
                                                    start_save_point,
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
                                            at!(),
                                            FunctionLikeMacroArgument {
                                                name: at!(),
                                                start_save_point,
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
                                        break 'base Some(Err(
                                            PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:
                                                        PreprocessorErrorType::UnexpectedEndOfInput(
                                                            "parsing function-like macro \
                                                             invocation",
                                                        ),
                                                    source_vectors: token.source_vectors,
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
                                    source_vectors: token.source_vectors.clone(),
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
                            let mut paren_depth = 1isize;

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
                                        break 'base Some(Err(
                                            PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:
                                                        PreprocessorErrorType::UnexpectedEndOfInput(
                                                            "parsing function-like macro \
                                                             invocation",
                                                        ),
                                                    source_vectors: token.source_vectors,
                                                },
                                            ),
                                        ));
                                    },
                                }
                            }
                        }
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                arguments: Arc::new(arguments),
                                is_variadic,
                                hash_hash_positions: hash_hash_positions.clone(),
                            },
                            save_point: start_save_point,
                            name:       token.contents,
                        };
                        self.push_tokenizer_frame(frame);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match get_from_cache!(self, token.contents) {
                        | "__FILE__" => {
                            let file_name = self.insert_into_cache("__builtin__macros");
                            let source_file = token.source_vectors[0].position.source_file;
                            let length = get_from_cache!(self, source_file).len();
                            break 'base Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       token.source_vectors[0].position.source_file,
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: SourcePosition {
                                        index:       0,
                                        line:        1,
                                        column:      1,
                                        source_file: file_name,
                                    },
                                    length,
                                }),
                            }));
                        },
                        | "__LINE__" => {
                            let string = token.source_vectors[0].position.line.to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            break 'base Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       self.insert_into_cache(&string),
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: SourcePosition {
                                        index:       0,
                                        line:        1,
                                        column:      1,
                                        source_file: file_name,
                                    },
                                    length:   string.len(),
                                }),
                            }));
                        },
                        | "__TIME__" => {
                            let now = Local::now();
                            let string = now.format("%H:%M:%S").to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            break 'base Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: SourcePosition {
                                        index:       0,
                                        line:        1,
                                        column:      1,
                                        source_file: file_name,
                                    },
                                    length:   string.len(),
                                }),
                            }));
                        },
                        | "__DATE__" => {
                            let now = Local::now();
                            let string = now.format("%b %e %Y").to_string();
                            let file_name = self.insert_into_cache("__builtin__macros");
                            break 'base Some(Ok(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       self.insert_into_cache(&string),
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: SourcePosition {
                                        index:       0,
                                        line:        1,
                                        column:      1,
                                        source_file: file_name,
                                    },
                                    length:   string.len(),
                                }),
                            }));
                        },
                        | "_Pragma" => {
                            if let Err(e) = self.expect_token(
                                |_, t| t.kind == PreprocessorTokenType::OpeningParenthesis,
                                |_, e|
                                    ControlFlow::Break(e)
                                ,
                                |_, token|
                                    ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    )),
                                "parsing pragma operator",
                            ) {
                                break 'base Some(Err(e));
                            }

                            let string_token = match self.expect_token(
                                |_, t| t.kind == PreprocessorTokenType::String,
                                |_, e|
                                    ControlFlow::Break(e)
                                ,
                                |_, token|
                                    ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingStringLiteralInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    )),
                                "parsing pragma operator",
                            ) {
                                | Ok(token) => token,
                                | Err(e) => break 'base Some(Err(e)),
                            };

                            let input = self.prepare_pragma_operator_string(string_token.contents);

                            let save = self.previous_phase.save();
                            let pragma_string = self.insert_into_cache("<pragma string>");
                            self.previous_phase
                                .restore(Prev::SavePoint::from_input(input, pragma_string));
                            if let Err(e) = self.parse_pragma_directive(&string_token) {
                                self.previous_phase.restore(save);
                                break 'base Some(Err(e));
                            };
                            if self.previous_phase.next().is_some() {
                                self.pending_results
                                    .push_back(Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOperator,
                                        source_vectors: string_token.source_vectors,
                                    },
                                )));
                            }
                            self.previous_phase.restore(save);

                            if let Err(e) = self.expect_token(
                                |_, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                                |_, e|
                                    ControlFlow::Break(e)
                                ,
                                |_, token|
                                    ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    )),
                                "parsing pragma operator",
                            ) {
                                break 'base Some(Err(e));
                            }

                            continue 'base;
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            } else {
                if let Some(frame) = self.handle_macro_argument(&token) {
                    self.push_tokenizer_frame(frame);
                    continue;
                }
                break 'base Some(Ok(token));
            }
        };
        // eprintln!(
        // "Ret in next_preprocessor_token: {:#?}",
        // ret.as_ref().map(|r| r.as_ref().map(|t| t.kind))
        // );
        // eprintln!("Next preprocessor token returning: {ret:#?}");
        self.generate_placeholders = false;
        match ret? {
            | Err(e) => Some(Err(e)),
            | Ok(t) => {
                self.last_preprocessor_token = last;
                self.current_preprocessor_token = Some(t.clone());
                Some(Ok(t))
            },
        }
    }

    fn prepare_pragma_operator_string(&self, string: StringCacheId) -> SharedString {
        let string = get_from_cache!(self, string);
        let mut ret = String::new();
        // Skip the leading quote.
        let mut index = 1;
        if string.char_at(0) == Some('L') {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            match c {
                | '\\' => match string.char_at(index + 1) {
                    | Some('"') => {
                        ret.push('"');
                        index += 2;
                    },
                    | Some('\\') => {
                        ret.push('\\');
                        index += 2;
                    },
                    | _ => (),
                },
                | _ => {
                    ret.push(c);
                    index += c.len_utf8();
                },
            }
        }
        // Pop the trailing quote.
        if ret.char_at(ret.len() - 1) == Some('"') {
            _ = ret.pop();
        }
        ret.push('\n');
        SharedString::from(ret)
    }

    fn get_arguments(
        &self,
    ) -> Option<Arc<HashMap<StringCacheId, FunctionLikeMacroArgument<Prev::SavePoint>>>> {
        for frame in self.tokenizer_stack.iter().rev() {
            return match frame {
                | TokenizerFrame {
                    frame_type: TokenizerFrameType::FunctionLikeMacroArgument { .. },
                    ..
                } => continue,
                | TokenizerFrame {
                    frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                    ..
                } => Some(arguments.clone()),
                | _ => break,
            };
        }
        None
    }

    fn handle_macro_argument(
        &mut self,
        token: &PreprocessorToken,
    ) -> Option<TokenizerFrame<Prev::SavePoint>> {
        if let Some(arguments) = self.get_arguments() {
            if let Some(arg) = arguments.get(&token.contents) {
                let frame = TokenizerFrame {
                    frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                        argument:            arg.clone(),
                        paren_depth:         1,
                        has_generated_token: false,
                    },
                    save_point: arg.start_save_point.clone(),
                    name:       token.contents,
                };
                return Some(frame);
            }
        }

        None
    }

    fn eval_escape_sequences(
        &mut self,
        token: PreprocessorToken,
    ) -> Result<String, <Self as TranslationPhase>::Error> {
        let mut ret = String::new();
        let mut index = 0;
        let string = get_from_cache!(self, token.contents);
        if let Some('L') = string.char_at(0) {
            index += 1;
        }
        if let Some('"' | '\'') = string.char_at(index) {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            if c == '\\' {
                let Some(c) = string.char_at(index + 1) else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnterminatedEscapeSequence,
                            source_vectors: token.source_vectors,
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
                            self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::HexEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors.clone(),
                                },
                            )));
                            continue;
                        };
                        let Ok(c) = char::try_from(code_point) else {
                            self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidHexEscapeSequence,
                                    source_vectors: token.source_vectors.clone(),
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
                            self.pending_results.push_back(Err(
                                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::OctalEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors.clone(),
                                }),
                            ));
                            continue;
                        };
                        let Ok(c) = char::try_from(u32::from(code_point)) else {
                            self.pending_results.push_back(Err(
                                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidOctalEscapeSequence,
                                    source_vectors: token.source_vectors.clone(),
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
                                self.pending_results.push_back(
                                    Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort,
                                            source_vectors: token.source_vectors.clone(),
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
                            self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors.clone(),
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
                                self.pending_results.push_back(
                                    Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall,
                                            source_vectors: token.source_vectors.clone(),
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
                            self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors.clone(),
                                },
                            )));
                            continue;
                        };
                        c
                    }
                    | _ => {
                        self.pending_results.push_back(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidEscapeSequence,
                                source_vectors: token.source_vectors.clone(),
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
        if ret.ends_with('"') || ret.ends_with('\'') {
            _ = ret.pop();
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
            source_vectors: token.source_vectors,
        }
    }

    fn build_operator_token(token: PreprocessorToken, kind: OperatorTokenType) -> Token {
        Self::build_token(token, TokenType::Operator(kind))
    }

    fn parse_number(
        &mut self,
        token: PreprocessorToken,
        contents: &mut TokenString,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
        let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
        let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
        if is_hex {
            if contents.contains(|c| c == '.' || c == 'p' || c == 'P') {
                self.parse_hexadecimal_float(token, contents)
            } else {
                self.parse_hexadecimal_integer(token, contents)
            }
        } else if is_binary {
            self.parse_binary_integer(token, contents)
        } else if contents.contains(|c| c == '.' || c == 'e' || c == 'E') {
            self.parse_decimal_float(token, contents)
        } else if is_octal {
            self.parse_octal_integer(token, contents)
        } else {
            self.parse_decimal_integer(token, contents)
        }
    }

    fn parse_string(
        &mut self,
        token: &PreprocessorToken,
    ) -> Result<StringTokenType, PreprocessorError<Prev::Error>> {
        self.eval_escape_sequences(token.clone()).map(|contents| {
            let cached_contents = self.insert_into_cache(&contents);
            if get_from_cache!(self, token.contents).starts_with('L') {
                StringTokenType::WideString(cached_contents)
            } else {
                StringTokenType::String(cached_contents)
            }
        })
    }

    fn parse_character(
        &mut self,
        token: &PreprocessorToken,
    ) -> Result<CharacterTokenType, PreprocessorError<Prev::Error>> {
        self.eval_escape_sequences(token.clone())
            .and_then(|contents| {
                if contents.chars().take(2).count() != 1 {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:
                                PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                            source_vectors: token.source_vectors.clone(),
                        },
                    ));
                }
                let char = contents.chars().next().unwrap();
                if get_from_cache!(self, token.contents).starts_with('L') {
                    Ok(CharacterTokenType::WideChar(char))
                } else {
                    Ok(CharacterTokenType::Char(char))
                }
            })
    }

    fn map_preprocessor_token(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<Result<Token, PreprocessorError<Prev::Error>>> {
        let mut contents = get_from_cache!(self, token.contents).to_token_string();

        Some(match token.kind {
            | PreprocessorTokenType::Number => self.parse_number(token, &mut contents),
            | PreprocessorTokenType::Newline => return None,
            | PreprocessorTokenType::Hash =>
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    unreachable!("Handled in next_preprocessor_token");
                } else {
                    match self.parse_directive(&token, &contents) {
                        | Ok(()) => return None,
                        | Err(e) => Err(e),
                    }
                },
            | PreprocessorTokenType::GeneratedString => Ok(Token {
                kind:           TokenType::String(StringTokenType::String(token.contents)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            }),
            | PreprocessorTokenType::WideGeneratedString => {
                // Discard the L prefix.
                let contents = &get_from_cache!(self, token.contents)[1..].to_token_string();
                let contents = self.insert_into_cache(contents);
                Ok(Token {
                    kind: TokenType::String(StringTokenType::WideString(contents)),
                    contents,
                    source_vectors: token.source_vectors,
                })
            },
            | PreprocessorTokenType::String => self.parse_string(&token).map(|kind| Token {
                kind:           TokenType::String(kind),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            }),
            | PreprocessorTokenType::Character => self.parse_character(&token).map(|kind| Token {
                kind:           TokenType::Character(kind),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            }),
            | PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined =>
                Ok(Self::build_token(
                    token.clone(),
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
            | PreprocessorTokenType::Ellipsis => Ok(Self::build_operator_token(
                token,
                OperatorTokenType::Ellipsis,
            )),
            | PreprocessorTokenType::HashHash => {
                return Some(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HashHashUsedOutsideOfMacro,
                        source_vectors: token.source_vectors,
                    },
                )));
            },

            | x => todo!("{x:#?}"),
        })
    }

    fn parse_hash_hash_operator(
        &mut self,
        lhs: &PreprocessorToken,
        _hash_hash: &PreprocessorToken,
        rhs: &PreprocessorToken,
    ) -> Option<Result<PreprocessorToken, PreprocessorError<Prev::Error>>> {
        // eprintln!("Parsing hash hash operator");
        // eprintln!("lhs: {lhs:#?}");
        // eprintln!("rhs: {rhs:#?}");
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = self.handle_macro_argument(rhs) {
            // eprintln!("rhs is macro argument");
            self.push_tokenizer_frame(frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs.clone());
            false
        };
        if let Some(frame) = self.handle_macro_argument(lhs) {
            // eprintln!("lhs is macro argument");
            self.push_tokenizer_frame(frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs.clone());
        } else {
            drop(self.hash_hash_stack.pop());
            return self.merge_tokens(lhs.clone(), rhs.clone());
            // drop(self.hash_hash_stack.pop());
        }
        None
    }

    fn parse_hash_operator(
        &mut self,
        token: PreprocessorToken,
    ) -> Result<PreprocessorToken, PreprocessorError<Prev::Error>> {
        let save_point = self.previous_phase.save();
        let argument_name = self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Identifier,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:
                            PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(
                                t.kind,
                            ),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing '#' operator in function-like macro invocation.",
        )?;
        let (token_start_save, argument_id) = match self.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match arguments.get(&argument_name.contents) {
                | None => {
                    self.previous_phase.restore(save_point);
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:
                                PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                    get_from_cache!(self, argument_name.contents).to_owned(),
                                ),
                            source_vectors: argument_name.source_vectors,
                        },
                    ));
                },
                | Some(v) => (v.start_save_point.clone(), v.name),
            },
            | _ => unreachable!(),
        };
        let save_point = self.previous_phase.save();
        self.previous_phase.restore(token_start_save);
        self.should_tokenize_whitespace = true;

        let mut synthetic_contents = String::new();
        let mut paren_depth = 1;
        loop {
            let token = loop {
                match self.previous_phase.next() {
                    | None => {
                        self.previous_phase.restore(save_point);
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                    "parsing '#' operator in function-like macro invocation",
                                ),
                                source_vectors: argument_name.source_vectors,
                            },
                        ));
                    },
                    | Some(Err(e)) => {
                        self.pending_results
                            .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                        continue;
                    },
                    | Some(Ok(t)) => break t,
                }
            };
            match self.update_macro_argument_paren_depth(&token, argument_id, paren_depth) {
                | Some(depth) => paren_depth = depth,
                | None => break,
            }
            synthetic_contents.push_str(get_from_cache!(self, token.contents));
        }
        self.previous_phase.restore(save_point);
        self.should_tokenize_whitespace = false;
        Ok(PreprocessorToken {
            kind:           PreprocessorTokenType::GeneratedString,
            contents:       self.insert_into_cache(&synthetic_contents),
            source_vectors: token.source_vectors,
        })
    }

    fn parse_directive(
        &mut self,
        token: &PreprocessorToken,
        _contents: &str,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if !matches!(
            self.last_preprocessor_token.as_ref().map(|p| p.kind),
            None | Some(PreprocessorTokenType::Newline)
        ) {
            eprintln!(
                "Last preprocessor token: {:#?}",
                self.last_preprocessor_token
            );
            self.pending_results
                .push_back(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                        source_vectors: token.source_vectors.clone(),
                    },
                )));
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
                        source_vectors: directive.source_vectors,
                    },
                )),
        }
        match get_from_cache!(self, directive.contents) {
            | "if" => self.parse_if_directive(&directive),
            | "ifdef" => self.parse_ifdef_directive(&directive),
            | "ifndef" => self.parse_ifndef_directive(&directive),
            | "elif" => self.parse_elif_directive(&directive),
            | "else" => self.parse_else_directive(&directive),
            | "endif" => self.parse_endif_directive(&directive),
            | "include" => self.parse_include_directive(&directive),
            | "define" => self.parse_define_directive(&directive),
            | "undef" => self.parse_undef_directive(&directive),
            | "line" => self.parse_line_directive(&directive),
            | "error" => Err(self.parse_error_directive(&directive)),
            | "pragma" => self.parse_pragma_directive(&directive),
            | _ => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                },
            )),
        }
    }

    fn map_operator(&mut self, operator: &PreprocessorToken) -> PreprocessorExpressionOperator {
        let state = replace(
            &mut self.expression_parser.state,
            PreprocessorExpressionParserState::Unary,
        );
        match operator.kind {
            | PreprocessorTokenType::Plus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryPlus
                } else {
                    PreprocessorExpressionOperator::BinaryPlus
                },
            | PreprocessorTokenType::Minus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryMinus
                } else {
                    PreprocessorExpressionOperator::BinaryMinus
                },
            | PreprocessorTokenType::Asterisk => PreprocessorExpressionOperator::Multiply,
            | PreprocessorTokenType::ForwardSlash => PreprocessorExpressionOperator::Divide,
            | PreprocessorTokenType::Percent => PreprocessorExpressionOperator::Modulo,
            | PreprocessorTokenType::LessThanLessThan => PreprocessorExpressionOperator::LeftShift,
            | PreprocessorTokenType::GreaterThanGreaterThan =>
                PreprocessorExpressionOperator::RightShift,
            | PreprocessorTokenType::LessThan => PreprocessorExpressionOperator::LessThan,
            | PreprocessorTokenType::LessThanEquals =>
                PreprocessorExpressionOperator::LessThanEquals,
            | PreprocessorTokenType::GreaterThan => PreprocessorExpressionOperator::GreaterThan,
            | PreprocessorTokenType::GreaterThanEquals =>
                PreprocessorExpressionOperator::GreaterThanEquals,
            | PreprocessorTokenType::EqualsEquals => PreprocessorExpressionOperator::Equals,
            | PreprocessorTokenType::ExclamationMarkEquals =>
                PreprocessorExpressionOperator::NotEquals,
            | PreprocessorTokenType::Ampersand => PreprocessorExpressionOperator::BitwiseAnd,
            | PreprocessorTokenType::Caret => PreprocessorExpressionOperator::BitwiseXor,
            | PreprocessorTokenType::Pipe => PreprocessorExpressionOperator::BitwiseOr,
            | PreprocessorTokenType::AmpersandAmpersand =>
                PreprocessorExpressionOperator::LogicalAnd,
            | PreprocessorTokenType::PipePipe => PreprocessorExpressionOperator::LogicalOr,
            | PreprocessorTokenType::QuestionMark => PreprocessorExpressionOperator::Ternary,
            | PreprocessorTokenType::Colon => PreprocessorExpressionOperator::Else,
            | PreprocessorTokenType::Tilde => PreprocessorExpressionOperator::BitwiseNot,
            | PreprocessorTokenType::ExclamationMark => PreprocessorExpressionOperator::LogicalNot,
            | _ => unreachable!(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_expression_operator(
        &mut self,
        op: PreprocessorExpressionOperator,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        match op {
            PreprocessorExpressionOperator::UnaryPlus => if self.expression_parser.operand_stack.is_empty() {
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::UnaryPlusWithoutOperand,
                        source_vectors: SourceVectors::from(
                            SourceVector {
                                position: self.current_position(),
                                length: 0,
                            }
                        ),
                    },
                ))
            } else {
                Ok(())
            },
            PreprocessorExpressionOperator::UnaryMinus => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::UnaryMinusWithoutOperand,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = operand.checked_neg() else {return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::UnaryMinusOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            },
            PreprocessorExpressionOperator::BitwiseNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BitwiseNotWithoutOperand,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(!operand);
                Ok(())
            },
            PreprocessorExpressionOperator::LogicalNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LogicalNotWithoutOperand,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(operand == 0));
                Ok(())
            },
            PreprocessorExpressionOperator::BinaryPlus => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: BinaryPlus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BinaryPlusWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = lhs.checked_add(rhs) else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BinaryPlusOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::BinaryMinus => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: BinaryMinus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BinaryMinusWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = lhs.checked_sub(rhs) else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BinaryMinusOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::Multiply => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: Multiply operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::MultiplyWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = lhs.checked_mul(rhs) else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::MultiplyOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::Divide => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: Divide operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::DivideWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                if rhs == 0 {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::DivideByZero,
                            source_vectors: SourceVectors::from(
                                SourceVector {
                                    position: self.current_position(),
                                    length: 0,
                                }
                            ),
                        },
                    ));
                }
                let Some(new) = lhs.checked_div(rhs) else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::DivideOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::Modulo => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: Modulo operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::ModuloWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                if rhs == 0 {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::ModuloByZero,
                            source_vectors: SourceVectors::from(
                                SourceVector {
                                    position: self.current_position(),
                                    length: 0,
                                }
                            ),
                        },
                    ));
                }
                let Some(new) = lhs.checked_rem(rhs) else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::ModuloOverflow,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::LeftShift => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: LeftShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LeftShiftWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = rhs.try_into().ok().and_then(|rhs| lhs.checked_shl(rhs)) else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::LeftShiftOverflow,
                            source_vectors: SourceVectors::from(
                                SourceVector {
                                    position: self.current_position(),
                                    length: 0,
                                }
                            ),
                        },
                    ));
                };
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::RightShift => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: RightShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::RightShiftWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(new) = rhs.try_into().ok().and_then(|rhs| lhs.checked_shr(rhs)) else { return Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::RightShiftOverflow,
                        source_vectors: SourceVectors::from(
                            SourceVector {
                                position: self.current_position(),
                                length: 0,
                            }
                        ),
                    },
                ));};
                self.expression_parser.operand_stack.push(new);
                Ok(())
            }
            PreprocessorExpressionOperator::LessThan => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: LessThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LessThanWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs < rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::LessThanEquals => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: LessThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LessThanEqualsWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs <= rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::GreaterThan => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: GreaterThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::GreaterThanWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs > rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::GreaterThanEquals => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: GreaterThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::GreaterThanEqualsWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs >= rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::Equals => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: Equals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::EqualsWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs == rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::NotEquals => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: NotEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::NotEqualsWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs != rhs));
                Ok(())
            }
            PreprocessorExpressionOperator::BitwiseAnd => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: BitwiseAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BitwiseAndWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(lhs & rhs);
                Ok(())
            }
            PreprocessorExpressionOperator::BitwiseXor => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: BitwiseXor operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BitwiseXorWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(lhs ^ rhs);
                Ok(())
            }
            PreprocessorExpressionOperator::BitwiseOr => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: BitwiseOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::BitwiseOrWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(lhs | rhs);
                Ok(())
            }
            PreprocessorExpressionOperator::LogicalAnd => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: LogicalAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LogicalAndWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs != 0 && rhs != 0));
                Ok(())
            }
            PreprocessorExpressionOperator::LogicalOr => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: LogicalOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::LogicalOrWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(i128::from(lhs != 0 || rhs != 0));
                Ok(())
            }
            PreprocessorExpressionOperator::Else => {assert!(!self.expression_parser.operand_stack.is_empty(), "Compiler bug: Else operator without lhs"); Ok(())},
            PreprocessorExpressionOperator::Ternary => {
                let rhs = self.expression_parser.operand_stack.pop().expect("Compiler bug: Ternary operator without condition");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::TernaryOperatorWithoutMhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                let Some(condition) = self.expression_parser.operand_stack.pop() else {
                    return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::TernaryOperatorWithoutRhs,
                                source_vectors: SourceVectors::from(
                                    SourceVector {
                                        position: self.current_position(),
                                        length: 0,
                                    }
                                ),
                            },
                        ));
                };
                self.expression_parser.operand_stack.push(if condition != 0 { lhs } else { rhs });
                Ok(())
            }
            PreprocessorExpressionOperator::OpeningParenthesis => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    source_vectors: SourceVectors::from(
                        SourceVector {
                            position: self.current_position(),
                            length: 0,
                        }
                    ),
                },
            )),
        }
    }

    fn parse_defined_operator(&mut self) -> Result<(), PreprocessorError<Prev::Error>> {
        let ident_or_opening_paren = self.expect_token_no_expand(
            |_, t| matches!(t.kind, PreprocessorTokenType::Identifier | PreprocessorTokenType::OpeningParenthesis),
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(t.kind),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing defined operator",
        )?;
        if ident_or_opening_paren.kind == PreprocessorTokenType::Identifier {
            let is_defined = self
                .macro_definitions
                .contains_key(&ident_or_opening_paren.contents);
            self.expression_parser
                .operand_stack
                .push(i128::from(is_defined));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return Ok(());
        }
        assert_eq!(
            ident_or_opening_paren.kind,
            PreprocessorTokenType::OpeningParenthesis,
            "Compiler bug: ident_or_opening_paren should be an opening parenthesis or identifier."
        );
        let ident = self.expect_token_no_expand(
            |_, t| matches!(t.kind, PreprocessorTokenType::Identifier),
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                            t.kind,
                        ),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing defined operator",
        )?;

        drop(self.expect_token_no_expand(
            |_, t| matches!(t.kind, PreprocessorTokenType::ClosingParenthesis),
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(
                                t.kind,
                            ),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing defined operator",
        )?);
        let is_defined = self.macro_definitions.contains_key(&ident.contents);
        self.expression_parser
            .operand_stack
            .push(i128::from(is_defined));
        self.expression_parser.state = PreprocessorExpressionParserState::Binary;
        Ok(())
    }

    fn last_binary_operator(&mut self) -> PreprocessorExpressionOperator {
        loop {
            match self
                .expression_parser
                .operator_stack
                .pop()
                .expect("Compiler bug: No binary operator in expression stack")
            {
                | PreprocessorExpressionOperator::OpeningParenthesis
                | PreprocessorExpressionOperator::UnaryMinus
                | PreprocessorExpressionOperator::UnaryPlus
                | PreprocessorExpressionOperator::BitwiseNot
                | PreprocessorExpressionOperator::LogicalNot => continue,
                | op => return op,
            }
        }
    }

    fn eval_preprocessor_expression(
        &mut self,
        on_no_expression_error: PreprocessorErrorType,
    ) -> Result<bool, PreprocessorError<Prev::Error>> {
        const UNARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Unary;
        const BINARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Binary;
        self.expression_parser.reset();
        loop {
            match self.next_preprocessor_token() {
                | Some(Err(e)) => {
                    self.pending_results
                        .push_back(Err(e));
                    continue;
                },
                | None => return Err(PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                    source_vectors: SourceVectors::from(
                        SourceVector {
                            position: self.current_position(),
                            length: 0,
                        }
                    )})),
                | Some(Ok(token)) => match (token.kind, self.expression_parser.state) {
                    | (PreprocessorTokenType::Newline, _) => {
                        break;
                    },
                    (PreprocessorTokenType::Plus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryPlus),
                    (PreprocessorTokenType::Minus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryMinus),
                    (PreprocessorTokenType::Tilde, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::BitwiseNot),
                    (PreprocessorTokenType::Tilde, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (PreprocessorTokenType::ExclamationMark, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::LogicalNot),
                    (PreprocessorTokenType::ExclamationMark, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                },
                            )),
                    (PreprocessorTokenType::OpeningParenthesis, UNARY) => {
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::OpeningParenthesis);
                    }
                    (PreprocessorTokenType::OpeningParenthesis, BINARY) => {
                        return Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        ));
                    }
                    (PreprocessorTokenType::ClosingParenthesis, _) => {
                        self.expression_parser.state = BINARY;
                        if !self.expression_parser.operator_stack.last().is_some_and(|op| *op != PreprocessorExpressionOperator::OpeningParenthesis) {
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                },
                            ));
                        }
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op == PreprocessorExpressionOperator::OpeningParenthesis {
                                break;
                            }
                            self.handle_expression_operator(op)?;
                        }
                    }
                    (PreprocessorTokenType::Defined, UNARY) =>
                        self.parse_defined_operator()?,
                    (PreprocessorTokenType::Defined, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (PreprocessorTokenType::Asterisk, UNARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (PreprocessorTokenType::Ampersand, UNARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (
                        | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                        PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                        PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals |
                        PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                        PreprocessorTokenType::Colon, UNARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(self.map_operator(&token)),
                            source_vectors: token.source_vectors,
                        })),
                    | (PreprocessorTokenType::Plus | PreprocessorTokenType::Minus | PreprocessorTokenType::Asterisk
                    | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                    PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                    PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals | PreprocessorTokenType::Ampersand |
                    PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                    PreprocessorTokenType::Colon, BINARY) => {
                        let token_op = self.map_operator(&token);
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op.has_precedence_over(token_op) {
                                self.handle_expression_operator(op)?;
                            } else {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            }
                        }
                        self.expression_parser.operator_stack.push(token_op);
                        self.expression_parser.state = UNARY;
                    },
                    (PreprocessorTokenType::Number, UNARY) => {
                        let mut contents = TokenString::from(get_from_cache!(self, token.contents));
                        let value = match self.parse_number(token.clone(), &mut contents)?.kind {
                            | TokenType::Float(_) => return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type: PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                },
                            )),
                            | TokenType::Integer(v) => i128::from(v),
                            _ => unreachable!("Compiler bug: parse_number should return a number token."),
                        };
                        self.expression_parser.operand_stack.push(value);
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Number, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (PreprocessorTokenType::Identifier, UNARY) => {
                        self.pending_results.push_back(
                            Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type: PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(get_from_cache!(self, token.contents).to_owned()),
                                    source_vectors: token.source_vectors,
                                },
                            )),
                        );
                        self.expression_parser.operand_stack.push(0);
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Identifier, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    (PreprocessorTokenType::Character, UNARY) => {
                        let value = i128::from(u32::from(char::from(self.parse_character(&token)?)));
                        self.expression_parser.operand_stack.push(value);
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Character, BINARY) => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    )),
                    _ => return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                            source_vectors: token.source_vectors,
                        },
                    )),
                },
            }
        }
        if self.expression_parser.state == UNARY {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(
                        self.last_binary_operator(),
                    ),
                    source_vectors: SourceVectors::from(
                        SourceVector {
                            position: self.current_position(),
                            length: 0,
                        }
                    )
                }
            ));
        }
        while let Some(op) = self.expression_parser.operator_stack.pop() {
            self.handle_expression_operator(op)?;
        }
        match self.expression_parser.operand_stack.len() {
            | 1 => Ok(self.expression_parser.operand_stack.pop().unwrap() != 0),
            | 0 => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     on_no_expression_error,
                    source_vectors: SourceVectors::from(SourceVector {
                        position: self.current_position(),
                        length:   0,
                    }),
                },
            )),
            | _ => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:
                        PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression,
                    source_vectors: SourceVectors::from(SourceVector {
                        position: self.current_position(),
                        length:   0,
                    }),
                },
            )),
        }
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
                | Some(Ok(token)) => {
                    eprintln!("Token in skipping over dead code: {token:?}");
                    match token.kind {
                        | PreprocessorTokenType::Newline => {
                            'newline: loop {
                                match self.next_preprocessor_token_no_expand() {
                                    | Some(Err(_)) => continue,
                                    | Some(Ok(token))
                                        if token.kind != PreprocessorTokenType::Hash =>
                                        if token.kind == PreprocessorTokenType::Newline {
                                            continue 'newline;
                                        } else {
                                            continue 'outer;
                                        },
                                    | Some(Ok(_)) => break 'newline,
                                    | None =>
                                        return Err(PreprocessorError::InnerPreprocessorError(
                                            InnerPreprocessorError {
                                                error_type:
                                                    PreprocessorErrorType::UnexpectedEndOfInput(
                                                        "parsing dead code. Expected #endif \
                                                         instead",
                                                    ),
                                                source_vectors: token.source_vectors,
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
                                        source_vectors: token.source_vectors,
                                    },
                                )),
                                | None => return Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing dead code. Expected #endif instead."),
                                        source_vectors: token.source_vectors,
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
                                    if self.eval_preprocessor_expression(
                                        PreprocessorErrorType::NoConditionInElifDirective,
                                    )? {
                                        break;
                                    }
                                },
                                | "else" if self.if_directive_balance == start_balance => break,
                                | _ => (),
                            }
                        },
                        | _ => continue,
                    }
                },
                | None => return Ok(()),
            }
        }
        Ok(())
    }

    fn parse_if_directive(
        &mut self,
        _directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance += 1;
        if !self.eval_preprocessor_expression(PreprocessorErrorType::NoConditionInIfDirective)? {
            self.skip_over_dead_code()?;
        }
        Ok(())
    }

    fn parse_elif_directive(
        &mut self,
        directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if self.if_directive_balance <= 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ElifDirectiveWithoutIfDirective,
                    source_vectors: directive.source_vectors.clone(),
                },
            ));
        }
        // Elif directives only matter if we are currently skipping over dead code.
        Ok(())
    }

    fn parse_else_directive(
        &mut self,
        directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        if self.if_directive_balance <= 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
                    source_vectors: directive.source_vectors.clone(),
                },
            ));
        }
        // Else directives only matter if we are currently skipping over dead code.
        Ok(())
    }

    fn parse_endif_directive(
        &mut self,
        directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        self.if_directive_balance -= 1;
        if self.if_directive_balance < 0 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                    source_vectors: directive.source_vectors.clone(),
                },
            ));
        }
        Ok(())
    }

    fn parse_ifdef_directive(
        &mut self,
        _directive: &PreprocessorToken,
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
                        source_vectors: token.source_vectors,
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
        _directive: &PreprocessorToken,
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
                        source_vectors: token.source_vectors,
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
        include_token: &PreprocessorToken,
        path: &Path,
        is_system_header: bool,
    ) -> Result<Option<PathBuf>, <Self as TranslationPhase>::Error> {
        eprintln!(
            "Including {}header from path: {:#?}",
            if is_system_header { "system " } else { "" },
            path
        );
        let ret = 'ret: {
            if path.is_absolute() {
                if path.exists() {
                    break 'ret Ok(path.to_owned());
                }
                break 'ret Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::HeaderNotFound,
                        source_vectors: include_token.source_vectors.clone(),
                    },
                ));
            }

            if is_system_header {
                if let Some(header) =
                    Self::search_for_header_in(path, &self.system_include_directories)
                {
                    break 'ret Ok(header);
                }
            }
            if let Some(header) = Self::search_for_header_in(path, &self.quote_include_directories)
            {
                break 'ret Ok(header);
            }

            let cwd = std::env::current_dir().map_err(|_| {
                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::CurrentWorkingDirectoryInaccessible,
                    source_vectors: include_token.source_vectors.clone(),
                })
            })?;
            if let Some(header) = Self::search_for_header_in(path, &[cwd]) {
                break 'ret Ok(header);
            }

            Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::HeaderNotFound,
                    source_vectors: include_token.source_vectors.clone(),
                },
            ))
        };
        eprintln!("Returning {ret:?}");
        match ret {
            | Ok(header) => {
                let s = header.to_string_lossy();
                let id = self.insert_into_cache(&s);
                if self.once_set.contains(&id) {
                    eprintln!(
                        "Not including header {s} because it should only be included once and \
                         already been included"
                    );
                    Ok(None)
                } else {
                    Ok(Some(header))
                }
            },
            | Err(e) => Err(e),
        }
    }

    fn parse_include_directive(
        &mut self,
        directive: &PreprocessorToken,
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
                        source_vectors: token.source_vectors,
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
                self.find_header_from_path(&include_string, path, false)
            },
            | PreprocessorTokenType::AngleBracketString => {
                let contents = get_from_cache!(self, include_string.contents);
                let contents = &contents[1..contents.len() - 1].to_token_string();
                let path = Path::new(contents.as_str());
                self.find_header_from_path(&include_string, path, true)
            },
            | _ => {
                self.should_tokenize_whitespace = true;
                let mut contents = TokenString::new();
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
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                        "parsing include directive",
                                    ),
                                    source_vectors: directive.source_vectors.clone(),
                                },
                            ));
                        },
                    }
                }
                self.should_tokenize_whitespace = false;
                let path = Path::new(contents.as_str());
                let synthetic_token = PreprocessorToken {
                    source_vectors: include_string.source_vectors,
                    contents:       self.insert_into_cache(&contents),
                    kind:           PreprocessorTokenType::AngleBracketString,
                };
                self.find_header_from_path(&synthetic_token, path, true)
            },
        }?;
        let Some(header_path) = header_path else {
            return Ok(());
        };
        let header_string = read_to_string_lossy(&header_path).map_err(|_| {
            PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                error_type:     PreprocessorErrorType::HeaderFileInaccessible,
                source_vectors: directive.source_vectors.clone(),
            })
        })?;
        let name = self.insert_into_cache(&header_path.to_string_lossy());
        let save_point = Prev::SavePoint::from_input(SharedString::from(header_string), name);
        let frame = TokenizerFrame {
            save_point,
            name,
            frame_type: TokenizerFrameType::SourceFile,
        };
        self.push_tokenizer_frame(frame);
        self.last_preprocessor_token = None;
        self.current_preprocessor_token = None;
        Ok(())
    }

    fn parse_define_directive(
        &mut self,
        _directive: &PreprocessorToken,
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
                        source_vectors: token.source_vectors,
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
                | MacroDefinition::ObjectLike {
                    start_save_point, ..
                } => Some(start_save_point.clone()),
                | MacroDefinition::BuiltIn => {
                    self.pending_results.push_back(Err(
                        PreprocessorError::<Prev::Error>::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                                    get_from_cache!(self, name.contents).to_owned(),
                                ),
                                source_vectors: name.source_vectors.clone(),
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
                    return Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                "parsing macro definition",
                            ),
                            source_vectors: name.source_vectors,
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
                        source_vectors: name.source_vectors.clone(),
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
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
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
                                source_vectors: token.source_vectors,
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
                                source_vectors: name.source_vectors.clone(),
                            },
                        )));
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind == PreprocessorTokenType::Identifier {
                    argument_names.push(name_or_ellipsis.contents);
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
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
                                source_vectors: token.source_vectors,
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
                    hash_hash_positions: Arc::new(HashSet::default()),
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
                        source_vectors: name.source_vectors.clone(),
                    },
                )));
            }
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::ObjectLike {
                    start_save_point:    save_point.clone(),
                    hash_hash_positions: Arc::new(HashSet::default()),
                },
            ));
        }
        let mut hash_hash_positions = HashSet::default();
        let mut last = Option::<PreprocessorToken>::None;
        if let Some(mut old_save_point) = old_save_point {
            let mut error_has_been_generated = false;
            let mut new_save_point = save_point;
            loop {
                self.previous_phase.restore(old_save_point);
                let old_next = loop {
                    match self.previous_phase.next() {
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
                    match self.previous_phase.next() {
                        | Some(Err(_)) => {
                            // Don't generate errors while validating macro definitions.
                            continue;
                        },
                        | Some(Ok(token)) => break Some(token),
                        | None => break None,
                    }
                };
                if let Some(t) = new_next.as_ref() {
                    if t.kind == PreprocessorTokenType::HashHash {
                        // Hash-hash tokens cannot be created as a result of token pasting, so they
                        // will always have only one source vector.
                        _ = hash_hash_positions.insert(t.source_vectors[0].position);
                        if last.is_none() {
                            self.pending_results
                                .push_back(Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                                        source_vectors: t.source_vectors.clone(),
                                    },
                                )));
                        }
                    }
                }
                new_save_point = self.previous_phase.save();
                if old_next != new_next && !error_has_been_generated {
                    self.pending_results
                        .push_back(Err(PreprocessorError::InnerPreprocessorError(
                            InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                        get_from_cache!(self, name.contents).to_owned(),
                                    ),
                                source_vectors: name.source_vectors.clone(),
                            },
                        )));
                    error_has_been_generated = true;
                }
                if !new_next
                    .as_ref()
                    .is_some_and(|t| t.kind != PreprocessorTokenType::Newline)
                {
                    if last
                        .as_ref()
                        .is_some_and(|t| t.kind == PreprocessorTokenType::HashHash)
                    {
                        self.pending_results.push_back(Err(
                            PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                                source_vectors: last.unwrap().source_vectors.clone(),
                            }),
                        ));
                    }
                    break;
                }
                last = new_next;
            }
        } else {
            loop {
                match self.next_preprocessor_token_no_expand() {
                    | Some(Err(_)) => {
                        // Don't generate errors while validating macro definitions.
                        continue;
                    },
                    | Some(Ok(token)) if token.kind == PreprocessorTokenType::HashHash => {
                        _ = hash_hash_positions.insert(token.source_vectors[0].position);
                    },
                    | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(Ok(_)) => continue,
                }
            }
        }
        if let Some(
            MacroDefinition::FunctionLike {
                hash_hash_positions: ref mut hhp,
                ..
            }
            | MacroDefinition::ObjectLike {
                hash_hash_positions: ref mut hhp,
                ..
            },
        ) = self.macro_definitions.get_mut(&name.contents)
        {
            *Arc::get_mut(hhp).unwrap() = hash_hash_positions;
        }
        Ok(())
    }

    fn parse_undef_directive(
        &mut self,
        _directive: &PreprocessorToken,
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
                        error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            "parsing undef directive",
        )?;
        drop(self.macro_definitions.remove(&name.contents));
        drop(self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Newline,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, token| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            "parsing undef directive",
        )?);
        Ok(())
    }

    fn parse_line_directive(
        &mut self,
        _directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        let token = self.expect_token_no_expand(
            |_this, t| t.kind == PreprocessorTokenType::Number,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing line directive",
        )?;
        let mut contents = TokenString::from(get_from_cache!(self, token.contents));
        let line_number = self.parse_number(token.clone(), &mut contents)?;
        for b in contents.bytes() {
            if !b.is_ascii_digit() {
                self.pending_results
                    .push_back(Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:
                                PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                            source_vectors: token.source_vectors.clone(),
                        },
                    )));
                break;
            }
        }
        if let Token {
            kind: TokenType::Float(f),
            ..
        } = line_number
        {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::FloatingPointNumberInLineDirective(f),
                    source_vectors: line_number.source_vectors,
                },
            ));
        }
        let Token {
            kind: TokenType::Integer(line_number),
            ..
        } = line_number
        else {
            unreachable!();
        };
        let value = i128::from(line_number);
        if value > usize::MAX as i128 {
            return Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::UnsupportedLineDirectiveValue(value),
                    source_vectors: token.source_vectors,
                },
            ));
        }
        if value < 0 {
            self.pending_results
                .push_back(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::NegativeNumberInLineDirective(value),
                        source_vectors: token.source_vectors.clone(),
                    },
                )));
        }
        if value > i128::from(i32::MAX) {
            self.pending_results
                .push_back(Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(value),
                        source_vectors: token.source_vectors,
                    },
                )));
        }
        self.set_line_number(value as usize);
        drop(self.expect_token_no_expand(
            |_, t| t.kind == PreprocessorTokenType::Newline,
            |this, e| {
                this.pending_results
                    .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                ControlFlow::Continue(())
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(
                            t.kind,
                        ),
                        source_vectors: t.source_vectors,
                    },
                ))
            },
            "parsing line directive",
        )?);
        Ok(())
    }

    fn parse_error_directive(
        &mut self,
        _directive: &PreprocessorToken,
    ) -> PreprocessorError<Prev::Error> {
        let mut contents = String::new();
        loop {
            match self.previous_phase.next() {
                | Some(Err(e)) => {
                    self.pending_results
                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                    continue;
                },
                | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline => break,
                | Some(Ok(token)) => {
                    contents.push_str(get_from_cache!(self, token.contents));
                },
                | None => {
                    return PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing error directive",
                        ),
                        source_vectors: SourceVectors::from(SourceVector {
                            position: self.previous_phase.current_position(),
                            length:   0,
                        }),
                    });
                },
            }
        }
        PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
            error_type:     PreprocessorErrorType::ErrorDirective(contents),
            source_vectors: SourceVectors::from(SourceVector {
                position: self.previous_phase.current_position(),
                length:   0,
            }),
        })
    }

    fn parse_pragma_directive(
        &mut self,
        _directive: &PreprocessorToken,
    ) -> Result<(), PreprocessorError<Prev::Error>> {
        let should_tokenize_whitespace = replace(&mut self.should_tokenize_whitespace, false);
        let ret = 'base: loop {
            let token = match self.previous_phase.next() {
                | Some(Err(e)) => {
                    self.pending_results
                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                    continue;
                },
                | Some(Ok(token)) => token,
                | None => {
                    break 'base Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                "parsing pragma directive",
                            ),
                            source_vectors: SourceVectors::from(SourceVector {
                                position: self.previous_phase.current_position(),
                                length:   0,
                            }),
                        },
                    ));
                },
            };
            match token.kind {
                | PreprocessorTokenType::Whitespace => continue 'base,
                | PreprocessorTokenType::Newline => break 'base Ok(()),
                | PreprocessorTokenType::Identifier =>
                    match get_from_cache!(self, token.contents) {
                        | "once" => {
                            if !self.current_is_header() {
                                self.pending_results.push_back(Err(
                                    PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::PragmaOnceInNonHeader,
                                            source_vectors: token.source_vectors.clone(),
                                        },
                                    ),
                                ));
                            }
                            _ = self.once_set.insert(self.current_position().source_file);
                            loop {
                                match self.previous_phase.next() {
                                | Some(Err(e)) => {
                                    self.pending_results
                                        .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                    continue;
                                },
                                | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline =>
                                    break 'base Ok(()),
                                | Some(Ok(t)) => break 'base Err(PreprocessorError::InnerPreprocessorError(
                                    InnerPreprocessorError {
                                        error_type:     PreprocessorErrorType::ExtraTokensAfterPragmaOnce(t.kind),
                                        source_vectors: token.source_vectors,
                                    },
                                )),
                                | None => {
                                    break 'base Err(PreprocessorError::InnerPreprocessorError(
                                        InnerPreprocessorError {
                                            error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                                "parsing pragma directive",
                                            ),
                                            source_vectors: SourceVectors::from(SourceVector {
                                                position: self.previous_phase.current_position(),
                                                length:   0,
                                            }),
                                        },
                                    ));
                                },
                            }
                            }
                        },
                        | "STDC" => {
                            loop {
                                match self.previous_phase.next() {
                                    | Some(Err(e)) => {
                                        self.pending_results
                                            .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                        continue;
                                    },
                                    | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline =>
                                        break 'base Err(PreprocessorError::InnerPreprocessorError(
                                            InnerPreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        )),
                                    | Some(Ok(token)) => {
                                        let s = get_from_cache!(self, token.contents);
                                        if token.kind != PreprocessorTokenType::Identifier || !matches!(s, "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE") {
                                            break 'base Err(PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:     PreprocessorErrorType::UnknownPragmaSTDCArgument(s.to_owned()),
                                                    source_vectors: token.source_vectors,
                                                },
                                            ));
                                        }
                                        break;
                                    }
                                    | None => {
                                        break 'base Err(PreprocessorError::InnerPreprocessorError(
                                            InnerPreprocessorError {
                                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing pragma directive",
                                                ),
                                                source_vectors: SourceVectors::from(SourceVector {
                                                    position: self.previous_phase.current_position(),
                                                    length:   0,
                                                }),
                                            },
                                        ));
                                    },
                                }
                            }
                            loop {
                                match self.previous_phase.next() {
                                    | Some(Err(e)) => {
                                        self.pending_results
                                            .push_back(Err(PreprocessorError::PreviousPhaseError(e)));
                                        continue;
                                    },
                                    | Some(Ok(token)) if token.kind == PreprocessorTokenType::Newline =>
                                        break 'base Err(PreprocessorError::InnerPreprocessorError(
                                            InnerPreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        )),
                                    | Some(Ok(token)) => {
                                        let s = get_from_cache!(self, token.contents);
                                        if token.kind != PreprocessorTokenType::Identifier || !matches!(s, "ON" | "OFF" | "DEFAULT") {
                                            break 'base Err(PreprocessorError::InnerPreprocessorError(
                                                InnerPreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(s.to_owned()),
                                                    source_vectors: token.source_vectors,
                                                },
                                            ));
                                        }
                                        break;
                                    }
                                    | None => {
                                        break 'base Err(PreprocessorError::InnerPreprocessorError(
                                            InnerPreprocessorError {
                                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing pragma directive",
                                                ),
                                                source_vectors: SourceVectors::from(SourceVector {
                                                    position: self.previous_phase.current_position(),
                                                    length:   0,
                                                }),
                                            },
                                        ));
                                    },
                                }
                            }
                        },
                        | _ => break 'base Ok(()),
                    },
                | _ =>
                    break 'base Err(PreprocessorError::InnerPreprocessorError(
                        InnerPreprocessorError {
                            error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                            source_vectors: token.source_vectors,
                        },
                    )),
            }
        };
        self.should_tokenize_whitespace = should_tokenize_whitespace;
        ret
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
                    source_vectors: token.source_vectors,
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
                    source_vectors: token.source_vectors,
                },
            ));
        }
        match suffix_type {
            | Some(IntegerSuffix::UnsignedLongLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::LongLong) if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors.clone(),
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::LongLong,
                            to:   UnsignedIntegerLiteralType::UnsignedLongLong,
                        },
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            | Some(IntegerSuffix::LongLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::LongLong(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::UnsignedLong) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::Long) if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors.clone(),
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::Long,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            | Some(IntegerSuffix::Long) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::Long(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | Some(IntegerSuffix::Unsigned) if result > u64::from(u32::MAX) => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors.clone(),
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedUnsignedPromotion {
                            from: UnsignedIntegerLiteralType::UnsignedInt,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            #[allow(clippy::cast_possible_truncation)]
            | Some(IntegerSuffix::Unsigned) => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedInt(result as u32)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | None if result > i64::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors.clone(),
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            from: SignedIntegerLiteralType::Int,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            | None if result > i32::MAX as u64 => {
                self.pending_results.push_back(Ok(Token {
                    kind:           TokenType::Integer(IntegerTokenType::Long(result as i64)),
                    source_vectors: token.source_vectors.clone(),
                    contents:       token.contents,
                }));
                Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedPromotion {
                            from: SignedIntegerLiteralType::Int,
                            to:   SignedIntegerLiteralType::Long,
                        },
                        source_vectors: token.source_vectors,
                    },
                ))
            },
            #[allow(clippy::cast_possible_truncation)]
            | None => Ok(Token {
                kind:           TokenType::Integer(IntegerTokenType::Int(result as i32)),
                source_vectors: token.source_vectors,
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
                source_vectors: token.source_vectors,
                contents:       token.contents,
            }),
            | Err(ParseFloatError::Invalid) => Err(PreprocessorError::InnerPreprocessorError(
                InnerPreprocessorError {
                    error_type:     invalid_float_literal_error,
                    source_vectors: token.source_vectors,
                },
            )),
            | Err(ParseFloatError::Overflow(kind)) => Err(
                PreprocessorError::InnerPreprocessorError(InnerPreprocessorError {
                    error_type:     PreprocessorErrorType::FloatLiteralOverflow(kind),
                    source_vectors: token.source_vectors,
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
    Prev::SavePoint: FromInput + PartialEq + Eq + Clone + Debug,
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
                                source_vectors: SourceVectors::from(SourceVector {
                                    position: self.current_position(),
                                    length:   0,
                                }),
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
    Prev::SavePoint: FromInput + PartialEq + Eq + Clone + Debug,
{
    type Error = PreprocessorError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint, Prev::Error>;
    type Yield = Token;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
            tokenizer_stack: self.tokenizer_stack.clone(),
            hash_hash_stack: self.hash_hash_stack.clone(),
            once_set: self.once_set.clone(),
            pending_results: self.pending_results.clone(),
            state: self.state,
            last_preprocessor_token: self.last_preprocessor_token.clone(),
            current_preprocessor_token: self.current_preprocessor_token.clone(),
            macro_definitions: self.macro_definitions.clone(),
            if_directive_balance: self.if_directive_balance,
            should_tokenize_whitespace: self.should_tokenize_whitespace,
            generate_placeholders: self.generate_placeholders,
            quote_include_directories: self.quote_include_directories.clone(),
            system_include_directories: self.system_include_directories.clone(),
            expression_parser: self.expression_parser.clone(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.tokenizer_stack = save_point.tokenizer_stack;
        self.hash_hash_stack = save_point.hash_hash_stack;
        self.once_set = save_point.once_set;
        self.pending_results = save_point.pending_results;
        self.state = save_point.state;
        self.last_preprocessor_token = save_point.last_preprocessor_token;
        self.current_preprocessor_token = save_point.current_preprocessor_token;
        self.macro_definitions = save_point.macro_definitions;
        self.if_directive_balance = save_point.if_directive_balance;
        self.should_tokenize_whitespace = save_point.should_tokenize_whitespace;
        self.generate_placeholders = save_point.generate_placeholders;
        self.quote_include_directories = save_point.quote_include_directories;
        self.system_include_directories = save_point.system_include_directories;
        self.expression_parser = save_point.expression_parser;
    }

    fn current_position(&self) -> SourcePosition {
        self.previous_phase.current_position()
    }

    fn set_line_number(&mut self, line: usize) {
        self.previous_phase.set_line_number(line);
    }
}
