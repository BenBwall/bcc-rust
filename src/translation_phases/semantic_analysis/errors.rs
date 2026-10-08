//! Phase-7 declaration constraints and source-backed semantic diagnostics.
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

/// Symbolic declaration diagnostic, independent of rendered text.
/// C99: §6.7p3-4, p. 97; PDF p. 109; subsidiary constraints are cited below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticErrorKind {
    /// C99: §6.7.7, p. 123-124; PDF p. 135-136.
    UnknownTypedef,
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
                "integer constant expression overflows or has an invalid operation",
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
