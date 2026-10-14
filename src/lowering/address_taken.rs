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
        Context,
        parsing::syntax::{
            ExpressionType,
            UnaryOperator,
        },
        semantic_analysis::{
            CommonBuiltin,
            SemanticTranslationUnit,
            common_builtin,
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
    context: &Context<'_>,
    arena: &'a Bump,
) -> Result<ArenaSet<'a, usize>, super::LoweringError> {
    let mut taken = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for info in sema.expressions {
        if let ExpressionType::Call { arguments, .. } = info.expression.kind {
            let call = sema.call_facts(info.expression).ok_or_else(|| {
                super::LoweringError::missing(
                    "call facts are not retained",
                    info.expression.source_vectors,
                )
            })?;
            if let crate::translation_phases::semantic_analysis::Callee::Builtin { binding, .. } =
                call.callee
                && let Some(CommonBuiltin::Varargs(keyword)) =
                    common_builtin(context.string_cache.at(sema.bindings[binding].name.name))
            {
                // Win64 cursors need storage even though these intrinsics use
                // ordinary call syntax. C99: §7.15.1, pp. 249-251; PDF pp.
                // 261-263.
                let count = if keyword
                    == crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaCopy
                {
                    2
                } else {
                    1
                };
                for &operand in arguments.iter().take(count) {
                    if let Some(binding) =
                        sema.expression_info(operand).and_then(|info| info.binding)
                    {
                        _ = taken.insert(binding);
                    }
                }
            }
        }
        if let ExpressionType::Builtin(b) = info.expression.kind {
            use crate::translation_phases::{
                parsing::SyntaxOperand,
                preprocessing::KeywordTokenType as K,
            };
            if matches!(
                b.keyword,
                K::BuiltinVaStart | K::BuiltinVaArg | K::BuiltinVaCopy | K::BuiltinVaEnd
            ) {
                for &operand in b.operands {
                    if let SyntaxOperand::Expression(e) = operand
                        && let Some(binding) = sema.expression_info(e).and_then(|info| info.binding)
                    {
                        _ = taken.insert(binding);
                    }
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
