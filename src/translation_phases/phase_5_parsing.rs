use std::{
    fmt::{
        Display,
        Formatter,
        Result as FmtResult,
    },
    sync::Arc,
};

use thiserror::Error;

use super::{
    phase_4_preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        Token,
    },
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    SavePoint as ISavePoint,
    SourcePosition,
    SourceVectors,
    TranslationPhase,
};
use crate::util::string_cache::Id as StringCacheId;

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser<Prev> {
    pub(crate) previous_phase: Prev,
    pub(crate) state:          State,
    pub(crate) types:          Vec<Type>,
    pub(crate) token_stack:    Vec<Token>,
    pub(crate) block_depth:    usize,
}

impl<Prev> Parser<Prev> {
    fn new(prev: Prev) -> Self {
        Self {
            previous_phase: prev,
            state:          State::Default,
            types:          Vec::new(),
            token_stack:    Vec::new(),
            block_depth:    0,
        }
    }
}

impl<Prev> Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token>,
{
    fn parse_top_level_statement(&mut self) -> Result<TopLevelStatement, ParserError<Prev::Error>> {
        todo!();
    }
    fn parse_statement(&mut self) -> Result<Statement, ParserError<Prev::Error>> {
        todo!();
    }
    fn parse_expression(&mut self) -> Result<Expression, ParserError<Prev::Error>> {
        todo!();
    }
    fn parse_type(&mut self) -> Result<Type, ParserError<Prev::Error>> {
        todo!();
    }
    fn map_token(&mut self, token: Token) -> Result<TopLevelStatement, ParserError<Prev::Error>> {
        match self.state {
            State::ParsingTopLevelStatement => self.parse_top_level_statement(token),
            State::ParsingStatement => self.parse_statement(token),
            State::ParsingExpression => self.parse_expression(token),
            State::ParsingType => self.parse_type(token),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    ParsingTopLevelStatement,
    ParsingStatement,
    ParsingExpression,
    ParsingType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TopLevelStatement {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) kind:           TopLevelStatementType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TopLevelStatementType {
    FunctionDeclaration(FunctionDeclaration),
    FunctionDefinition(FunctionDefinition),
    VariableDeclaration(Variable),
    VariableDefinition(VariableDefinition),
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct ExpressionIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct StatementIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct TypeIndex(usize);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) kind:           StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum StatementType {
    Compound(Vec<Statement>),
    Expression(ExpressionIndex),
    If {
        condition_index: ExpressionIndex,
        then_index:      StatementIndex,
        else_index:      Option<StatementIndex>,
    },
    While {
        condition_index: ExpressionIndex,
        body_index:      StatementIndex,
    },
    DoWhile {
        condition_index: ExpressionIndex,
        body_index:      StatementIndex,
    },
    For {
        initializer_index: Option<StatementIndex>,
        condition:         Option<ExpressionIndex>,
        increment:         Option<ExpressionIndex>,
        body:              StatementIndex,
    },
    Return(ExpressionIndex),
    Break,
    Continue,
    Goto(Identifier),
    Label(Identifier, StatementIndex),
    Case(i64, StatementIndex),
    Default(StatementIndex),
    Null,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Expression {
    pub(crate) result_type:    TypeIndex,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) kind:           ExpressionType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ExpressionType {
    Conditional {
        condition_index: ExpressionIndex,
        then_index:      ExpressionIndex,
        else_index:      ExpressionIndex,
    },
    Binary {
        op:          BinaryOperator,
        left_index:  ExpressionIndex,
        right_index: ExpressionIndex,
    },
    Unary {
        op:            UnaryOperator,
        operand_index: ExpressionIndex,
    },
    Call {
        function_index: ExpressionIndex,
        arguments:      Arc<[ExpressionIndex]>,
    },
    CompoundLiteral {
        type_index: Type,
        values:     Arc<[ExpressionIndex]>,
    },
    Identifier(Identifier),
    Constant(Constant),
    StringLiteral(StringCacheId),
    SizeofType(Type),
    SizeofExpr(ExpressionIndex),
    Cast {
        type_index:    Type,
        operand_index: ExpressionIndex,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum BinaryOperator {
    Multiplication,
    Division,
    Modulo,
    Addition,
    Subtraction,
    LeftShift,
    RightShift,
    LessThan,
    GreaterThan,
    LessThanOrEqual,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    Comma,
    Subscript,
    Assignment,
    MultiplicationAssignment,
    DivisionAssignment,
    ModuloAssignment,
    AdditionAssignment,
    SubtractionAssignment,
    LeftShiftAssignment,
    RightShiftAssignment,
    BitwiseAndAssignment,
    BitwiseXorAssignment,
    BitwiseOrAssignment,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum UnaryOperator {
    AddressOf,
    Indirection,
    Plus,
    Minus,
    BitwiseNot,
    LogicalNot,
    PreIncrement,
    PreDecrement,
    PostIncrement,
    PostDecrement,
    Cast,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Identifier {
    pub(crate) name: StringCacheId,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct Variable {
    pub(crate) name:          Identifier,
    pub(crate) type_:         Type,
    pub(crate) storage_class: StorageClass,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct VariableDefinition {
    pub(crate) variable:    Variable,
    pub(crate) initializer: Expression,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum StorageClass {
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum PrimitiveType {
    Char,
    Short,
    Int,
    Long,
    LongLong,
    UnsignedChar,
    UnsignedShort,
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
    Float,
    Double,
    LongDouble,
    Void,
    Bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct Type {
    is_const:    bool,
    is_volatile: bool,
    kind:        TypeKind,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum TypeKind {
    Primitive(PrimitiveType),
    Pointer {
        pointee_index: usize,
    },
    Struct {
        name:   Identifier,
        fields: Arc<[Variable]>,
    },
    Union {
        name:   Identifier,
        fields: Arc<[Variable]>,
    },
    Typedef {
        name:           Identifier,
        referent_index: usize,
    },
    Enum {
        name:   Identifier,
        values: Arc<[EnumValue]>,
    },
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct EnumValue {
    pub(crate) name:  Identifier,
    pub(crate) value: i64,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct FunctionDeclaration {
    pub(crate) name:        Identifier,
    pub(crate) parameters:  Arc<[Variable]>,
    pub(crate) return_type: Option<Type>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct FunctionDefinition {
    pub(crate) declaration: FunctionDeclaration,
    pub(crate) statements:  Vec<Statement>,
    pub(crate) expressions: Vec<Expression>,
}

#[derive(Debug, PartialEq, Clone, Error)]
pub(crate) enum ParserError<PrevError> {
    #[error(transparent)]
    InnerParserError(InnerParserError),
    #[error(transparent)]
    PreviousPhaseError(PrevError),
}

impl<PrevError> GetSeverity for ParserError<PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        match self {
            | ParserError::InnerParserError(e) => e.error_type.severity(),
            | ParserError::PreviousPhaseError(e) => e.severity(),
        }
    }
}

impl<PrevError> GetPosition for ParserError<PrevError>
where
    PrevError: GetPosition,
{
    fn position(&self) -> SourcePosition {
        match self {
            | ParserError::InnerParserError(e) => e.source_vectors[0].position,
            | ParserError::PreviousPhaseError(e) => e.position(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct InnerParserError {
    pub(crate) error_type:     ParserErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for InnerParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl std::error::Error for InnerParserError {}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SavePoint<PrevSavePoint> {
    pub(crate) state:          State,
    pub(crate) types:          Vec<Type>,
    pub(crate) previous_phase: PrevSavePoint,
    pub(crate) token_stack:    Vec<Token>,
    pub(crate) block_depth:    usize,
}

impl<PrevSavePoint> ISavePoint for SavePoint<PrevSavePoint>
where
    PrevSavePoint: ISavePoint,
{
    fn current_position(&self) -> SourcePosition {
        self.previous_phase.current_position()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ParserErrorType {}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match *self {}
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, _f: &mut Formatter<'_>) -> FmtResult {
        match *self {}
    }
}

impl<Prev, PrevError> Iterator for Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token, Error = PrevError>,
{
    type Item = Result<TopLevelStatement, ParserError<PrevError>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.previous_phase.next() {
                | Some(Ok(token)) => match self.map_token(token) {
                    | Ok(()) => {},
                    | Err(e) => {
                        return Some(Err(e));
                    },
                },
                | Some(Err(e)) => {
                    return Some(Err(ParserError::PreviousPhaseError(e)));
                },
                | None => {
                    if self.token_stack.is_empty() {
                        return None;
                    }
                    todo!();
                },
            }
        }
    }
}

impl<Prev> TranslationPhase for Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token>,
{
    type Error = ParserError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = TopLevelStatement;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            state:          self.state,
            types:          self.types.clone(),
            previous_phase: self.previous_phase.save(),
            token_stack:    self.token_stack.clone(),
            block_depth:    self.block_depth,
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.state = save_point.state;
        self.types = save_point.types;
        self.token_stack = save_point.token_stack;
        self.block_depth = save_point.block_depth;
        self.previous_phase.restore(save_point.previous_phase);
    }

    fn current_position(&self) -> SourcePosition {
        self.previous_phase.current_position()
    }

    fn set_line_number(&mut self, line: usize) {
        self.previous_phase.set_line_number(line);
    }
}
