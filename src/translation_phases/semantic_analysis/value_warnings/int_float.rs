//! Clang's `-Wimplicit-const-int-float-conversion`: an integer constant
//! whose implicit conversion to a real floating type changes its value,
//! because the type's significand holds fewer bits than the constant needs.
//! C99 rounds the value in an implementation-defined direction (§6.3.1.4p2),
//! so this is a warning §5.1.1.3p1 permits. Clang: the integer-to-floating
//! branch of `CheckImplicitConversion` in `SemaChecking.cpp`, which prints
//! the new value with `APFloat::toString`.
//! C99: §6.3.1.4p2, p. 43; PDF p. 55.

use super::super::{
    Analyzer,
    ExpressionInfo,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            ExpressionType as E,
        },
    },
    util::string_cache::StringCacheId,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The common type the usual arithmetic conversions give `left` and
    /// `right`, checking each operand they bring to a floating type.
    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57.
    pub(in crate::translation_phases::semantic_analysis) fn checked_common_arithmetic(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        lt: TypeId,
        rt: TypeId,
    ) -> TypeId {
        let ty = self.common_arithmetic(left, right, lt, rt);
        self.check_int_float_source(ty, left);
        self.check_int_float_source(ty, right);
        ty
    }

    /// Checks the value `source` converts to `target`. Clang checks the arms
    /// of a `?:` against the type its value converts to, not the `?:`'s own
    /// type, and leaves out an arm a constant condition does not take.
    /// C99: §6.5.15p4-5, p. 90; PDF p. 102.
    pub(in crate::translation_phases::semantic_analysis) fn check_int_float_source(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) {
        let mut pending = crate::util::bump::ArenaVec::new_in(self.scratch);
        pending.push(source);
        while let Some(source) = pending.pop() {
            let mut e = source.expression;
            while let E::Parenthesized { expression } = e.kind {
                e = expression;
            }
            let E::Conditional(c) = e.kind else {
                self.check_int_float_constant(target, source);
                continue;
            };
            let taken = Self::constant_truth(self.expression_info(c.condition_expression));
            if taken != Some(true) {
                pending.push(self.expression_info(c.else_expression));
            }
            if taken != Some(false) {
                pending.push(self.expression_info(c.then_expression));
            }
        }
    }

    /// Warns about the integer constant `source` when its conversion to the
    /// real floating type `target` is inexact, under Clang's rules.
    /// C99: §6.3.1.4p2, p. 43; PDF p. 55.
    pub(in crate::translation_phases::semantic_analysis) fn check_int_float_constant(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) {
        // Clang's `DiagRuntimeBehavior`: only code that may run, outside
        // unevaluated operands, untaken branches and constant contexts.
        if self.tainted
            || self.context.configuration.compiler_identity().rules()
                != crate::configuration::RuleSet::Clang
            || self.inhibited_evaluations != 0
            || self.constant_contexts != 0
            || self.functions.dead
        {
            return;
        }
        let integer = match source.integer {
            | Some(value) => Some(value),
            | None if self.constant_with_builtin_calls(source.expression) =>
                self.folded_integer(source.expression),
            | None => None,
        };
        let Some(value) = integer.filter(|value| value.wide.is_none()) else {
            return;
        };
        if self.integer_type(source.ty).is_none() {
            return;
        }
        let target = self.types.non_atomic(target).unqualified();
        let TypeKind::Scalar(scalar) = self.types.nodes[target.index] else {
            return;
        };
        let Some(format) = self.significand_format(scalar) else {
            return;
        };
        let (negative, magnitude) = if value.signed && value.value < 0 {
            (true, value.value.unsigned_abs())
        } else {
            (false, value.value.cast_unsigned())
        };
        let mut digits = [0_u8; TEXT_CAPACITY];
        let Some(new) = rounded_text(negative, magnitude, format, &mut digits) else {
            return;
        };
        let (Some(from), Some(to)) = (
            self.type_spelling(source.ty, false),
            self.type_spelling(target, false),
        ) else {
            return;
        };
        let old = if negative {
            format_in!(self.scratch, "-{magnitude}")
        } else {
            format_in!(self.scratch, "{magnitude}")
        };
        let text = format_in!(
            self.scratch,
            "implicit conversion from '{from}' to '{to}' changes value from {old} to {new}"
        );
        // Clang's `getExprLoc` of the source without its parentheses: an
        // operator's, or where a call or operand begins.
        let mut e = source.expression;
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        let site = match e.kind {
            | E::Call { .. } => e.source_vectors,
            | _ => e.operator_source_vectors.unwrap_or(e.source_vectors),
        };
        let text = self.context.string_cache.intern(text);
        if self.functions.generic_marks_open() {
            self.functions.pending_int_float.push((site, text));
        } else {
            self.int_float_warning(site, text);
        }
    }

    /// Reports the integer-to-floating warnings a generic selection held
    /// back, in order.
    pub(in crate::translation_phases::semantic_analysis) fn report_pending_int_float(&mut self) {
        let pending = &mut self.functions.pending_int_float;
        let warnings = self.scratch.alloc_slice_copy(pending);
        pending.clear();
        for &(site, text) in &*warnings {
            self.int_float_warning(site, text);
        }
    }

    fn int_float_warning(&mut self, site: SourceVectors, text: StringCacheId) {
        let group = G::ImplicitConstIntFloatConversion;
        let kind = super::super::SemanticErrorKind::ValueWarning(group);
        self.spelled_group_diagnostic(group, kind, site, Some(text), site);
    }

    /// Whether `e`, which has no integer constant value, is an integer
    /// constant expression to Clang all the same: one whose operands are
    /// constants or calls of builtins, which Clang's `CheckICE` takes when
    /// they evaluate to an integer (`CheckEvalInICE`), as
    /// `__builtin_expect(16777217, 1)` does.
    /// C99: §6.6p6, p. 95; PDF p. 107.
    fn constant_with_builtin_calls(&self, e: &'tu Expression<'tu>) -> bool {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            UnaryOperator as U,
        };
        let mut pending = crate::util::bump::ArenaVec::new_in(self.scratch);
        pending.push(e);
        let mut call = false;
        while let Some(e) = pending.pop() {
            if self.expression_info(e).integer.is_some() {
                continue;
            }
            match e.kind {
                | E::Parenthesized { expression } => pending.push(expression),
                | E::Call { .. } => call = true,
                | E::Unary {
                    operator: U::Plus | U::Minus | U::BitwiseNot | U::LogicalNot,
                    operand_expression,
                } => pending.push(operand_expression),
                | E::Binary {
                    operator:
                        B::Comma
                        | B::Subscript
                        | B::Assignment
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
                } => return false,
                | E::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    pending.push(left_expression);
                    pending.push(right_expression);
                },
                | E::Cast {
                    operand_expression, ..
                } if self.integer_type(self.expression_info(e).ty).is_some() =>
                    pending.push(operand_expression),
                | E::Conditional(c) => {
                    pending.push(c.condition_expression);
                    pending.push(c.then_expression);
                    pending.push(c.else_expression);
                },
                | _ => return false,
            }
        }
        call
    }

    /// The significand bits and largest binary exponent of a real floating
    /// scalar; `None` for any other type.
    /// C99: §5.2.4.2.2p2, p. 22; PDF p. 34.
    fn significand_format(&self, scalar: Scalar) -> Option<Format> {
        let (precision, max_exponent) = match scalar.representation() {
            | Scalar::Float16 | Scalar::Fp16 => (11, 16),
            | Scalar::BFloat16 => (8, 128),
            | Scalar::Float => (24, 128),
            | Scalar::Double => (53, 1024),
            | Scalar::LongDouble if self.types.target.long_double_is_double() => (53, 1024),
            | Scalar::LongDouble if self.types.target.long_double_is_binary128() => (113, 16384),
            | Scalar::LongDouble => (64, 16384),
            | Scalar::Float128 => (113, 16384),
            | _ => return None,
        };
        Some(Format {
            precision,
            max_exponent,
        })
    }
}

/// A binary floating format: its significand bits, and the exponent of the
/// smallest power of two too large for it.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct Format {
    pub(in crate::translation_phases::semantic_analysis) precision:    u32,
    pub(in crate::translation_phases::semantic_analysis) max_exponent: u32,
}

/// Room for the text of any integer below 2^129: a sign, 39 digits, and a
/// point and exponent in scientific form.
pub(in crate::translation_phases::semantic_analysis) const TEXT_CAPACITY: usize = 64;

/// The value `±magnitude` takes in `format`, rounded to nearest with ties to
/// even, as Clang prints it (`APFloat::toString` with the format's
/// significand bits as its precision and a padding of 3): `None` when the
/// value is exact and in range. `buffer` holds the text.
pub(in crate::translation_phases::semantic_analysis) fn rounded_text(
    negative: bool,
    magnitude: u128,
    format: Format,
    buffer: &mut [u8; TEXT_CAPACITY],
) -> Option<&str> {
    let width = magnitude.bit_width();
    if width <= format.precision {
        return None;
    }
    let mut shift = width - format.precision;
    let mut significand = magnitude >> shift;
    let rest = magnitude & ((1 << shift) - 1);
    let half = 1 << (shift - 1);
    if rest > half || (rest == half && significand & 1 == 1) {
        significand += 1;
        if significand >> format.precision != 0 {
            significand >>= 1;
            shift += 1;
        }
    }
    // An exact significand still overflows a too small exponent.
    if format.precision - 1 + shift >= format.max_exponent {
        let text = if negative { "-Inf" } else { "+Inf" };
        buffer[..text.len()].copy_from_slice(text.as_bytes());
        return core::str::from_utf8(&buffer[..text.len()]).ok();
    }
    if rest == 0 {
        return None;
    }
    // The exact decimal digits of significand × 2^shift, least significant
    // first.
    let mut digits = [0_u8; TEXT_CAPACITY];
    let mut count = 0;
    let mut rest = significand;
    while rest != 0 {
        digits[count] = (rest % 10) as u8;
        rest /= 10;
        count += 1;
    }
    for _ in 0..shift {
        let mut carry = 0;
        for digit in &mut digits[..count] {
            let doubled = *digit * 2 + carry;
            *digit = doubled % 10;
            carry = doubled / 10;
        }
        if carry != 0 {
            digits[count] = carry;
            count += 1;
        }
    }
    // APFloat first truncates the binary value to about as many bits as
    // the precision's decimal digits need, dropping whole decimal digits
    // without rounding (`AdjustToPrecision` of an `APInt`), and so may
    // keep too few to round below.
    let bits = significand.bit_width() + shift;
    let required = (format.precision * 196 + 58) / 59;
    let mut low = if bits > required {
        ((bits - required) * 59 / 196) as usize
    } else {
        0
    };
    // Trailing zeros become the exponent.
    while digits[low] == 0 {
        low += 1;
    }
    let mut exponent = low;
    let precision = format.precision as usize;
    if count - low > precision {
        let mut first = count - precision;
        if digits[first - 1] < 5 {
            while first < count && digits[first] == 0 {
                first += 1;
            }
        } else {
            while first < count && digits[first] == 9 {
                first += 1;
            }
            if first == count {
                digits[count] = 1;
                count += 1;
            } else {
                digits[first] += 1;
            }
        }
        exponent = first;
        low = first;
    }
    let kept = &digits[low..count];
    let mut length = 0;
    let mut push = |byte: u8| {
        buffer[length] = byte;
        length += 1;
    };
    if negative {
        push(b'-');
    }
    let scientific = exponent > 3 || kept.len() + exponent > precision;
    if scientific {
        let mut most = kept.iter().rev();
        push(b'0' + most.next().copied().unwrap_or(0));
        push(b'.');
        if kept.len() == 1 {
            push(b'0');
        }
        for &digit in most {
            push(b'0' + digit);
        }
        push(b'E');
        push(b'+');
        let power = exponent + kept.len() - 1;
        let mut power_digits = [0_u8; 8];
        let mut places = 0;
        let mut rest = power;
        loop {
            power_digits[places] = u8::try_from(rest % 10).unwrap_or(0);
            rest /= 10;
            places += 1;
            if rest == 0 {
                break;
            }
        }
        for &digit in power_digits[..places].iter().rev() {
            push(b'0' + digit);
        }
    } else {
        for &digit in kept.iter().rev() {
            push(b'0' + digit);
        }
        for _ in 0..exponent {
            push(b'0');
        }
    }
    core::str::from_utf8(&buffer[..length]).ok()
}
