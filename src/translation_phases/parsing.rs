use std::{
    fmt::{
        Display,
        Formatter,
        Result as FmtResult,
    },
    sync::Arc,
};

use super::{
    preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        Preprocessor,
        Token,
    },
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileName,
    GetSourceVectors,
    SetPosition,
    SetSourceFileName,
    SourcePosition,
    SourceVectors,
    TranslationPhase,
};
use crate::util::{
    shared::SharedPath,
    string_cache::StringCacheId,
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser {
    pub(crate) preprocessor: Preprocessor,
    pub(crate) state_stack:  Vec<State>,
    pub(crate) types:        Vec<Type>,
    pub(crate) token_stack:  Vec<Token>,
    pub(crate) expressions:  Vec<Expression>,
    pub(crate) statements:   Vec<Statement>,
    pub(crate) block_depth:  usize,
}

impl GetPosition for Parser {
    fn position(&self, context: &Context) -> SourcePosition {
        self.preprocessor.position(context)
    }
}

impl SetPosition for Parser {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.preprocessor.set_position(context, position);
    }
}

impl GetSourceFileName for Parser {
    fn source_file_name(&self) -> SharedPath {
        self.preprocessor.source_file_name()
    }
}

impl SetSourceFileName for Parser {
    fn set_source_file_name(&mut self, context: &mut Context, name: SharedPath) {
        self.preprocessor.set_source_file_name(context, name);
    }
}

impl Parser {
    #[allow(dead_code)]
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            state_stack: vec![State::ParsingTopLevelStatement, State::ParsingType],
            types: Vec::new(),
            token_stack: Vec::new(),
            expressions: Vec::new(),
            statements: Vec::new(),
            block_depth: 0,
        }
    }
}

impl Parser {
    fn parse_statement(&mut self) -> Statement {
        todo!();
    }

    fn parse_expression(&mut self) -> Expression {
        todo!();
    }

    fn parse_type(&mut self) -> Type {
        todo!();
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[allow(dead_code)]
#[allow(clippy::enum_variant_names)]
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

#[allow(dead_code)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TopLevelStatementType {
    FunctionDeclaration(FunctionDeclaration),
    FunctionDefinition(FunctionDefinition),
    VariableDeclaration(Variable),
    VariableDefinition(VariableDefinition),
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ExpressionIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StatementIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct TypeIndex(usize);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) kind:           StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) enum StorageClass {
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[allow(dead_code)]
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

#[allow(dead_code)]
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
pub(crate) struct ParserError {
    pub(crate) error_type:     ParserErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl GetSeverity for ParserError {
    fn severity(&self) -> ErrorSeverity {
        self.error_type.severity()
    }
}

impl GetPosition for ParserError {
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for ParserError {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

impl std::error::Error for ParserError {}

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

impl TranslationPhase for Parser {
    type Item = TopLevelStatement;

    fn next_item(&mut self, _context: &mut Context) -> Option<Self::Item> {
        loop {
            match self.state_stack.pop().unwrap() {
                | State::ParsingTopLevelStatement => {
                    let type_ = self.parse_type();
                    match type_.kind {
                        | _ => todo!(),
                    }
                },
                | State::ParsingType => {
                    let type_ = self.parse_type();
                    self.types.push(type_);
                },
                | State::ParsingStatement => {
                    let statement = self.parse_statement();
                    self.statements.push(statement);
                },
                | State::ParsingExpression => {
                    let expression = self.parse_expression();
                    self.expressions.push(expression);
                },
            }
        }
        // Some(TopLevelStatement {
        // source_vectors: self.previous_phase.current_position().into(),
        // kind:
        // TopLevelStatementType::FunctionDeclaration(FunctionDeclaration {
        // name:        Identifier {
        // name: StringCacheId::new(0),
        // },
        // parameters:  Arc::new([]),
        // return_type: None,
        // }),
        // })
    }
}
