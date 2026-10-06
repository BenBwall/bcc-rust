//! Syntax-tree roots, statements, expressions, and identifiers.

use super::declaration_syntax::{
    Declaration,
    DeclarationSpecifiers,
    Declarator,
    Initializer,
    TypeName,
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            CharacterTokenType,
            FloatTokenType,
            IntegerTokenType,
            StringTokenType,
            Token,
        },
    },
    util::{
        arena_list::ArenaList,
        string_cache::StringCacheId,
    },
};

/// One top-level parser result.
///
/// Valid and recovered declarations both retain their syntax. Consumers may
/// continue semantic analysis on recovered syntax while treating it as
/// diagnostic-tainted. `Error` is reserved for a future case where recovery
/// cannot construct a meaningful declaration at all.
///
/// C99: external-declaration and translation-unit are §6.9, p. 140; PDF
/// p. 152. Retaining recovered syntax after diagnosis is permitted by
/// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExternalDeclaration<'tu> {
    /// Declaration parsed without a hard syntax diagnostic.
    Declaration(&'tu Declaration<'tu>),
    /// Repaired declaration produced after at least one hard syntax diagnostic.
    RecoveredDeclaration(&'tu Declaration<'tu>),
    /// Function definition parsed without a hard syntax diagnostic.
    FunctionDefinition(&'tu FunctionDefinition<'tu>),
    /// Function definition containing locally recovered syntax.
    RecoveredFunctionDefinition(&'tu FunctionDefinition<'tu>),
    /// Provenance-only placeholder when no meaningful AST can be recovered.
    Error(SourceVectors),
}

/// Expression whose grammar guarantees constant-expression syntax.
///
/// C99: constant-expression is §6.6, pp. 95-96; PDF pp. 107-108.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct ConstantExpression<'tu>(pub(super) &'tu Expression<'tu>);

impl<'tu> ConstantExpression<'tu> {
    /// The expression this constant expression is.
    pub(crate) fn expression(self) -> &'tu Expression<'tu> {
        self.0
    }
}

impl<'tu> From<ConstantExpression<'tu>> for &'tu Expression<'tu> {
    fn from(expression: ConstantExpression<'tu>) -> Self {
        expression.0
    }
}

/// Complete function-definition syntax produced at file scope.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct FunctionDefinition<'tu> {
    pub(crate) declaration_specifiers: DeclarationSpecifiers<'tu>,
    pub(crate) declarator:             Declarator<'tu>,
    pub(crate) declaration_list:       ArenaList<'tu, &'tu Declaration<'tu>>,
    pub(crate) body:                   &'tu Statement<'tu>,
    pub(crate) source_vectors:         SourceVectors,
    pub(crate) recovered:              bool,
}

/// One source-ordered item in a compound statement.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum BlockItem<'tu> {
    Declaration(&'tu Declaration<'tu>),
    Statement(&'tu Statement<'tu>),
}

/// A statement expression is parsed or missing because recovery repaired a
/// required position.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExpressionSlot<'tu> {
    Parsed(&'tu Expression<'tu>),
    Missing(SourceVectors),
}

/// A statement constant-expression preserves its narrower grammar type while
/// parsed or synthesized during recovery.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ConstantExpressionSlot<'tu> {
    Parsed(ConstantExpression<'tu>),
    Missing(SourceVectors),
}

/// Statement syntax node constructed by the statement frame.
///
/// C99: §6.8-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Statement<'tu> {
    /// Grammar form and children of this statement.
    pub(crate) kind:           StatementType<'tu>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// C statement grammar forms and their children.
///
/// C99: statement alternatives are §6.8, p. 131; PDF p. 143; their detailed
/// productions are §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StatementType<'tu> {
    Compound {
        items: ArenaList<'tu, BlockItem<'tu>>,
    },
    Expression(ExpressionSlot<'tu>),
    If {
        condition_expression: ExpressionSlot<'tu>,
        then_statement:       &'tu Statement<'tu>,
        else_statement:       Option<&'tu Statement<'tu>>,
    },
    Switch {
        condition_expression: ExpressionSlot<'tu>,
        body_statement:       &'tu Statement<'tu>,
    },
    While {
        condition_expression: ExpressionSlot<'tu>,
        body_statement:       &'tu Statement<'tu>,
    },
    DoWhile {
        condition_expression: ExpressionSlot<'tu>,
        body_statement:       &'tu Statement<'tu>,
    },
    For(&'tu ForStatement<'tu>),
    Return(Option<ExpressionSlot<'tu>>),
    Break,
    Continue,
    Goto(Identifier),
    Label(Identifier, &'tu Statement<'tu>),
    Case(ConstantExpressionSlot<'tu>, &'tu Statement<'tu>),
    Default(&'tu Statement<'tu>),
    Null,
}

/// The three header clauses and body of a `for` statement. They are kept
/// apart from [`StatementType`] so the other statement forms do not pay
/// for the widest one.
///
/// C99: iteration-statement is §6.8.5, p. 135; PDF p. 147.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct ForStatement<'tu> {
    pub(crate) initializer:          Option<ForInitializer<'tu>>,
    pub(crate) condition_expression: Option<ExpressionSlot<'tu>>,
    pub(crate) iteration_expression: Option<ExpressionSlot<'tu>>,
    pub(crate) body_statement:       &'tu Statement<'tu>,
}

/// First clause of a `for` statement, which is syntactically either an
/// expression or a declaration.
///
/// C99: iteration-statement is §6.8.5, p. 135; PDF p. 147.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ForInitializer<'tu> {
    Expression(ExpressionSlot<'tu>),
    Declaration(&'tu Declaration<'tu>),
}

/// Expression syntax node with exact source provenance.
///
/// C99: §6.5-§6.5.17, pp. 67-94; PDF pp. 79-106.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Expression<'tu> {
    /// Operator/operand grammar form.
    pub(crate) kind:                    ExpressionType<'tu>,
    /// Original-source segments contributing to the expression.
    pub(crate) source_vectors:          SourceVectors,
    /// Exact operator or owned-delimiter provenance for this expression form.
    pub(crate) operator_source_vectors: Option<SourceVectors>,
    /// Whether this expression or one of its grammar children required repair.
    pub(crate) recovered:               bool,
}

/// C expression grammar forms represented through child references.
///
/// A conditional's three operands would not fit beside the operator in 24
/// bytes, so they live in a separate arena record and the common binary,
/// unary, and leaf forms do not pay for the rare conditional.
///
/// C99: primary through comma expressions are §6.5.1-§6.5.17,
/// pp. 69-94; PDF pp. 81-106.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExpressionType<'tu> {
    Parenthesized {
        expression: &'tu Expression<'tu>,
    },
    Conditional(&'tu ConditionalExpression<'tu>),
    Binary {
        operator:         BinaryOperator,
        left_expression:  &'tu Expression<'tu>,
        right_expression: &'tu Expression<'tu>,
    },
    Unary {
        operator:           UnaryOperator,
        operand_expression: &'tu Expression<'tu>,
    },
    Call {
        function_expression: &'tu Expression<'tu>,
        arguments:           ArenaList<'tu, &'tu Expression<'tu>>,
    },
    DirectMember {
        base_expression: &'tu Expression<'tu>,
        member:          Identifier,
    },
    IndirectMember {
        base_expression: &'tu Expression<'tu>,
        member:          Identifier,
    },
    CompoundLiteral {
        type_name:   &'tu TypeName<'tu>,
        initializer: &'tu Initializer<'tu>,
    },
    Identifier(Identifier),
    Constant(Constant),
    StringLiteral(StringTokenType),
    SizeofType(&'tu TypeName<'tu>),
    SizeofExpr(&'tu Expression<'tu>),
    Cast {
        target_type:        &'tu TypeName<'tu>,
        operand_expression: &'tu Expression<'tu>,
    },
    Error,
}

/// The operands of a conditional expression `condition ? then : else`.
///
/// C99: the conditional operator is §6.5.15.
#[derive(Debug, PartialEq, Clone, Copy)]
#[expect(
    clippy::struct_field_names,
    reason = "The operands keep the names every other expression form gives its operands."
)]
pub(crate) struct ConditionalExpression<'tu> {
    pub(crate) condition_expression: &'tu Expression<'tu>,
    pub(crate) then_expression:      &'tu Expression<'tu>,
    pub(crate) else_expression:      &'tu Expression<'tu>,
}

/// Typed literal value accepted by a primary expression.
///
/// C99: primary-expression is §6.5.1, p. 69; PDF p. 81; constants are §6.4.4,
/// pp. 54-62; PDF pp. 66-74.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

/// Binary and postfix operators represented by expression nodes.
///
/// C99: postfix and binary expression productions are §6.5.2 and
/// §6.5.5-§6.5.17, pp. 69-94; PDF pp. 81-106.
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

/// Prefix, postfix, and cast-like unary operators.
///
/// C99: postfix, unary, and cast expressions are §6.5.2-§6.5.4,
/// pp. 69-81; PDF pp. 81-93.
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
}

/// Interned identifier spelling used by syntax nodes and scope classification.
///
/// C99: identifiers are §6.4.2.1, p. 51; PDF p. 63; their scopes and
/// namespaces are §6.2.1-§6.2.3, pp. 29-31; PDF pp. 41-43.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Identifier {
    /// Handle into [`Context`](crate::translation_phases::Context)'s shared
    /// string cache.
    pub(crate) name:           StringCacheId,
    /// Exact provenance of this identifier token, including macro/include
    /// contributions.
    pub(crate) source_vectors: SourceVectors,
}

impl Identifier {
    pub(crate) fn new(name: StringCacheId, source_vectors: SourceVectors) -> Self {
        Self {
            name,
            source_vectors,
        }
    }

    pub(super) fn from_token(token: Token) -> Self {
        Self::new(token.contents, token.source_vectors)
    }
}

/// storage-class-specifier:
/// typedef
/// extern
/// static
/// auto
/// register
///
/// C99: §6.7.1, p. 98; PDF p. 110.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) enum StorageClass {
    #[default]
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

impl StorageClass {
    /// The storage-class keyword as written in C source.
    pub(super) fn spelling(self) -> &'static str {
        match self {
            | Self::Auto => "auto",
            | Self::Register => "register",
            | Self::Static => "static",
            | Self::Extern => "extern",
            | Self::Typedef => "typedef",
        }
    }
}
