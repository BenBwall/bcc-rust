//! GCC's and Clang's `-Wsign-compare`: a comparison whose usual arithmetic
//! conversions turn a signed operand into an unsigned type, unless that
//! operand cannot be negative. It is a warning C99 §5.1.1.3p1 permits about
//! conversions §6.3.1.8p1 defines. Both compilers range-check the signed
//! operand further than [`Analyzer::cannot_be_negative`] does; that function
//! errs toward silence, since a false warning stops a `-Werror` build.
//! GCC words the same conversion in a conditional expression's operand.
//! C99: §6.5.8p3, p. 86; PDF p. 98; §6.5.9p4, p. 87; PDF p. 99;
//! §6.5.15p5, p. 90; PDF p. 102.

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    types::TypeId,
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
        let equality = matches!(operator, B::Equal | B::NotEqual);
        let Some(left_signed) = self.converts_sign(left, right) else {
            return;
        };
        let (signed, unsigned) = if left_signed {
            (left, right)
        } else {
            (right, left)
        };
        // An unsigned constant that fits the signed type compares equal or
        // not whichever way the conversion goes.
        let unsigned_type = self.promoted(unsigned);
        if equality
            && unsigned.ice
            && let Some((bits, _)) = self.integer_type(unsigned_type)
            && unsigned
                .integer
                .and_then(super::Integer::to_i128)
                .is_some_and(|value| value >= 0 && (bits >= 128 || value < 1i128 << (bits - 1)))
        {
            return;
        }
        if self.cannot_be_negative(signed.expression) {
            return;
        }
        let clang = self.context.configuration.compiler_identity() != CompilerIdentity::Gcc;
        // A narrow unsigned value cannot collide with a converted negative
        // one in Clang's reading of an equality.
        if clang
            && equality
            && let Some((bits, _)) = self.integer_type(unsigned_type)
            && self
                .unsigned_range(unsigned.expression)
                .is_some_and(|range| range < bits)
        {
            return;
        }
        // Clang takes this for a tautology, which another option reports.
        if clang && !equality && signed.ice {
            return;
        }
        let Some(name) = self.sign_compare_wording(left.ty, right.ty) else {
            return;
        };
        self.group_diagnostic(
            DiagnosticGroup::SignCompare,
            SemanticErrorKind::SignCompare,
            e.operator_source_vectors.unwrap_or(e.source_vectors),
            Some(name),
            None,
        );
    }

    /// GCC's words for a conditional whose signed operand converts to the
    /// other's unsigned type.
    /// C99: §6.5.15p5, p. 90; PDF p. 102.
    pub(super) fn check_conditional_sign(
        &mut self,
        then_value: &'tu Expression<'tu>,
        else_value: &'tu Expression<'tu>,
    ) {
        if self.context.configuration.compiler_identity() != CompilerIdentity::Gcc {
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
            self.group_diagnostic(
                DiagnosticGroup::SignCompare,
                SemanticErrorKind::SignCompare,
                signed.expression.source_vectors,
                Some(name),
                None,
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

    /// Whether `e`'s value is known not to be negative: a nonnegative
    /// constant, a value of an unsigned type before promotion, or an operator
    /// whose operands bound its result. Unknown forms answer true when both
    /// compilers do; the rest answer false.
    /// C99: §6.5.3.3p5, p. 79; PDF p. 91; §6.5.8p6, p. 86; PDF p. 98;
    /// §6.5.9p3, p. 87; PDF p. 99; §6.5.10-§6.5.14, pp. 87-89; PDF pp. 99-101.
    fn cannot_be_negative(&self, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        if info.ice {
            return info
                .integer
                .and_then(super::Integer::to_i128)
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
            } => match operator {
                | B::LessThan
                | B::LessThanOrEqual
                | B::GreaterThan
                | B::GreaterThanOrEqual
                | B::Equal
                | B::NotEqual
                | B::LogicalAnd
                | B::LogicalOr => true,
                | B::BitwiseAnd =>
                    self.cannot_be_negative(left_expression)
                        || self.cannot_be_negative(right_expression),
                | B::BitwiseOr | B::BitwiseXor | B::Addition | B::Multiplication | B::Division =>
                    self.cannot_be_negative(left_expression)
                        && self.cannot_be_negative(right_expression),
                | B::Modulo | B::RightShift => self.cannot_be_negative(left_expression),
                | B::Comma => self.cannot_be_negative(right_expression),
                | _ => false,
            },
            | E::Conditional(c) =>
                self.cannot_be_negative(c.then_expression)
                    && self.cannot_be_negative(c.else_expression),
            | E::Cast {
                operand_expression, ..
            } => {
                let operand = self.expression_info(operand_expression);
                match (self.integer_type(info.ty), self.integer_type(operand.ty)) {
                    | (Some((to, _)), Some((from, _))) =>
                        to >= from && self.cannot_be_negative(operand_expression),
                    | _ => false,
                }
            },
            | _ => false,
        }
    }

    /// A bound in bits on the value of the unsigned expression `e`, for the
    /// forms Clang's `GetExprRange` bounds.
    fn unsigned_range(&self, e: &'tu Expression<'tu>) -> Option<u32> {
        let info = self.expression_info(e);
        if info.ice {
            let value = info.integer.and_then(super::Integer::to_i128)?;
            return (value >= 0).then(|| 128 - value.leading_zeros());
        }
        match e.kind {
            | E::Parenthesized { expression } => self.unsigned_range(expression),
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                let left = self.unsigned_range(left_expression);
                let right = self.unsigned_range(right_expression);
                match operator {
                    | B::BitwiseAnd | B::Modulo => match (left, right) {
                        | (Some(left), Some(right)) => Some(left.min(right)),
                        | (known, None) | (None, known) => known,
                    },
                    | B::RightShift => {
                        let shift = self.expression_info(right_expression);
                        let shift = shift.integer.filter(|_| shift.ice)?.to_i128()?;
                        let shift = u32::try_from(shift).ok()?;
                        Some(left?.saturating_sub(shift))
                    },
                    | B::BitwiseOr | B::BitwiseXor => Some(left?.max(right?)),
                    | _ => None,
                }
            },
            | _ => None,
        }
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
            (left.unqualified(), right.unqualified())
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
