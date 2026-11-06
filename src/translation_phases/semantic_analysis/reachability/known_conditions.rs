//! Which conditions Clang's CFG takes for decided, and so which edges it
//! prunes: `CFGBuilder::tryEvaluateBool`. It evaluates a condition ignoring its
//! side effects, where a constant does not need the operand it short-circuits
//! or discards, and recognizes conditions the tautology warnings are about: a
//! comparison that a mask or a Boolean operand decides, and two comparisons of
//! one operand that a logical operator makes contradictory.
//! C99: §6.5.8-§6.5.14, pp. 85-89; PDF pp. 97-101; §6.6, pp. 95-96;
//! PDF pp. 107-108.

use super::{
    super::{
        Analyzer,
        BindingKind,
        ExpressionType as E,
        Integer,
        types::{
            Scalar,
            TypeKind,
        },
    },
    Flow,
};
use crate::translation_phases::parsing::syntax::{
    BinaryOperator as B,
    Constant,
    Expression,
    UnaryOperator as U,
};

/// A comparison normalized to `operand operator constant`.
#[derive(Clone, Copy)]
struct Bound<'tu> {
    operand:  &'tu Expression<'tu>,
    operator: B,
    constant: &'tu Expression<'tu>,
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// `tryEvaluateBool`: the truth of `e` where the CFG knows it. Only an
    /// operator directly at `e` gets the special readings, not one inside
    /// parentheses, which the evaluation alone sees through.
    pub(super) fn known_condition(
        &mut self,
        flow: &mut Flow<'s>,
        e: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let key = std::ptr::from_ref(e).addr();
        if let Some(&known) = flow.known.get(&key) {
            return known;
        }
        let known = self.compute_known_condition(flow, e);
        _ = flow.known.insert(key, known);
        known
    }

    fn compute_known_condition(
        &mut self,
        flow: &mut Flow<'s>,
        e: &'tu Expression<'tu>,
    ) -> Option<bool> {
        if let E::Binary {
            operator,
            left_expression: left,
            right_expression: right,
        } = e.kind
        {
            let special = match operator {
                | B::LogicalAnd | B::LogicalOr =>
                    return self.known_logical(flow, operator, left, right),
                | B::Equal | B::NotEqual => self.known_equality(operator, left, right),
                | B::LessThan | B::GreaterThan | B::LessThanOrEqual | B::GreaterThanOrEqual =>
                    self.known_relational(operator, left, right),
                | B::BitwiseOr => self.known_bitwise_or(left, right),
                | B::BitwiseAnd | B::Multiplication =>
                    (self.folds_to_zero(left) || self.folds_to_zero(right)).then_some(false),
                | _ => None,
            };
            if special.is_some() {
                return special;
            }
        }
        self.evaluate_ignoring_side_effects(e)?.truth()
    }

    fn folds_to_zero(&mut self, e: &'tu Expression<'tu>) -> bool {
        self.folded_integer(e).is_some_and(Integer::is_zero)
    }

    /// `X && 0` is false and `X || 1` true whatever `X` is, and so on.
    /// C99: §6.5.13p3, p. 89; PDF p. 101; §6.5.14p3, p. 89; PDF p. 101.
    fn known_logical(
        &mut self,
        flow: &mut Flow<'s>,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let deciding = operator == B::LogicalOr;
        let left_known = self.known_condition(flow, left);
        if left_known == Some(deciding) {
            return left_known;
        }
        let right_known = self.known_condition(flow, right);
        match (left_known, right_known) {
            | (Some(_), known) => known,
            | (None, Some(known)) => (known == deciding).then_some(known),
            | (None, None) => self.known_overlap(operator, left, right),
        }
    }

    /// `checkIncorrectEqualityOperator`: a value no mask can produce, or that
    /// a Boolean operand cannot have.
    fn known_equality(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let (constant, other) = self.literal_and_other(left, right)?.0;
        let different = Some(operator != B::Equal);
        if let E::Binary {
            operator: mask @ (B::BitwiseAnd | B::BitwiseOr),
            left_expression,
            right_expression,
        } = other.kind
        {
            let bits = self
                .integer_literal(left_expression)
                .or_else(|| self.integer_literal(right_expression))?;
            let result = if mask == B::BitwiseAnd {
                bits & constant
            } else {
                bits | constant
            };
            (result != constant).then_some(different?)
        } else if self.boolean_valued(other) {
            different.filter(|_| constant != 0 && constant != 1)
        } else {
            None
        }
    }

    /// `checkIncorrectRelationalOperator`: a Boolean operand against a greater
    /// constant. Clang takes no position on a negative one.
    fn known_relational(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let ((constant, other), constant_first) = self.literal_and_other(left, right)?;
        if !self.boolean_valued(other) || constant <= 1 {
            return None;
        }
        Some(constant_first == matches!(operator, B::GreaterThan | B::GreaterThanOrEqual))
    }

    /// `checkIncorrectBitwiseOrOperator`: a nonzero literal or enumerator
    /// sets a bit whatever the other operand is.
    fn known_bitwise_or(
        &self,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let value = |e| {
            self.integer_literal(e).or_else(|| {
                _ = self.enumerator_tag(e)?;
                self.expression_info(e).integer?.to_i128()
            })
        };
        match (value(left), value(right)) {
            | (Some(constant), None) | (None, Some(constant)) => (constant != 0).then_some(true),
            | _ => None,
        }
    }

    /// `checkIncorrectLogicOperator`: two comparisons of one operand with
    /// constants, read at each kind of value the operand can have.
    fn known_overlap(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        let first = self.bound(left)?;
        let second = self.bound(right)?;
        if !self.same_operand(first.operand, second.operand)
            || self.enumerator_tag(first.constant) != self.enumerator_tag(second.constant)
        {
            return None;
        }
        let low = self.expression_info(first.constant).integer?;
        let high = self.expression_info(second.constant).integer?;
        if low.signed != high.signed || low.bits != high.bits || low.bits >= 127 {
            return None;
        }
        let (first_value, second_value) = (low.to_i128()?, high.to_i128()?);
        let (minimum, maximum) = if low.signed {
            (-(1_i128 << (low.bits - 1)), (1_i128 << (low.bits - 1)) - 1)
        } else {
            (0, (1_i128 << low.bits) - 1)
        };
        let mut always_true = true;
        let mut always_false = true;
        for value in [
            minimum,
            first_value,
            first_value.min(second_value) + 1,
            second_value,
            maximum,
        ] {
            let one = Self::compares(first.operator, value, first_value)?;
            let other = Self::compares(second.operator, value, second_value)?;
            let result = if operator == B::LogicalAnd {
                one && other
            } else {
                one || other
            };
            always_true &= result;
            always_false &= !result;
        }
        if always_true {
            Some(true)
        } else if always_false {
            Some(false)
        } else {
            None
        }
    }

    const fn compares(operator: B, left: i128, right: i128) -> Option<bool> {
        Some(match operator {
            | B::Equal => left == right,
            | B::NotEqual => left != right,
            | B::LessThan => left < right,
            | B::LessThanOrEqual => left <= right,
            | B::GreaterThan => left > right,
            | B::GreaterThanOrEqual => left >= right,
            | _ => return None,
        })
    }

    /// `tryNormalizeBinaryOperator`.
    fn bound(&self, e: &'tu Expression<'tu>) -> Option<Bound<'tu>> {
        let E::Binary {
            operator,
            left_expression,
            right_expression,
        } = Self::without_parentheses(e).kind
        else {
            return None;
        };
        let constant = |e: &'tu Expression<'tu>| {
            let e = Self::without_parentheses(e);
            let literal = match e.kind {
                | E::Unary {
                    operator: U::Minus,
                    operand_expression,
                } => Self::is_integer_literal(operand_expression),
                | _ => Self::is_integer_literal(e),
            };
            (literal || self.enumerator_tag(e).is_some()).then_some(e)
        };
        if let Some(found) = constant(right_expression) {
            return Some(Bound {
                operand: left_expression,
                operator,
                constant: found,
            });
        }
        let found = constant(left_expression)?;
        Some(Bound {
            operand:  right_expression,
            operator: match operator {
                | B::LessThan => B::GreaterThan,
                | B::LessThanOrEqual => B::GreaterThanOrEqual,
                | B::GreaterThan => B::LessThan,
                | B::GreaterThanOrEqual => B::LessThanOrEqual,
                | other => other,
            },
            constant: found,
        })
    }

    /// `Expr::isSameComparisonOperand`, for what can be read twice without
    /// differing: a name, a member of one, and a subscript.
    fn same_operand(&self, a: &'tu Expression<'tu>, b: &'tu Expression<'tu>) -> bool {
        let a = Self::without_parentheses(a);
        let b = Self::without_parentheses(b);
        match (a.kind, b.kind) {
            | (E::Identifier(_), E::Identifier(_)) => {
                let binding = self.expression_info(a).binding;
                binding.is_some() && binding == self.expression_info(b).binding
            },
            | (
                E::DirectMember {
                    base_expression: first,
                    member: first_member,
                },
                E::DirectMember {
                    base_expression: second,
                    member: second_member,
                },
            )
            | (
                E::IndirectMember {
                    base_expression: first,
                    member: first_member,
                },
                E::IndirectMember {
                    base_expression: second,
                    member: second_member,
                },
            ) => first_member.name == second_member.name && self.same_operand(first, second),
            | (
                E::Binary {
                    operator: B::Subscript,
                    left_expression: first_base,
                    right_expression: first_index,
                },
                E::Binary {
                    operator: B::Subscript,
                    left_expression: second_base,
                    right_expression: second_index,
                },
            ) =>
                self.same_operand(first_base, second_base)
                    && self.same_operand(first_index, second_index),
            | (E::Constant(_), E::Constant(_)) => {
                let value = self.expression_info(a).integer.and_then(Integer::to_i128);
                value.is_some()
                    && value == self.expression_info(b).integer.and_then(Integer::to_i128)
            },
            | _ => false,
        }
    }

    /// The enumeration whose enumerator `e` names.
    fn enumerator_tag(&self, e: &'tu Expression<'tu>) -> Option<usize> {
        let e = Self::without_parentheses(e);
        if !matches!(e.kind, E::Identifier(_)) {
            return None;
        }
        let binding = self.expression_info(e).binding?;
        if self.bindings[binding].kind != BindingKind::Enumerator {
            return None;
        }
        self.reachability.enumerator_tags.get(&binding).copied()
    }

    /// The operands of a comparison when one is an integer literal, and
    /// whether that one comes first.
    fn literal_and_other(
        &self,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<((i128, &'tu Expression<'tu>), bool)> {
        if let Some(constant) = self.integer_literal(left) {
            return Some(((constant, Self::without_parentheses(right)), true));
        }
        let constant = self.integer_literal(right)?;
        Some(((constant, Self::without_parentheses(left)), false))
    }

    /// `getIntegerLiteralSubexpressionValue`: a literal, or a unary operator
    /// directly on one.
    fn integer_literal(&self, e: &'tu Expression<'tu>) -> Option<i128> {
        let e = Self::without_parentheses(e);
        let literal = match e.kind {
            | E::Unary {
                operator: U::Plus | U::Minus | U::BitwiseNot | U::LogicalNot,
                operand_expression,
            } => Self::is_integer_literal(operand_expression),
            | _ => Self::is_integer_literal(e),
        };
        if literal {
            self.expression_info(e).integer?.to_i128()
        } else {
            None
        }
    }

    fn is_integer_literal(e: &'tu Expression<'tu>) -> bool {
        matches!(
            Self::without_parentheses(e).kind,
            E::Constant(Constant::Integer(_))
        )
    }

    /// `Expr::isKnownToHaveBooleanValue`.
    fn boolean_valued(&self, e: &'tu Expression<'tu>) -> bool {
        let e = Self::without_parentheses(e);
        if self.types.nodes[self.expression_info(e).ty.index] == TypeKind::Scalar(Scalar::Bool) {
            return true;
        }
        match e.kind {
            | E::Constant(Constant::Integer(_)) => self
                .expression_info(e)
                .integer
                .and_then(Integer::to_i128)
                .is_some_and(|value| value == 0 || value == 1),
            | E::Unary {
                operator: U::LogicalNot,
                ..
            }
            | E::Binary {
                operator:
                    B::LessThan
                    | B::GreaterThan
                    | B::LessThanOrEqual
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual
                    | B::LogicalAnd
                    | B::LogicalOr,
                ..
            } => true,
            | E::Binary {
                operator: B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor,
                left_expression,
                right_expression,
            } => self.boolean_valued(left_expression) && self.boolean_valued(right_expression),
            | E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } => self.boolean_valued(right_expression),
            | E::Conditional(c) =>
                self.boolean_valued(c.then_expression) && self.boolean_valued(c.else_expression),
            | _ => false,
        }
    }
}
