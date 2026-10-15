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
        semantic_analysis::UnsupportedAttribute,
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
    /// A fact lowering relies on is missing from the semantic results: a
    /// compiler bug, reported rather than turned into wrong code.
    MissingFact(&'static str),
}

/// A construct outside the prototype subset of `middle-end.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Construct {
    Float128,
    ComplexArithmetic,
    Atomic,
    Vector,
    VariableLengthArray,
    LabelAddress,
    InlineAssembly,
    AssemblerLabel,
    /// An attribute semantic analysis accepted whose meaning code generation
    /// lacks.
    Attribute(UnsupportedAttribute),
    StructuredExceptionHandling,
    NamedJump,
    Designator,
    NestedFunction,
    Builtin,
    EncodedString,
    StaticInitializer,
    Initializer,
    SelectionDeclaration,
    Setjmp,
    Extension,
    CompoundLiteralStorage,
}

/// The errors of one translation unit, in the `'ir` arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LoweringErrors<'ir> {
    pub(crate) errors: &'ir [LoweringError],
}

impl LoweringError {
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

    pub(super) const fn unanalyzed(source: SourceVectors) -> Self {
        Self {
            kind:   LoweringErrorKind::UnanalyzedType,
            source: Some(source),
        }
    }
}

impl Construct {
    /// The construct as the diagnostic names it.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            | Self::Float128 => "binary128 floating-point types",
            | Self::ComplexArithmetic => "complex arithmetic",
            | Self::Atomic => "atomic types",
            | Self::Vector => "vector types",
            | Self::VariableLengthArray => "variable length arrays",
            | Self::LabelAddress => "label addresses",
            | Self::InlineAssembly => "inline assembly",
            | Self::AssemblerLabel => "assembler labels",
            | Self::Attribute(attribute) => attribute.description(),
            | Self::StructuredExceptionHandling => "structured exception handling",
            | Self::NamedJump => "named break and continue",
            | Self::Designator => "designated initializers",
            | Self::NestedFunction => "nested functions",
            | Self::Builtin => "this builtin",
            | Self::EncodedString => "encoded string literals",
            | Self::StaticInitializer => "this static initializer",
            | Self::Initializer => "this initializer",
            | Self::SelectionDeclaration => "declarations in selection statements",
            | Self::Setjmp => "setjmp",
            | Self::Extension => "this extension",
            | Self::CompoundLiteralStorage =>
                "compound literals with static or thread storage duration",
        }
    }
}

impl LoweringErrorKind {
    fn message(self) -> &'static str {
        match self {
            | Self::UnitHasErrors => "code is not generated for a translation unit with errors",
            | Self::Unsupported(_) => "not yet supported by lowering",
            | Self::UnanalyzedType =>
                "not yet supported by lowering: a type built from an extension semantic analysis \
                 does not model",
            | Self::MissingFact(_) => "internal error: lowering is missing a semantic fact",
        }
    }
}

impl ToDiagnostic for LoweringError {
    fn diagnostic_in<'d>(
        &self,
        _context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let message = match self.kind {
            | LoweringErrorKind::Unsupported(construct) => format_in!(
                arena,
                "{}: {}",
                self.kind.message(),
                construct.description()
            ),
            | LoweringErrorKind::MissingFact(fact) =>
                format_in!(arena, "{}: {fact}", self.kind.message()),
            | kind => kind.message(),
        };
        let explanation = Explanation::new(arena, message);
        let explanation = match self.kind {
            | LoweringErrorKind::Unsupported(_) => explanation
                .label("lowering stops here")
                .note("the program is valid C; this compiler cannot translate it yet"),
            | LoweringErrorKind::UnanalyzedType => explanation
                .label("this expression or object has that type")
                .note(
                    "unmodeled are `constexpr` objects, `_BitInt`, decimal floating, inferred and \
                     fixed-underlying enum types, MSVC pointer-size modifiers, and attributes \
                     that change layout, such as `align` or `ext_vector_type`",
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
            | LoweringErrorKind::MissingFact(fact) => write!(f, ": {fact}"),
            | _ => Ok(()),
        }
    }
}
