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
        let (operand, constant, operator, constant_first) = match (left_value, right_value) {
            | (None, Some(value)) => (left, value, operator, false),
            | (Some(value), None) => (right, value, flipped(operator), true),
            | _ => return,
        };
        let ty = self
            .types
            .non_atomic(self.expression_info(operand).ty)
            .unqualified();
        let bool_type = matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool));
        if !matches!(self.types.nodes[ty.index], TypeKind::Scalar(_)) {
            return;
        }
        let Some((bits, signed)) = self.integer_type(ty) else {
            return;
        };
        let int = self.types.scalar(Scalar::Int);
        let int_bits = self.integer_type(int).map_or(32, |(b, _)| b);
        let narrow = bits < int_bits;
        // An `i128` holds every bound but the unsigned 128-bit maximum, which
        // only a comparison with zero can reach here.
        let (low, high) = match (signed, bits) {
            | (true, 128..) => (i128::MIN, i128::MAX),
            | (true, _) => (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1),
            | (false, 127..) => (0, i128::MAX),
            | (false, _) => (0, (1_i128 << bits) - 1),
        };
        let Some(result) = decided(operator, constant, low, high) else {
            return;
        };
        let site = e.operator_source_vectors.unwrap_or(e.source_vectors);
        let truth = if result { "true" } else { "false" };
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            if narrow && !bool_type {
                let text = format_in!(
                    self.scratch,
                    "comparison is always {truth} due to limited range of data type"
                );
                self.limits_warning(G::TypeLimits, site, text);
            } else if !signed && constant == 0 && !bool_type {
                let shown = if result { ">= 0" } else { "< 0" };
                let text = format_in!(
                    self.scratch,
                    "comparison of unsigned expression in '{shown}' is always {truth}"
                );
                self.limits_warning(G::TypeLimits, site, text);
            }
            return;
        }
        let Some(name) = self.type_spelling(ty, false) else {
            return;
        };
        let outside = constant < low || constant > high;
        if outside && (narrow || bool_type) {
            let text = format_in!(
                self.scratch,
                "result of comparison of constant {constant} with expression of type '{name}' is \
                 always {truth}"
            );
            self.limits_warning(G::TautologicalConstantOutOfRangeCompare, site, text);
        } else if outside {
        } else if !signed && constant == 0 && !bool_type {
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
        } else if bool_type {
            let text = format_in!(
                self.scratch,
                "result of comparison of constant {constant} with expression of type '{name}' is \
                 always {truth}"
            );
            self.limits_warning(G::TautologicalConstantCompare, site, text);
        } else if narrow {
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
