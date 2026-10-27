//! What lowering reports instead of IR: a translation unit it refuses, and
//! valid C it does not lower yet. Each error carries the syntax node's
//! provenance and renders through the shared diagnostic interface.

use std::fmt;

use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        SourceVectors,
        semantic_analysis::{
            TypeId,
            Types,
            UnmodeledType,
            UnsupportedAttribute,
        },
    },
    util::bump::Bump,
};

/// One reason lowering produced no IR for a function, an object or the
/// whole unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LoweringError {
    pub(crate) kind:   LoweringErrorKind,
    /// The syntax the error is about; `None` when it concerns the whole
    /// unit.
    pub(crate) source: Option<SourceVectors>,
}

/// What went wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "Errors are rare; naming the missing fact is worth the bytes."
)]
pub(crate) enum LoweringErrorKind {
    /// An earlier phase reported an error, so no code is generated.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    UnitHasErrors,
    /// Valid C that this stage of lowering does not handle yet.
    Unsupported(Construct),
    /// Semantic analysis gave up on a type without a diagnostic, as it does
    /// for unmodeled extensions.
    UnanalyzedType,
    #[cfg(test)]
    CallDiagnostic(crate::util::string_cache::StringCacheId),
    UnmodeledType(UnmodeledType),
    /// File-scope assembly in a form GCC's grammar and Clang reject.
    InvalidFileScopeAsm(FileScopeAsmFault),
    /// A fact lowering relies on is missing from the semantic results: a
    /// compiler bug, reported rather than turned into wrong code.
    MissingFact(&'static str),
}

/// A construct outside the prototype subset of `middle-end.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Construct {
    Float128,
    ComplexAtomic,
    Atomic,
    Vector,
    VariableLengthArray,
    LabelAddress,
    /// MSVC `__asm` blocks.
    InlineAssembly,
    /// A GNU `asm` operand in a register class or of a type lowering does
    /// not implement: x87, MMX and register pairs, `__int128`, aggregates in
    /// registers, or an immediate address with an offset.
    InlineAssemblyOperand,
    /// A file-scope assembly template with NUL or bytes that are not
    /// UTF-8, which Clang accepts.
    FileScopeAssembly,
    AssemblerLabel,
    /// An attribute semantic analysis accepted whose meaning code generation
    /// lacks.
    Attribute(UnsupportedAttribute),
    StructuredExceptionHandling,
    NamedJump,
    NestedFunction,
    Builtin,
    StaticInitializer,
    Initializer,
    SelectionDeclaration,
    Extension,
    CompoundLiteralStorage,
    /// An automatic object or compound literal of 4 GiB or more, beyond the
    /// IR's 32-bit stack slot sizes.
    LargeAutomaticObject,
}

/// The errors of one translation unit, in the `'ir` arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LoweringErrors<'ir> {
    pub(crate) errors: &'ir [LoweringError],
}

impl LoweringError {
    pub(super) fn unanalyzed_kind(types: &Types<'_>, ty: TypeId) -> LoweringErrorKind {
        types.unmodeled_type(ty).map_or(
            LoweringErrorKind::UnanalyzedType,
            LoweringErrorKind::UnmodeledType,
        )
    }

    pub(super) const fn unsupported(construct: Construct, source: SourceVectors) -> Self {
        Self {
            kind:   LoweringErrorKind::Unsupported(construct),
            source: Some(source),
        }
    }

    pub(super) const fn missing(fact: &'static str, source: SourceVectors) -> Self {
        Self {
            kind:   LoweringErrorKind::MissingFact(fact),
            source: Some(source),
        }
    }

    pub(super) fn unanalyzed(types: &Types<'_>, ty: TypeId, source: SourceVectors) -> Self {
        Self {
            kind:   Self::unanalyzed_kind(types, ty),
            source: Some(source),
        }
    }
}

impl Construct {
    /// The construct as the diagnostic names it.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            | Self::Float128 => "binary128 floating-point types",
            | Self::ComplexAtomic => "atomic complex values",
            | Self::Atomic => "atomic types",
            | Self::Vector => "vector types",
            | Self::VariableLengthArray => "variable length arrays",
            | Self::LabelAddress => "label addresses",
            | Self::InlineAssembly => "MSVC inline assembly",
            | Self::InlineAssemblyOperand =>
                "inline assembly operands of this register class or type",
            | Self::FileScopeAssembly =>
                "file-scope assembly templates containing NUL or bytes that are not UTF-8",
            | Self::AssemblerLabel => "assembler labels",
            | Self::Attribute(attribute) => attribute.description(),
            | Self::StructuredExceptionHandling => "structured exception handling",
            | Self::NamedJump => "named break and continue",
            | Self::NestedFunction => "nested functions",
            | Self::Builtin => "this builtin",
            | Self::StaticInitializer => "this static initializer",
            | Self::Initializer => "this initializer",
            | Self::SelectionDeclaration => "declarations in selection statements",
            | Self::Extension => "this extension",
            | Self::LargeAutomaticObject => "automatic objects of 4 GiB or more",
            | Self::CompoundLiteralStorage =>
                "compound literals with static or thread storage duration",
        }
    }
}

/// Why a file-scope `asm` is invalid: basic assembly outside a function
/// takes one narrow string literal and nothing else.
/// GNU extension: GCC manual, "Basic Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Basic-Asm.html>
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileScopeAsmFault {
    Volatile,
    Inline,
    Goto,
    /// Output or input operands, clobbers, goto labels or their colons.
    Operands,
    /// An `L`, `u` or `U` template.
    WideTemplate,
    /// A `u8` template.
    UnicodeTemplate,
}

impl FileScopeAsmFault {
    /// The diagnostic message, in Clang's wording where Clang has one.
    pub(crate) const fn message(self) -> &'static str {
        match self {
            | Self::Volatile => "meaningless 'volatile' on asm outside function",
            | Self::Inline => "meaningless 'inline' on asm outside function",
            | Self::Goto => "meaningless 'goto' on asm outside function",
            | Self::Operands => "file-scope asm cannot have operands, clobbers or labels",
            | Self::WideTemplate => "cannot use wide string literal in 'asm'",
            | Self::UnicodeTemplate => "cannot use unicode string literal in 'asm'",
        }
    }
}

impl LoweringErrorKind {
    fn message(self) -> &'static str {
        match self {
            #[cfg(test)]
            | Self::CallDiagnostic(_) => "function attribute diagnostic",
            | Self::UnitHasErrors => "code is not generated for a translation unit with errors",
            | Self::Unsupported(_) | Self::UnmodeledType(_) => "not yet supported by lowering",
            | Self::UnanalyzedType =>
                "not yet supported by lowering: a type built from an extension semantic analysis \
                 does not model",
            | Self::MissingFact(_) => "internal error: lowering is missing a semantic fact",
            | Self::InvalidFileScopeAsm(fault) => fault.message(),
        }
    }
}

impl ToDiagnostic for LoweringError {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        #[cfg(not(test))]
        let _ = context;
        let message = match self.kind {
            #[cfg(test)]
            | LoweringErrorKind::CallDiagnostic(message) =>
                arena.alloc_str(context.string_cache.at(message)),
            | LoweringErrorKind::Unsupported(construct) => format_in!(
                arena,
                "{}: {}",
                self.kind.message(),
                construct.description()
            ),
            | LoweringErrorKind::UnmodeledType(reason) =>
                format_in!(arena, "{}: {}", self.kind.message(), reason.description()),
            | LoweringErrorKind::MissingFact(fact) =>
                format_in!(arena, "{}: {fact}", self.kind.message()),
            | kind => kind.message(),
        };
        let explanation = Explanation::new(arena, message);
        let explanation = match self.kind {
            | LoweringErrorKind::Unsupported(_) => explanation
                .label("lowering stops here")
                .note("the program is valid C; this compiler cannot translate it yet"),
            | LoweringErrorKind::UnanalyzedType =>
                explanation.label("the type could not be analyzed"),
            | LoweringErrorKind::UnmodeledType(_) => explanation
                .label("this expression or object uses the unsupported construct")
                .note("semantic analysis does not yet model this construct"),
            | LoweringErrorKind::InvalidFileScopeAsm(_) => explanation
                .label("invalid file-scope asm")
                .note("asm outside a function takes exactly one narrow string literal"),
            | _ => explanation,
        };
        explanation.at(ErrorSeverity::Error, source)
    }
}

impl fmt::Display for LoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.message())?;
        match self.kind {
            | LoweringErrorKind::Unsupported(construct) =>
                write!(f, ": {}", construct.description()),
            | LoweringErrorKind::UnmodeledType(reason) => write!(f, ": {}", reason.description()),
            | LoweringErrorKind::MissingFact(fact) => write!(f, ": {fact}"),
            | _ => Ok(()),
        }
    }
}
