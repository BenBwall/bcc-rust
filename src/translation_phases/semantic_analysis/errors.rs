//! Phase-7 constraints and source-backed semantic diagnostics.
//! C99: diagnostic requirement §5.1.1.3p1, p. 11; PDF p. 23.

use std::fmt;

use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
        quote_spelling,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceVectors,
        SourcePosition,
        SourceVectors,
    },
    util::{
        bump::Bump,
        string_cache::StringCacheId,
    },
};

/// Symbolic semantic diagnostic, independent of rendered text.
/// C99: §6.7p3-4, p. 97; PDF p. 109; subsidiary constraints are cited below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticErrorKind {
    /// C99: §6.7.7, p. 123-124; PDF p. 135-136.
    UnknownTypedef,
    /// C99: §6.5.1p2, p. 69; PDF p. 81.
    UndeclaredIdentifier,
    /// C99: §6.5.3.2p1, p. 78; PDF p. 90.
    InvalidAddressOperand,
    /// C99: §6.5.3p1, p. 78; PDF p. 90.
    InvalidUnaryOperand,
    /// C99: §6.5.16p2, p. 91; PDF p. 103.
    ExpectedModifiableLvalue,
    /// C99: §6.5.2.1p1, p. 70; PDF p. 82.
    InvalidSubscript,
    /// C99: §6.5.5p2, p. 82; PDF p. 94.
    InvalidArithmeticOperands,
    /// C99: §6.5.6p2-3, p. 82; PDF p. 94.
    InvalidAdditiveOperands,
    /// C99: §6.5.7p2, p. 84; PDF p. 96.
    InvalidIntegerOperands,
    /// C99: §6.5.13p2, p. 89; PDF p. 101.
    InvalidLogicalOperands,
    /// C99: §6.5.8p2, p. 85; PDF p. 97.
    InvalidComparisonOperands,
    /// C99: §6.5.15p2-3, p. 90; PDF p. 102.
    InvalidConditionalOperands,
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    InvalidAssignment,
    /// C99: §6.5.4p2-4, p. 81; PDF p. 93.
    InvalidCast,
    /// C99: §6.5.3.4p1, p. 80; PDF p. 92.
    InvalidSizeof,
    /// C99: §6.5.2.3p1-2, p. 72; PDF p. 84.
    InvalidMemberAccess,
    /// C99: §6.5.2.2p1, p. 71; PDF p. 83.
    InvalidCall,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentCount,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentType,
    /// C99: §6.5.2.5p1, p. 75; PDF p. 87.
    InvalidCompoundLiteral,
    /// C99: §6.8.4.1p1, p. 133; PDF p. 145.
    InvalidCondition,
    /// C99: §6.8.4.2p1, p. 134; PDF p. 146.
    InvalidSwitchExpression,
    /// C11: §6.7.10p2-3, p. 145; PDF p. 163. C99 ICE evaluation:
    /// §6.6p6, p. 95; PDF p. 107.
    FailedAssertion,
    /// C99: §6.7.8p2-3, p. 125; PDF p. 137.
    InvalidInitializer,
    /// C99: §6.7.8p2, p. 125; PDF p. 137.
    ExcessInitializer,
    /// C99: §6.7.8p6-7, p. 125; PDF p. 137.
    InvalidDesignator,
    /// C99: §6.7.8p4, p. 125; PDF p. 137.
    NonConstantInitializer,
    /// C99: §6.7.1, p. 98; PDF p. 110.
    InvalidStorage,
    /// C99: §6.7.3p2, p. 108; PDF p. 120.
    InvalidRestrict,
    /// C99: §6.7.3p8, p. 109; PDF p. 121. Qualifying a function type is
    /// undefined.
    QualifiedFunction,
    /// C99: §6.7.4p2-4, p. 112; PDF p. 124.
    InvalidInline,
    /// C99: §6.7.5.2p1; 6.7.5.3p1, p. 116-118; PDF p. 128-130.
    InvalidDerivedType,
    /// C99: §6.7.5.2p1, p. 116; PDF p. 128.
    InvalidArrayBound,
    /// C99: §6.7.5.2p2, p. 116; PDF p. 128.
    FileScopeVariableType,
    /// C99: §6.7p4; 6.2.7p2, p. 97; 40; PDF p. 109; 52.
    IncompatibleDeclaration,
    /// C99: §6.7p3, p. 97; PDF p. 109.
    DuplicateDeclaration,
    /// C99: §6.2.2p4-7, p. 30-31; PDF p. 42-43.
    ConflictingLinkage,
    /// C99: §6.7.2.3p2, p. 106; PDF p. 118.
    TagKindMismatch,
    /// C99: §6.7.2.3p1, p. 106; PDF p. 118.
    TagRedefinition,
    /// C99: §6.7.2.3p3, p. 106; PDF p. 118.
    IncompleteEnum,
    /// C99: §6.6p3-6, p. 95; PDF p. 107.
    InvalidConstant,
    /// C99: §6.6p4; 6.5p5, p. 95; 67; PDF p. 107; 79.
    ConstantOverflow,
    /// C99: §6.7.2.2p2, p. 105; PDF p. 117.
    EnumeratorRange,
    /// C99: §6.7.2.1p2, p. 101; PDF p. 113.
    InvalidMember,
    /// C99: §6.7p3; 6.2.3p1, p. 97; 31; PDF p. 109; 43.
    DuplicateMember,
    /// C99: §6.7.2.1p3-4, p. 101; PDF p. 113.
    InvalidBitField,
    /// C99: §6.7.5.2p1; §6.7.5.3p2 and p10, pp. 116-119; PDF pp. 128-131.
    InvalidParameter,
    /// C99: §6.7p7, p. 98; PDF p. 110.
    IncompleteObject,
}

impl SemanticErrorKind {
    fn explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::UndeclaredIdentifier => (
                "identifier has no visible declaration",
                "C99 §6.5.1p2: an identifier expression designates a declared object, function or \
                 enumeration constant",
            ),
            | Self::InvalidAddressOperand => (
                "address operator requires an addressable object or function",
                "C99 §6.5.3.2p1: the operand is a function designator, subscript, indirection, or \
                 a non-register, non-bit-field lvalue",
            ),
            | Self::InvalidUnaryOperand => (
                "invalid operand type for unary operator",
                "C99 §6.5.3p1: unary operators require their specified arithmetic, scalar or \
                 pointer operands",
            ),
            | Self::ExpectedModifiableLvalue => (
                "operator requires a modifiable lvalue",
                "C99 §6.5.16p2: assignment and increment operators require a modifiable lvalue",
            ),
            | Self::InvalidSubscript => (
                "subscript requires an integer and a pointer to complete object type",
                "C99 §6.5.2.1p1: one operand is integer and the other points to a complete object \
                 type",
            ),
            | Self::InvalidArithmeticOperands => (
                "multiplicative operator requires arithmetic operands",
                "C99 §6.5.5p2: multiplicative operators require arithmetic operands; remainder \
                 requires integers",
            ),
            | Self::InvalidAdditiveOperands => (
                "invalid operand types for pointer or arithmetic addition",
                "C99 §6.5.6p2-3: addition and subtraction require arithmetic operands or the \
                 specified complete-object pointer combinations",
            ),
            | Self::InvalidIntegerOperands => (
                "operator requires integer operands",
                "C99 §6.5.7p2: shift and bitwise operators require integer operands",
            ),
            | Self::InvalidLogicalOperands => (
                "logical operator requires scalar operands",
                "C99 §6.5.13p2: both operands of a logical operator have scalar type",
            ),
            | Self::InvalidComparisonOperands => (
                "invalid operand types for comparison",
                "C99 §6.5.8p2: comparisons require the specified arithmetic or compatible pointer \
                 operands",
            ),
            | Self::InvalidConditionalOperands => (
                "invalid operand types for conditional operator",
                "C99 §6.5.15p2-3: the condition is scalar and the alternatives have compatible \
                 arithmetic, aggregate, void or pointer types",
            ),
            | Self::InvalidAssignment => (
                "incompatible assignment conversion",
                "C99 §6.5.16.1p1: assignment requires compatible values and cannot discard \
                 pointer-target qualifiers",
            ),
            | Self::InvalidCast => (
                "invalid operand or target type for cast",
                "C99 §6.5.4p2-4: casts require scalar operands and targets except conversion to \
                 void; floating and pointer types cannot be interconverted",
            ),
            | Self::InvalidSizeof => (
                "size query requires a complete object type and no bit-field",
                "C99 §6.5.3.4p1: sizeof cannot be applied to function, incomplete or bit-field \
                 types",
            ),
            | Self::InvalidMemberAccess => (
                "member access requires a declared record member",
                "C99 §6.5.2.3p1-2: member access names a member of the designated structure or \
                 union",
            ),
            | Self::InvalidCall => (
                "call requires a function pointer returning void or complete object type",
                "C99 §6.5.2.2p1: the called expression points to a function with a valid return \
                 type",
            ),
            | Self::InvalidArgumentCount => (
                "function call has the wrong number of arguments",
                "C99 §6.5.2.2p2: a prototype fixes the required arguments; only an ellipsis \
                 permits additional arguments",
            ),
            | Self::InvalidArgumentType => (
                "function argument cannot be converted to its parameter type",
                "C99 §6.5.2.2p2: each argument has a type assignable to its corresponding \
                 parameter",
            ),
            | Self::InvalidCompoundLiteral => (
                "compound literal requires a non-variable object type",
                "C99 §6.5.2.5p1: a compound literal specifies an object type or an array of \
                 unknown size, not a variable-length array",
            ),
            | Self::InvalidCondition => (
                "condition requires scalar type",
                "C99 §6.8.4.1p1: selection and iteration conditions have scalar type",
            ),
            | Self::InvalidSwitchExpression => (
                "switch controlling expression requires integer type",
                "C99 §6.8.4.2p1: the controlling expression of a switch has integer type",
            ),
            | Self::FailedAssertion => (
                "static assertion failed",
                "C11 §6.7.10p2-3: the assertion requires a nonzero integer constant expression",
            ),
            | Self::InvalidInitializer => (
                "initializer cannot initialize this object",
                "C99 §6.7.8p2-3: initialization requires an appropriate complete object or \
                 incomplete array and compatible scalar values",
            ),
            | Self::ExcessInitializer => (
                "excess initializer for the current object",
                "C99 §6.7.8p2: an initializer shall not provide a value for an object outside the \
                 initialized entity",
            ),
            | Self::InvalidDesignator => (
                "designator does not identify an available subobject",
                "C99 §6.7.8p6-7: array indices are nonnegative integer constants within bounds \
                 and field designators name record members",
            ),
            | Self::NonConstantInitializer => (
                "static-storage initializer requires a constant expression",
                "C99 §6.7.8p4: every expression in an initializer for an object of static storage \
                 duration is constant or a string literal",
            ),
            | Self::QualifiedFunction => (
                "qualifiers on a function type are ignored",
                "C99 §6.7.3p8: qualifying a function type has undefined behavior",
            ),
            | Self::IncompleteObject => (
                "object requires a complete type",
                "C99 §6.7p7: an object with no linkage has complete type by the end of its \
                 declarator or initializer",
            ),
            | Self::UnknownTypedef => (
                "typedef name has no visible type binding",
                "C99 §6.7.7: a typedef name denotes its declared type",
            ),
            | Self::InvalidStorage => (
                "storage class is not permitted here",
                "C99 §6.7.1 and §6.9p2: auto and register require block scope",
            ),
            | Self::InvalidRestrict => (
                "restrict requires a pointer to an object or incomplete type",
                "C99 §6.7.3p2: restrict qualifies only pointers to object or incomplete types",
            ),
            | Self::InvalidInline => (
                "inline requires a function other than main",
                "C99 §6.7.4p2-4: inline applies only to function identifiers other than main",
            ),
            | Self::InvalidDerivedType => (
                "invalid array element or function return type",
                "C99 §6.7.5.2p1 and §6.7.5.3p1: arrays need complete object elements; functions \
                 cannot return arrays or functions",
            ),
            | Self::InvalidArrayBound => (
                "array bound requires integer type; a constant bound must be positive",
                "C99 §6.7.5.2p1: array sizes have integer type; a constant size shall be greater \
                 than zero",
            ),
            | Self::FileScopeVariableType => (
                "variably modified type is not permitted here",
                "C99 §6.7.5.2p2: variably modified types require block or prototype scope and no \
                 linkage",
            ),
            | Self::IncompatibleDeclaration => (
                "redeclaration has an incompatible type",
                "C99 §6.7p4 and §6.2.7p2: declarations of the same entity require compatible types",
            ),
            | Self::DuplicateDeclaration => (
                "identifier with no linkage is declared more than once",
                "C99 §6.7p3: a no-linkage identifier has at most one declaration in the same scope",
            ),
            | Self::ConflictingLinkage => (
                "identifier has both internal and external linkage",
                "C99 §6.2.2p4 and p7: extern inherits visible linkage; mixing internal and \
                 external linkage is undefined",
            ),
            | Self::TagKindMismatch => (
                "tag is used with a different kind",
                "C99 §6.7.2.3p2: declarations of a tag shall use the same struct, union or enum \
                 kind",
            ),
            | Self::TagRedefinition => (
                "tag is already complete",
                "C99 §6.7.2.3p1: a specific type has its contents defined at most once",
            ),
            | Self::IncompleteEnum => (
                "enum tag has no preceding complete declaration",
                "C99 §6.7.2.3p3: an enum specifier without an enumerator list follows a complete \
                 declaration",
            ),
            | Self::InvalidConstant => (
                "an integer constant expression is required",
                "C99 §6.6p3 and p6: integer constant expressions restrict operators and operands",
            ),
            | Self::ConstantOverflow => (
                "constant expression overflows or has an invalid operation",
                "C99 §6.6p4: a constant expression is representable in its type; §6.5p5 forbids \
                 exceptional evaluation",
            ),
            | Self::EnumeratorRange => (
                "enumerator value is not representable as int",
                "C99 §6.7.2.2p2: an enumeration constant has a value representable as int",
            ),
            | Self::InvalidMember => (
                "member requires a complete object type",
                "C99 §6.7.2.1p2: members cannot have incomplete or function type except a final \
                 flexible array member",
            ),
            | Self::DuplicateMember => (
                "member name is declared more than once",
                "C99 §6.7p3 and §6.2.3p1: members have no linkage and a separate namespace per \
                 aggregate",
            ),
            | Self::InvalidBitField => (
                "invalid bit-field type or width",
                "C99 §6.7.2.1p3-4: bit-fields require an integer type, a fitting nonnegative \
                 width, and no name at width zero",
            ),
            | Self::InvalidParameter => (
                "invalid function parameter declaration",
                "C99 §6.7.5.3p2 and p10: only register storage is allowed; void must be the sole \
                 unnamed parameter; §6.7.5.2p1 limits qualifiers/static to the outermost array",
            ),
        }
    }
}

/// Semantic error with primary and optional prior-declaration provenance.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SemanticError {
    pub(crate) kind:           SemanticErrorKind,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) name:           Option<StringCacheId>,
    pub(crate) previous:       Option<SourceVectors>,
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.explanation().0)
    }
}
impl std::error::Error for SemanticError {}
impl GetSeverity for SemanticError {
    fn severity(&self) -> ErrorSeverity {
        if self.kind == SemanticErrorKind::QualifiedFunction {
            ErrorSeverity::Warning
        } else {
            ErrorSeverity::Error
        }
    }
}
impl GetPosition for SemanticError {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        context
            .get_source_vectors(self.source_vectors)
            .first()
            .map_or_else(SourcePosition::default, |v| v.position(context))
    }
}
impl GetSourceVectors for SemanticError {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}
impl ToDiagnostic for SemanticError {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let (message, note) = self.kind.explanation();
        let message = if let Some(name) = self.name {
            format_in!(
                arena,
                "{message}: {}",
                quote_spelling(context.string_cache.at(name))
            )
        } else {
            message
        };
        let mut diagnostic = Explanation::new(arena, message)
            .label(if self.kind == SemanticErrorKind::QualifiedFunction {
                "function type qualifiers are ignored"
            } else if matches!(
                self.kind,
                SemanticErrorKind::UndeclaredIdentifier
                    | SemanticErrorKind::InvalidAddressOperand
                    | SemanticErrorKind::InvalidUnaryOperand
                    | SemanticErrorKind::ExpectedModifiableLvalue
                    | SemanticErrorKind::InvalidSubscript
                    | SemanticErrorKind::InvalidArithmeticOperands
                    | SemanticErrorKind::InvalidAdditiveOperands
                    | SemanticErrorKind::InvalidIntegerOperands
                    | SemanticErrorKind::InvalidLogicalOperands
                    | SemanticErrorKind::InvalidComparisonOperands
                    | SemanticErrorKind::InvalidConditionalOperands
                    | SemanticErrorKind::InvalidAssignment
                    | SemanticErrorKind::InvalidCast
                    | SemanticErrorKind::InvalidSizeof
                    | SemanticErrorKind::InvalidMemberAccess
                    | SemanticErrorKind::InvalidCall
                    | SemanticErrorKind::InvalidArgumentCount
                    | SemanticErrorKind::InvalidArgumentType
                    | SemanticErrorKind::InvalidCompoundLiteral
                    | SemanticErrorKind::InvalidCondition
                    | SemanticErrorKind::InvalidSwitchExpression
                    | SemanticErrorKind::FailedAssertion
                    | SemanticErrorKind::InvalidInitializer
                    | SemanticErrorKind::ExcessInitializer
                    | SemanticErrorKind::InvalidDesignator
                    | SemanticErrorKind::NonConstantInitializer
            ) {
                "expression or initializer constraint violated"
            } else {
                "declaration constraint violated"
            })
            .note(note)
            .at(self.severity(), source);
        if let Some(previous) = self.previous {
            diagnostic = diagnostic.secondary(previous, "previous declaration is here");
        }
        diagnostic
    }
}
