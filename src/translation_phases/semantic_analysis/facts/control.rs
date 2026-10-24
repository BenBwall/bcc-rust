//! Durable region, statement, label and edge records. No IR or scratch data.
//! GNU labels as values add each function's address-taken labels, the label
//! each `&&label` resolves to, the label pair of each constant label
//! difference, and computed gotos as jumps to [`ControlTarget::IndirectGoto`].
//! C99: §6.2.4p6, p. 32; PDF p. 44; §6.8.6.1p1, p. 137; PDF p. 149.
//! GNU extension: GCC "Labels as Values".

use super::super::{
    Integer,
    TypeId,
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            Identifier,
            Statement,
        },
    },
    util::bump::ArenaMap,
};

#[derive(Debug)]
pub(crate) struct ControlFacts<'tu> {
    pub(crate) regions: &'tu [Region],
    pub(crate) statements: &'tu [StatementFacts<'tu>],
    pub(crate) labels: &'tu [LabelFacts<'tu>],
    pub(crate) jumps: &'tu [JumpFacts],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained loop point interface; execution consumes resolved jump \
                      destinations."
        )
    )]
    pub(crate) loops: &'tu [LoopFacts],
    pub(crate) switches: &'tu [SwitchFacts<'tu>],
    pub(in crate::translation_phases::semantic_analysis) statement_indices:
        ArenaMap<'tu, usize, StatementId>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained executed-expression points for the semantic interface."
        )
    )]
    pub(in crate::translation_phases::semantic_analysis) expression_points:
        ArenaMap<'tu, usize, ControlPoint>,
    pub(in crate::translation_phases::semantic_analysis) vla_regions:
        ArenaMap<'tu, BindingId, RegionId>,
    /// Each function's address-taken labels, in the order of their first
    /// `&&label`; a label's position is its [`LabelFacts::address`].
    pub(in crate::translation_phases::semantic_analysis) address_taken:
        ArenaMap<'tu, FunctionId, &'tu [LabelId]>,
    /// The label each `&&label` expression resolves to, by expression
    /// identity.
    pub(in crate::translation_phases::semantic_analysis) label_addresses:
        ArenaMap<'tu, usize, LabelId>,
    /// The `(label, base)` pair of each constant label difference
    /// `&&label - &&base`, and of the conversions that keep one constant, by
    /// expression identity.
    pub(in crate::translation_phases::semantic_analysis) label_differences:
        ArenaMap<'tu, usize, (LabelId, LabelId)>,
    #[cfg(test)]
    pub(crate) construction_work: usize,
    #[cfg(test)]
    pub(crate) query_work: usize,
}

impl<'tu> ControlFacts<'tu> {
    pub(crate) fn statement(&self, statement: &Statement<'_>) -> Option<&StatementFacts<'tu>> {
        self.statement_indices
            .get(&std::ptr::from_ref(statement).addr())
            .map(|id| &self.statements[id.0])
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained executed-expression point query inspected by interface tests."
        )
    )]
    pub(crate) fn expression_point(&self, expression: &Expression<'_>) -> Option<ControlPoint> {
        self.expression_points
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    pub(crate) fn vla_region(&self, binding: usize) -> Option<RegionId> {
        self.vla_regions.get(&BindingId(binding)).copied()
    }

    /// The labels of `function` whose address is taken, in address order:
    /// every possible target of its computed gotos.
    pub(crate) fn address_taken(&self, function: FunctionId) -> &'tu [LabelId] {
        self.address_taken
            .get(&function)
            .copied()
            .unwrap_or_default()
    }

    /// The label an `&&label` expression names.
    pub(crate) fn label_address(&self, expression: &Expression<'_>) -> Option<LabelId> {
        self.label_addresses
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    /// The `(label, base)` labels of a constant label difference.
    pub(crate) fn label_difference(
        &self,
        expression: &Expression<'_>,
    ) -> Option<(LabelId, LabelId)> {
        self.label_differences
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }
}

macro_rules! index {
    ($($name:ident),*) => { $(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub(crate) struct $name(pub(crate) usize);
    )* };
}
index!(
    RegionId,
    ScopeId,
    FunctionId,
    BindingId,
    ExprId,
    StatementId,
    LabelId,
    JumpId,
    SwitchId
);

#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionControl {
    pub(crate) id:   FunctionId,
    pub(crate) root: RegionId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Region {
    pub(crate) parent: Option<RegionId>,
    pub(crate) lexical_scope: ScopeId,
    pub(crate) function: FunctionId,
    pub(crate) kind: RegionKind,
    pub(crate) preorder: usize,
    pub(crate) subtree_end: usize,
    pub(crate) nearest_vm: Option<RegionId>,
    /// The innermost region whose exit runs a cleanup: VLA storage release
    /// or a GNU cleanup function call.
    pub(crate) nearest_cleanup: Option<RegionId>,
    /// The innermost GNU cleanup declaration region, which jumps may leave
    /// but not enter.
    pub(crate) nearest_cleanup_declaration: Option<RegionId>,
    pub(crate) nearest_statement_expression: Option<RegionId>,
    pub(crate) cleanup: CleanupKind,
    pub(crate) source: SourceVectors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionKind {
    Lexical,
    VmDeclaration(BindingId),
    StatementExpression(ExprId),
    /// From the end of a GNU `cleanup` variable's declarator and initializer
    /// to the end of its block.
    /// GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
    CleanupDeclaration(BindingId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupKind {
    None,
    AutomaticVla(BindingId),
    /// Every exit from the region calls `function` with the address of
    /// `variable`, innermost region first.
    /// GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
    Call {
        variable: BindingId,
        function: BindingId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ControlPoint {
    pub(crate) region:   RegionId,
    pub(crate) function: FunctionId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegionRun {
    pub(crate) first:       RegionId,
    pub(crate) stop_before: RegionId,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Transition {
    pub(crate) left:    RegionRun,
    pub(crate) entered: RegionRun,
}

impl Transition {
    pub(in crate::translation_phases::semantic_analysis) fn empty(region: RegionId) -> Self {
        let run = RegionRun {
            first:       region,
            stop_before: region,
        };
        Self {
            left:    run,
            entered: run,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ControlTarget {
    Label(LabelId),
    LoopBreak(StatementId),
    LoopContinue(StatementId),
    SwitchBreak(StatementId),
    FunctionExit(FunctionId),
    /// A computed goto: any address-taken label of the function. Its
    /// transition is empty; like Clang, leaving a variable length array's
    /// scope this way does not release its storage before the function
    /// returns. GNU extension: GCC "Labels as Values".
    IndirectGoto(FunctionId),
}

/// Control flow chosen by the same emission decision that omitted its arms.
/// C99: §6.8.4.1p2, p. 133; PDF p. 145; §6.8.4.2p4-5, p. 134; PDF p. 146.
#[expect(
    variant_size_differences,
    reason = "Switch decisions retain a selected syntax label"
)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum GeneratedControl<'tu> {
    Runtime,
    If(bool),
    Switch(Option<&'tu Statement<'tu>>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StatementFacts<'tu> {
    pub(crate) generated: GeneratedControl<'tu>,
    /// A whole statement is omitted by code generation,
    /// including its static declarations. C99: §6.8.4.1p2, pp. 133-134;
    /// PDF pp. 145-146; storage emission follows Clang.
    pub(crate) elided:    bool,
    pub(crate) id:        StatementId,
    pub(crate) syntax:    &'tu Statement<'tu>,
    pub(crate) entry:     ControlPoint,
    pub(crate) exit:      Transition,
    pub(crate) label:     Option<LabelId>,
    pub(crate) jump:      Option<JumpId>,
    pub(crate) switch:    Option<SwitchId>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabelFacts<'tu> {
    pub(crate) id:         LabelId,
    pub(crate) name:       Option<Identifier>,
    pub(crate) function:   FunctionId,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Resolved GNU local label identity; local-label execution remains \
                      unsupported."
        )
    )]
    pub(crate) local:      bool,
    pub(crate) definition: Option<&'tu Statement<'tu>>,
    pub(crate) point:      Option<ControlPoint>,
    /// The label's position among its function's address-taken labels, if
    /// some `&&label` names it.
    pub(crate) address:    Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct JumpFacts {
    pub(crate) source:      ControlPoint,
    pub(crate) target:      ControlTarget,
    pub(crate) destination: ControlPoint,
    pub(crate) transition:  Transition,
    pub(crate) provenance:  SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LoopFacts {
    pub(crate) statement: StatementId,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained loop header point; lowering uses resolved jump transitions."
        )
    )]
    pub(crate) header:    ControlPoint,
    pub(crate) body:      ControlPoint,
    pub(crate) step:      ControlPoint,
    pub(crate) exit:      ControlPoint,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CaseFacts<'tu> {
    pub(crate) label:     LabelId,
    pub(crate) statement: &'tu Statement<'tu>,
    pub(crate) point:     ControlPoint,
    pub(crate) lower:     Integer,
    pub(crate) upper:     Integer,
    pub(crate) range:     bool,
    pub(crate) ordinal:   usize,
    /// Empty GNU intervals retain their label without a dispatch transition.
    pub(crate) dispatch:  Option<Transition>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SwitchFacts<'tu> {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained switch owner identity.")
    )]
    pub(crate) statement:        StatementId,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained switch controlling expression identity.")
    )]
    pub(crate) control:          &'tu Expression<'tu>,
    pub(crate) ty:               TypeId,
    pub(crate) dispatch:         ControlPoint,
    pub(crate) exit:             ControlPoint,
    pub(crate) default:          Option<LabelId>,
    pub(crate) default_dispatch: Option<Transition>,
    pub(crate) no_match:         Transition,
    pub(crate) cases:            &'tu [CaseFacts<'tu>],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained sorted case index; emission preserves source-order block numbering."
        )
    )]
    pub(crate) sorted:           &'tu [usize],
}
