//! Comparisons whose result the operands' types already decide: GCC's
//! `-Wtype-limits` and Clang's `-Wtautological-constant-out-of-range-compare`,
//! `-Wtautological-constant-compare`, `-Wtautological-unsigned-zero-compare`
//! and `-Wtautological-type-limit-compare`. C99 §6.5.8p6 and §6.5.9p3 give
//! the result, so each is a warning §5.1.1.3p1 permits.
//! GCC: `shorten_compare` and `warn_type_limits`; Clang:
//! `CheckTautologicalComparison` in `SemaChecking.cpp`.
//! C99: §6.5.8p6, p. 86; PDF p. 98; §6.5.9p3, p. 87; PDF p. 99.

use super::{
    Analyzer,
    SemanticErrorKind,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup as G,
    },
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionType as E,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the comparison `e` of `left` and `right`.
    /// C99: §6.5.8p6, p. 86; PDF p. 98.
    pub(super) fn check_type_limits(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        if !self.unused.type_limits
            || self.tainted
            || !matches!(
                operator,
                B::LessThan
                    | B::LessThanOrEqual
                    | B::GreaterThan
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual
            )
        {
            return;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        // Clang does not analyze the operand of `sizeof` and its kin; GCC
        // does.
        if !gcc && self.unevaluated_operand() {
            return;
        }
        let (left_value, right_value) = (
            self.expression_info(left)
                .integer
                .and_then(super::Integer::to_i128),
            self.expression_info(right)
                .integer
                .and_then(super::Integer::to_i128),
        );
        // One side is a constant and the other is not; the constant goes on
        // the right, with the comparison turned around.
        let (operand, constant_expression, value, operator, constant_first) =
            match (left_value, right_value) {
                | (None, Some(value)) => (left, right, value, operator, false),
                | (Some(value), None) => (right, left, value, flipped(operator), true),
                | _ => return,
            };
        let operand_info = self.expression_info(operand);
        let ty = self.types.non_atomic(operand_info.ty).unqualified();
        let bool_type = matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool));
        if !matches!(self.types.nodes[ty.index], TypeKind::Scalar(_)) {
            return;
        }
        let Some((bits, signed)) = self.integer_type(ty) else {
            return;
        };
        // The comparison is made in the type the usual arithmetic
        // conversions give the promoted operands (§6.5.8p3, §6.5.9p4).
        let constant_info = self.expression_info(constant_expression);
        let operand_promoted = self.promote(operand_info, ty);
        let constant_promoted = self.promote(constant_info, constant_info.ty.unqualified());
        let (Some(promoted), Some(constant_type)) = (
            self.integer_type(operand_promoted),
            self.integer_type(constant_promoted),
        ) else {
            return;
        };
        let (common_bits, common_signed) = common_type(promoted, constant_type);
        let Some(constant) = converted(value, common_bits, common_signed) else {
            return;
        };
        // An `i128` holds every bound but the unsigned 128-bit maximum, so
        // only a comparison with zero is decided there.
        if !signed && bits >= 128 && constant != 0 {
            return;
        }
        let (low, high) = match (signed, bits) {
            | (true, 128..) => (i128::MIN, i128::MAX),
            | (true, _) => (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1),
            | (false, 127..) => (0, i128::MAX),
            | (false, _) => (0, (1_i128 << bits) - 1),
        };
        // A signed operand converted to an unsigned type takes two runs of
        // values, its negative ones wrapped to the top (§6.3.1.3p2).
        let ranges = if signed && !common_signed {
            if common_bits >= 127 {
                return;
            }
            let modulus = 1_i128 << common_bits;
            [(0, high), (low + modulus, modulus - 1)]
        } else {
            [(low, high), (low, high)]
        };
        let (Some(result), Some(other)) = (
            decided(operator, constant, ranges[0].0, ranges[0].1),
            decided(operator, constant, ranges[1].0, ranges[1].1),
        ) else {
            return;
        };
        if result != other {
            return;
        }
        let in_range = ranges
            .iter()
            .any(|&(low, high)| low <= constant && constant <= high);
        let site = e.operator_source_vectors.unwrap_or(e.source_vectors);
        let truth = if result { "true" } else { "false" };
        if gcc {
            // `shorten_compare` decides a comparison only of an operand
            // narrower than the comparison's type; a signed one compared as
            // unsigned only for equality, the constant read back as signed.
            let shortened = bits < common_bits;
            if shortened && !bool_type {
                if signed && !common_signed && !matches!(operator, B::Equal | B::NotEqual) {
                    return;
                }
                let text = format_in!(
                    self.scratch,
                    "comparison is always {truth} due to limited range of data type"
                );
                self.limits_warning(G::TypeLimits, site, text);
            } else if !shortened && !common_signed && constant == 0 && !bool_type {
                let shown = if result { ">= 0" } else { "< 0" };
                let text = format_in!(
                    self.scratch,
                    "comparison of unsigned expression in '{shown}' is always {truth}"
                );
                self.limits_warning(G::TypeLimits, site, text);
            }
            return;
        }
        // Clang leaves a constant the range holds alone when an enumerator or
        // a macro wrote it (`IsEnumConstOrFromMacro`).
        if in_range {
            let written = Self::without_parentheses(constant_expression);
            if matches!(written.kind, E::Identifier(_))
                || self.context.written_by_macro_body(written.source_vectors)
            {
                return;
            }
        }
        let Some(name) = self.type_spelling(ty, false) else {
            return;
        };
        if !in_range || bool_type {
            // Clang's `DiagRuntimeBehavior`, which code that does not run
            // leaves out. `_Generic` is not resolved yet, so this errs toward
            // silence in its selected association too.
            if self.skipped_operand() {
                return;
            }
            let group = if in_range {
                G::TautologicalConstantCompare
            } else {
                G::TautologicalConstantOutOfRangeCompare
            };
            let text = format_in!(
                self.scratch,
                "result of comparison of constant {constant} with expression of type '{name}' is \
                 always {truth}"
            );
            let text = self.context.string_cache.intern(text);
            self.warn_if_reachable(
                e,
                group,
                SemanticErrorKind::TypeLimits(group),
                site,
                Some(text),
            );
        } else if self.functions.skipped_operand {
            // A `_Generic` association may not be the selected one; a `&&` or
            // `||` operand a constant skips would warn.
        } else if (!signed || !common_signed) && constant == 0 {
            let shown = spelling(if constant_first {
                flipped(operator)
            } else {
                operator
            });
            let text = if constant_first {
                format_in!(
                    self.scratch,
                    "result of comparison of 0 {shown} unsigned expression is always {truth}"
                )
            } else {
                format_in!(
                    self.scratch,
                    "result of comparison of unsigned expression {shown} 0 is always {truth}"
                )
            };
            self.limits_warning(G::TautologicalUnsignedZeroCompare, site, text);
        } else {
            let text = if constant_first {
                let shown = spelling(flipped(operator));
                format_in!(
                    self.scratch,
                    "result of comparison {constant} {shown} '{name}' is always {truth}"
                )
            } else {
                let shown = spelling(operator);
                format_in!(
                    self.scratch,
                    "result of comparison '{name}' {shown} {constant} is always {truth}"
                )
            };
            self.limits_warning(G::TautologicalTypeLimitCompare, site, text);
        }
    }

    fn limits_warning(&mut self, group: G, site: SourceVectors, text: &str) {
        let name = self.context.string_cache.intern(text);
        self.spelled_group_diagnostic(
            group,
            SemanticErrorKind::TypeLimits(group),
            site,
            Some(name),
            site,
        );
    }
}

/// The width and signedness the usual arithmetic conversions give two
/// promoted integer types: the wider, or the unsigned one of a width the
/// signed one cannot exceed.
/// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57.
const fn common_type(left: (u32, bool), right: (u32, bool)) -> (u32, bool) {
    let ((left_bits, left_signed), (right_bits, right_signed)) = (left, right);
    if left_signed == right_signed {
        (
            if left_bits > right_bits {
                left_bits
            } else {
                right_bits
            },
            left_signed,
        )
    } else {
        let (unsigned, signed) = if left_signed {
            (right_bits, left_bits)
        } else {
            (left_bits, right_bits)
        };
        if signed > unsigned {
            (signed, true)
        } else {
            (unsigned, false)
        }
    }
}

/// `value` converted to an integer type of `bits` and signedness `signed`,
/// if an `i128` holds the result. The usual arithmetic conversions only
/// widen a signed constant, so only an unsigned type changes it.
/// C99: §6.3.1.3p2, p. 43; PDF p. 55.
const fn converted(value: i128, bits: u32, signed: bool) -> Option<i128> {
    if signed || value >= 0 && (bits >= 127 || value < 1_i128 << bits) {
        Some(value)
    } else if bits >= 127 {
        None
    } else {
        Some(value.rem_euclid(1_i128 << bits))
    }
}

/// `operator` with its operands swapped.
const fn flipped(operator: B) -> B {
    match operator {
        | B::LessThan => B::GreaterThan,
        | B::GreaterThan => B::LessThan,
        | B::LessThanOrEqual => B::GreaterThanOrEqual,
        | B::GreaterThanOrEqual => B::LessThanOrEqual,
        | other => other,
    }
}

const fn spelling(operator: B) -> &'static str {
    match operator {
        | B::LessThan => "<",
        | B::GreaterThan => ">",
        | B::LessThanOrEqual => "<=",
        | B::GreaterThanOrEqual => ">=",
        | B::Equal => "==",
        | _ => "!=",
    }
}

/// The result `x operator constant` has for every `x` in `low..=high`, if
/// it is the same for all of them.
const fn decided(operator: B, constant: i128, low: i128, high: i128) -> Option<bool> {
    let (at_low, at_high) = match operator {
        | B::LessThan => (low < constant, high < constant),
        | B::LessThanOrEqual => (low <= constant, high <= constant),
        | B::GreaterThan => (low > constant, high > constant),
        | B::GreaterThanOrEqual => (low >= constant, high >= constant),
        | B::Equal => {
            return if constant < low || constant > high {
                Some(false)
            } else {
                None
            };
        },
        | _ => {
            return if constant < low || constant > high {
                Some(true)
            } else {
                None
            };
        },
    };
    // Each relation is monotonic over the range, so its ends decide it.
    if at_low == at_high {
        Some(at_low)
    } else {
        None
    }
}
