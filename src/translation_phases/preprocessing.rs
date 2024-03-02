use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    io::Error as IoError,
    mem::{
        replace,
        take,
    },
    ops::ControlFlow,
    path::{
        Path,
        PathBuf,
    },
    rc::Rc,
};

use chrono::Local;

use super::{
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
        PreprocessorTokenizer,
    },
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileName,
    GetSourceVectors,
    SaveCurrentPosition,
    SavedPosition,
    SetPosition,
    SetSourceFileName,
    SourcePosition,
    SourceVectors,
    StrExt,
    TokenString,
    TranslationPhase,
    ONE,
};
use crate::{
    float_parsing::{
        long_double_to_operand,
        string_to_double,
        string_to_float,
        string_to_long_double,
        LongDouble,
        ParseFloatError,
    },
    util::{
        read_to_string_lossy,
        shared::{
            SharedPath,
            SharedString,
            SharedVec,
        },
        string_cache::StringCacheId,
        unlikely,
        HashMap,
        HashSet,
    },
};

const PREDEFINED_MACRO_NAMES: [&str; 5] =
    ["__LINE__", "__FILE__", "__DATE__", "__TIME__", "_Pragma"];

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType {
    SourceFile,
    ObjectLikeMacroInvocation,
    FunctionLikeMacroInvocation {
        arguments:           Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        is_variadic:         bool,
    },
    FunctionLikeMacroArgument {
        argument:            Box<FunctionLikeMacroArgument>,
        paren_depth:         usize,
        has_generated_token: bool,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition {
    ObjectLike {
        tokenizer:           PreprocessorTokenizer,
    },
    FunctionLike {
        argument_names:      Rc<[StringCacheId]>,
        tokenizer:           PreprocessorTokenizer,
        is_variadic:         bool,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame {
    frame_type: TokenizerFrameType,
    pub(crate) tokenizer:  PreprocessorTokenizer,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum HashHash {
    Lhs(PreprocessorToken),
    Rhs(PreprocessorToken),
    Empty,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Preprocessor {
    pub(crate) tokenizer:       PreprocessorTokenizer,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    once_set:                   HashSet<SharedPath>,
    macro_definitions:          HashMap<StringCacheId, MacroDefinition>,
    current_is_newline:         bool,
    last_was_newline:           bool,
    if_directive_balance:       isize,

    generate_placeholders:      bool,
    quote_include_directories:  SharedVec<PathBuf>,
    system_include_directories: SharedVec<PathBuf>,
    expression_parser:          PreprocessorExpressionParser,
}

impl GetPosition for Preprocessor {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.tokenizer.position(context)
    }
}

impl SetPosition for Preprocessor {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.tokenizer.set_position(context, position);
    }
}

impl GetSourceFileName for Preprocessor {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn source_file_name(&self) -> SharedPath {
        self.tokenizer.source_file_name()
    }
}

impl SetSourceFileName for Preprocessor {
    fn set_source_file_name(&mut self, context: &mut Context, name: SharedPath) {
        self.tokenizer.set_source_file_name(context, name);
    }
}

impl SaveCurrentPosition for Preprocessor {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn save_position(&self, context: &mut Context) -> SavedPosition {
        self.tokenizer.save_position(context)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn restore_position(&mut self, context: &mut Context, saved_position: SavedPosition) {
        self.tokenizer.restore_position(context, saved_position);
    }
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
    operand_stack:  Vec<PreprocessorExpressionOperand>,
    state:          PreprocessorExpressionParserState,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperand {
    Signed(i64),
    Unsigned(u64),
}

impl PreprocessorExpressionOperand {
    fn as_signed(self) -> i64 {
        match self {
            | Self::Signed(v) => v,
            | Self::Unsigned(v) => v as i64,
        }
    }

    fn as_unsigned(self) -> u64 {
        match self {
            | Self::Signed(v) => v as u64,
            | Self::Unsigned(v) => v,
        }
    }

    fn is_signed(self) -> bool {
        match self {
            | Self::Signed(_) => true,
            | Self::Unsigned(_) => false,
        }
    }

    fn is_unsigned(self) -> bool {
        match self {
            | Self::Signed(_) => false,
            | Self::Unsigned(_) => true,
        }
    }

    fn set_signed(self, value: i64) -> Self {
        match self {
            | Self::Signed(_) => Self::Signed(value),
            | Self::Unsigned(_) => Self::Unsigned(value as u64),
        }
    }

    #[allow(dead_code)]
    fn set_unsigned(self, value: u64) -> Self {
        self.set_signed(value as i64)
    }

    fn map_signed(self, f: impl FnOnce(i64) -> i64) -> Self {
        match self {
            | Self::Signed(v) => Self::Signed(f(v)),
            | Self::Unsigned(v) => Self::Unsigned(f(v as i64) as u64),
        }
    }

    fn map_unsigned(self, f: impl FnOnce(u64) -> u64) -> Self {
        self.map_signed(|v| f(v as u64) as i64)
    }
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
            | PreprocessorErrorType::CurrentWorkingDirectoryInaccessible(..)
            | PreprocessorErrorType::HeaderFileInaccessible(..)
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
            | PreprocessorErrorType::ExtraTokensAfterIncludeDirective
            | PreprocessorErrorType::ExtraTokensAfterIfdefDirective
            | PreprocessorErrorType::ExtraTokensAfterIfndefDirective
            | PreprocessorErrorType::FloatCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(..)
            | PreprocessorErrorType::DoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(..)
            | PreprocessorErrorType::LongDoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(..)
            | PreprocessorErrorType::PragmaOnceInNonHeader => ErrorSeverity::Warning,
        }
    }
}

#[derive(Debug)]
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
    FloatCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(f32),
    DoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(f64),
    LongDoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(LongDouble),
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
    CurrentWorkingDirectoryInaccessible(IoError),
    HeaderFileInaccessible(IoError),
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
    LineDirectiveIsNotASimpleDigitSequence,
    LineDirectiveNumberTooLarge(i128),
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
            | Self::FloatCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(v) => {
                write!(
                    f,
                    "Floating point literal {v} could not be losslessly converted to an integer \
                     in preprocessor expression",
                )
            },
            | Self::DoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(v) => {
                write!(
                    f,
                    "Double literal {v} could not be losslessly converted to an integer in \
                     preprocessor expression",
                )
            },
            | Self::LongDoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(v) => {
                write!(
                    f,
                    "Long double literal {v} could not be losslessly converted to an integer in \
                     preprocessor expression",
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
            | Self::CurrentWorkingDirectoryInaccessible(error) => {
                write!(
                    f,
                    "Current working directory inaccessible! The operating system returned an \
                     error when trying to get the current working directory. IO error: {error:?}"
                )
            },
            | Self::HeaderFileInaccessible(error) => {
                write!(
                    f,
                    "Header file inaccessible! The header file was deleted or moved while the \
                     preprocessor was trying to read it. IO error: {error:?}"
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
            | Self::LineDirectiveIsNotASimpleDigitSequence => {
                write!(
                    f,
                    "The 'line' directive must be followed by a simple digit sequence according \
                     to the C standard!"
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
            | Self::ExtraTokensAfterIncludeDirective => {
                write!(f, "Extra tokens after 'include' operator!")
            },
            | Self::ExtraTokensAfterIfdefDirective => {
                write!(f, "Extra tokens after 'ifdef' operator!")
            },
            | Self::ExtraTokensAfterIfndefDirective => {
                write!(f, "Extra tokens after 'ifndef' operator!")
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
pub(crate) struct FunctionLikeMacroArgument {
    name:      StringCacheId,
    tokenizer: PreprocessorTokenizer,
}

impl Preprocessor {
    pub(crate) fn new(
        context: &mut Context,
        source_name: SharedPath,
        source: SharedString,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
    ) -> Self {
        let macro_definitions = PREDEFINED_MACRO_NAMES
            .into_iter()
            .map(|s| -> (StringCacheId, MacroDefinition) {
                (context.string_cache.intern(s), MacroDefinition::BuiltIn)
            })
            .collect();
        let tokenizer = PreprocessorTokenizer::new(source_name, source);
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                tokenizer:  tokenizer.clone(),
            }],
            hash_hash_stack: Vec::new(),
            once_set: HashSet::default(),
            tokenizer,
            macro_definitions,
            last_was_newline: true,
            current_is_newline: true,
            if_directive_balance: 0,
            generate_placeholders: false,
            quote_include_directories,
            system_include_directories,
            expression_parser: PreprocessorExpressionParser::new(),
        }
    }

    fn skip_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.tokenizer.next_item(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    fn skip_and_expand_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.next_preprocessor_token::<false>(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    fn push_tokenizer_frame(&mut self, _context: &mut Context, frame: TokenizerFrame) {
        self.tokenizer_stack.last_mut().unwrap().tokenizer = take(&mut self.tokenizer);
        self.tokenizer = frame.tokenizer.clone();
        self.tokenizer_stack.push(frame);
    }

    fn pop_tokenizer_frame(&mut self, _context: &mut Context) {
        let f = self.tokenizer_stack.pop();
        drop(f);
        if let Some(last) = self.tokenizer_stack.last() {
            self.tokenizer = last.tokenizer.clone();
        }
    }

    fn expect_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.save_position(context);
            match self.next_preprocessor_token::<SHOULD_IGNORE_WHITESPACE>(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            self.restore_position(context, start);
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    self.restore_position(context, start);
                    let start_position = start.position(context);
                    let source_vectors =
                        context.create_source_vectors(start_position, self.source_file_name(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    fn expect_token_from_previous_phase<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.save_position(context);
            match self.tokenizer.next_item(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            self.restore_position(context, start);
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    self.restore_position(context, start);
                    let start_position = start.position(context);
                    let source_vectors =
                        context.create_source_vectors(start_position, self.source_file_name(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    fn merge_token_contents(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        _ = self;
        let mut new_contents = TokenString::new();
        new_contents.push_str(context.string_cache.at(lhs.contents));
        new_contents.push_str(context.string_cache.at(rhs.contents));
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: result_token_type,
            contents: context.string_cache.intern(&new_contents),
            source_vectors,
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn create_merge_error(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        _ = self;
        let lhs_contents = context.string_cache.at(lhs.contents).to_string();
        let rhs_contents = context.string_cache.at(rhs.contents).to_string();
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    fn merge_tokens(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        match (lhs.kind, rhs.kind) {
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(rhs),
            | (_, PreprocessorTokenType::Placeholder) => Some(lhs),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                if context.string_cache.at(rhs.contents).contains('.') {
                    return Some(self.create_merge_error(context, lhs, rhs));
                }
                let new =
                    self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::Identifier);
                let kind = if context.string_cache.at(new.contents) == "defined" {
                    PreprocessorTokenType::Defined
                } else {
                    PreprocessorTokenType::Identifier
                };
                Some(PreprocessorToken {
                    kind,
                    contents: new.contents,
                    source_vectors: new.source_vectors,
                })
            },
            | (
                PreprocessorTokenType::Identifier,
                PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString,
            ) => {
                if context.string_cache.at(lhs.contents) == "L"
                    && context.string_cache.at(rhs.contents).starts_with('L')
                {
                    Some(self.merge_token_contents(
                        context,
                        lhs,
                        rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    ))
                } else {
                    Some(self.create_merge_error(context, lhs, rhs))
                }
            },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            )
            | (PreprocessorTokenType::Number, PreprocessorTokenType::Period) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::Number)),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusMinus),
            ),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusEquals),
            ),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusEquals),
            ),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::AsteriskEquals),
            ),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ForwardSlashEquals,
                )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PercentEquals),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThan,
                )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThan,
                )),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::LessThanEquals),
            ),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanEquals,
                )),
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThanEquals,
                )),
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandEquals,
                )),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::CaretEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipeEquals),
            ),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ExclamationMarkEquals,
                )),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::EqualsEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipePipe)),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandAmpersand,
                )),

            | _ => Some(self.create_merge_error(context, lhs, rhs)),
        }
    }

    #[allow(clippy::redundant_else)]
    fn expand_macros<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            if unlikely(self.tokenizer_stack.is_empty()) {
                break 'base None;
            }
            match self.tokenizer.next_item(context) {
                | Some(token)
                    if token.kind == PreprocessorTokenType::Whitespace
                        && SHOULD_IGNORE_WHITESPACE =>
                {
                    continue 'base;
                },
                | Some(mut token) => {
                    match self.tokenizer_stack.last_mut().unwrap() {
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                                | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                            ..
                        } =>
                            if token.kind == PreprocessorTokenType::Newline {
                                self.pop_tokenizer_frame(context);
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
                                context,
                                token,
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
                                self.pop_tokenizer_frame(context);
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(PreprocessorToken {
                                        kind:           PreprocessorTokenType::Placeholder,
                                        contents:       context.string_cache.intern(""),
                                        source_vectors: SourceVectors::default(),
                                    });
                                }
                                continue 'base;
                            }
                            if token.kind == PreprocessorTokenType::Newline {
                                if SHOULD_IGNORE_WHITESPACE {
                                    continue 'base;
                                }
                                token.kind = PreprocessorTokenType::Whitespace;
                                token.contents = context.string_cache.intern(" ");
                            }
                        },
                        | TokenizerFrame {
                            frame_type: TokenizerFrameType::SourceFile,
                            ..
                        } => (),
                    }
                    break Some(token);
                },
                | None => {
                    self.pop_tokenizer_frame(context);
                    continue 'base;
                },
            }
        }
    }

    fn update_macro_argument_paren_depth(
        &self,
        context: &Context,
        token: PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        if (token.kind == PreprocessorTokenType::Comma
            && context.string_cache.at(argument_name) != "__VA_ARGS__")
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

    fn current_is_header(&self, _context: &Context) -> bool {
        // The first in the tokenizer stack is the original source file.
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
        _context: &Context,
    ) -> Option<(
        SharedPath,
        PreprocessorTokenizer,
        Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        bool,
    )> {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation {
                        arguments,
                        is_variadic,
                    },
                tokenizer,
            }) => Some((
                tokenizer.source_file_name(),
                tokenizer.clone(),
                arguments.clone(),
                *is_variadic,
            )),
            | _ => None,
        }
    }

    #[allow(dead_code)]
    fn current_is_function_like_macro(&self, _context: &Context) -> bool {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                ..
            }) => true,
            | _ => false,
        }
    }

    fn handle_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        let Some(token) = self.expand_macros::<SHOULD_IGNORE_WHITESPACE>(context) else {
            return None;
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
                    Some(self.parse_hash_operator(context, token))
                } else {
                    Some(token)
                }
            },
            | _ => Some(token),
        }
    }

    fn handle_hash_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            let Some(token) = if let Some(t) = self.pe.take() {Some(t)} else {self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context) else {
                return None;
            }};
            let position = self.save_position(context);
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
                {
                    let hash_hash_positions = hash_hash_positions.clone();
                    let mut t = self.tokenizer.next_item(context);
                    if matches!(
                        t,
                        Some(PreprocessorToken {
                            kind: PreprocessorTokenType::Whitespace,
                            ..
                        })
                    ) {
                        t = self.tokenizer.next_item(context);
                    }
                    eprintln!("Current position: {:?}", self.position(context));
                    eprintln!("Hash hash positions: {hash_hash_positions:?}");
                    if hash_hash_positions.contains(&self.position(context)) {
                        match self.tokenizer.next_item(context) {
                            | Some(token) if token.kind == PreprocessorTokenType::HashHash =>
                                Some(token),
                            | Some(_) | None => {
                                unreachable!();
                            },
                        }
                    } else {
                        self.restore_position(context, position);
                        None
                    }
                }
            } else {
                None
            };
            if let Some(h) = hash_hash {
                let Some(rhs) = self.handle_hash_hash_operator::<true>(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.tokenizer.position(context),
                        self.tokenizer.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing hash-hash operator. Hash hash operator must be followed by a \
                             preprocessor token on the same line.",
                        ),
                        source_vectors,
                    });
                    return Some(token);
                };
                if let Some(r) = self.parse_hash_hash_operator(context, token, h, rhs) {
                    return Some(r);
                }
            } else {
                return Some(token);
            }
        }
    }

    fn next_preprocessor_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        self.last_was_newline = self.current_is_newline;
        let ret = 'base: loop {
            self.generate_placeholders = true;
            let Some(mut token) =
                self.handle_hash_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)
            else {
                break 'base None;
            };
            self.generate_placeholders = false;
            if !self.hash_hash_stack.is_empty() {
                'merge: loop {
                    let new = match self.hash_hash_stack.last() {
                        | None | Some(HashHash::Empty) => None,
                        | Some(HashHash::Lhs(lhs)) => {
                            let lhs = *lhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, lhs, token)
                        },
                        | Some(HashHash::Rhs(rhs)) => {
                            let rhs = *rhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, token, rhs)
                        },
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
                        let position = self.save_position(context);
                        let next_is_end = match self.tokenizer.next_item(context) {
                            | Some(token) => self
                                .update_macro_argument_paren_depth(
                                    context,
                                    token,
                                    argument.name,
                                    paren_depth,
                                )
                                .is_none(),
                            | None => true,
                        };
                        self.restore_position(context, position);
                        next_is_end
                    } else {
                        false
                    };
                    if next_is_end || token.kind == PreprocessorTokenType::Placeholder {
                        if let Some(x @ HashHash::Empty) = self.hash_hash_stack.last_mut() {
                            *x = HashHash::Lhs(if let Some(new) = new { new } else { token });
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
                break 'base Some(token);
            }

            if let Some(md) = self.macro_definitions.get(&token.contents).cloned() {
                match md {
                    | MacroDefinition::ObjectLike {
                        tokenizer,
                    } => {
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation,
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        tokenizer,
                        is_variadic,
                    } => {
                        let position = self.save_position(context);
                        let file = self.source_file_name();
                        loop {
                            match self.tokenizer.next_item(context) {
                                | Some(brace) if brace.kind == PreprocessorTokenType::Whitespace =>
                                    continue,
                                | Some(brace)
                                    if brace.kind == PreprocessorTokenType::OpeningParenthesis =>
                                    break,
                                | Some(_) => {
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisInFunctionLikeMacroInvocation,
                                        source_vectors: token.source_vectors,
                                    });
                                    self.restore_position(context, position);
                                    break 'base Some(token);
                                },
                                | None => {
                                    let start_position = position.position(context);
                                    let source_vectors =
                                        context.create_source_vectors(start_position, file, 1);
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisInFunctionLikeMacroInvocation,
                                        source_vectors,
                                    });
                                    self.restore_position(context, position);
                                    break 'base Some(token);
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
                                    .unwrap_or(context.string_cache.intern("<undefined>"))
                            };
                        }
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let tokenizer = self.tokenizer.clone();
                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            drop(arguments.insert(
                                                at!(),
                                                FunctionLikeMacroArgument {
                                                    name: at!(),
                                                    tokenizer,
                                                },
                                            ));
                                            break 'outer;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token) if token.kind == PreprocessorTokenType::Comma => {
                                        drop(arguments.insert(
                                            at!(),
                                            FunctionLikeMacroArgument {
                                                name: at!(),
                                                tokenizer,
                                            },
                                        ));
                                        i += 1;
                                        continue 'outer;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.restore_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }

                        if arguments.len() != argument_names.len() && !is_variadic {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      arguments.len(),
                                    },
                                    source_vectors: token.source_vectors,
                                },
                            );
                        }
                        if is_variadic {
                            drop(arguments.insert(
                                context.string_cache.intern("__VA_ARGS__"),
                                FunctionLikeMacroArgument {
                                    name:      context.string_cache.intern("__VA_ARGS__"),
                                    tokenizer: self.tokenizer.clone(),
                                },
                            ));
                            let mut paren_depth = 1isize;

                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            break;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.restore_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                arguments: Rc::new(arguments),
                                is_variadic,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match context.string_cache.at(token.contents) {
                        | "__FILE__" => {
                            let source_file = self.source_file_name();
                            let length = source_file.as_os_str().len();
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context
                                    .string_cache
                                    .intern(&source_file.to_string_lossy()),
                                source_vectors: context.create_source_vectors(
                                    SourcePosition {
                                        index:  0,
                                        line:   1,
                                        column: ONE,
                                    },
                                    SharedPath::from_path_buf(PathBuf::from("__builtin__macros")),
                                    length,
                                ),
                            });
                        },
                        | "__LINE__" => {
                            let string = self.line(context).to_string();
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       context.string_cache.intern(&string),
                                source_vectors: context.create_source_vectors(
                                    SourcePosition {
                                        index:  0,
                                        line:   1,
                                        column: ONE,
                                    },
                                    SharedPath::from_path_buf(PathBuf::from("__builtin__macros")),
                                    string.len(),
                                ),
                            });
                        },
                        | "__TIME__" => {
                            let now = Local::now();
                            let string = now.format("%H:%M:%S").to_string();
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&string),
                                source_vectors: context.create_source_vectors(
                                    SourcePosition {
                                        index:  0,
                                        line:   1,
                                        column: ONE,
                                    },
                                    SharedPath::from_path_buf(PathBuf::from("__builtin__macros")),
                                    string.len(),
                                ),
                            });
                        },
                        | "__DATE__" => {
                            let now = Local::now();
                            let string = now.format("%b %e %Y").to_string();
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&string),
                                source_vectors: context.create_source_vectors(
                                    SourcePosition {
                                        index:  0,
                                        line:   1,
                                        column: ONE,
                                    },
                                    SharedPath::from_path_buf(PathBuf::from("__builtin__macros")),
                                    string.len(),
                                ),
                            });
                        },
                        | "_Pragma" => {
                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::OpeningParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            let Some(string_token) = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::String,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingStringLiteralInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            ) else {
                                continue 'base;
                            };

                            let input =
                                self.prepare_pragma_operator_string(context, string_token.contents);

                            let tokenizer = take(&mut self.tokenizer);
                            let pragma_string =
                                SharedPath::from_path_buf(PathBuf::from("<pragma string>"));
                            self.tokenizer = PreprocessorTokenizer::new(pragma_string, input);
                            self.parse_pragma_directive(context, string_token);
                            if self.tokenizer.next_item(context).is_some() {
                                let source_vectors = context.create_source_vectors(
                                    self.position(context),
                                    self.source_file_name(),
                                    0,
                                );
                                context.preprocessor_error(PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::ExtraTokensAfterPragmaOperator,
                                    source_vectors,
                                });
                            }
                            self.tokenizer = tokenizer;

                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            continue 'base;
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            } else {
                if let Some(frame) = self.handle_macro_argument(context, token) {
                    self.push_tokenizer_frame(context, frame);
                    continue;
                }
                break 'base Some(token);
            }
        };
        self.generate_placeholders = false;
        self.current_is_newline = ret.map_or(true, |t| t.kind == PreprocessorTokenType::Newline);
        ret
    }

    fn next_ignore_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(t) if t.kind == PreprocessorTokenType::Whitespace => continue,
                | Some(t) => return Some(t),
                | None => return None,
            }
        }
    }

    fn next_treat_newlines_as_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
        last_was_whitespace: &mut bool,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(mut t) => {
                    match t.kind {
                        | PreprocessorTokenType::Whitespace => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                        },
                        | PreprocessorTokenType::Newline => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                            t.contents = context.string_cache.intern(" ");
                            t.kind = PreprocessorTokenType::Whitespace;
                        },
                        | _ => *last_was_whitespace = false,
                    }
                    return Some(t);
                },
                | None => return None,
            }
        }
    }

    fn prepare_pragma_operator_string(
        &self,
        context: &Context,
        string: StringCacheId,
    ) -> SharedString {
        _ = self;
        let string = context.string_cache.at(string);
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
        _context: &Context,
    ) -> Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>> {
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
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        if let Some(arguments) = self.get_arguments(context) {
            if let Some(arg) = arguments.get(&token.contents) {
                let frame = TokenizerFrame {
                    frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                        argument:            Box::new(arg.clone()),
                        paren_depth:         1,
                        has_generated_token: false,
                    },
                    tokenizer:  arg.tokenizer.clone(),
                };
                return Some(frame);
            }
        }

        None
    }

    fn eval_escape_sequences(&mut self, context: &mut Context, token: PreprocessorToken) -> String {
        _ = self;
        let mut ret = String::new();
        let mut index = 0;
        let string = context.string_cache.at(token.contents);
        if let Some('L') = string.char_at(0) {
            index += 1;
        }
        if let Some('"' | '\'') = string.char_at(index) {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            if c == '\\' {
                let Some(c) = string.char_at(index + 1) else {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnterminatedEscapeSequence,
                        source_vectors: token.source_vectors,
                    });
                    return ret;
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
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::HexEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidHexEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
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
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::OctalEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors,
                                },
                            );

                            continue;
                        };
                        let Ok(c) = char::try_from(u32::from(code_point)) else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidOctalEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | 'u' => {
                        let mut code_point = 0u32;
                        for _ in 0..4 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) else {
                                Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort,
                                    source_vectors: token.source_vectors,
                                });
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | 'U' => {
                        let mut code_point = 0u32;
                        for _ in 0..8 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) else {
                                Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall,
                                    source_vectors: token.source_vectors,
                                });
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | _ => {
                        Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidEscapeSequence,
                                source_vectors: token.source_vectors,
                            },
                        );
                        continue;
                    },
                });
            } else {
                ret.push(c);
                index += c.len_utf8();
            }
        }
        if ret.ends_with('"') || ret.ends_with('\'') {
            _ = ret.pop();
        }
        ret
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

    fn parse_number(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        let contents = context.string_cache.at(token.contents);
        let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
        let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
        let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
        if is_hex {
            if contents.contains(|c| c == '.' || c == 'p' || c == 'P') {
                self.parse_hexadecimal_float(context, token)
            } else {
                self.parse_hexadecimal_integer(context, token)
            }
        } else if is_binary {
            self.parse_binary_integer(context, token)
        } else if contents.contains(|c| c == '.' || c == 'e' || c == 'E') {
            self.parse_decimal_float(context, token)
        } else if is_octal {
            self.parse_octal_integer(context, token)
        } else {
            self.parse_decimal_integer(context, token)
        }
    }

    fn parse_string(&mut self, context: &mut Context, token: PreprocessorToken) -> StringTokenType {
        let contents = self.eval_escape_sequences(context, token);
        let cached_contents = context.string_cache.intern(&contents);
        if context.string_cache.at(token.contents).starts_with('L') {
            StringTokenType::WideString(cached_contents)
        } else {
            StringTokenType::String(cached_contents)
        }
    }

    fn parse_character(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> CharacterTokenType {
        let contents = self.eval_escape_sequences(context, token);
        if contents.chars().take(2).count() != 1 {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                source_vectors: token.source_vectors,
            });
        }
        let char = contents.chars().next().unwrap_or('\0');
        if context.string_cache.at(token.contents).starts_with('L') {
            CharacterTokenType::WideChar(char)
        } else {
            CharacterTokenType::Char(char)
        }
    }

    fn map_preprocessor_token(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Option<Token> {
        Some(match token.kind {
            | PreprocessorTokenType::Number => self.parse_number(context, token),
            | PreprocessorTokenType::Newline => return None,
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    unreachable!("Handled in next_preprocessor_token");
                } else {
                    self.parse_directive(context, token);
                    return None;
                }
            },
            | PreprocessorTokenType::GeneratedString => Token {
                kind:           TokenType::String(StringTokenType::String(token.contents)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::WideGeneratedString => {
                // Discard the L prefix.
                let contents = &context.string_cache.at(token.contents)[1..].to_token_string();
                let contents = context.string_cache.intern(contents);
                Token {
                    kind: TokenType::String(StringTokenType::WideString(contents)),
                    contents,
                    source_vectors: token.source_vectors,
                }
            },
            | PreprocessorTokenType::String => Token {
                kind:           TokenType::String(self.parse_string(context, token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Character => Token {
                kind:           TokenType::Character(self.parse_character(context, token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined =>
                Self::build_token(
                    token,
                    match context.string_cache.at(token.contents) {
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
                ),
            | PreprocessorTokenType::Plus =>
                Self::build_operator_token(token, OperatorTokenType::Plus),
            | PreprocessorTokenType::Minus =>
                Self::build_operator_token(token, OperatorTokenType::Minus),
            | PreprocessorTokenType::Asterisk =>
                Self::build_operator_token(token, OperatorTokenType::Asterisk),
            | PreprocessorTokenType::ForwardSlash =>
                Self::build_operator_token(token, OperatorTokenType::ForwardSlash),
            | PreprocessorTokenType::Percent =>
                Self::build_operator_token(token, OperatorTokenType::Percent),
            | PreprocessorTokenType::LessThanLessThan =>
                Self::build_operator_token(token, OperatorTokenType::LessThanLessThan),
            | PreprocessorTokenType::GreaterThanGreaterThan =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanGreaterThan),
            | PreprocessorTokenType::LessThan =>
                Self::build_operator_token(token, OperatorTokenType::LessThan),
            | PreprocessorTokenType::LessThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::LessThanEquals),
            | PreprocessorTokenType::GreaterThan =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThan),
            | PreprocessorTokenType::GreaterThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanEquals),
            | PreprocessorTokenType::EqualsEquals =>
                Self::build_operator_token(token, OperatorTokenType::EqualsEquals),
            | PreprocessorTokenType::ExclamationMarkEquals =>
                Self::build_operator_token(token, OperatorTokenType::ExclamationMarkEquals),
            | PreprocessorTokenType::Ampersand =>
                Self::build_operator_token(token, OperatorTokenType::Ampersand),
            | PreprocessorTokenType::Caret =>
                Self::build_operator_token(token, OperatorTokenType::Caret),
            | PreprocessorTokenType::Pipe =>
                Self::build_operator_token(token, OperatorTokenType::Pipe),
            | PreprocessorTokenType::AmpersandAmpersand =>
                Self::build_operator_token(token, OperatorTokenType::AmpersandAmpersand),
            | PreprocessorTokenType::PipePipe =>
                Self::build_operator_token(token, OperatorTokenType::PipePipe),
            | PreprocessorTokenType::QuestionMark =>
                Self::build_operator_token(token, OperatorTokenType::QuestionMark),
            | PreprocessorTokenType::Colon =>
                Self::build_operator_token(token, OperatorTokenType::Colon),
            | PreprocessorTokenType::SemiColon =>
                Self::build_operator_token(token, OperatorTokenType::SemiColon),
            | PreprocessorTokenType::OpeningParenthesis =>
                Self::build_operator_token(token, OperatorTokenType::OpeningParenthesis),
            | PreprocessorTokenType::ClosingParenthesis =>
                Self::build_operator_token(token, OperatorTokenType::ClosingParenthesis),
            | PreprocessorTokenType::OpeningSquareBracket =>
                Self::build_operator_token(token, OperatorTokenType::OpeningSquareBracket),
            | PreprocessorTokenType::ClosingSquareBracket =>
                Self::build_operator_token(token, OperatorTokenType::ClosingSquareBracket),
            | PreprocessorTokenType::OpeningCurlyBrace =>
                Self::build_operator_token(token, OperatorTokenType::OpeningCurlyBrace),
            | PreprocessorTokenType::ClosingCurlyBrace =>
                Self::build_operator_token(token, OperatorTokenType::ClosingCurlyBrace),
            | PreprocessorTokenType::Period =>
                Self::build_operator_token(token, OperatorTokenType::Period),
            | PreprocessorTokenType::Arrow =>
                Self::build_operator_token(token, OperatorTokenType::Arrow),
            | PreprocessorTokenType::PlusPlus =>
                Self::build_operator_token(token, OperatorTokenType::PlusPlus),
            | PreprocessorTokenType::MinusMinus =>
                Self::build_operator_token(token, OperatorTokenType::MinusMinus),
            | PreprocessorTokenType::AsteriskEquals =>
                Self::build_operator_token(token, OperatorTokenType::AsteriskEquals),
            | PreprocessorTokenType::ForwardSlashEquals =>
                Self::build_operator_token(token, OperatorTokenType::ForwardSlashEquals),
            | PreprocessorTokenType::PercentEquals =>
                Self::build_operator_token(token, OperatorTokenType::PercentEquals),
            | PreprocessorTokenType::PlusEquals =>
                Self::build_operator_token(token, OperatorTokenType::PlusEquals),
            | PreprocessorTokenType::MinusEquals =>
                Self::build_operator_token(token, OperatorTokenType::MinusEquals),
            | PreprocessorTokenType::LessThanLessThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::LessThanLessThanEquals),
            | PreprocessorTokenType::GreaterThanGreaterThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanGreaterThanEquals),
            | PreprocessorTokenType::AmpersandEquals =>
                Self::build_operator_token(token, OperatorTokenType::AmpersandEquals),
            | PreprocessorTokenType::CaretEquals =>
                Self::build_operator_token(token, OperatorTokenType::CaretEquals),
            | PreprocessorTokenType::PipeEquals =>
                Self::build_operator_token(token, OperatorTokenType::PipeEquals),
            | PreprocessorTokenType::Equals =>
                Self::build_operator_token(token, OperatorTokenType::Equals),
            | PreprocessorTokenType::Comma =>
                Self::build_operator_token(token, OperatorTokenType::Comma),
            | PreprocessorTokenType::Tilde =>
                Self::build_operator_token(token, OperatorTokenType::Tilde),
            | PreprocessorTokenType::ExclamationMark =>
                Self::build_operator_token(token, OperatorTokenType::ExclamationMark),
            | PreprocessorTokenType::Ellipsis =>
                Self::build_operator_token(token, OperatorTokenType::Ellipsis),
            | PreprocessorTokenType::HashHash => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashHashUsedOutsideOfMacro,
                    source_vectors: token.source_vectors,
                });
                return None;
            },

            | x => todo!("{x:#?}"),
        })
    }

    fn parse_hash_hash_operator(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = self.handle_macro_argument(context, rhs) {
            self.push_tokenizer_frame(context, frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs);
            false
        };
        if let Some(frame) = self.handle_macro_argument(context, lhs) {
            self.push_tokenizer_frame(context, frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs);
        } else {
            _ = self.hash_hash_stack.pop();
            return self.merge_tokens(context, lhs, rhs);
        }
        None
    }

    fn parse_hash_operator(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> PreprocessorToken {
        eprintln!("Parsing hash operator");
        let position = self.save_position(context);
        let Some(argument_name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing '#' operator in function-like macro invocation.",
        ) else {
            self.restore_position(context, position);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::GeneratedString,
                contents:       context.string_cache.intern(""),
                source_vectors: token.source_vectors,
            };
        };
        let (mut token_tokenizer, argument_id) = match self.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match arguments.get(&argument_name.contents) {
                | None => {
                    self.restore_position(context, position);
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                context.string_cache.at(argument_name.contents).to_owned(),
                            ),
                        source_vectors: argument_name.source_vectors,
                    });
                    return PreprocessorToken {
                        kind:           PreprocessorTokenType::GeneratedString,
                        contents:       context.string_cache.intern(""),
                        source_vectors: token.source_vectors,
                    };
                },
                | Some(v) => (v.tokenizer.clone(), v.name),
            },
            | _ => unreachable!(),
        };
        let mut last_was_whitespace = true;
        let mut synthetic_contents = String::new();
        let mut paren_depth = 1;
        'base: loop {
            let Some(token) = Self::next_treat_newlines_as_whitespace(
                &mut token_tokenizer,
                context,
                &mut last_was_whitespace,
            ) else {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing '#' operator in function-like macro invocation",
                    ),
                    source_vectors: argument_name.source_vectors,
                });
                break 'base;
            };
            match self.update_macro_argument_paren_depth(context, token, argument_id, paren_depth) {
                | Some(depth) => paren_depth = depth,
                | None => break,
            }
            let mut s = context.string_cache.at(token.contents);
            if token.kind == PreprocessorTokenType::Number {
                // Remove trailing null byte.
                s = &s[..s.len() - 1];
            }
            synthetic_contents.push_str(s);
        }
        PreprocessorToken {
            kind:           PreprocessorTokenType::GeneratedString,
            contents:       context.string_cache.intern(&synthetic_contents),
            source_vectors: token.source_vectors,
        }
    }

    fn parse_directive(&mut self, context: &mut Context, token: PreprocessorToken) {
        if !self.last_was_newline {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                source_vectors: token.source_vectors,
            });
        }
        let Some(directive) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
            return;
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => return,
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined | PreprocessorTokenType::Identifier => (),
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
                return;
            },
        }
        match context.string_cache.at(directive.contents) {
            | "if" => self.parse_if_directive(context, directive),
            | "ifdef" => self.parse_ifdef_directive(context, directive),
            | "ifndef" => self.parse_ifndef_directive(context, directive),
            | "elif" => self.parse_elif_directive(context, directive),
            | "else" => self.parse_else_directive(context, directive),
            | "endif" => self.parse_endif_directive(context, directive),
            | "include" => self.parse_include_directive(context, directive),
            | "define" => self.parse_define_directive(context, directive),
            | "undef" => self.parse_undef_directive(context, directive),
            | "line" => self.parse_line_directive(context, directive),
            | "error" => self.parse_error_directive(context, directive),
            | "pragma" => self.parse_pragma_directive(context, directive),
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
            },
        }
    }

    fn map_operator(
        &mut self,
        _context: &Context,
        operator: PreprocessorToken,
    ) -> PreprocessorExpressionOperator {
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

    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_lines)]
    fn handle_expression_operator(
        &mut self,
        context: &mut Context,
        op: PreprocessorExpressionOperator,
    ) {
        match op {
            | PreprocessorExpressionOperator::UnaryPlus =>
                if self.expression_parser.operand_stack.is_empty() {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryPlusWithoutOperand,
                        source_vectors,
                    });
                },
            | PreprocessorExpressionOperator::UnaryMinus => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryMinusWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                let (new, did_overflow) = operand.as_signed().overflowing_neg();
                if operand.is_signed() && did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryMinusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser
                    .operand_stack
                    .push(operand.set_signed(new));
            },
            | PreprocessorExpressionOperator::BitwiseNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(operand.map_unsigned(|v| !v));
            },
            | PreprocessorExpressionOperator::LogicalNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        operand.as_signed() == 0,
                    )));
            },
            | PreprocessorExpressionOperator::BinaryPlus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BinaryPlus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryPlusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_add(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryPlusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::BinaryMinus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BinaryMinus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryMinusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_sub(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryMinusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Multiply => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Multiply operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MultiplyWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_mul(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MultiplyOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Divide => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Divide operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideByZero,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = lhs.as_signed().overflowing_div(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Modulo => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Modulo operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloByZero,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = lhs.as_signed().overflowing_rem(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LeftShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LeftShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LeftShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = lhs.as_signed().overflowing_shl(rhs.as_unsigned() as u32);
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LeftShiftOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::RightShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: RightShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::RightShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = lhs.as_signed().overflowing_shr(rhs.as_unsigned() as u32);
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::RightShiftOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LessThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LessThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() < rhs.as_unsigned()
                } else {
                    lhs.as_signed() < rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::LessThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LessThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() <= rhs.as_unsigned()
                } else {
                    lhs.as_signed() <= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: GreaterThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() > rhs.as_unsigned()
                } else {
                    lhs.as_signed() > rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: GreaterThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() >= rhs.as_unsigned()
                } else {
                    lhs.as_signed() >= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::Equals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Equals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::EqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() == rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::NotEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: NotEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::NotEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() != rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::BitwiseAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() & rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseXor => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseXor operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseXorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() ^ rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() | rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::LogicalAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LogicalAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() != 0 && rhs.as_signed() != 0,
                    )));
            },
            | PreprocessorExpressionOperator::LogicalOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LogicalOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() != 0 || rhs.as_signed() != 0,
                    )));
            },
            | PreprocessorExpressionOperator::Else => assert!(
                !self.expression_parser.operand_stack.is_empty(),
                "Compiler bug: Else operator without lhs"
            ),
            | PreprocessorExpressionOperator::Ternary => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Ternary operator without condition");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let Some(condition) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(if condition.as_signed() != 0 { lhs } else { rhs });
            },
            | PreprocessorExpressionOperator::OpeningParenthesis => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_name(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    source_vectors,
                });
            },
        }
    }

    fn parse_defined_operator(&mut self, context: &mut Context) {
        let Some(ident_or_opening_paren) = self.expect_token_from_previous_phase::<true>(context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::Identifier | PreprocessorTokenType::OpeningParenthesis),
            |_, _, t|
                ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(t.kind),
                        source_vectors: t.source_vectors,
                    },
                )
            ,
            "parsing defined operator",
        ) else {
            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
            return;
        };
        if ident_or_opening_paren.kind == PreprocessorTokenType::Identifier {
            let is_defined = self
                .macro_definitions
                .contains_key(&ident_or_opening_paren.contents);
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        }
        assert_eq!(
            ident_or_opening_paren.kind,
            PreprocessorTokenType::OpeningParenthesis,
            "Compiler bug: ident_or_opening_paren should be an opening parenthesis or identifier."
        );
        let Some(ident) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::Identifier),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                        t.kind,
                    ),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        ) else {
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
            return;
        };

        _ = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::ClosingParenthesis),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        );
        let is_defined = self.macro_definitions.contains_key(&ident.contents);
        self.expression_parser
            .operand_stack
            .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
        self.expression_parser.state = PreprocessorExpressionParserState::Binary;
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

    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_precision_loss)]
    fn eval_preprocessor_expression(
        &mut self,
        context: &mut Context,
        on_no_expression_error: PreprocessorErrorType,
    ) -> bool {
        const UNARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Unary;
        const BINARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Binary;
        self.expression_parser.reset();
        'main: loop {
            match self.next_preprocessor_token::<true>(context) {
                | None => {
                    let source_vectors = context.create_source_vectors(self.position(context), self.source_file_name(), 0);
                    context.preprocessor_error(
                        PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                            source_vectors,
                        });
                    break 'main;
                },
                | Some(token) => match (token.kind, self.expression_parser.state) {
                    | (PreprocessorTokenType::Newline, _) =>
                        break 'main,
                    (PreprocessorTokenType::Plus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryPlus),
                    (PreprocessorTokenType::Minus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryMinus),
                    (PreprocessorTokenType::Tilde, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::BitwiseNot),
                    (PreprocessorTokenType::Tilde, BINARY) =>
                        context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                ,
                    (PreprocessorTokenType::ExclamationMark, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::LogicalNot),
                    (PreprocessorTokenType::ExclamationMark, BINARY) =>
                        context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    (PreprocessorTokenType::OpeningParenthesis, UNARY) => {
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::OpeningParenthesis);
                    }
                    (PreprocessorTokenType::OpeningParenthesis, BINARY) => {
                        context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        );
                        let mut paren_depth = 1;
                        // Step over function call.
                        while paren_depth > 0 {
                            match self.next_preprocessor_token::<true>(context) {
                                | None => {
                                    let source_vectors = context.create_source_vectors(self.position(context), self.source_file_name(), 0);
                                    context.preprocessor_error(PreprocessorError {
                                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                                            source_vectors,
                                        },
                                    );
                                    break 'main;
                                },
                                | Some(token) => match token.kind {
                                    | PreprocessorTokenType::OpeningParenthesis => paren_depth += 1,
                                    | PreprocessorTokenType::ClosingParenthesis => paren_depth -= 1,
                                    | _ => (),
                                },
                            }
                        }
                    }
                    (PreprocessorTokenType::ClosingParenthesis, _) => {
                        self.expression_parser.state = BINARY;
                        if !self.expression_parser.operator_stack.last().is_some_and(|op| *op != PreprocessorExpressionOperator::OpeningParenthesis) {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue 'main;
                        }
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op == PreprocessorExpressionOperator::OpeningParenthesis {
                                break;
                            }
                            self.handle_expression_operator(context, op);
                        }
                    }
                    (PreprocessorTokenType::Defined, UNARY) =>
                        self.parse_defined_operator(context),
                    (PreprocessorTokenType::Defined, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Asterisk, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Ampersand, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (
                        | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                        PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                        PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals |
                        PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                        PreprocessorTokenType::Colon, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(self.map_operator(context, token)),
                            source_vectors: token.source_vectors,
                        }),
                    | (PreprocessorTokenType::Plus | PreprocessorTokenType::Minus | PreprocessorTokenType::Asterisk
                    | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                    PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                    PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals | PreprocessorTokenType::Ampersand |
                    PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                    PreprocessorTokenType::Colon, BINARY) => {
                        let token_op = self.map_operator(context, token);
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op.has_precedence_over(token_op) {
                                self.handle_expression_operator(context, op);
                            } else {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            }
                        }
                        self.expression_parser.operator_stack.push(token_op);
                        self.expression_parser.state = UNARY;
                    },
                    (PreprocessorTokenType::Number, UNARY) => {
                        match self.parse_number(context, token,).kind {
                            | TokenType::Float(f) => {
                                context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                });
                                match f {
                                    FloatTokenType::Float(f) => {
                                        if f.fract() != 0.0 || !f.is_finite() || f < i32::MIN as f32 || f > u32::MIN as f32 {
                                            context.preprocessor_error(PreprocessorError {
                                                error_type: PreprocessorErrorType::FloatCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(f),
                                                source_vectors: token.source_vectors,
                                            });
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                                        } else if f.is_sign_negative() {
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(f as i64));
                                        } else {
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(f as u64));
                                        }
                                    }
                                    FloatTokenType::Double(d) => {
                                        if d.fract() != 0.0 || !d.is_finite() || d < i64::MIN as f64 || d > u64::MIN as f64 {
                                            context.preprocessor_error(PreprocessorError {
                                                error_type: PreprocessorErrorType::DoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(d),
                                                source_vectors: token.source_vectors,
                                            });
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                                        } else if d.is_sign_negative() {
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(d as i64));
                                        } else {
                                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(d as u64));
                                        }
                                    }
                                    FloatTokenType::LongDouble(ld) => {
                                        match long_double_to_operand(ld) {
                                            Err(e) => {
                                                match e {
                                                    1 => context.preprocessor_error(PreprocessorError {
                                                        error_type: PreprocessorErrorType::LongDoubleCouldNotBeLosslesslyConvertedToIntegerInPreprocessorExpression(ld),
                                                        source_vectors: token.source_vectors,
                                                    }),
                                                    _ => unreachable!("Compiler bug: long_double_to_operand should only return 1 or 0."),
                                                }
                                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                                            }
                                            Ok(o) => self.expression_parser.operand_stack.push(o),
                                        }
                                    }
                                }
                            },
                            | TokenType::Integer(v) => match v {
                                | IntegerTokenType::Int(i) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(i64::from(i))),
                                | IntegerTokenType::Long(l) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(l)),
                                | IntegerTokenType::LongLong(ll) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(ll)),
                                | IntegerTokenType::UnsignedInt(ui) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(u64::from(ui))),
                                | IntegerTokenType::UnsignedLong(ul) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ul)),
                                | IntegerTokenType::UnsignedLongLong(ull) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ull)),
                            },
                            _ => unreachable!("Compiler bug: parse_number should return a number token."),
                        };
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Number, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Identifier, UNARY) => {
                        context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(context.string_cache.at(token.contents).to_owned()),
                                    source_vectors: token.source_vectors,
                                },
                        );
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Identifier, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Character, UNARY) => {
                        let value = i64::from(u32::from(char::from(self.parse_character(context, token))));
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(value));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Character, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    _ => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                            source_vectors: token.source_vectors,
                        },
                    ),
                },
            }
        }
        if self.expression_parser.state == UNARY {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_name(), 0);
            context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(
                        self.last_binary_operator(),
                    ),
                    source_vectors
                }
            );
        }
        while let Some(op) = self.expression_parser.operator_stack.pop() {
            self.handle_expression_operator(context, op);
        }
        match self.expression_parser.operand_stack.len() {
            | 1 =>
                self.expression_parser
                    .operand_stack
                    .pop()
                    .unwrap()
                    .as_signed()
                    != 0,
            | 0 => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_name(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type: on_no_expression_error,
                    source_vectors,
                });
                true
            },
            | _ => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_name(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression,
                    source_vectors,
                });
                true
            },
        }
    }

    fn skip_over_dead_code(&mut self, context: &mut Context) {
        let start_balance = self.if_directive_balance;
        context.set_is_skipping_over_dead_code(true);
        'outer: while start_balance <= self.if_directive_balance {
            match self.tokenizer.next_item(context) {
                | Some(token) => match token.kind {
                    | PreprocessorTokenType::Newline => {
                        'newline: loop {
                            match self.tokenizer.next_item(context) {
                                | Some(token) if token.kind != PreprocessorTokenType::Hash =>
                                    if token.kind == PreprocessorTokenType::Newline {
                                        continue 'newline;
                                    } else {
                                        continue 'outer;
                                    },
                                | Some(_) => break 'newline,
                                | None => {
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing dead code. Expected #endif instead",
                                        ),
                                        source_vectors: token.source_vectors,
                                    });
                                    return;
                                },
                            }
                        }
                        let directive_name = match Self::next_ignore_whitespace(
                            &mut self.tokenizer,
                            context,
                        ) {
                            | Some(token) if token.kind == PreprocessorTokenType::Identifier =>
                                token,
                            | Some(token) => {
                                context.preprocessor_error(PreprocessorError {
                                            error_type: PreprocessorErrorType::ExpectedIdentifierInPreprocessorDirective(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    );
                                return;
                            },
                            | None => {
                                context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                        "parsing dead code. Expected #endif instead.",
                                    ),
                                    source_vectors: token.source_vectors,
                                });
                                return;
                            },
                        };
                        match context.string_cache.at(directive_name.contents) {
                            | "endif" => {
                                self.if_directive_balance -= 1;
                            },
                            | "if" | "ifndef" | "ifdef" => {
                                self.if_directive_balance += 1;
                            },
                            | "elif" if self.if_directive_balance == start_balance => {
                                if self.eval_preprocessor_expression(
                                    context,
                                    PreprocessorErrorType::NoConditionInElifDirective,
                                ) {
                                    break;
                                }
                            },
                            | "else" if self.if_directive_balance == start_balance => break,
                            | _ => (),
                        }
                    },
                    | _ => continue,
                },
                | None => {
                    context.set_is_skipping_over_dead_code(false);
                    return;
                },
            }
        }
        context.set_is_skipping_over_dead_code(false);
    }

    fn parse_if_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        self.if_directive_balance += 1;
        if self
            .eval_preprocessor_expression(context, PreprocessorErrorType::NoConditionInIfDirective)
        {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context);
        }
    }

    fn parse_elif_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        if self.if_directive_balance <= 0 {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ElifDirectiveWithoutIfDirective,
                source_vectors: directive.source_vectors,
            });
        }
        self.skip_until_newline(context);
        // Elif directives only matter if we are currently skipping over dead
        // code.
    }

    fn parse_else_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        if self.if_directive_balance <= 0 {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
                source_vectors: directive.source_vectors,
            });
        }
        self.skip_until_newline(context);
        // Else directives only matter if we are currently skipping over dead
        // code.
    }

    fn parse_endif_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.if_directive_balance -= 1;
        if self.if_directive_balance < 0 {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                source_vectors: directive.source_vectors,
            });
        }
        self.skip_until_newline(context);
    }

    fn parse_ifdef_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        self.if_directive_balance += 1;
        if let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing ifdef directive",
        ) {
            if !self.macro_definitions.contains_key(&name.contents) {
                self.skip_over_dead_code(context);
                return;
            }
        }
        if self
            .expect_token_from_previous_phase::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::ExtraTokensAfterIfdefDirective,
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing ifdef directive.",
            )
            .is_none()
        {
            self.skip_until_newline(context);
        }
    }

    fn parse_ifndef_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        self.if_directive_balance += 1;
        if let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing ifndef directive",
        ) {
            if !self.macro_definitions.contains_key(&name.contents) {
                self.skip_over_dead_code(context);
                return;
            }
        }
        if self
            .expect_token_from_previous_phase::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::ExtraTokensAfterIfndefDirective,
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing ifndef directive.",
            )
            .is_none()
        {
            self.skip_until_newline(context);
        }
    }

    fn search_for_header_in(path: &Path, dirs: &[PathBuf]) -> Option<PathBuf> {
        for dir in dirs {
            let mut header_path = dir.clone();
            header_path.push(path);
            if header_path.exists() {
                return Some(header_path);
            }
        }
        None
    }

    fn find_header_from_path(
        &mut self,
        context: &mut Context,
        include_token: PreprocessorToken,
        path: &Path,
        is_system_header: bool,
    ) -> Option<SharedPath> {
        let header = 'ret: {
            if path.is_absolute() {
                if path.exists() {
                    break 'ret path.to_owned();
                }
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HeaderNotFound,
                    source_vectors: include_token.source_vectors,
                });
                return None;
            }

            if is_system_header {
                if let Some(header) =
                    Self::search_for_header_in(path, &self.system_include_directories)
                {
                    break 'ret header;
                }
            }
            if let Some(header) = Self::search_for_header_in(path, &self.quote_include_directories)
            {
                break 'ret header;
            }

            let Ok(cwd) = std::env::current_dir().map_err(|e| {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::CurrentWorkingDirectoryInaccessible(e),
                    source_vectors: include_token.source_vectors,
                });
            }) else {
                return None;
            };
            if let Some(header) = Self::search_for_header_in(path, &[cwd]) {
                break 'ret header;
            }

            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderNotFound,
                source_vectors: include_token.source_vectors,
            });
            return None;
        };
        let header = SharedPath::from_path_buf(header);
        if self.once_set.contains(&header) {
            None
        } else {
            Some(header)
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn parse_include_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        context.set_is_tokenizing_include_string(true);
        let Some(include_string) =
            self.expect_token::<true>(
                context,
                |_, context, token| match token.kind {
                    | PreprocessorTokenType::AngleBracketString
                    | PreprocessorTokenType::IncludeString => true,
                    | _ if context.string_cache.at(token.contents).starts_with('<') => true,
                    | _ => false,
                },
                |_, _, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing include directive",
            )
        else {
            return;
        };
        context.set_is_tokenizing_include_string(false);
        let header_path = match include_string.kind {
            | PreprocessorTokenType::IncludeString => {
                let contents = context.string_cache.at(include_string.contents);
                let contents = &contents[1..contents.len() - 1].to_token_string();
                let path = Path::new(contents.as_str());
                if self
                    .expect_token_from_previous_phase::<true>(
                        context,
                        |_, _, t| t.kind == PreprocessorTokenType::Newline,
                        |_, _, t| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                                source_vectors: t.source_vectors,
                            })
                        },
                        "parsing include directive.",
                    )
                    .is_none()
                {
                    self.skip_until_newline(context);
                }

                self.find_header_from_path(context, include_string, path, false)
            },
            | PreprocessorTokenType::AngleBracketString => {
                let contents = context.string_cache.at(include_string.contents);
                let contents = &contents[1..contents.len() - 1].to_token_string();
                let path = Path::new(contents.as_str());
                if self
                    .expect_token_from_previous_phase::<true>(
                        context,
                        |_, _, t| t.kind == PreprocessorTokenType::Newline,
                        |_, _, t| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                                source_vectors: t.source_vectors,
                            })
                        },
                        "parsing include directive.",
                    )
                    .is_none()
                {
                    self.skip_until_newline(context);
                }
                self.find_header_from_path(context, include_string, path, true)
            },
            | _ => {
                let mut contents = TokenString::new();
                contents.push_str(&context.string_cache.at(include_string.contents)[1..]);
                let start_index = Context::duplicate_source_vectors(
                    &mut context.source_vectors.0,
                    include_string.source_vectors,
                );
                loop {
                    match self.next_preprocessor_token::<false>(context) {
                        | Some(token) => {
                            if token.kind == PreprocessorTokenType::Newline {
                                break;
                            }
                            let token_contents = context.string_cache.at(token.contents);
                            _ = Context::duplicate_source_vectors(
                                &mut context.source_vectors.0,
                                token.source_vectors,
                            );
                            if let Some(idx) = token_contents.find('>') {
                                contents.push_str(&token_contents[..idx]);
                                break;
                            }
                            contents.push_str(context.string_cache.at(token.contents));
                        },
                        | None => {
                            context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                    "parsing include directive",
                                ),
                                source_vectors: directive.source_vectors,
                            });
                            break;
                        },
                    }
                }
                let path = Path::new(contents.as_str());
                let synthetic_token = PreprocessorToken {
                    source_vectors: SourceVectors {
                        start_index,
                        length: context.source_vectors.0.len() as u32 - start_index,
                    },
                    contents:       context.string_cache.intern(&contents),
                    kind:           PreprocessorTokenType::AngleBracketString,
                };
                self.find_header_from_path(context, synthetic_token, path, true)
            },
        };
        let Some(header_path) = header_path else {
            return;
        };
        let Ok(header_string) = read_to_string_lossy(&header_path).map_err(|e| {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderFileInaccessible(e),
                source_vectors: directive.source_vectors,
            });
        }) else {
            return;
        };
        self.push_tokenizer_frame(
            context,
            TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                tokenizer:  PreprocessorTokenizer::new(
                    header_path,
                    SharedString::from(header_string),
                ),
            },
        );
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_define_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing define directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        let old_definition = self.macro_definitions.get(&name.contents).cloned();
        let tokenizer = self.tokenizer.clone();
        let old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                | MacroDefinition::BuiltIn => {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                    None
                },
            },
        };
        let opening_paren = match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
            | Some(
                token @ PreprocessorToken {
                    kind: PreprocessorTokenType::OpeningParenthesis,
                    ..
                },
            ) => Some(token),
            | Some(_) => None,
            | None => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing macro definition",
                    ),
                    source_vectors: name.source_vectors,
                });
                None
            },
        };
        if opening_paren.is_some() {
            if old_definition.is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                | MacroDefinition::FunctionLike { .. } => false,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            let mut argument_names = Vec::new();
            let mut is_variadic = false;
            loop {
                let Some(name_or_ellipsis) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| {
                        t.kind == PreprocessorTokenType::Identifier
                            || t.kind == PreprocessorTokenType::Ellipsis
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
                    },
                    |_, _, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(
                                    token.kind,
                                ),
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing macro definition",
                ) else {
                    break;
                };
                if is_variadic {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind == PreprocessorTokenType::Identifier {
                    argument_names.push(name_or_ellipsis.contents);
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis,
                    |_, _, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            let tokenizer = self.tokenizer.clone();
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::FunctionLike {
                    tokenizer,
                    argument_names: argument_names.into(),
                    is_variadic,
                },
            ));
        } else {
            if old_definition.is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => false,
                | MacroDefinition::FunctionLike { .. } => true,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::ObjectLike {
                    tokenizer:           tokenizer.clone(),
                },
            ));
        }
        let mut last = Option::<PreprocessorToken>::None;
        if let Some(mut old_tokenizer) = old_tokenizer {
            let mut error_has_been_generated = false;
            let mut new_tokenizer = tokenizer;
            loop {
                let old_next = old_tokenizer.next_item(context);
                let new_next = new_tokenizer.next_item(context);
                if let Some(t) = new_next.as_ref() {
                    if t.kind == PreprocessorTokenType::HashHash {
                        // Hash-hash tokens cannot be created as a result of token pasting, so they
                        // will always have only one source vector.
                        _ = hash_hash_positions.insert(
                            context.source_vectors.0[t.source_vectors.start_index as usize]
                                .position(context),
                        );
                        if last.is_none() {
                            context.preprocessor_error(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                                source_vectors: t.source_vectors,
                            });
                        }
                    }
                }
                if old_next != new_next && !error_has_been_generated {
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                context.string_cache.at(name.contents).to_owned(),
                            ),
                        source_vectors: name.source_vectors,
                    });
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
                        context.preprocessor_error(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                            source_vectors: last.unwrap().source_vectors,
                        });
                    }
                    break;
                }
                last = new_next;
            }
        } else {
            loop {
                match self.tokenizer.next_item(context) {
                    | Some(token) if token.kind == PreprocessorTokenType::HashHash => {
                        _ = hash_hash_positions.insert(
                            context.source_vectors.0[token.source_vectors.start_index as usize]
                                .position(context),
                        );
                    },
                    | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(_) => continue,
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
            *Rc::get_mut(hhp).unwrap() = hash_hash_positions;
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_undef_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        drop(self.macro_definitions.remove(&name.contents));
        _ = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Newline,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        );
    }

    #[allow(clippy::cast_possible_truncation)]
    fn parse_line_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(token) = self.expect_token::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Number,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        ) else {
            self.skip_and_expand_until_newline(context);
            return;
        };
        let (value, did_overflow) = {
            let mut value = 0i128;
            let mut did_generate_error = false;
            let mut did_overflow = false;
            for b in context.string_cache.at(token.contents).bytes() {
                if !b.is_ascii_digit() && !did_generate_error {
                    did_generate_error = true;
                    Context::raw_preprocessor_error(
                        &mut context.pending_errors,
                        PreprocessorError {
                            error_type:
                                PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                            source_vectors: token.source_vectors,
                        },
                    );
                    continue;
                }
                value *= 10;
                if value > i128::from(i32::MAX) {
                    did_overflow = true;
                }
                value += i128::from(b - b'0');
                if value > i128::from(i32::MAX) {
                    did_overflow = true;
                }
            }
            (value, did_overflow)
        };
        if did_overflow {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(value),
                source_vectors: token.source_vectors,
            });
        } else {
            self.set_line(context, value as u32);
        }
        let name = self.expect_token::<true>(
            context,
            |_, _, t| {
                matches!(
                    t.kind,
                    PreprocessorTokenType::String | PreprocessorTokenType::Newline
                )
            },
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        );
        if let Some(
            t @ PreprocessorToken {
                kind: PreprocessorTokenType::String,
                ..
            },
        ) = name
        {
            self.set_source_file_name(
                context,
                SharedPath::from_path_buf(PathBuf::from(context.string_cache.at(t.contents))),
            );
            _ = self.expect_token::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(
                            t.kind,
                        ),
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing line directive",
            );
        }
    }

    fn parse_error_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let mut contents = String::new();
        loop {
            match self.tokenizer.next_item(context) {
                | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                | Some(token) => {
                    let mut s = context.string_cache.at(token.contents);
                    if token.kind == PreprocessorTokenType::Number {
                        // Remove trailing null byte.
                        s = &s[..s.len() - 1];
                    }
                    contents.push_str(s);
                },
                | None => {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing error directive",
                        ),
                        source_vectors,
                    });
                    break;
                },
            }
        }
        let source_vectors =
            context.create_source_vectors(self.position(context), self.source_file_name(), 0);
        context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::ErrorDirective(contents),
            source_vectors,
        });
    }

    fn parse_pragma_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        'base: loop {
            let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_name(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing pragma directive",
                    ),
                    source_vectors,
                });
                break 'base;
            };
            match token.kind {
                | PreprocessorTokenType::Whitespace => continue 'base,
                | PreprocessorTokenType::Newline => break 'base,
                | PreprocessorTokenType::Identifier => {
                    match context.string_cache.at(token.contents) {
                        | "once" => {
                            if !self.current_is_header(context) {
                                context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::PragmaOnceInNonHeader,
                                    source_vectors: token.source_vectors,
                                });
                            }
                            _ = self.once_set.insert(self.source_file_name());
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline =>
                                    break 'base,
                                | Some(t) => {
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                t.kind,
                                            ),
                                        source_vectors: token.source_vectors,
                                    });
                                    break 'base;
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_name(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | "STDC" => {
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if token.kind != PreprocessorTokenType::Identifier
                                        || !matches!(
                                            s,
                                            "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE"
                                        )
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                    s.to_owned(),
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_name(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }

                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if token.kind != PreprocessorTokenType::Identifier
                                        || !matches!(s, "ON" | "OFF" | "DEFAULT")
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(s.to_owned()),
                                                    source_vectors: token.source_vectors,
                                                },
                                            );
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_name(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | _ => break 'base,
                    }
                },
                | _ => {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                        source_vectors: token.source_vectors,
                    });
                    break 'base;
                },
            }
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn parse_integer_radix(
        &mut self,
        context: &mut Context,
        radix: u32,
        start_index: usize,
        invalid_integer_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
    ) -> Token {
        _ = self;
        let mut index = start_index;
        let (result, did_overflow) = {
            let contents = context.string_cache.at(token.contents);
            let mut result = 0u64;
            let mut did_overflow = false;
            while let Some(digit) = contents.char_at(index).and_then(|c| c.to_digit(radix)) {
                let (new_result, overflow) = result.overflowing_mul(u64::from(radix));
                if overflow {
                    did_overflow = true;
                }
                result = new_result;
                let (new_result, overflow) = result.overflowing_add(u64::from(digit));
                if overflow {
                    did_overflow = true;
                }
                result = new_result;
                index += 1;
            }
            (result, did_overflow)
        };
        if did_overflow {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::IntegerLiteralOverflow,
                source_vectors: token.source_vectors,
            });
        }
        let contents = context.string_cache.at(token.contents);
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
        if index != contents.len() - 1 {
            context.preprocessor_error(PreprocessorError {
                error_type:     invalid_integer_literal_error,
                source_vectors: token.source_vectors,
            });
        }
        match suffix_type {
            | Some(IntegerSuffix::UnsignedLongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::LongLong) if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::LongLong,
                        to:   UnsignedIntegerLiteralType::UnsignedLongLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::LongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::LongLong(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::UnsignedLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::Long) if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::Long,
                        to:   UnsignedIntegerLiteralType::UnsignedLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::Long) => Token {
                kind:           TokenType::Integer(IntegerTokenType::Long(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::Unsigned) if result > u64::from(u32::MAX) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedUnsignedPromotion {
                        from: UnsignedIntegerLiteralType::UnsignedInt,
                        to:   UnsignedIntegerLiteralType::UnsignedLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            #[allow(clippy::cast_possible_truncation)]
            | Some(IntegerSuffix::Unsigned) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedInt(result as u32)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | None if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::Int,
                        to:   UnsignedIntegerLiteralType::UnsignedLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | None if result > i32::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedPromotion {
                        from: SignedIntegerLiteralType::Int,
                        to:   SignedIntegerLiteralType::Long,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::Long(result as i64)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            #[allow(clippy::cast_possible_truncation)]
            | None => Token {
                kind:           TokenType::Integer(IntegerTokenType::Int(result as i32)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
        }
    }

    fn parse_hexadecimal_integer(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_integer_radix(
            context,
            16,
            2,
            PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            token,
        )
    }

    fn parse_binary_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            2,
            2,
            PreprocessorErrorType::InvalidBinaryIntegerLiteral,
            token,
        )
    }

    fn parse_octal_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            8,
            1,
            PreprocessorErrorType::InvalidOctalIntegerLiteral,
            token,
        )
    }

    fn parse_decimal_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            10,
            0,
            PreprocessorErrorType::InvalidDecimalIntegerLiteral,
            token,
        )
    }

    #[allow(clippy::inline_always)]
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn parse_float(
        &mut self,
        context: &mut Context,
        invalid_float_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
    ) -> Token {
        _ = self;
        let contents = context.string_cache.at(token.contents);

        let res = match contents.char_at(contents.len() - 2) {
            | Some('f' | 'F') => string_to_float(contents).map(FloatTokenType::Float),
            | Some('l' | 'L') => string_to_long_double(contents).map(FloatTokenType::LongDouble),
            | _ => string_to_double(contents).map(FloatTokenType::Double),
        };
        match res {
            | Ok(kind) => Token {
                kind:           TokenType::Float(kind),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Err(ParseFloatError::Invalid(kind)) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     invalid_float_literal_error,
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Float(kind),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Err(ParseFloatError::Overflow(kind)) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::FloatLiteralOverflow(kind),
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Float(kind),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
        }
    }

    fn parse_hexadecimal_float(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidHexadecimalFloatLiteral,
            token,
        )
    }

    fn parse_decimal_float(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidDecimalFloatLiteral,
            token,
        )
    }
}

impl TranslationPhase for Preprocessor {
    type Item = Token;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        loop {
            let Some(token) = self.next_preprocessor_token::<true>(context) else {
                if self.if_directive_balance != 0 {
                    self.if_directive_balance = 0;
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_name(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(context, token) {
                return Some(result);
            }
        }
    }
}
