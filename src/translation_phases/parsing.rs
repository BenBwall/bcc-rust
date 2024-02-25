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
    preprocessing::{
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
use crate::util::string_cache::StringCacheId;

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser<Prev> {
    pub(crate) previous_phase: Prev,
    pub(crate) state_stack:    Vec<State>,
    pub(crate) types:          Vec<Type>,
    pub(crate) token_stack:    Vec<Token>,
    pub(crate) expressions:    Vec<Expression>,
    pub(crate) statements:     Vec<Statement>,
    pub(crate) block_depth:    usize,
}

impl<Prev> Parser<Prev> {
    fn new(prev: Prev) -> Self {
        Self {
            previous_phase: prev,
            state_stack:    vec![State::ParsingTopLevelStatement, State::ParsingType],
            types:          Vec::new(),
            token_stack:    Vec::new(),
            expressions:    Vec::new(),
            statements:     Vec::new(),
            block_depth:    0,
        }
    }
}

impl<Prev> Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token>,
{
    fn parse_statement(&mut self) -> Result<Statement, ParsingError<Prev::Error>> {
        todo!();
    }

    fn parse_expression(&mut self) -> Result<Expression, ParsingError<Prev::Error>> {
        todo!();
    }

    fn parse_type(&mut self) -> Result<Type, ParsingError<Prev::Error>> {
        todo!();
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

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
struct ExpressionIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
struct StatementIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
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
    pub(crate) type_index:    TypeIndex,
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

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct FunctionTypeArgument {
    pub(crate) type_index: TypeIndex,
    pub(crate) name:       Option<Identifier>,
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
    Function {
        return_type_index: Option<usize>,
        /// None symbolizes a function with an unspecified number of arguments
        /// (i.e. `int f()`).
        parameters:        Option<Arc<[FunctionTypeArgument]>>,
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

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParsingError {
    pub(crate) error_type:     ParserErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for ParsingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl GetSeverity for ParsingError {
    fn severity(&self) -> ErrorSeverity {
        self.error_type.severity()
    }
}

impl GetPosition for ParsingError {
    fn position(&self) -> SourcePosition {
        self.source_vectors[0]
    }
}

impl std::error::Error for ParsingError {}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SavePoint<PrevSavePoint> {
    pub(crate) state_stack:    Vec<State>,
    pub(crate) types:          Vec<Type>,
    pub(crate) previous_phase: PrevSavePoint,
    pub(crate) token_stack:    Vec<Token>,
    pub(crate) block_depth:    usize,
    pub(crate) expressions:    Vec<Expression>,
    pub(crate) statements:     Vec<Statement>,
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
    type Item = Result<TopLevelStatement, ParsingError<PrevError>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.state_stack.pop().unwrap() {
                | State::ParsingTopLevelStatement => {
                    let type_ = match self.parse_type() {
                        | Ok(t) => t,
                        | Err(e) => return Some(Err(ParsingError::PreviousPhaseError(e))),
                    };
                    match type_.kind {
                        TypeKind::Function { return_type_index, parameters }
                    }
                },
                | State::ParsingType => self.types.push(match self.parse_type() {
                    | Ok(t) => t,
                    | Err(e) => return Some(Err(ParsingError::PreviousPhaseError(e))),
                }),
                | State::ParsingStatement => {
                    let statement = match self.parse_statement() {
                        | Ok(s) => s,
                        | Err(e) => return Some(Err(e)),
                    };
                    self.statements.push(statement);
                },
                | State::ParsingExpression => {
                    let expression = match self.parse_expression() {
                        | Ok(e) => e,
                        | Err(e) => return Some(Err(ParsingError::PreviousPhaseError(e))),
                    };
                    self.expressions.push(expression);
                },
            }
        }
        Some(Ok(TopLevelStatement {
            source_vectors: self.previous_phase.current_position().into(),
            kind:           TopLevelStatementType::FunctionDeclaration(FunctionDeclaration {
                name:        Identifier {
                    name: StringCacheId::new(0),
                },
                parameters:  Arc::new([]),
                return_type: None,
            }),
        }))
    }
}

impl<Prev> TranslationPhase for Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token>,
{
    type Error = ParsingError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = TopLevelStatement;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            state_stack:    self.state_stack.clone(),
            types:          self.types.clone(),
            previous_phase: self.previous_phase.save(),
            token_stack:    self.token_stack.clone(),
            block_depth:    self.block_depth,
            expressions:    self.expressions.clone(),
            statements:     self.statements.clone(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.state_stack = save_point.state_stack;
        self.expressions = save_point.expressions;
        self.statements = save_point.statements;
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
