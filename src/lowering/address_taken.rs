//! The pre-pass that finds locals whose address is taken.
//!
//! A local whose address escapes through `&` may be read or written through
//! a pointer, so it must live in memory rather than in SSA values. Semantic
//! analysis retained a record for every expression it typed, so one linear
//! scan over those records finds every `&` operand in the unit without
//! walking any syntax tree; the memory operands of `asm` statements are
//! addressed too. Arrays and structures, whose elements are
//! addressed implicitly, live in memory whatever this finds.
//! C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.

use crate::{
    translation_phases::{
        parsing::syntax::{
            ExpressionType,
            UnaryOperator,
        },
        semantic_analysis::{
            AsmOperandKind,
            BindingKind,
            SemanticTranslationUnit,
        },
    },
    util::bump::{
        ArenaSet,
        Bump,
    },
};

/// The bindings that are the operand of `&`, through parentheses,
/// `__extension__` and generic selections.
pub(super) fn find<'a>(
    sema: &SemanticTranslationUnit<'_>,
    arena: &'a Bump,
) -> Result<ArenaSet<'a, usize>, super::LoweringError> {
    let mut taken = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for info in sema.expressions {
        if matches!(info.expression.kind, ExpressionType::Call { .. }) {
            let call = sema.call_facts(info.expression).ok_or_else(|| {
                super::LoweringError::missing(
                    "call facts are not retained",
                    info.expression.source_vectors,
                )
            })?;
            if let crate::translation_phases::semantic_analysis::Callee::Builtin {
                binding: Some(binding),
                instance: None,
                ..
            } = call.callee
                && sema.bindings[binding]
                    .origin
                    .descriptor()
                    .is_some_and(|descriptor| descriptor.operation.retains_instance())
            {
                return Err(super::LoweringError::missing(
                    "checked named builtin is not retained",
                    info.expression.source_vectors,
                ));
            }
        }
        if let Some(instance) = sema.builtin_call_facts(info.expression) {
            for operand in instance.operands {
                if let Some(binding) = operand.storage_binding {
                    _ = taken.insert(binding);
                }
            }
        }
        let ExpressionType::Unary {
            operator: UnaryOperator::AddressOf,
            operand_expression,
        } = info.expression.kind
        else {
            continue;
        };
        let operand = sema.designation(operand_expression).ok_or_else(|| {
            super::LoweringError::missing(
                "designation facts are not retained",
                operand_expression.source_vectors,
            )
        })?;
        if matches!(operand.leaf.kind, ExpressionType::Identifier(_))
            && let Some(binding) = operand.binding
        {
            _ = taken.insert(binding);
        }
    }
    // A memory operand of inline assembly is addressed by the assembly.
    // GNU extension: GCC manual, "Extended Asm".
    for statement in sema.assembly.iter() {
        for operand in statement.operands {
            if operand.kind == AsmOperandKind::Memory
                && let Some(designation) = sema.designation(operand.expression)
                && matches!(designation.leaf.kind, ExpressionType::Identifier(_))
                && let Some(binding) = designation.binding
            {
                _ = taken.insert(binding);
            }
        }
    }
    // A GNU cleanup function receives its variable's address on every exit.
    // GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    for region in sema.control.regions {
        if let crate::translation_phases::semantic_analysis::CleanupKind::Call {
            variable, ..
        } = region.cleanup
        {
            _ = taken.insert(variable.0);
        }
    }
    // Prototype bound expressions and body uses designate the same parameter
    // object. Propagate the storage requirement before constructing any local.
    // C99: §6.9.1p10, p. 142; PDF p. 154.
    for function in sema.functions {
        for &(prototype, body) in function.parameter_aliases {
            if taken.contains(&prototype) || taken.contains(&body) {
                _ = taken.insert(prototype);
                _ = taken.insert(body);
            }
        }
    }
    Ok(taken)
}

/// The functions whose designator the unit uses other than as the callee of
/// a direct call: their address may reach another object, which calls them
/// with the convention their own compiler would give them.
/// C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58 (function designators).
pub(super) fn addressed_functions<'a>(
    sema: &SemanticTranslationUnit<'_>,
    arena: &'a Bump,
) -> ArenaSet<'a, usize> {
    let mut callees = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for info in sema.expressions {
        let ExpressionType::Call {
            function_expression,
            ..
        } = info.expression.kind
        else {
            continue;
        };
        let mut callee = function_expression;
        while let ExpressionType::Parenthesized { expression }
        | ExpressionType::Unary {
            operator: UnaryOperator::Extension,
            operand_expression: expression,
        } = callee.kind
        {
            callee = expression;
        }
        _ = callees.insert(core::ptr::from_ref(callee) as usize);
    }
    let mut addressed = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for info in sema.expressions {
        if let ExpressionType::Identifier(_) = info.expression.kind
            && let Some(binding) = info.binding
            && sema.bindings[binding].kind == BindingKind::Function
            && !callees.contains(&(core::ptr::from_ref(info.expression) as usize))
        {
            _ = addressed.insert(binding);
        }
    }
    addressed
}
