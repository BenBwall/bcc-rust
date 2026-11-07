//! Phase-7 folding of an address's truth where a constant is required: an
//! object or function has a non-null address (C99 §6.3.2.3p3), so `!&g`,
//! `&g ? 1 : 2`, `&g != 0` and `&g && 1` have values no link can change, and a
//! static `_Bool` can hold one (§6.3.1.2p1) without a relocation.
//!
//! C99 leaves these as other forms of constant expression (§6.6p10). GCC and
//! Clang fold them, under the same `--compiler-extensions` that
//! `exceptional_constants.rs` uses, with these differences, measured on GCC
//! 14.2 and Clang 23.1:
//!
//! | | GCC | Clang |
//! | --- | --- | --- |
//! | A weak entity | known non-null if this unit defines it, except to `==` | never known |
//! | `_Bool b = E;` | `&g`, an array or function, `a + 0`, through pointer casts | any address, string or offset |
//! | `(long)&g != 0`, `&g > 0` | folds | no constant |
//!
//! The address of an automatic or thread-local object counts too: only its
//! truth is read, never its value.
//!
//! C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55; §6.3.2.3 paragraph 3, p. 47;
//! PDF p. 59; §6.5.3.3 paragraph 5, p. 79; PDF p. 91; §6.5.8-§6.5.9, pp. 86-87;
//! PDF pp. 98-99; §6.5.13-§6.5.15, pp. 89-91; PDF pp. 101-103; §6.6 paragraph
//! 10, p. 96; PDF p. 108.

use super::{
    Analyzer,
    ExpressionInfo,
    Integer,
    expressions::AddressBase,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    configuration::RuleSet,
    translation_phases::parsing::syntax::{
        BinaryOperator,
        Expression,
        ExpressionType,
        UnaryOperator,
    },
};

/// How deep the folding follows operands, which it does by recursion, as the
/// condition evaluator in `emission/folding.rs` does. Deeper expressions stay
/// unfolded.
const DEPTH: u32 = 64;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The value of the integer expression `e` that semantic analysis found
    /// no constant for, if an address's truth decides it.
    /// C99: §6.6 paragraph 10, p. 96; PDF p. 108.
    pub(super) fn folded_address_value(
        &mut self,
        e: &'tu Expression<'tu>,
        depth: u32,
    ) -> Option<Integer> {
        use BinaryOperator as B;
        use ExpressionType as E;
        use UnaryOperator as U;
        let info = self.expression_info(e);
        if let Some(value) = info.integer.filter(|_| !self.addresslike(info)) {
            return Some(value);
        }
        if depth > DEPTH {
            return None;
        }
        let depth = depth + 1;
        let (bits, signed) = self.integer_type(info.ty)?;
        let value = match e.kind {
            | E::Parenthesized { expression }
            | E::Unary {
                operator: U::Extension | U::Plus,
                operand_expression: expression,
            } => self.folded_address_value(expression, depth)?,
            | E::Unary {
                operator: operator @ (U::Minus | U::BitwiseNot),
                operand_expression,
            } => {
                let value = self.folded_address_value(operand_expression, depth)?;
                self.wide.unary(value, operator)?
            },
            | E::Unary {
                operator: U::LogicalNot,
                operand_expression,
            } => Integer::int(i128::from(
                !self.constant_truth_of(operand_expression, depth)?,
            )),
            | E::Binary {
                operator: operator @ (B::LogicalAnd | B::LogicalOr),
                left_expression,
                right_expression,
            } => {
                let left = self.constant_truth_of(left_expression, depth)?;
                Integer::int(i128::from(if left == (operator == B::LogicalOr) {
                    left
                } else {
                    self.constant_truth_of(right_expression, depth)?
                }))
            },
            | E::Binary {
                operator:
                    operator @ (B::Equal
                    | B::NotEqual
                    | B::LessThan
                    | B::LessThanOrEqual
                    | B::GreaterThan
                    | B::GreaterThanOrEqual),
                left_expression,
                right_expression,
            } => {
                if let Some(result) =
                    self.address_comparison(operator, left_expression, right_expression)
                {
                    Integer::int(i128::from(result))
                } else {
                    let left = self.folded_address_value(left_expression, depth)?;
                    let right = self.folded_address_value(right_expression, depth)?;
                    self.wide.binary(left, operator, right)?
                }
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } if !matches!(operator, B::Comma | B::Subscript) => {
                let left = self.folded_address_value(left_expression, depth)?;
                let right = self.folded_address_value(right_expression, depth)?;
                self.wide.binary(left, operator, right)?
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                let selected = if self.constant_truth_of(c.condition_expression, depth)? {
                    c.then_expression
                } else {
                    c.else_expression
                };
                self.folded_address_value(selected, depth)?
            },
            | E::Cast {
                operand_expression, ..
            } if matches!(
                self.types.nodes[self.types.non_atomic(info.ty).index],
                TypeKind::Scalar(Scalar::Bool)
            ) =>
                Integer::int(i128::from(
                    self.constant_truth_of(operand_expression, depth)?,
                )),
            | E::Cast {
                operand_expression, ..
            } => self.folded_address_value(operand_expression, depth)?,
            | _ => return None,
        };
        Some(self.wide.convert(value, bits, signed))
    }

    /// The value a constant context reads from `e`, whose result `info` has
    /// no integer. Clang's `_Static_assert` also reads an address's truth.
    /// C11: §6.7.10 paragraph 3, p. 145; PDF p. 163.
    pub(super) fn address_truth_constant(
        &mut self,
        e: &'tu Expression<'tu>,
        info: ExpressionInfo<'tu>,
    ) -> Option<Integer> {
        if self.integer_type(info.ty).is_some() {
            return self.folded_address_value(e, 0);
        }
        (self.address_rules() == RuleSet::Clang
            && self.constant_site == super::exceptional_constants::ConstantSite::StaticAssertion
            && self.pointer_like(info))
        .then(|| self.constant_truth_of(e, 0))
        .flatten()
        .map(|truth| Integer::int(i128::from(truth)))
    }

    /// The truth of `e`, a scalar, when it is known: a constant's, or an
    /// address's that no link can make null.
    /// C99: §6.3.2.3 paragraph 3, p. 47; PDF p. 59.
    pub(super) fn constant_truth_of(
        &mut self,
        e: &'tu Expression<'tu>,
        depth: u32,
    ) -> Option<bool> {
        let info = self.expression_info(e);
        if !self.addresslike(info)
            && let Some(truth) = Self::constant_truth(info)
        {
            return Some(truth);
        }
        let mut stripped = e;
        while let ExpressionType::Parenthesized { expression } = stripped.kind {
            stripped = expression;
        }
        if matches!(stripped.kind, ExpressionType::LabelAddress(_)) {
            return Some(true);
        }
        if self.addresslike(info) {
            return match self.address_parts(e, false)? {
                | (AddressBase::Absolute, value) => Some(value != 0),
                | (AddressBase::String(_), _) => Some(true),
                | (AddressBase::Binding(binding), _) =>
                    self.never_null(binding, false).then_some(true),
            };
        }
        self.folded_address_value(e, depth)
            .map(|value| !value.is_zero())
    }

    /// The result of comparing two addresses, or an address and null: two
    /// entities are distinct and one has its offsets in order. Clang's
    /// evaluator takes neither an address converted to an integer nor an
    /// order against null.
    /// C99: §6.5.8 paragraph 5, p. 86; PDF p. 98; §6.5.9 paragraph 6, p. 87;
    /// PDF p. 99.
    fn address_comparison(
        &mut self,
        operator: BinaryOperator,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> Option<bool> {
        use BinaryOperator as B;
        let gcc = self.address_rules() == RuleSet::Gcc;
        let (left_info, right_info) = (self.expression_info(left), self.expression_info(right));
        let compared = |this: &Self, info: ExpressionInfo<'tu>| {
            gcc || this.pointer_like(info) || info.integer.is_some_and(Integer::is_zero)
        };
        if !compared(self, left_info) || !compared(self, right_info) {
            return None;
        }
        let parts = (
            self.comparison_operand(left, left_info),
            self.comparison_operand(right, right_info),
        );
        let (order, equality_only) = match parts {
            | (Some((AddressBase::Binding(a), x)), Some((AddressBase::Binding(b), y))) => {
                if !self.never_null(a, true) || !self.never_null(b, true) {
                    return None;
                }
                if self.entity_of(a) == self.entity_of(b) {
                    (x.cmp(&y), false)
                } else {
                    (std::cmp::Ordering::Greater, true)
                }
            },
            | (Some((AddressBase::Binding(a), _)), Some((AddressBase::Absolute, 0))) => {
                if !self.never_null(a, true) {
                    return None;
                }
                (std::cmp::Ordering::Greater, !gcc)
            },
            | (Some((AddressBase::Absolute, 0)), Some((AddressBase::Binding(a), _))) => {
                if !self.never_null(a, true) {
                    return None;
                }
                (std::cmp::Ordering::Less, !gcc)
            },
            | (Some((AddressBase::String(_), _)), Some((AddressBase::Absolute, 0))) =>
                (std::cmp::Ordering::Greater, true),
            | (Some((AddressBase::Absolute, 0)), Some((AddressBase::String(_), _))) =>
                (std::cmp::Ordering::Less, true),
            | _ => return None,
        };
        match operator {
            | B::Equal => Some(order.is_eq()),
            | B::NotEqual => Some(order.is_ne()),
            | _ if equality_only => None,
            | B::LessThan => Some(order.is_lt()),
            | B::LessThanOrEqual => Some(order.is_le()),
            | B::GreaterThan => Some(order.is_gt()),
            | B::GreaterThanOrEqual => Some(order.is_ge()),
            | _ => None,
        }
    }

    /// What a comparison reads of `e`: an address, or the null an integer
    /// constant zero is.
    fn comparison_operand(
        &self,
        e: &'tu Expression<'tu>,
        info: ExpressionInfo<'tu>,
    ) -> Option<(AddressBase<'tu>, i128)> {
        if !self.addresslike(info) && info.integer.is_some_and(Integer::is_zero) {
            return Some((AddressBase::Absolute, 0));
        }
        self.address_parts(e, false)
    }

    /// Whether the entity of `binding` has an address known not to be null.
    /// GCC trusts a weak entity this unit defines, except in a comparison;
    /// Clang never does.
    /// GNU extension: GCC manual, "Common Variable Attributes", `weak`.
    fn never_null(&mut self, binding: usize, comparison: bool) -> bool {
        if !self.may_be_weak(binding) {
            return true;
        }
        self.address_rules() == RuleSet::Gcc
            && !comparison
            && self.defined_in_unit(self.bindings[binding].name.name)
    }

    /// Whether the static `_Bool` initializer `e`, an address, is one its
    /// compiler converts. GCC takes the entity itself, with no offset,
    /// through pointer casts: `&g`, an array or function, `a + 0`, but not
    /// `&a[0]` or `&s.m`.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    pub(super) fn boolean_address_initializer(&mut self, e: &'tu Expression<'tu>) -> bool {
        if matches!(
            self.stripped_pointer_cast(e).kind,
            ExpressionType::LabelAddress(_)
        ) {
            return true;
        }
        let Some(parts) = self.address_parts(e, false) else {
            return self.constant_truth_of(e, 0).is_some();
        };
        match (self.address_rules(), parts) {
            | (_, (AddressBase::Absolute, _)) | (RuleSet::Clang, (AddressBase::String(_), _)) =>
                true,
            | (RuleSet::Gcc, (AddressBase::String(_), _)) => false,
            | (RuleSet::Clang, (AddressBase::Binding(binding), _)) =>
                self.never_null(binding, false),
            | (RuleSet::Gcc, (AddressBase::Binding(binding), offset)) =>
                offset == 0 && self.whole_entity_address(e) && self.never_null(binding, false),
        }
    }

    /// Whether `e`, through parentheses and pointer casts, names a whole
    /// entity: `&g`, an array or function, or `a + 0`.
    fn whole_entity_address(&self, e: &'tu Expression<'tu>) -> bool {
        use ExpressionType as E;
        let e = self.stripped_pointer_cast(e);
        match e.kind {
            | E::Identifier(_) => true,
            | E::Unary {
                operator: UnaryOperator::AddressOf,
                operand_expression,
            } => {
                let mut operand = operand_expression;
                while let E::Parenthesized { expression } = operand.kind {
                    operand = expression;
                }
                matches!(operand.kind, E::Identifier(_))
            },
            | E::Binary {
                operator: BinaryOperator::Addition | BinaryOperator::Subtraction,
                left_expression,
                ..
            } => self.whole_entity_address(left_expression),
            | _ => false,
        }
    }

    /// `e` without the parentheses and the casts to pointer types around it.
    fn stripped_pointer_cast(&self, mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            match e.kind {
                | ExpressionType::Parenthesized { expression } => e = expression,
                | ExpressionType::Cast {
                    operand_expression, ..
                } if self.pointer_like(self.expression_info(e)) => e = operand_expression,
                | _ => return e,
            }
        }
    }

    /// Whether a value of `info`'s type is an address by its type.
    pub(super) fn pointer_like(&self, info: ExpressionInfo<'tu>) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(info.ty).index],
            TypeKind::Pointer(_) | TypeKind::Array(..) | TypeKind::Function { .. }
        )
    }

    /// Whether `info` reads as an address, even converted to an integer, so
    /// that a value it holds is no plain constant.
    fn addresslike(&self, info: ExpressionInfo<'tu>) -> bool {
        self.pointer_like(info)
            || (info.constant == super::ConstantClass::Address && info.integer.is_none())
    }

    fn address_rules(&self) -> RuleSet {
        self.context.configuration.compiler_identity().rules()
    }
}
