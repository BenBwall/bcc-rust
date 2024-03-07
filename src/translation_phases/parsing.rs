use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
};

use super::{
    preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        KeywordTokenType,
        Preprocessor,
        Token,
        TokenType,
    },
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVectors,
    TranslationPhase,
};
use crate::{
    translation_phases::preprocessing::OperatorTokenType,
    util::{
        string_cache::StringCacheId,
        vector_slice::VectorSlice,
    },
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser {
    pub(crate) preprocessor: Preprocessor,
    pub(crate) types:        Vec<Type>,
    pub(crate) token_stack:  Vec<Token>,
    pub(crate) expressions:  Vec<Expression>,
    pub(crate) statements:   Vec<Statement>,
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

impl GetSourceFileIndex for Parser {
    fn source_file_index(&self) -> u32 {
        self.preprocessor.source_file_index()
    }
}

impl SetSourceFileIndex for Parser {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.preprocessor
            .set_source_file_index(context, source_file_index);
    }
}

impl Parser {
    #[allow(dead_code)]
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            types: Vec::new(),
            token_stack: Vec::new(),
            expressions: Vec::new(),
            statements: Vec::new(),
        }
    }

    fn next_token(&mut self, context: &mut Context) -> Option<Token> {
        if let Some(token) = self.token_stack.pop() {
            return Some(token);
        }
        self.preprocessor.next_item(context)
    }

    #[allow(dead_code)]
    fn parse_statement(&mut self, context: &mut Context) -> Statement {
        todo!();
    }

    #[allow(dead_code)]
    fn parse_expression(&mut self, context: &mut Context) -> Expression {
        todo!();
    }

    #[allow(dead_code)]
    fn parse_type(&mut self, context: &mut Context) -> Type {
        todo!();
    }

    fn parse_identifier(&mut self, context: &mut Context, eof_message: &'static str, on_error: impl FnOnce(Token) -> ParserErrorType) -> Identifier {
        let Some(token) = self.next_token(context) else {
            let source_vectors = context.create_source_vectors(
                self.position(context),
                self.source_file_index(),
                0,
            );
            context.parser_error(ParserError {
                error_type: ParserErrorType::UnexpectedEndOfInput(eof_message),
                source_vectors,
            });
            return Identifier { name: context.string_cache.intern("<non-existent-identifier>") };
        };
        match token.kind {
            | TokenType::Identifier => Identifier { name: token.contents },
            | tt => {
                context.parser_error(ParserError {
                    error_type: on_error(token),
                    source_vectors: token.source_vectors,
                });
                self.token_stack.push(token);
                Identifier { name: context.string_cache.intern("<non-existent-identifier>") }
            },
        }
    }

    fn parse_typedef(&mut self, context: &mut Context, typedef: Token) -> Type {
        let referent_type = self.parse_type(context);
        let referent_type_index = self.types.len() - 1;
        
        let name = self.parse_identifier(context, "parsing typedef. Expected identifier.",|token| ParserErrorType::ExpectedIdentifierInTypedef(token.kind));
        match self.next_token(context) {
            | Some(token)
                if token.kind
                    == TokenType::Operator(OperatorTokenType::Semicolon) =>
                (),
            | Some(token) => {
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ExpectedSemicolonAfterTypedef(token.kind),
                    source_vectors: token.source_vectors,
                });
                self.token_stack.push(token);
            },
            | None => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing typedef. Expected a semicolon.",
                    ),
                    source_vectors,
                });
            },
        }
        let t = Type {
            is_const: false,
            is_volatile: false,
            kind: TypeKind::Typedef { name, referent_type: TypeIndex(referent_type_index) },
        };
        self.types.push(t);
        t
    }

    fn parse_variable_declaration(&mut self, context: &mut Context) -> VariableDeclaration {
        todo!();
    }

    #[allow(dead_code)]
    fn parse_top_level_statement(&mut self, context: &mut Context) -> Option<TopLevelStatement> {
        let token = self.next_token(context)?;
        if token.kind == TokenType::Keyword(KeywordTokenType::Typedef) {
            return Some(TopLevelStatement {
                kind: TopLevelStatementType::TypeDeclaration(
                    self.parse_typedef(context, token),
                ),
            });
        }
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Enum))
        let type_ = self.parse_type(context);
        let is_declaration = type_parse_result == TypeParseResult::Declaration;
        if is_declaration {
            return Some(TopLevelStatement {
                kind: TopLevelStatementType::TypeDeclaration(type_),
            });
        }
        match type_.kind {
            | TypeKind::Function {
                name,
                parameters,
                return_type,
            } => {
                let statement_start_index: u32 = self
                    .statements
                    .len()
                    .try_into()
                    .expect("More than u32::MAX statements.");
                loop {
                    match self.next_token(context) {
                        | Some(token)
                            if token.kind
                                == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) =>
                            break,
                        | None => break,
                        | Some(token) => {
                            self.token_stack.push(token);
                            let statement = self.parse_statement(context);
                            self.statements.push(statement);
                        },
                    }
                }
                let length: u32 = self
                    .statements
                    .len()
                    .try_into()
                    .expect("More than u32::MAX statements.");
                let length = length - statement_start_index;
                Some(TopLevelStatement {
                    kind: TopLevelStatementType::FunctionDefinition(FunctionDefinition {
                        declaration: FunctionDeclaration {
                            name,
                            parameters,
                            return_type,
                        },
                        statements:  VectorSlice::new(statement_start_index, length),
                    }),
                })
            },
            | _ => {
                let expression = self.parse_expression(context);
                self.expressions.push(expression);
                Some(TopLevelStatement {
                    kind: TopLevelStatementType::VariableDefinition(VariableDefinition {
                        variable:    VariableDeclaration {
                            name:          //type_.,
                            todo!(),
                            var_type:      todo!(),
                            storage_class: todo!(),
                        },
                        initializer: todo!(),
                    }),
                })
            },
        }
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
    pub(crate) kind: TopLevelStatementType,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TopLevelStatementType {
    FunctionDefinition(FunctionDefinition),
    VariableDefinition(VariableDeclaration),
    FunctionDeclaration(FunctionDeclaration),
    TypeDeclaration(Type),
}

// A top level statement could be:
// * A function definition.
// * A global variable definition.
// * A function declaration.
// * A global variable declaration.
// * A type definition.

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ExpressionIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StatementIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct TypeIndex(usize);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) kind: StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[allow(dead_code)]
pub(crate) enum StatementType {
    Compound(Vec<Statement>),
    Expression(ExpressionIndex),
    If {
        condition_expression: ExpressionIndex,
        then_statement:       StatementIndex,
        else_statement:       Option<StatementIndex>,
    },
    While {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    DoWhile {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    For {
        initializer_statement: Option<StatementIndex>,
        condition_expression:  Option<ExpressionIndex>,
        post_expression:       Option<ExpressionIndex>,
        body_statement:        StatementIndex,
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
    pub(crate) result_type: TypeIndex,
    pub(crate) kind:        ExpressionType,
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub(crate) enum ExpressionType {
    Conditional {
        condition_expression: ExpressionIndex,
        then_expression:      ExpressionIndex,
        else_expression:      ExpressionIndex,
    },
    Binary {
        operator:         BinaryOperator,
        left_expression:  ExpressionIndex,
        right_expression: ExpressionIndex,
    },
    Unary {
        operator:           UnaryOperator,
        operand_expression: ExpressionIndex,
    },
    Call {
        function_expression: ExpressionIndex,
        arguments:           VectorSlice<Expression>,
    },
    CompoundLiteral {
        var_type: Type,
        values:   VectorSlice<Expression>,
    },
    Identifier(Identifier),
    Constant(Constant),
    StringLiteral(StringCacheId),
    SizeofType(Type),
    SizeofExpr(ExpressionIndex),
    Cast {
        target_type:        Type,
        operand_expression: ExpressionIndex,
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

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct VariableDeclaration {
    pub(crate) name:          Identifier,
    pub(crate) var_type:      TypeIndex,
    pub(crate) storage_class: StorageClass,
    pub(crate) initializer:   Option<Expression>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct VariableDefinition {
    pub(crate) variable:    VariableDeclaration,
    pub(crate) initializer: Expression,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum StorageClass {
    Auto,
    Register,
    Static,
    Extern,
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
pub(crate) struct FunctionDefinitionArgument {
    pub(crate) function_type: TypeIndex,
    pub(crate) name:          Option<Identifier>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Type {
    is_const:    bool,
    is_volatile: bool,
    kind:        TypeKind,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum TypeKind {
    Primitive(PrimitiveType),
    Pointer {
        pointee_type: TypeIndex,
    },
    Struct {
        name:   Option<Identifier>,
        fields: VectorSlice<VariableDeclaration>,
    },
    Union {
        name:   Identifier,
        fields: VectorSlice<VariableDeclaration>,
    },
    Typedef {
        name:          Identifier,
        referent_type: TypeIndex,
    },
    Enum {
        name:   Identifier,
        values: VectorSlice<EnumValue>,
    },
    Function {
        name:        Identifier,
        return_type: Option<TypeIndex>,
        /// None symbolizes a function with an unspecified number of arguments
        /// (i.e. `int f()`).
        parameters:  Option<VectorSlice<FunctionDefinitionArgument>>,
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
    /// None symbolizes a function with an unspecified number of arguments
    /// (i.e. `int f()`).
    pub(crate) parameters:  Option<VectorSlice<FunctionDefinitionArgument>>,
    pub(crate) return_type: Option<TypeIndex>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct FunctionDefinition {
    pub(crate) declaration: FunctionDeclaration,
    pub(crate) statements:  VectorSlice<Statement>,
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
pub(crate) enum ParserErrorType {
    UnexpectedEndOfInput(&'static str),
    ExpectedIdentifierInTypedef(TokenType),
    ExpectedSemicolonAfterTypedef(TokenType),
}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(..)
            | ParserErrorType::ExpectedIdentifierInTypedef(..) => ErrorSeverity::Error,
            | ParserErrorType::ExpectedSemicolonAfterTypedef(..) => ErrorSeverity::Warning,
        }
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(message) => write!(f, "Unexpected end of input while {}!", message),
            | ParserErrorType::ExpectedIdentifierInTypedef(tt) => write!(f, "Expected an identifier in typedef, found instead {:?}!", tt),
            | ParserErrorType::ExpectedSemicolonAfterTypedef(tt) => write!(f, "Expected a semicolon after typedef, found instead {:?}!", tt),
        }
    }
}

impl TranslationPhase for Parser {
    type Item = TopLevelStatement;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        self.parse_top_level_statement(context)
    }
}
