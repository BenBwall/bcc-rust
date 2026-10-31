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
    /// A call of a builtin semantic analysis has no contract for: GCC's
    /// spelling where [`crate::builtins::unimplemented`] names it, else
    /// `None` for one this compiler does not know.
    UnsupportedBuiltin(Option<&'static str>),
    /// File-scope assembly in a form GCC's grammar and Clang reject.
    InvalidFileScopeAsm(FileScopeAsmFault),
    /// A fact lowering relies on is missing from the semantic results: a
    /// compiler bug, reported rather than turned into wrong code.
    MissingFact(&'static str),
    /// `__builtin_va_arg_pack ()` (`false`) or `__builtin_va_arg_pack_len ()`
    /// (`true`) where no call supplies the arguments: outside a copy of its
    /// function expanded for a call, or the pack not as the last argument
    /// after a callee's named parameters. GCC reports the same error.
    /// GNU extension: GCC manual, "Constructing Calls".
    InvalidArgumentPack(bool),
}

/// A construct outside the prototype subset of `middle-end.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Construct {
    ComplexAtomic,
    Atomic,
    VariableLengthArray,
    LabelAddress,
    /// MSVC `__asm` blocks.
    InlineAssembly,
    /// A GNU `asm` operand in a register class or of a type lowering does
    /// not implement: x87, MMX and register pairs, `__int128`, aggregates in
    /// registers, or an immediate address with an offset.
    InlineAssemblyOperand,
    /// An `asm` operand that is a GNU local register variable whose
    /// register its constraint cannot take or lowering cannot place it in.
    RegisterVariableOperand,
    /// A file-scope assembly template with NUL or bytes that are not
    /// UTF-8, which Clang accepts.
    FileScopeAssembly,
    AssemblerLabel,
    /// An attribute semantic analysis accepted whose meaning code generation
    /// lacks.
    Attribute(UnsupportedAttribute),
    StructuredExceptionHandling,
    NamedJump,
    /// A `goto` from a nested function to a label of a function around
    /// it. GNU extension: GCC manual, "Nested Functions".
    NonLocalGoto,
    /// A nested function on `x86_64-pc-windows-msvc`, whose stack probe
    /// `__chkstk` clobbers the `%r10` its static chain arrives in.
    NestedChainTarget,
    /// The address of a nested function on `x86_64-w64-windows-gnu`, where
    /// Windows enforces DEP: its trampoline would run from a stack that is
    /// not executable, which GCC's MinGW port makes so through
    /// `__enable_execute_stack` at each trampoline.
    NestedTrampolineTarget,
    /// A nested function's use of a variably modified object of the
    /// function around it, whose bounds that function keeps.
    NestedVariablyModified,
    Builtin,
    StaticInitializer,
    Initializer,
    SelectionDeclaration,
    Extension,
    CompoundLiteralStorage,
    /// An automatic object or compound literal of 4 GiB or more, beyond the
    /// IR's 32-bit stack slot sizes.
    LargeAutomaticObject,
    /// A conversion between a `_BitInt` of more than 128 bits and a
    /// complex type.
    WideBitIntConversion,
    /// A function whose `_BitInt` results of more than 128 bits would need
    /// more stack than a default thread has.
    WideBitIntTemporaries,
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
            | Self::ComplexAtomic => "atomic complex values",
            | Self::Atomic => "atomic types",
            | Self::VariableLengthArray => "variable length arrays",
            | Self::LabelAddress => "label addresses",
            | Self::InlineAssembly => "MSVC inline assembly",
            | Self::InlineAssemblyOperand =>
                "inline assembly operands of this register class or type",
            | Self::RegisterVariableOperand =>
                "an asm operand in this local register variable's register",
            | Self::FileScopeAssembly =>
                "file-scope assembly templates containing NUL or bytes that are not UTF-8",
            | Self::AssemblerLabel => "assembler labels",
            | Self::Attribute(attribute) => attribute.description(),
            | Self::StructuredExceptionHandling => "structured exception handling",
            | Self::NamedJump => "named break and continue",
            | Self::NonLocalGoto => "a jump out of a nested function",
            | Self::NestedChainTarget => "nested functions on x86_64-pc-windows-msvc",
            | Self::NestedTrampolineTarget =>
                "the address of a nested function on x86_64-w64-windows-gnu",
            | Self::NestedVariablyModified =>
                "a nested function's use of a variably modified object of its definer",
            | Self::Builtin => "this builtin",
            | Self::StaticInitializer => "this static initializer",
            | Self::Initializer => "this initializer",
            | Self::SelectionDeclaration => "declarations in selection statements",
            | Self::Extension => "this extension",
            | Self::LargeAutomaticObject => "automatic objects of 4 GiB or more",
            | Self::WideBitIntConversion =>
                "conversions between complex types and _BitInt types of more than 128 bits",
            | Self::WideBitIntTemporaries =>
                "functions whose wide _BitInt results need more than 6 MiB of stack",
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
            | Self::Unsupported(_) | Self::UnmodeledType(_) | Self::UnsupportedBuiltin(_) =>
                "not yet supported by lowering",
            | Self::UnanalyzedType =>
                "not yet supported by lowering: a type built from an extension semantic analysis \
                 does not model",
            | Self::MissingFact(_) => "internal error: lowering is missing a semantic fact",
            | Self::InvalidFileScopeAsm(fault) => fault.message(),
            | Self::InvalidArgumentPack(false) => "invalid use of '__builtin_va_arg_pack ()'",
            | Self::InvalidArgumentPack(true) => "invalid use of '__builtin_va_arg_pack_len ()'",
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
            | LoweringErrorKind::UnsupportedBuiltin(Some(name)) =>
                format_in!(arena, "{}: `{name}`", self.kind.message()),
            | LoweringErrorKind::UnsupportedBuiltin(None) => format_in!(
                arena,
                "{}: a builtin this compiler does not know",
                self.kind.message()
            ),
            | kind => kind.message(),
        };
        let explanation = Explanation::new(arena, message);
        let explanation = match self.kind {
            | LoweringErrorKind::Unsupported(_) => explanation
                .label("lowering stops here")
                .note("the program is valid C; this compiler cannot translate it yet"),
            | LoweringErrorKind::UnanalyzedType =>
                explanation.label("the type could not be analyzed"),
            | LoweringErrorKind::UnsupportedBuiltin(_) =>
                explanation.label("this builtin is called here"),
            | LoweringErrorKind::UnmodeledType(_) => explanation
                .label("this expression or object uses the unsupported construct")
                .note("semantic analysis does not yet model this construct"),
            | LoweringErrorKind::InvalidFileScopeAsm(_) => explanation
                .label("invalid file-scope asm")
                .note("asm outside a function takes exactly one narrow string literal"),
            | LoweringErrorKind::InvalidArgumentPack(_) => explanation
                .label("no call supplies the anonymous arguments here")
                .note(
                    "only a direct call of the variadic function using it, which is then always \
                     inlined, can forward its anonymous arguments, as the last argument of a call",
                ),
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
            | LoweringErrorKind::UnsupportedBuiltin(Some(name)) => write!(f, ": `{name}`"),
            | LoweringErrorKind::UnsupportedBuiltin(None) =>
                f.write_str(": a builtin this compiler does not know"),
            | _ => Ok(()),
        }
    }
}
