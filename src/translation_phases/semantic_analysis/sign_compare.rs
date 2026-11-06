//! GCC's and Clang's `-Wsign-compare`: a comparison whose usual arithmetic
//! conversions turn a signed operand into an unsigned type, unless that
//! operand cannot be negative. It is a warning C99 §5.1.1.3p1 permits about
//! conversions §6.3.1.8p1 defines. The compilers decide that differently, so
//! each has its own reading: GCC asks whether the folded tree is nonnegative
//! ([`Analyzer::cannot_be_negative`]), and Clang computes a value range
//! ([`Analyzer::clang_range`]). Neither looks at an operand that is not
//! evaluated. GCC words the same conversion in a conditional expression's
//! operand.
//! C99: §6.5.8p3, p. 86; PDF p. 98; §6.5.9p4, p. 87; PDF p. 99;
//! §6.5.15p5, p. 90; PDF p. 102.

use super::{
    Analyzer,
    ExpressionInfo,
    Integer,
    SemanticErrorKind,
    types::{
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticFormat,
        DiagnosticGroup,
    },
    diagnostics::format_in,
    translation_phases::parsing::syntax::{
        BinaryOperator as B,
        ConditionalExpression,
        Expression,
        ExpressionType as E,
        UnaryOperator as U,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the binary expression `e`, if `-Wsign-compare` is on and it
    /// compares.
    /// C99: §6.5.8p3, p. 86; PDF p. 98; §6.5.9p4, p. 87; PDF p. 99.
    pub(super) fn check_compared_signs(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        if self.unused.sign_compare
            && matches!(
                operator,
                B::LessThan
                    | B::LessThanOrEqual
                    | B::GreaterThan
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual
            )
            && !self.evaluation_inhibited()
        {
            self.check_sign_compare(
                e,
                operator,
                self.expression_info(left),
                self.expression_info(right),
            );
        }
    }

    /// Checks the comparison `e` of `left` and `right`.
    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57.
    fn check_sign_compare(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: B,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) {
        let Some(left_signed) = self.converts_sign(left, right) else {
            return;
        };
        let (signed, unsigned) = if left_signed {
            (left, right)
        } else {
            (right, left)
        };
        let compared = self.promoted(unsigned);
        let Some((bits, _)) = self.integer_type(compared) else {
            return;
        };
        let comparison = Comparison {
            operator,
            left,
            right,
            signed,
            unsigned,
            bits,
        };
        let quiet = if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            self.gcc_comparison_is_quiet(comparison)
        } else {
            self.clang_comparison_is_quiet(comparison)
        };
        if quiet {
            return;
        }
        let Some(name) = self.sign_compare_wording(left.ty, right.ty) else {
            return;
        };
        self.spelled_group_diagnostic(
            DiagnosticGroup::SignCompare,
            SemanticErrorKind::SignCompare,
            e.operator_source_vectors.unwrap_or(e.source_vectors),
            Some(name),
        );
    }

    /// GCC's `warn_for_sign_compare`, and `shorten_compare`, which settles a
    /// relational comparison of two constants or of an unsigned value with
    /// zero before it gets there.
    fn gcc_comparison_is_quiet(&self, comparison: Comparison<'tu>) -> bool {
        let Comparison {
            operator,
            left,
            right,
            signed,
            unsigned,
            bits,
        } = comparison;
        let equality = matches!(operator, B::Equal | B::NotEqual);
        if !equality && left.ice && right.ice {
            return true;
        }
        let zero =
            |info: ExpressionInfo<'tu>| info.ice && info.integer.is_some_and(Integer::is_zero);
        if matches!(operator, B::LessThan | B::GreaterThanOrEqual) && zero(right)
            || matches!(operator, B::GreaterThan | B::LessThanOrEqual) && zero(left)
        {
            return true;
        }
        if self.gcc_comparison_is_shortened(comparison) {
            return true;
        }
        if self.cannot_be_negative(signed.expression) {
            return true;
        }
        // An unsigned constant that fits the signed type compares equal or
        // not whichever way the conversion goes.
        equality
            && unsigned.ice
            && unsigned
                .integer
                .and_then(Integer::to_i128)
                .is_some_and(|value| value >= 0 && (bits >= 128 || value < 1i128 << (bits - 1)))
    }

    /// Whether `shorten_compare` settles a comparison of a constant with an
    /// operand widened from a narrower type: GCC reads the operand through
    /// its widening casts, and a value outside that type's range decides the
    /// result. For a signed operand it decides only an equality, and the
    /// constant counts as signed.
    fn gcc_comparison_is_shortened(&self, comparison: Comparison<'tu>) -> bool {
        let Comparison {
            operator,
            left,
            right,
            bits,
            ..
        } = comparison;
        let (constant, operand, operator) = match (left.ice, right.ice) {
            | (false, true) => (right, left, operator),
            | (true, false) => (
                left,
                right,
                match operator {
                    | B::LessThan => B::GreaterThan,
                    | B::GreaterThan => B::LessThan,
                    | B::LessThanOrEqual => B::GreaterThanOrEqual,
                    | B::GreaterThanOrEqual => B::LessThanOrEqual,
                    | other => other,
                },
            ),
            | _ => return false,
        };
        let Some(value) = constant.integer.and_then(Integer::to_i128) else {
            return false;
        };
        let Some((narrow, signed)) = self.narrowest_source(operand.expression) else {
            return false;
        };
        if narrow >= bits || bits > MAX_MODELED_BITS {
            return false;
        }
        let value = truncated(value, bits);
        let equality = matches!(operator, B::Equal | B::NotEqual);
        if signed {
            // Sign-extend from `bits`.
            let unused = 128 - bits;
            let value = (value << unused).cast_signed() >> unused;
            return equality && !(-(1 << (narrow - 1))..1 << (narrow - 1)).contains(&value);
        }
        let maximum = truncated(-1, narrow);
        match operator {
            | B::LessThan | B::GreaterThanOrEqual => value == 0 || value > maximum,
            | B::GreaterThan | B::LessThanOrEqual => value >= maximum,
            | _ => value > maximum,
        }
    }

    /// The bits and signedness of `e` through its widening casts.
    fn narrowest_source(&self, e: &'tu Expression<'tu>) -> Option<(u32, bool)> {
        let e = Self::without_parentheses(e);
        let found = self.integer_type(self.expression_info(e).ty)?;
        if let E::Cast {
            operand_expression, ..
        } = e.kind
            && let Some(source) = self.narrowest_source(operand_expression)
            && source.0 < found.0
        {
            return Some(source);
        }
        Some(found)
    }

    /// Clang's `AnalyzeComparison`, which ranges both operands, leaves alone
    /// a comparison whose result is a constant, and warns through
    /// `DiagRuntimeBehavior`, so never in code it can tell is unreachable.
    fn clang_comparison_is_quiet(&mut self, comparison: Comparison<'tu>) -> bool {
        let Comparison {
            operator,
            left,
            right,
            signed,
            unsigned,
            bits,
        } = comparison;
        if self.functions.dead || (left.ice && right.ice) {
            return true;
        }
        if left.ice != right.ice
            && self.clang_tautology(
                operator,
                if left.ice { left } else { right },
                left.ice,
                if left.ice { right } else { left },
                bits,
            )
        {
            return true;
        }
        // An `_Atomic` signed type does not have signed integer
        // representation, so there is nothing for Clang to convert.
        if matches!(self.types.nodes[signed.ty.index], TypeKind::Atomic(_))
            || self.clang_range(signed.expression, u32::MAX).non_negative
        {
            return true;
        }
        // A narrow unsigned value cannot collide with a converted negative
        // one.
        matches!(operator, B::Equal | B::NotEqual)
            && self.clang_range(unsigned.expression, u32::MAX).width < bits
    }

    /// Clang's `CheckTautologicalComparison`: whether every value of `other`
    /// compares the same way with `constant`, which another option reports.
    /// An in-range constant from a macro or an enumerator does not count.
    fn clang_tautology(
        &mut self,
        operator: B,
        constant: ExpressionInfo<'tu>,
        constant_on_left: bool,
        other: ExpressionInfo<'tu>,
        bits: u32,
    ) -> bool {
        let Some(value) = constant
            .integer
            .and_then(Integer::to_i128)
            .filter(|_| bits <= MAX_MODELED_BITS)
        else {
            return false;
        };
        let value = truncated(value, bits);
        let range = self.clang_range(other.expression, u32::MAX);
        let order = range.compare(value, bits);
        if order.constant_result(operator, constant_on_left).is_none() {
            return false;
        }
        let type_range = self.clang_type_range(other.ty);
        let by_type = type_range
            .compare(value, bits)
            .constant_result(operator, constant_on_left)
            .is_some();
        if !by_type && range.width == 0 {
            return false;
        }
        !(order.in_range
            && (matches!(
                Self::without_parentheses(constant.expression).kind,
                E::Identifier(_)
            ) || self
                .context
                .written_by_macro_body(constant.expression.source_vectors)))
    }

    /// Clang's `TryGetExprRange`, with its approximation of sums: the fewest
    /// bits that hold `e`'s value, at most `limit`, and whether it can be
    /// negative. An implicit conversion narrows nothing but passes on a
    /// narrower operand; an explicit one is as wide as its type.
    fn clang_range(&mut self, e: &'tu Expression<'tu>, limit: u32) -> IntRange {
        let e = Self::without_parentheses(e);
        if let Some(value) = self.folded_integer(e).and_then(Integer::to_i128) {
            return IntRange::of_value(value, limit);
        }
        let info = self.expression_info(e);
        let whole = self.clang_type_range(info.ty);
        match e.kind {
            | E::Conditional(c) => {
                if let Some(taken) = self.folded_condition(c.condition_expression) {
                    let arm = if taken {
                        c.then_expression
                    } else {
                        c.else_expression
                    };
                    return self.clang_converted_range(arm, info.ty, limit);
                }
                let then = self.clang_converted_range(c.then_expression, info.ty, limit);
                let other = self.clang_converted_range(c.else_expression, info.ty, limit);
                IntRange::join(then, other)
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => self.clang_binary_range(
                info.ty,
                whole,
                operator,
                left_expression,
                right_expression,
                limit,
            ),
            | E::Unary {
                operator,
                operand_expression,
            } => match operator {
                | U::LogicalNot => IntRange::BOOL,
                | U::Indirection | U::AddressOf => whole,
                | U::Minus | U::BitwiseNot if whole.non_negative =>
                    self.clang_converted_range(operand_expression, info.ty, limit),
                | U::Minus => {
                    let operand = self.clang_converted_range(operand_expression, info.ty, limit);
                    IntRange {
                        width:        operand.width.saturating_add(1).min(limit),
                        non_negative: false,
                    }
                },
                | U::BitwiseNot => {
                    let operand = self.clang_converted_range(operand_expression, info.ty, limit);
                    IntRange {
                        width:        (operand.width + u32::from(operand.non_negative)).min(limit),
                        non_negative: false,
                    }
                },
                | _ => self.clang_range(operand_expression, limit),
            },
            | _ => info.bit_field.map_or(whole, |width| IntRange {
                width,
                non_negative: whole.non_negative,
            }),
        }
    }

    /// The binary half of [`Self::clang_range`], in a type of range `whole`.
    fn clang_binary_range(
        &mut self,
        ty: TypeId,
        whole: IntRange,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
        limit: u32,
    ) -> IntRange {
        let width = whole.width;
        let combine: fn(IntRange, IntRange) -> IntRange = match operator {
            | B::LogicalAnd
            | B::LogicalOr
            | B::LessThan
            | B::LessThanOrEqual
            | B::GreaterThan
            | B::GreaterThanOrEqual
            | B::Equal
            | B::NotEqual => return IntRange::BOOL,
            | B::Assignment => return self.clang_converted_range(right, ty, limit),
            | B::Comma => return self.clang_range(right, limit),
            | B::RightShift | B::RightShiftAssignment => {
                let mut range = self.clang_converted_range(left, ty, limit);
                let shift = self.expression_info(right);
                if let Some(shift) = shift
                    .integer
                    .filter(|_| shift.ice)
                    .and_then(Integer::to_i128)
                    && shift >= 0
                {
                    range.narrow_by(shift);
                }
                return range;
            },
            | B::LeftShift => {
                let one = matches!(Self::without_casts(left).kind, E::Constant(_))
                    && self
                        .expression_info(Self::without_casts(left))
                        .integer
                        .and_then(Integer::to_i128)
                        == Some(1);
                return IntRange {
                    width,
                    non_negative: one || whole.non_negative,
                };
            },
            | B::Division | B::DivisionAssignment => {
                let mut range = self.clang_converted_range(left, ty, width);
                let divisor = self.expression_info(right);
                if let Some(divisor) = divisor
                    .integer
                    .filter(|_| divisor.ice)
                    .and_then(Integer::to_i128)
                    .filter(|&divisor| divisor > 0)
                {
                    range.narrow_by(i128::from(divisor.ilog2()));
                    range.width = range.width.min(limit);
                    return range;
                }
                let other = self.clang_converted_range(right, ty, width);
                return IntRange {
                    width:        range.width,
                    non_negative: range.non_negative && other.non_negative,
                };
            },
            | B::BitwiseAnd | B::BitwiseAndAssignment => IntRange::bit_and,
            | B::Modulo => IntRange::rem,
            | B::Addition | B::Multiplication | B::BitwiseOr | B::BitwiseXor => IntRange::join,
            | B::Subtraction
                if !matches!(
                    self.types.nodes[self.expression_info(left).ty.index],
                    TypeKind::Pointer(_) | TypeKind::Array(..)
                ) =>
                IntRange::join,
            | _ => return whole,
        };
        let left = self.clang_converted_range(left, ty, width);
        let right = self.clang_converted_range(right, ty, width);
        let combined = combine(left, right);
        IntRange {
            width:        combined.width.min(limit),
            non_negative: combined.non_negative || whole.non_negative,
        }
    }

    /// [`Self::clang_range`] of `e` converted implicitly to `ty`, as the
    /// usual arithmetic conversions leave an operand.
    fn clang_converted_range(
        &mut self,
        e: &'tu Expression<'tu>,
        ty: TypeId,
        limit: u32,
    ) -> IntRange {
        let output = self.clang_type_range(ty);
        let operand = self.clang_range(e, limit.min(output.width));
        if operand.width >= output.width {
            output
        } else {
            IntRange {
                width:        operand.width,
                non_negative: operand.non_negative || output.non_negative,
            }
        }
    }

    /// Clang's `IntRange::forValueOfType`; any other type has no useful range.
    fn clang_type_range(&self, ty: TypeId) -> IntRange {
        self.integer_type(ty)
            .map_or(IntRange::UNKNOWN, |(width, signed)| IntRange {
                width,
                non_negative: !signed,
            })
    }

    fn without_casts(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Cast {
                    operand_expression, ..
                } => e = operand_expression,
                | _ => return e,
            }
        }
    }

    /// GCC's words for a conditional whose signed operand converts to the
    /// other's unsigned type.
    /// C99: §6.5.15p5, p. 90; PDF p. 102.
    pub(super) fn check_conditional_sign(
        &mut self,
        then_value: &'tu Expression<'tu>,
        else_value: &'tu Expression<'tu>,
    ) {
        if self.context.configuration.compiler_identity() != CompilerIdentity::Gcc
            || self.evaluation_inhibited()
        {
            return;
        }
        let (left, right) = (
            self.expression_info(then_value),
            self.expression_info(else_value),
        );
        let Some(left_signed) = self.converts_sign(left, right) else {
            return;
        };
        let (signed, unsigned) = if left_signed {
            (left, right)
        } else {
            (right, left)
        };
        if self.cannot_be_negative(signed.expression) {
            return;
        }
        let unsigned_type = self.promoted(unsigned).unqualified();
        let from = self.type_spelling(signed.ty.unqualified(), true);
        let to = self.type_spelling(unsigned_type, true);
        if let (Some(from), Some(to)) = (from, to) {
            let text = format_in!(
                self.scratch,
                "operand of '?:' changes signedness from '{from}' to '{to}' due to unsignedness \
                 of other operand"
            );
            let name = self.context.string_cache.intern(text);
            self.spelled_group_diagnostic(
                DiagnosticGroup::SignCompare,
                SemanticErrorKind::SignCompare,
                signed.expression.source_vectors,
                Some(name),
            );
        }
    }

    /// Whether the usual arithmetic conversions of two integer operands make
    /// a signed one unsigned, and if so whether that one is `left`.
    fn converts_sign(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> Option<bool> {
        let (left_type, right_type) = (self.promoted(left), self.promoted(right));
        let (left_bits, left_signed) = self.integer_type(left_type)?;
        let (right_bits, right_signed) = self.integer_type(right_type)?;
        if left_signed == right_signed {
            return None;
        }
        // The wider signed type holds every value of the unsigned one.
        let signed_bits = if left_signed { left_bits } else { right_bits };
        let unsigned_bits = if left_signed { right_bits } else { left_bits };
        (signed_bits <= unsigned_bits).then_some(left_signed)
    }

    fn promoted(&mut self, info: ExpressionInfo<'tu>) -> TypeId {
        self.promote(info, info.ty.unqualified())
    }

    /// GCC's `tree_expr_nonnegative_warnv_p` after `fold_for_warn`: whether
    /// `e`'s value is known not to be negative. A clamp folds to `MAX_EXPR`,
    /// and a mask moves through a cast or a shift. Unknown forms answer false.
    /// C99: §6.5.3.3p5, p. 79; PDF p. 91; §6.5.8p6, p. 86; PDF p. 98;
    /// §6.5.9p3, p. 87; PDF p. 99; §6.5.10-§6.5.14, pp. 87-89; PDF pp. 99-101.
    fn cannot_be_negative(&self, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        if info.ice {
            return info
                .integer
                .and_then(Integer::to_i128)
                .is_some_and(|value| value >= 0);
        }
        if self
            .integer_type(info.ty)
            .is_some_and(|(_, signed)| !signed)
        {
            return true;
        }
        match e.kind {
            | E::Parenthesized { expression } => self.cannot_be_negative(expression),
            | E::Unary {
                operator: U::LogicalNot,
                ..
            } => true,
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => self.binary_cannot_be_negative(operator, left_expression, right_expression),
            | E::Conditional(c) =>
                self.clamp_cannot_be_negative(c)
                    || (self.cannot_be_negative(c.then_expression)
                        && self.cannot_be_negative(c.else_expression)),
            | E::Cast {
                operand_expression, ..
            } => self.cast_cannot_be_negative(info.ty, operand_expression),
            | E::Call {
                function_expression,
                ..
            } => self.call_cannot_be_negative(function_expression),
            | E::StatementExpression(block) => self
                .statement_expression_tail(block)
                .is_some_and(|(_, tail)| self.cannot_be_negative(tail)),
            | _ => false,
        }
    }

    fn binary_cannot_be_negative(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> bool {
        match operator {
            | B::LessThan
            | B::LessThanOrEqual
            | B::GreaterThan
            | B::GreaterThanOrEqual
            | B::Equal
            | B::NotEqual
            | B::LogicalAnd
            | B::LogicalOr => true,
            | B::BitwiseAnd => self.cannot_be_negative(left) || self.cannot_be_negative(right),
            | B::Multiplication if self.same_operand(left, right) => true,
            | B::BitwiseOr | B::BitwiseXor | B::Addition | B::Multiplication | B::Division =>
                self.cannot_be_negative(left) && self.cannot_be_negative(right),
            | B::Modulo => self.cannot_be_negative(left),
            | B::RightShift => self.expression_info(right).ice && self.cannot_be_negative(left),
            // `(a & m) << n` folds to `(a << n) & (m << n)`.
            | B::LeftShift => {
                let bits = self.integer_type(self.expression_info(left).ty);
                let shift = self.expression_info(right);
                match (
                    self.positive_mask(left),
                    shift
                        .integer
                        .filter(|_| shift.ice)
                        .and_then(Integer::to_i128),
                    bits,
                ) {
                    | (Some(mask), Some(shift), Some((bits, _))) =>
                        (0..i128::from(bits)).contains(&shift)
                            && mask
                                .checked_shl(u32::try_from(shift).unwrap_or(u32::MAX))
                                .is_some_and(|value| bits >= 128 || value < 1i128 << (bits - 1)),
                    | _ => false,
                }
            },
            | B::Comma | B::Assignment => self.cannot_be_negative(right),
            | _ => false,
        }
    }

    /// The nonnegative constant `e` masks with, if it is such a bitwise and.
    fn positive_mask(&self, e: &'tu Expression<'tu>) -> Option<i128> {
        let e = Self::without_parentheses(e);
        let E::Binary {
            operator: B::BitwiseAnd,
            left_expression,
            right_expression,
        } = e.kind
        else {
            return None;
        };
        [left_expression, right_expression]
            .into_iter()
            .find_map(|operand| {
                let operand = self.expression_info(operand);
                operand
                    .integer
                    .filter(|_| operand.ice)
                    .and_then(Integer::to_i128)
                    .filter(|&value| value >= 0)
            })
    }

    /// A conditional GCC folds to `MAX_EXPR`, which either nonnegative
    /// operand bounds, to `MIN_EXPR`, which needs both, or to `ABS_EXPR`.
    fn clamp_cannot_be_negative(&self, c: &'tu ConditionalExpression<'tu>) -> bool {
        let E::Binary {
            operator,
            left_expression: left,
            right_expression: right,
        } = Self::without_parentheses(c.condition_expression).kind
        else {
            return false;
        };
        let less = match operator {
            | B::LessThan | B::LessThanOrEqual => true,
            | B::GreaterThan | B::GreaterThanOrEqual => false,
            | _ => return false,
        };
        let (then_value, else_value) = (c.then_expression, c.else_expression);
        if self.constant_clamp_cannot_be_negative(operator, left, right, then_value, else_value) {
            return true;
        }
        let larger = if self.same_operand(left, then_value) && self.same_operand(right, else_value)
        {
            !less
        } else if self.same_operand(left, else_value) && self.same_operand(right, then_value) {
            less
        } else {
            let zero = |e: &'tu Expression<'tu>| {
                let info = self.expression_info(e);
                info.ice && info.integer.is_some_and(Integer::is_zero)
            };
            let (value, value_is_less) = if zero(right) {
                (left, less)
            } else if zero(left) {
                (right, !less)
            } else {
                return false;
            };
            // `a < 0 ? -a : a` and `a > 0 ? a : -a`.
            let negated = |e: &'tu Expression<'tu>| {
                matches!(Self::without_parentheses(e).kind, E::Unary {
                    operator: U::Minus,
                    operand_expression,
                } if self.same_operand(operand_expression, value))
            };
            return if value_is_less {
                negated(then_value) && self.same_operand(else_value, value)
            } else {
                negated(else_value) && self.same_operand(then_value, value)
            };
        };
        if larger {
            self.cannot_be_negative(left) || self.cannot_be_negative(right)
        } else {
            self.cannot_be_negative(left) && self.cannot_be_negative(right)
        }
    }

    /// `a < 1 ? 0 : a` and its like, whose bound is one off the constant the
    /// comparison names, which GCC also folds to `MAX_EXPR`.
    fn constant_clamp_cannot_be_negative(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
        then_value: &'tu Expression<'tu>,
        else_value: &'tu Expression<'tu>,
    ) -> bool {
        let constant = |e: &'tu Expression<'tu>| {
            let info = self.expression_info(e);
            info.integer.filter(|_| info.ice).and_then(Integer::to_i128)
        };
        let (value, compared, operator) = match (constant(left), constant(right)) {
            | (None, Some(compared)) => (left, compared, operator),
            | (Some(compared), None) => (
                right,
                compared,
                match operator {
                    | B::LessThan => B::GreaterThan,
                    | B::GreaterThan => B::LessThan,
                    | B::LessThanOrEqual => B::GreaterThanOrEqual,
                    | _ => B::LessThanOrEqual,
                },
            ),
            | _ => return false,
        };
        // The bound is what the constant arm gives for every value up to
        // `limit`.
        let (limit, bound) = match operator {
            | B::LessThan if self.same_operand(else_value, value) => (compared - 1, then_value),
            | B::LessThanOrEqual if self.same_operand(else_value, value) => (compared, then_value),
            | B::GreaterThan if self.same_operand(then_value, value) => (compared, else_value),
            | B::GreaterThanOrEqual if self.same_operand(then_value, value) =>
                (compared - 1, else_value),
            | _ => return false,
        };
        constant(bound).is_some_and(|bound| bound >= 0 && (bound == limit || bound == limit + 1))
    }

    /// GCC's `operand_equal_p`, for operands without side effects.
    fn same_operand(&self, a: &'tu Expression<'tu>, b: &'tu Expression<'tu>) -> bool {
        let (a, b) = (Self::without_parentheses(a), Self::without_parentheses(b));
        let (left, right) = (self.expression_info(a), self.expression_info(b));
        if left.ty.is_volatile() || right.ty.is_volatile() {
            return false;
        }
        if left.ice && right.ice {
            return left.integer.and_then(Integer::to_i128)
                == right.integer.and_then(Integer::to_i128);
        }
        match (a.kind, b.kind) {
            | (E::Identifier(_), E::Identifier(_)) =>
                left.binding.is_some() && left.binding == right.binding,
            | (
                E::Binary {
                    operator: first,
                    left_expression: first_left,
                    right_expression: first_right,
                },
                E::Binary {
                    operator: second,
                    left_expression: second_left,
                    right_expression: second_right,
                },
            ) =>
                first == second
                    && !Self::is_assignment(first)
                    && self.same_operand(first_left, second_left)
                    && self.same_operand(first_right, second_right),
            | (
                E::Unary {
                    operator: first,
                    operand_expression: first_operand,
                },
                E::Unary {
                    operator: second,
                    operand_expression: second_operand,
                },
            ) =>
                first == second
                    && matches!(
                        first,
                        U::Minus | U::Plus | U::BitwiseNot | U::LogicalNot | U::Indirection
                    )
                    && self.same_operand(first_operand, second_operand),
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
            | _ => false,
        }
    }

    /// A conversion keeps a value nonnegative if the type is wider than an
    /// unsigned source, or holds a nonnegative signed one; a mask that fits
    /// moves through a narrowing one.
    fn cast_cannot_be_negative(&self, ty: TypeId, operand: &'tu Expression<'tu>) -> bool {
        let source = self.expression_info(operand).ty;
        let (Some((to, _)), Some((from, signed))) =
            (self.integer_type(ty), self.integer_type(source))
        else {
            return false;
        };
        if signed {
            if to >= from && self.cannot_be_negative(operand) {
                return true;
            }
        } else if to > from {
            return true;
        }
        self.positive_mask(operand)
            .is_some_and(|mask| to >= 128 || mask < 1i128 << (to - 1))
    }

    /// GCC folds `abs` and knows the results of the bit-counting builtins.
    /// GNU extension: GCC manual, "Other Builtins".
    fn call_cannot_be_negative(&self, function: &'tu Expression<'tu>) -> bool {
        let E::Identifier(name) = Self::without_parentheses(function).kind else {
            return false;
        };
        let name = self.context.string_cache.at(name.name);
        matches!(name, "abs" | "labs" | "llabs" | "imaxabs")
            || name.strip_prefix("__builtin_").is_some_and(|builtin| {
                matches!(
                    builtin,
                    "abs"
                        | "labs"
                        | "llabs"
                        | "imaxabs"
                        | "ffs"
                        | "ffsl"
                        | "ffsll"
                        | "clz"
                        | "clzl"
                        | "clzll"
                        | "clrsb"
                        | "clrsbl"
                        | "clrsbll"
                        | "popcount"
                        | "popcountl"
                        | "popcountll"
                        | "parity"
                        | "parityl"
                        | "parityll"
                )
            })
    }

    /// The comparison's words with both operand types as each compiler spells
    /// them: GCC drops their qualifiers.
    fn sign_compare_wording(
        &mut self,
        left: TypeId,
        right: TypeId,
    ) -> Option<crate::util::string_cache::StringCacheId> {
        let gcc = self.context.configuration.diagnostic_format() == DiagnosticFormat::Gcc;
        let (left, right) = if gcc {
            (
                self.types.non_atomic(left).unqualified(),
                self.types.non_atomic(right).unqualified(),
            )
        } else {
            (left, right)
        };
        let left = self.type_spelling(left, gcc)?;
        let right = self.type_spelling(right, gcc)?;
        let text = if gcc {
            format_in!(
                self.scratch,
                "comparison of integer expressions of different signedness: '{left}' and '{right}'"
            )
        } else {
            format_in!(
                self.scratch,
                "comparison of integers of different signs: '{left}' and '{right}'"
            )
        };
        Some(self.context.string_cache.intern(text))
    }
}

/// One comparison [`Analyzer::check_sign_compare`] found to convert a signed
/// operand; `bits` is the width of the type it compares in.
#[derive(Clone, Copy)]
struct Comparison<'tu> {
    operator: B,
    left:     ExpressionInfo<'tu>,
    right:    ExpressionInfo<'tu>,
    signed:   ExpressionInfo<'tu>,
    unsigned: ExpressionInfo<'tu>,
    bits:     u32,
}

/// Clang's `IntRange`: a value fits `width` bits, a sign bit among them unless
/// it is `non_negative`.
#[derive(Clone, Copy)]
struct IntRange {
    width:        u32,
    non_negative: bool,
}

impl IntRange {
    const BOOL: Self = Self {
        width:        1,
        non_negative: true,
    };
    const UNKNOWN: Self = Self {
        width:        64,
        non_negative: false,
    };

    /// `GetValueRange`.
    fn of_value(value: i128, limit: u32) -> Self {
        if value < 0 {
            return Self {
                width:        129 - value.leading_ones(),
                non_negative: false,
            };
        }
        let value = truncated(value, limit);
        Self {
            width:        128 - value.leading_zeros(),
            non_negative: true,
        }
    }

    const fn value_bits(self) -> u32 {
        if self.non_negative {
            self.width
        } else {
            self.width.saturating_sub(1)
        }
    }

    fn join(left: Self, right: Self) -> Self {
        let non_negative = left.non_negative && right.non_negative;
        Self {
            width: left.value_bits().max(right.value_bits()) + u32::from(!non_negative),
            non_negative,
        }
    }

    fn bit_and(left: Self, right: Self) -> Self {
        let mut width = left.width.max(right.width);
        if left.non_negative {
            width = width.min(left.width);
        }
        if right.non_negative {
            width = width.min(right.width);
        }
        Self {
            width,
            non_negative: left.non_negative || right.non_negative,
        }
    }

    fn rem(left: Self, right: Self) -> Self {
        Self {
            width:        left.value_bits().min(right.value_bits()) + u32::from(!left.non_negative),
            non_negative: left.non_negative,
        }
    }

    /// Drops `amount` low bits, as a right shift or a division does.
    fn narrow_by(&mut self, amount: i128) {
        self.width = u32::try_from(amount)
            .ok()
            .filter(|&amount| amount < self.width)
            .map_or(u32::from(!self.non_negative), |amount| self.width - amount);
    }

    /// Clang's `PromotedRange::compare`: `value`, already in the comparison's
    /// unsigned type of `bits`, against this range converted to that type.
    fn compare(self, value: u128, bits: u32) -> ComparedValue {
        let top = truncated(-1, bits);
        let (minimum, maximum) = if self.width == 0 {
            (0, 0)
        } else if self.non_negative {
            (
                0,
                if self.width >= bits {
                    top
                } else {
                    (1 << self.width) - 1
                },
            )
        } else if self.width > bits {
            (0, top)
        } else {
            let half = 1u128 << (self.width - 1);
            (half.wrapping_neg() & top, half - 1)
        };
        let (below, above, in_range) = if minimum > maximum {
            // The range wraps around through the type's largest value.
            if value == 0 {
                (true, false, true)
            } else if value == top {
                (false, true, true)
            } else if value >= minimum || value <= maximum {
                (false, false, true)
            } else {
                return ComparedValue::NOT_EQUAL;
            }
        } else if value < minimum {
            return ComparedValue::LESS;
        } else if value > maximum {
            return ComparedValue::GREATER;
        } else {
            (value == minimum, value == maximum, true)
        };
        ComparedValue {
            less: false,
            below,
            greater: false,
            above,
            equal: below && above,
            not_equal: false,
            in_range,
        }
    }
}

/// What always holds of a constant against a range, as flags: Clang's
/// `PromotedRange::ComparisonResult`.
#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Clang's own flags, each an independent fact."
)]
struct ComparedValue {
    /// Below every value of the range.
    less:      bool,
    /// At most every value.
    below:     bool,
    /// Above every value.
    greater:   bool,
    /// At least every value.
    above:     bool,
    equal:     bool,
    not_equal: bool,
    in_range:  bool,
}

impl ComparedValue {
    const GREATER: Self = Self {
        less:      false,
        below:     false,
        greater:   true,
        above:     true,
        equal:     false,
        not_equal: true,
        in_range:  false,
    };
    const LESS: Self = Self {
        less:      true,
        below:     true,
        greater:   false,
        above:     false,
        equal:     false,
        not_equal: true,
        in_range:  false,
    };
    const NOT_EQUAL: Self = Self {
        less:      false,
        below:     false,
        greater:   false,
        above:     false,
        equal:     false,
        not_equal: true,
        in_range:  false,
    };

    /// `PromotedRange::constantValue`: the comparison's result, if these
    /// flags settle it.
    fn constant_result(self, operator: B, constant_on_left: bool) -> Option<bool> {
        let constant_on_right = !constant_on_left;
        let (holds, fails) = match operator {
            | B::Equal => (self.equal, self.not_equal),
            | B::NotEqual => (self.not_equal, self.equal),
            | _ => {
                let less_or_at_least =
                    matches!(operator, B::LessThan | B::GreaterThanOrEqual) != constant_on_right;
                let (holds, fails) = if less_or_at_least {
                    (self.less, self.above)
                } else {
                    (self.greater, self.below)
                };
                if matches!(operator, B::GreaterThanOrEqual | B::LessThanOrEqual) {
                    (fails, holds)
                } else {
                    (holds, fails)
                }
            },
        };
        if holds {
            Some(true)
        } else if fails {
            Some(false)
        } else {
            None
        }
    }
}

/// The widest comparison whose constants fit the range arithmetic; a wider
/// `_BitInt` has no constant result here.
const MAX_MODELED_BITS: u32 = 128;

/// `value` as an unsigned number of `bits`, at most all of them.
const fn truncated(value: i128, bits: u32) -> u128 {
    if bits >= MAX_MODELED_BITS {
        value.cast_unsigned()
    } else {
        value.cast_unsigned() & ((1 << bits) - 1)
    }
}
