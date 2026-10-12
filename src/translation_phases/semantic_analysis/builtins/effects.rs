//! Cached effect summaries for GCC overflow predicates' type operands.
//! An explicit postorder stack visits each (expression, read) pair once;
//! lowering retains the cache while scheduling short-circuit effects.
//! GNU extension: GCC "Integer Overflow Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! C99: §6.7.3p6, p. 109; PDF p. 121 (volatile reads).

use super::super::ExpressionInfo;
use crate::{
    translation_phases::parsing::{
        declaration_syntax::TypeQualifiers,
        syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionType as E,
            UnaryOperator as U,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The read flag distinguishes an object's value from its address. A volatile
/// member's address can be pure even when reading that member is an effect.
/// GNU extension: GCC "Integer Overflow Builtins"; C99: §6.7.3p6, p. 109;
/// PDF p. 121.
pub(crate) struct EffectSummaries<'s> {
    pure: ArenaMap<'s, (usize, bool), bool>,
}

impl<'s> EffectSummaries<'s> {
    /// Postorder summaries are independent of scheduling order. Even an
    /// effectful parent retains its children's summaries, preventing repeated
    /// suffix walks when short-circuit tasks later inspect those children.
    pub(crate) fn pure<'tu>(
        &mut self,
        expression: &'tu Expression<'tu>,
        read: bool,
        info: impl Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
        scratch: &'s Bump,
    ) -> bool {
        let key = |e: &Expression<'_>, read| (std::ptr::from_ref(e).addr(), read);
        if let Some(&pure) = self.pure.get(&key(expression, read)) {
            return pure;
        }
        let mut pending = ArenaVec::new_in(scratch);
        pending.push((expression, read, false));
        while let Some((e, read, done)) = pending.pop() {
            #[cfg(test)]
            crate::test_support::OVERFLOW_EFFECT_VISITS.with(|visits| visits.set(visits.get() + 1));
            let key = key(e, read);
            if self.pure.contains_key(&key) {
                continue;
            }
            let pure = match dependencies(e, read, &info) {
                | Rule::Known(pure) => pure,
                | Rule::Children(children) if done =>
                    children.into_iter().flatten().all(|(e, read)| {
                        self.pure
                            .get(&(std::ptr::from_ref(e).addr(), read))
                            .copied()
                            .unwrap_or(false)
                    }),
                | Rule::Children(children) => {
                    pending.push((e, read, true));
                    pending.extend(
                        children
                            .into_iter()
                            .rev()
                            .flatten()
                            .map(|(e, read)| (e, read, false)),
                    );
                    continue;
                },
            };
            _ = self.pure.insert(key, pure);
        }
        self.pure
            .get(&key(expression, read))
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn new(scratch: &'s Bump) -> Self {
        Self {
            pure: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
        }
    }
}

/// A single query used for semantic ICE classification; lowering retains an
/// `EffectSummaries` for the function instead of rebuilding it per operator.
/// GNU extension: GCC "Integer Overflow Builtins".
pub(crate) fn overflow_type_operand_is_pure<'tu>(
    expression: &'tu Expression<'tu>,
    info: impl Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
    scratch: &Bump,
) -> bool {
    EffectSummaries::new(scratch).pure(expression, true, info, scratch)
}

enum Rule<'tu> {
    Known(bool),
    Children([Option<(&'tu Expression<'tu>, bool)>; 3]),
}

/// Inspect only the current node; child effects are combined by the explicit
/// postorder driver. Address-of and member-base reads retain their C meaning.
/// C99: §6.5.3.2p3, pp. 78-79; PDF pp. 90-91; §6.7.3p6, p. 109; PDF p. 121.
fn dependencies<'tu>(
    e: &'tu Expression<'tu>,
    read: bool,
    info: &impl Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> Rule<'tu> {
    let Some(result) = info(e) else {
        return Rule::Known(false);
    };
    if read && result.ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
        return Rule::Known(false);
    }
    if result.ice && result.integer.is_some() {
        return Rule::Known(true);
    }
    let one = |e, read| Rule::Children([Some((e, read)), None, None]);
    match e.kind {
        | E::Identifier(_) | E::Constant(_) | E::StringLiteral(_) => Rule::Known(true),
        | E::SizeofExpr(_) | E::SizeofType(_) | E::AlignofExpr(_) | E::AlignofType(_)
            if result.ice =>
            Rule::Known(true),
        | E::Parenthesized { expression }
        | E::Cast {
            operand_expression: expression,
            ..
        } => one(expression, read),
        | E::Unary {
            operator: U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
            ..
        }
        | E::Binary {
            operator:
                B::Assignment
                | B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::AdditionAssignment
                | B::SubtractionAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseXorAssignment
                | B::BitwiseOrAssignment,
            ..
        } => Rule::Known(false),
        | E::Unary {
            operator,
            operand_expression,
        } => one(operand_expression, operator != U::AddressOf),
        | E::Binary {
            left_expression,
            right_expression,
            ..
        } => Rule::Children([
            Some((left_expression, true)),
            Some((right_expression, true)),
            None,
        ]),
        | E::Conditional(c) | E::OmittedConditional(c) => Rule::Children([
            Some((c.condition_expression, true)),
            Some((c.then_expression, true)),
            Some((c.else_expression, true)),
        ]),
        | E::DirectMember {
            base_expression, ..
        } => one(base_expression, false),
        | E::IndirectMember {
            base_expression, ..
        } => one(base_expression, true),
        | _ => Rule::Known(false),
    }
}
