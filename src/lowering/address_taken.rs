//! The pre-pass that finds locals whose address is taken.
//!
//! A local whose address escapes through `&` may be read or written through
//! a pointer, so it must live in memory rather than in SSA values. Semantic
//! analysis retained a record for every expression it typed, so one linear
//! scan over those records finds every `&` operand in the unit without
//! walking any syntax tree. Arrays and structures, whose elements are
//! addressed implicitly, live in memory whatever this finds.
//! C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.

use crate::{
    translation_phases::{
        parsing::syntax::{
            ExpressionType,
            UnaryOperator,
        },
        semantic_analysis::SemanticTranslationUnit,
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
                && sema.bindings[binding].origin.descriptor().is_some()
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
