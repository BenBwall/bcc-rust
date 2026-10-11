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
    FloatingPoint,
    ComplexArithmetic,
    Atomic,
    Vector,
    BitField,
    VariableLengthArray,
    AggregateArgument,
    AggregateResult,
    StatementExpression,
    LabelAddress,
    ComputedGoto,
    InlineAssembly,
    StructuredExceptionHandling,
    LocalLabel,
    NamedJump,
    CaseRange,
    Designator,
    NestedFunction,
    Builtin,
    OmittedConditional,
    WideString,
    EncodedString,
    StaticInitializer,
    Initializer,
    SelectionDeclaration,
    Setjmp,
    Extension,
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
            | Self::FloatingPoint => "floating-point arithmetic",
            | Self::ComplexArithmetic => "complex arithmetic",
            | Self::Atomic => "atomic types",
            | Self::Vector => "vector types",
            | Self::BitField => "bit-fields",
            | Self::VariableLengthArray => "variable length arrays",
            | Self::AggregateArgument => "structures and unions passed by value",
            | Self::AggregateResult => "structures and unions returned by value",
            | Self::StatementExpression => "statement expressions",
            | Self::LabelAddress => "label addresses",
            | Self::ComputedGoto => "computed goto",
            | Self::InlineAssembly => "inline assembly",
            | Self::StructuredExceptionHandling => "structured exception handling",
            | Self::LocalLabel => "local labels",
            | Self::NamedJump => "named break and continue",
            | Self::CaseRange => "case ranges",
            | Self::Designator => "designated initializers",
            | Self::NestedFunction => "nested functions",
            | Self::Builtin => "this builtin",
            | Self::OmittedConditional => "conditionals with an omitted operand",
            | Self::WideString => "wide string literals",
            | Self::EncodedString => "encoded string literals",
            | Self::StaticInitializer => "this static initializer",
            | Self::Initializer => "this initializer",
            | Self::SelectionDeclaration => "declarations in selection statements",
            | Self::Setjmp => "setjmp",
            | Self::Extension => "this extension",
        }
    }
}

impl LoweringErrorKind {
    fn message(self) -> &'static str {
        match self {
            | Self::UnitHasErrors => "code is not generated for a translation unit with errors",
            | Self::Unsupported(_) => "not yet supported by lowering",
            | Self::UnanalyzedType =>
                "construct not supported by code generation: its type was not analyzed",
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
            | LoweringErrorKind::UnanalyzedType => explanation.label("type unavailable"),
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
