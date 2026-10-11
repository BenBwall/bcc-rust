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
pub(super) fn find<'a>(sema: &SemanticTranslationUnit<'_>, arena: &'a Bump) -> ArenaSet<'a, usize> {
    let mut taken = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for info in sema.expressions {
        let ExpressionType::Unary {
            operator: UnaryOperator::AddressOf,
            mut operand_expression,
        } = info.expression.kind
        else {
            continue;
        };
        loop {
            match operand_expression.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: expression,
                } => operand_expression = expression,
                | ExpressionType::Generic(_) => {
                    match sema
                        .expression_info(operand_expression)
                        .and_then(|operand| operand.selected_expression)
                    {
                        | Some(selected) => operand_expression = selected,
                        | None => break,
                    }
                },
                | ExpressionType::Identifier(_) => {
                    if let Some(binding) = sema
                        .expression_info(operand_expression)
                        .and_then(|operand| operand.binding)
                    {
                        _ = taken.insert(binding);
                    }
                    break;
                },
                | _ => break,
            }
        }
    }
    taken
}
