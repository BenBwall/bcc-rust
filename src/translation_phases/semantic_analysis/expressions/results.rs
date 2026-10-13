//! Retained phase-7 value categories, constant classes and conversion sites.
//! These records describe checked expressions; they do not schedule syntax or
//! perform backend lowering.
//! C99: §6.3.2.1, pp. 46-47; PDF pp. 58-59;
//! §6.6, pp. 95-96; PDF pp. 107-108.

use super::ExpressionInfo;
use crate::translation_phases::{
    parsing::syntax::{
        BlockItem,
        Expression,
        ExpressionSlot,
        ExpressionType,
        Statement,
        StatementType,
        UnaryOperator,
    },
    semantic_analysis::types::TypeId,
};

/// Follows designation-preserving syntax and retained compile-time selections.
/// Each step visits a strict child, without recursive syntax traversal.
/// The callback observes every visited node, including transparent wrappers,
/// so consumers can check conversions without duplicating the syntax walk.
/// C99: §6.5.1p5, p. 69; PDF p. 81 (parentheses).
/// C11: §6.5.1.1p4, p. 79; PDF p. 97 (generic selection).
/// GNU extension: GCC "Other Builtins", `__builtin_choose_expr`.
/// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
pub(crate) fn selected_designator<'tu>(
    mut expression: &'tu Expression<'tu>,
    mut info: impl FnMut(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> &'tu Expression<'tu> {
    loop {
        let selected = info(expression).and_then(|i| i.selected_expression);
        expression = match expression.kind {
            | ExpressionType::Parenthesized { expression } => expression,
            | ExpressionType::Unary {
                operator: UnaryOperator::Extension,
                operand_expression,
            } => operand_expression,
            | _ => match selected {
                | Some(selected) => selected,
                | None => return expression,
            },
        };
    }
}

/// Finds the final value statement of a GNU statement-expression block.
/// Labels do not change its value. Return the outer statement too, so lowering
/// enters every label target before scheduling the expression. A final nested
/// compound statement has void type; only the owning block is unwrapped.
/// C99: §6.8.1p4, p. 132; PDF p. 144 (labels preserve control flow).
/// GNU extension: GCC "Statement Exprs".
/// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
pub(crate) fn statement_expression_tail<'tu>(
    block: &'tu Statement<'tu>,
) -> Option<(&'tu Statement<'tu>, &'tu Expression<'tu>)> {
    let StatementType::Compound { items } = block.kind else {
        return None;
    };
    let BlockItem::Statement(outer) = *items.last()? else {
        return None;
    };
    let mut statement = outer;
    loop {
        statement = match statement.kind {
            | StatementType::Label(_, inner)
            | StatementType::Case(_, inner)
            | StatementType::Default(inner) => inner,
            | StatementType::CaseRange(range) => range.statement,
            | StatementType::Attributed(attributed) => attributed.statement,
            | StatementType::Expression(ExpressionSlot::Parsed(e)) => return Some((outer, e)),
            | _ => return None,
        };
    }
}

/// One contextual conversion and its resulting type.
/// C99: §6.3.2.1 paragraphs 2-4, p. 46; PDF p. 58.
/// C99: §6.5.2.2 paragraphs 6-7, pp. 71-72; PDF pp. 83-84.
/// C99: §6.5.16.1 paragraph 2, p. 92; PDF p. 104.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Conversion<'tu> {
    pub(crate) expression: &'tu Expression<'tu>,
    pub(crate) ty:         TypeId,
    pub(crate) kind:       ConversionKind,
}

/// An explicit contextual conversion for a particular operand occurrence.
/// C99: §6.3, pp. 42-50; PDF pp. 54-62.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversionKind {
    Lvalue,
    ArrayDecay,
    FunctionDecay,
    Arithmetic,
    Assignment,
    DefaultArgument,
}

/// The category before contextual lvalue conversion or decay.
/// C99: §6.3.2.1p1-4, p. 46; PDF p. 58.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueCategory {
    Lvalue,
    ModifiableLvalue,
    FunctionDesignator,
    Rvalue,
}

/// Constant-expression classes; integer values use the shared ICE arithmetic.
/// Address expressions retain their syntax and resolved bindings for lowering.
/// C99: §6.6p6-9, pp. 95-96; PDF pp. 107-108.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstantClass {
    None,
    Arithmetic,
    Address,
}

/// Folding restrictions and GNU address-difference eligibility are separate
/// from initializer constant classes. Atomic value casts remain non-ICEs.
/// C99: §6.6p10, p. 96; PDF p. 108 (additional constant forms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstantFolding {
    Permitted,
    /// An address difference folded under C99 §6.6p10 and GNU policy.
    AddressDifference,
    AtomicCast,
}

impl ExpressionInfo<'_> {
    /// For `.` and `->` (and records copied from one, such as parentheses),
    /// the index of the selected member in the record's `Tag::fields`.
    /// C99: §6.5.2.3 paragraphs 3-4, p. 73; PDF p. 85.
    pub(crate) fn field_index(&self) -> Option<usize> {
        self.field.map(|field| field.get() as usize - 1)
    }

    pub(in crate::translation_phases::semantic_analysis) fn atomic_cast(self) -> bool {
        self.folding == ConstantFolding::AtomicCast
    }

    pub(in crate::translation_phases::semantic_analysis) fn folded_address(self) -> bool {
        self.folding == ConstantFolding::AddressDifference
    }
}
