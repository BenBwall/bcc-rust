//! Phase-7 folding of integer constant arithmetic that C99 §6.6p4 leaves
//! without a value: a signed result outside its type, a shift count that is
//! negative or not below the operand's width, and a division by zero.
//!
//! Plain bcc refuses such an operation where a constant is required. GCC and
//! Clang fold most of them and warn, so `--compiler-extensions`, which only
//! the GNU personalities pass, turns this on; the identity cannot, because
//! plain bcc has Clang's. [`Analyzer::fold_exceptional_binary`] and
//! [`Analyzer::fold_exceptional_unary`] give typing the value and the
//! [`ConstantFolding`] mark, [`Analyzer::carried_exceptional`] carries the
//! mark to what is computed from it, and
//! [`Analyzer::evaluate_exceptional`] says what each constant context makes
//! of a marked value. The compilers differ throughout:
//!
//! | | GCC | Clang |
//! | --- | --- | --- |
//! | Signed overflow | wraps, `-Woverflow` | wraps, `-Winteger-overflow` |
//! | Minimum over -1 | wraps, `-Woverflow` | wraps silently |
//! | Count too large | 0, or -1 shifting a negative value right | clamps to width - 1 |
//! | Negative count | no value, unless the operand is 0, or -1 shifted right | shifts the other way |
//! | Array bound | variable length, or negative if it only wrapped | folds with `-Wgnu-folding-constant` |
//!
//! GCC warns where it parses the operator, unless the operand is unevaluated
//! or skipped ([`Analyzer::report_exceptional_operands`]). Clang warns about
//! a shift or a division where code runs (`DiagRuntimeBehavior`), so not in a
//! constant context or in unreachable code, and about overflow from one
//! evaluation of each finished full expression (`Sema::CheckForIntOverflow`),
//! which [`Analyzer::finish_full_expression`] walks.
//!
//! C99: §6.6 paragraph 4, p. 95; PDF p. 107; §6.5 paragraph 5, p. 67; PDF
//! p. 79; §6.5.7 paragraphs 3-5, p. 85; PDF p. 97; §6.5.5 paragraphs 5-6, p.
//! 82; PDF p. 94. GNU extension: GCC manual, "Warning Options", `-Woverflow`
//! and `-Wshift-count-overflow`.
//! <https://gcc.gnu.org/onlinedocs/gcc/Warning-Options.html>

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    WideStore,
    expressions::ConstantFolding,
    integer::Integer,
    types::TypeId,
};
use crate::{
    configuration::{
        DiagnosticGroup,
        ExtensionPolicy,
        RuleSet,
    },
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            Expression,
            ExpressionType,
            UnaryOperator,
        },
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

/// What requires the constant being evaluated, which GCC's `-pedantic` words
/// name.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ConstantSite {
    /// C99: §6.7.2.2 paragraph 2, p. 104; PDF p. 116.
    Enumerator(StringCacheId),
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
    CaseLabel,
    /// C11: §6.7.10 paragraph 2, p. 145; PDF p. 163.
    StaticAssertion,
    /// C99: §6.7.2.1 paragraph 3, p. 101; PDF p. 113.
    BitFieldWidth(Option<StringCacheId>),
    /// An array bound, which GCC's folding policy decides apart.
    #[default]
    Other,
}

/// An overflow Clang reports only if the evaluation of its full expression
/// reaches it: the operator and the warning's text.
#[derive(Clone, Copy)]
pub(super) struct PendingOverflow<'tu> {
    expression: &'tu Expression<'tu>,
    text:       StringCacheId,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Gives `info`, the result of `left op right`, which [`Integer::binary`]
    /// finds no value for, the value and the mark its compiler gives it, and
    /// reports a signed overflow.
    /// C99: §6.6 paragraph 4, p. 95; PDF p. 107.
    pub(super) fn fold_exceptional_binary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        info: &mut ExpressionInfo<'tu>,
    ) {
        use BinaryOperator as B;
        if !self.context.configuration.folds_exceptional_constants() {
            return;
        }
        let Some((bits, signed)) = self.integer_type(info.ty) else {
            return;
        };
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        let (value, folding) = match op {
            | B::LeftShift | B::RightShift =>
                self.exceptional_shift(op, left, right, bits, signed, gcc),
            | B::Division | B::Modulo if right.is_zero() => (None, ConstantFolding::Undefined),
            | B::Division | B::Modulo => {
                let value = self.wide.wrapping_binary(op, left, right, bits, signed);
                if gcc {
                    self.report_overflow(e, value, None, info.ty);
                }
                (
                    Some(value),
                    if gcc {
                        ConstantFolding::Wrapped
                    } else {
                        ConstantFolding::UndefinedQuotient
                    },
                )
            },
            | B::Addition | B::Subtraction | B::Multiplication => {
                let value = self.wide.wrapping_binary(op, left, right, bits, signed);
                self.report_overflow(e, value, None, info.ty);
                (Some(value), ConstantFolding::Wrapped)
            },
            | _ => return,
        };
        info.integer = value;
        info.folding = folding;
        info.ice = false;
    }

    /// [`Self::fold_exceptional_binary`] for the unary minus of a signed
    /// type's minimum, the one unary operator that overflows: `value` is
    /// the minimum, and also its wrapped negation.
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    pub(super) fn fold_exceptional_unary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: UnaryOperator,
        value: Integer,
        info: &mut ExpressionInfo<'tu>,
    ) {
        if op != UnaryOperator::Minus || !self.context.configuration.folds_exceptional_constants() {
            return;
        }
        let Some((bits, signed)) = self.integer_type(info.ty) else {
            return;
        };
        let value = self.wide.convert(value, bits, signed);
        self.report_overflow(e, value, Some(value), info.ty);
        info.integer = Some(value);
        info.folding = ConstantFolding::Wrapped;
        info.ice = false;
    }

    /// Marks `info`, the result of `left << right` whose `value` was folded
    /// by [`Integer::sign_bit_shift`] (`into_sign_bit`, a nonnegative value
    /// into but not past the sign bit) or [`Integer::wrapping_shift`], and
    /// warns of a value past the sign bit. For GCC this is an integer
    /// constant expression only before C99, which made the shift undefined.
    /// Clang accepts a shift into the sign bit, and warns of a negative
    /// operand where code runs.
    /// C99: §6.5.7 paragraph 4, p. 85; PDF p. 97.
    pub(super) fn fold_sign_bit_shift(
        &mut self,
        left: Integer,
        right: Integer,
        value: Integer,
        into_sign_bit: bool,
        e: &'tu Expression<'tu>,
        info: &mut ExpressionInfo<'tu>,
    ) {
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        let c99 = self.context.configuration.standard() >= crate::configuration::CStandard::C99;
        if gcc && c99 {
            info.folding = ConstantFolding::Undefined;
            info.ice = false;
        } else if !gcc && !into_sign_bit {
            info.folding = ConstantFolding::Wrapped;
            info.ice = false;
        }
        if self.inhibited_evaluations != 0 || self.tainted {
            return;
        }
        if self.negative_integer(left) {
            if !gcc && self.constant_contexts == 0 && !self.functions.dead {
                let text = self
                    .context
                    .string_cache
                    .intern("shifting a negative signed value is undefined");
                self.exceptional_warning(
                    DiagnosticGroup::ShiftNegativeValue,
                    text,
                    operator_source(e),
                );
            }
            return;
        }
        if gcc && !c99 {
            return;
        }
        self.report_shift_overflow(left, right, value, info.ty, e, gcc);
    }

    /// `-Wshift-overflow`: a nonnegative value shifted left needs more bits
    /// than the type has, and one for its sign, beyond the sign bit itself.
    /// C99: §6.5.7 paragraph 4, p. 85; PDF p. 97.
    fn report_shift_overflow(
        &mut self,
        left: Integer,
        right: Integer,
        value: Integer,
        ty: TypeId,
        e: &'tu Expression<'tu>,
        gcc: bool,
    ) {
        let magnitude = left.value.cast_unsigned();
        let Ok(count) = u32::try_from(right.value) else {
            return;
        };
        let required = (128 - magnitude.leading_zeros()) + count + 1;
        if required <= value.bits + 1 {
            return;
        }
        let ty = self.type_spelling(ty, gcc).unwrap_or("int");
        let bits = value.bits;
        let text = if gcc {
            format_in!(
                self.scratch,
                "result of '{left} << {right}' requires {required} bits to represent, but '{ty}' \
                 only has {bits} bits"
            )
        } else if required > 128 {
            return;
        } else {
            let exact = magnitude << count;
            format_in!(
                self.scratch,
                "signed shift result (0x{exact:x}) requires {required} bits to represent, but \
                 '{ty}' only has {bits} bits"
            )
        };
        let text = self.context.string_cache.intern(text);
        self.exceptional_warning(
            if gcc {
                DiagnosticGroup::GccShiftOverflow
            } else {
                DiagnosticGroup::ShiftOverflow
            },
            text,
            operator_source(e),
        );
    }

    /// The value and mark of a shift of `left` at the type of `bits` and
    /// `signed`ness whose count, or whose left operand, makes it undefined.
    /// C99: §6.5.7 paragraphs 3-4, p. 85; PDF p. 97.
    fn exceptional_shift(
        &self,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        bits: u32,
        signed: bool,
        gcc: bool,
    ) -> (Option<Integer>, ConstantFolding) {
        let negative_count = self.negative_integer(right);
        let in_range = !negative_count && !self.count_reaches_width(right, bits);
        if in_range {
            // Only a left shift of a negative value or past the sign bit of
            // a bit-precise type comes here. Clang's code keeps this value.
            let count = u32::try_from(right.value).unwrap_or(0);
            let value = self.wide.wrapping_shift(op, left, count, bits, signed);
            return (
                Some(value),
                if gcc {
                    ConstantFolding::Undefined
                } else {
                    ConstantFolding::Wrapped
                },
            );
        }
        let value = if gcc {
            self.gcc_shift(op, left, right, bits, signed, negative_count)
        } else {
            Some(self.clang_shift(op, left, right, bits, signed, negative_count))
        };
        (value, ConstantFolding::Undefined)
    }

    /// GCC's value: the count converts to `int`, which keeps a low part
    /// below the width as the count, or else shifts everything out. A count
    /// that is negative in `int` without having lost any value folds only
    /// where the operand decides the result.
    /// GNU extension: GCC `build_binary_op` and `int_const_binop`, as
    /// measured on GCC 14.2.
    fn gcc_shift(
        &self,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        bits: u32,
        signed: bool,
        negative_count: bool,
    ) -> Option<Integer> {
        let left = self.wide.convert(left, bits, signed);
        let all_ones = self.wide.compare(left, Integer::int(-1)).is_eq();
        let shifted_out = |this: &Self| {
            let value = if op == BinaryOperator::RightShift && this.negative_integer(left) {
                -1
            } else {
                0
            };
            this.wide.convert(Integer::int(value), bits, signed)
        };
        if negative_count {
            return (left.is_zero() || (op == BinaryOperator::RightShift && all_ones))
                .then_some(left);
        }
        if right.wide.is_some() {
            return Some(shifted_out(self));
        }
        let raw = right.value.cast_unsigned();
        let low = u32::try_from(raw & u128::from(u32::MAX)).unwrap_or(0);
        if low.cast_signed() < 0 {
            let high = (raw >> 32) & u128::from(u32::MAX);
            let kept = right.bits <= 32 || high == u128::from(u32::MAX);
            return (!kept).then(|| shifted_out(self));
        }
        Some(if low < bits {
            self.wide.wrapping_shift(op, left, low, bits, signed)
        } else {
            shifted_out(self)
        })
    }

    /// Clang's value: a negative count shifts the other way, and a count
    /// clamps to one below the width.
    /// Clang: `handleIntIntBinOp` in `ExprConstant.cpp`.
    fn clang_shift(
        &self,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        bits: u32,
        signed: bool,
        negative_count: bool,
    ) -> Integer {
        use BinaryOperator as B;
        let op = match (op, negative_count) {
            | (B::LeftShift, true) => B::RightShift,
            | (B::RightShift, true) => B::LeftShift,
            | (op, _) => op,
        };
        let magnitude = if negative_count {
            right.value.unsigned_abs()
        } else if right.wide.is_some() {
            u128::MAX
        } else {
            right.value.cast_unsigned()
        };
        let count = u32::try_from(magnitude.min(u128::from(bits - 1))).unwrap_or(bits - 1);
        self.wide.wrapping_shift(op, left, count, bits, signed)
    }

    /// Whether `value` is below zero, at any width.
    fn negative_integer(&self, value: Integer) -> bool {
        self.wide.compare(value, Integer::int(0)).is_lt()
    }

    /// Whether the nonnegative shift count `count` is not below `bits`.
    fn count_reaches_width(&self, count: Integer, bits: u32) -> bool {
        self.wide
            .compare(count, Integer::int(i128::from(bits)))
            .is_ge()
    }

    /// Reports a signed overflow to `value` in `e`: GCC at once, Clang if
    /// its full expression's evaluation reaches it. `operand` is the value
    /// a unary operator's operand folded to, which GCC names.
    /// C99: §6.6 paragraph 4, p. 95; PDF p. 107.
    fn report_overflow(
        &mut self,
        e: &'tu Expression<'tu>,
        value: Integer,
        operand: Option<Integer>,
        ty: TypeId,
    ) {
        if self.inhibited_evaluations != 0 || self.tainted {
            return;
        }
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        let ty = self.type_spelling(ty, gcc).unwrap_or("int");
        if gcc {
            let result = self.gcc_constant_text(value);
            let text = match operand {
                | Some(operand) => format_in!(
                    self.scratch,
                    "integer overflow in expression '{}' of type '{ty}' results in '{result}'",
                    self.gcc_constant_text(operand)
                ),
                | None => format_in!(
                    self.scratch,
                    "integer overflow in expression of type '{ty}' results in '{result}'"
                ),
            };
            let text = self.context.string_cache.intern(text);
            self.exceptional_warning(DiagnosticGroup::Overflow, text, operator_source(e));
        } else {
            let result = self.clang_constant_text(value);
            let text = format_in!(
                self.scratch,
                "overflow in expression; result is {result} with type '{ty}'"
            );
            let text = self.context.string_cache.intern(text);
            self.pending_overflows.push(PendingOverflow {
                expression: e,
                text,
            });
        }
    }

    /// `value` as GCC's pretty printer spells a constant: decimal if a
    /// 64-bit `long` holds it, and else its magnitude in hexadecimal.
    fn gcc_constant_text(&self, value: Integer) -> &'tu str {
        if value.wide.is_none()
            && (value.signed || value.value >= 0)
            && i64::try_from(value.value).is_ok()
        {
            return format_in!(self.types.tu, "{value}");
        }
        self.wide.hexadecimal(value, self.types.tu)
    }

    /// `value` as Clang spells a constant: decimal, with a `'` between each
    /// three digits.
    fn clang_constant_text(&self, value: Integer) -> &'tu str {
        let digits = format_in!(self.scratch, "{}", self.wide.display(value));
        let (sign, digits) = digits
            .strip_prefix('-')
            .map_or(("", digits), |digits| ("-", digits));
        let mut text = crate::util::bump::ArenaString::new_in(self.types.tu);
        text.push_str(sign);
        for (index, digit) in digits.char_indices() {
            if index != 0 && (digits.len() - index).is_multiple_of(3) {
                text.push('\'');
            }
            text.push(digit);
        }
        text.into_str()
    }

    /// Warns of a shift count or a zero divisor of `op`, at type `ty`, in an
    /// expression whose left operand may be anything; the operands of a
    /// compound assignment count too.
    /// C99: §6.5.7 paragraph 3, p. 85; PDF p. 97; §6.5.5 paragraph 5, p. 82;
    /// PDF p. 94.
    pub(super) fn report_exceptional_operands(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        ty: TypeId,
    ) {
        use BinaryOperator as B;
        if !self.context.configuration.folds_exceptional_constants()
            || self.inhibited_evaluations != 0
            || self.tainted
        {
            return;
        }
        let Some(count) = right.integer else {
            return;
        };
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        // Clang's `DiagRuntimeBehavior`: only code that may run.
        if !gcc && (self.constant_contexts != 0 || self.functions.dead) {
            return;
        }
        let (group, text) = match op {
            | B::Division | B::Modulo if count.is_zero() =>
                if gcc {
                    (DiagnosticGroup::DivByZero, "division by zero")
                } else if self.integer_type(left.ty).is_none() {
                    return;
                } else if op == B::Division {
                    (
                        DiagnosticGroup::DivisionByZero,
                        "division by zero is undefined",
                    )
                } else {
                    (
                        DiagnosticGroup::DivisionByZero,
                        "remainder by zero is undefined",
                    )
                },
            | B::LeftShift | B::RightShift => {
                let Some((bits, _)) = self.integer_type(ty) else {
                    return;
                };
                let (group, gcc_words, clang_words) = if self.negative_integer(count) {
                    (
                        DiagnosticGroup::ShiftCountNegative,
                        if op == B::LeftShift {
                            "left shift count is negative"
                        } else {
                            "right shift count is negative"
                        },
                        "shift count is negative",
                    )
                } else if self.count_reaches_width(count, bits) {
                    (
                        DiagnosticGroup::ShiftCountOverflow,
                        if op == B::LeftShift {
                            "left shift count >= width of type"
                        } else {
                            "right shift count >= width of type"
                        },
                        "shift count >= width of type",
                    )
                } else {
                    return;
                };
                (group, if gcc { gcc_words } else { clang_words })
            },
            | _ => return,
        };
        let text = self.context.string_cache.intern(text);
        self.exceptional_warning(group, text, operator_source(e));
    }

    /// Reports `text` under `group`, which selects its option and severity.
    fn exceptional_warning(
        &mut self,
        group: DiagnosticGroup,
        text: StringCacheId,
        source: SourceVectors,
    ) {
        self.group_diagnostic(
            group,
            SemanticErrorKind::ExceptionalConstant(group, text),
            source,
            None,
            None,
        );
    }

    /// The mark `e` takes from the operands it evaluates: `None` when none
    /// has one. GCC's result is still a constant with an overflow flag only
    /// through arithmetic and conversions; anything else makes it no
    /// constant its parser recognizes. Clang's code folds a condition
    /// away, so a mark on the condition alone does not reach the value.
    /// C99: §6.6 paragraph 3, p. 95; PDF p. 107.
    pub(super) fn carried_exceptional(
        &self,
        e: &'tu Expression<'tu>,
        info: &ExpressionInfo<'tu>,
    ) -> Option<ConstantFolding> {
        use BinaryOperator as B;
        use ExpressionType as E;
        use UnaryOperator as U;
        let mark = |operand: &'tu Expression<'tu>| {
            let folding = self.expression_info(operand).folding;
            folding.exceptional().then_some(folding)
        };
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        let strongest = |marks: [Option<ConstantFolding>; 2]| {
            let [first, second] = marks;
            match (first, second) {
                | (Some(ConstantFolding::Undefined), _) | (_, Some(ConstantFolding::Undefined)) =>
                    Some(ConstantFolding::Undefined),
                | (Some(ConstantFolding::UndefinedQuotient), _)
                | (_, Some(ConstantFolding::UndefinedQuotient)) =>
                    Some(ConstantFolding::UndefinedQuotient),
                | (Some(folding), _) | (None, Some(folding)) => Some(folding),
                | (None, None) => None,
            }
        };
        // A comparison, a logical operator or `?:` gives GCC's result a
        // fresh tree that no longer reads as a constant.
        let opaque = |folding: Option<ConstantFolding>| {
            folding.map(|folding| {
                if gcc {
                    ConstantFolding::Undefined
                } else {
                    folding
                }
            })
        };
        let carried = match e.kind {
            | E::Unary {
                operator,
                operand_expression,
            } => match operator {
                | U::Plus | U::Minus | U::BitwiseNot | U::Extension => mark(operand_expression),
                | U::LogicalNot => opaque(mark(operand_expression)),
                | _ => None,
            },
            | E::Cast {
                operand_expression, ..
            } => mark(operand_expression),
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => match operator {
                | B::LogicalAnd | B::LogicalOr => {
                    let truth = Self::constant_truth(self.expression_info(left_expression));
                    let skipped = truth == Some(operator == B::LogicalOr);
                    let right = if skipped {
                        None
                    } else {
                        mark(right_expression)
                    };
                    if !gcc && right.is_none() {
                        mark(left_expression).map(|_| ConstantFolding::Wrapped)
                    } else {
                        opaque(strongest([mark(left_expression), right]))
                    }
                },
                | B::Multiplication
                | B::Division
                | B::Modulo
                | B::Addition
                | B::Subtraction
                | B::LeftShift
                | B::RightShift
                | B::BitwiseAnd
                | B::BitwiseXor
                | B::BitwiseOr => strongest([mark(left_expression), mark(right_expression)]),
                | B::LessThan
                | B::LessThanOrEqual
                | B::GreaterThan
                | B::GreaterThanOrEqual
                | B::Equal
                | B::NotEqual => opaque(strongest([mark(left_expression), mark(right_expression)])),
                | _ => None,
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                let condition = mark(c.condition_expression);
                let selected = Self::constant_truth(self.expression_info(c.condition_expression))
                    .and_then(|truth| {
                        mark(if truth {
                            c.then_expression
                        } else {
                            c.else_expression
                        })
                    });
                if !gcc && selected.is_none() {
                    condition.map(|_| ConstantFolding::Wrapped)
                } else {
                    opaque(strongest([condition, selected]))
                }
            },
            | _ => None,
        };
        // What the operands carry never weakens this operator's own mark.
        strongest([info.folding.exceptional().then_some(info.folding), carried])
    }

    /// Pushes the value a constant context reads from `e`, whose result
    /// `info` has a mark, and reports what the context finds wrong with it.
    /// C99: §6.6 paragraphs 4 and 6, p. 95; PDF p. 107; §6.7.5.2 paragraph 4,
    /// pp. 116-117; PDF pp. 128-129.
    pub(super) fn evaluate_exceptional(
        &mut self,
        e: &'tu Expression<'tu>,
        info: ExpressionInfo<'tu>,
    ) {
        let gcc = self.exceptional_rules() == RuleSet::Gcc;
        let Some(value) = info.integer else {
            self.integers.push(None);
            return;
        };
        if self.runtime_bound {
            // GCC keeps a wrapped constant as such, which a negative size
            // refuses; anything else is variable length. Clang folds what
            // its evaluator can and `ArrayDone` warns.
            let folds = if gcc {
                info.folding == ConstantFolding::Wrapped && self.negative_integer(value)
            } else {
                info.folding != ConstantFolding::UndefinedQuotient
            };
            self.integers.push(folds.then_some(value));
            return;
        }
        if gcc && info.folding == ConstantFolding::Wrapped {
            self.group_diagnostic(
                DiagnosticGroup::PedanticOverflow,
                SemanticErrorKind::PedanticOverflow,
                e.source_vectors,
                None,
                None,
            );
        } else if !self.tainted {
            self.report_folded_constant(e);
        }
        self.integers.push(Some(value));
    }

    /// Reports what `-pedantic` says of a value folded where an integer
    /// constant expression is required, in the context's words for GCC.
    /// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
    pub(super) fn report_folded_constant(&mut self, e: &'tu Expression<'tu>) {
        if self.exceptional_rules() == RuleSet::Gcc {
            let text = match self.constant_site {
                | ConstantSite::Enumerator(name) => format_in!(
                    self.scratch,
                    "enumerator value for '{}' is not an integer constant expression",
                    self.context.string_cache.at(name)
                ),
                | ConstantSite::CaseLabel => "case label is not an integer constant expression",
                | ConstantSite::StaticAssertion =>
                    "expression in static assertion is not an integer constant expression",
                | ConstantSite::BitFieldWidth(Some(name)) => format_in!(
                    self.scratch,
                    "bit-field '{}' width not an integer constant expression",
                    self.context.string_cache.at(name)
                ),
                | ConstantSite::BitFieldWidth(None) =>
                    "bit-field width not an integer constant expression",
                | ConstantSite::Other => "folded integer constant expression",
            };
            self.report_pedantic_constant(text, e.source_vectors);
            return;
        }
        // Clang checks a case label's overflow, but not its form.
        if self.constant_site == ConstantSite::CaseLabel
            && self.expression_info(e).folding.exceptional()
        {
            return;
        }
        let configuration = self.context.configuration_at(e.source_vectors);
        if configuration.extension_policy() != ExtensionPolicy::Allow
            || configuration.diagnostic_requested(DiagnosticGroup::GnuFoldingConstant)
        {
            self.error(
                SemanticErrorKind::FoldedIntegerConstant,
                e.source_vectors,
                None,
                None,
            );
        }
    }

    /// What GCC's `-pedantic` says of the static initializer `e` the mark on
    /// its result makes no constant expression: an overflow if arithmetic
    /// wrapped it.
    /// C99: §6.6 paragraphs 4 and 7, p. 95; PDF p. 107.
    pub(super) fn report_exceptional_initializer(
        &mut self,
        e: &'tu Expression<'tu>,
        info: ExpressionInfo<'tu>,
    ) {
        if self.exceptional_rules() != RuleSet::Gcc || !info.folding.exceptional() {
            return;
        }
        if info.folding == ConstantFolding::Wrapped {
            self.group_diagnostic(
                DiagnosticGroup::PedanticOverflow,
                SemanticErrorKind::PedanticOverflow,
                e.source_vectors,
                None,
                None,
            );
        } else if info.integer.is_some() {
            self.report_pedantic_constant(
                "initializer element is not a constant expression",
                e.source_vectors,
            );
        }
    }

    /// Reports `text` as `-pedantic` reports an extension.
    pub(super) fn report_pedantic_constant(&mut self, text: &str, source: SourceVectors) {
        let text = self.context.string_cache.intern(text);
        self.extension_error(
            crate::configuration::Feature::ConstantFolding,
            super::DiagnosticPolicy::Extension,
            SemanticErrorKind::PedanticConstant(text),
            source,
            None,
        );
    }

    /// The value of the alignment specifier's expression `e`. GCC keeps a
    /// wrapped value as a constant and refuses what it no longer reads as
    /// one; Clang reads the bits its evaluator gives, unsigned, unless the
    /// evaluation divided a minimum by -1.
    /// C11: §6.7.5 paragraph 3, p. 127; PDF p. 145.
    pub(super) fn alignment_value(&self, e: &'tu Expression<'tu>) -> Option<u64> {
        let info = self.expression_info(e);
        let value = info.integer?;
        if !info.folding.exceptional() {
            return info.ice.then(|| value.to_u64()).flatten();
        }
        match (self.exceptional_rules(), info.folding) {
            | (RuleSet::Gcc, ConstantFolding::Wrapped) => value.to_u64(),
            | (RuleSet::Clang, ConstantFolding::Undefined | ConstantFolding::Wrapped) =>
                value.cast(value.bits.min(64), false).to_u64(),
            | _ => None,
        }
    }

    /// [`Self::alignment_value`] for an `aligned` attribute, which GCC folds:
    /// it takes any value that remains.
    /// GNU extension: GCC manual, "Common Variable Attributes".
    pub(super) fn attribute_alignment_value(&self, e: &'tu Expression<'tu>) -> Option<u64> {
        let info = self.expression_info(e);
        if info.folding.exceptional() && self.exceptional_rules() == RuleSet::Gcc {
            return info.integer.and_then(Integer::to_u64);
        }
        self.alignment_value(e)
    }

    /// Whose rules decide exceptional constants.
    fn exceptional_rules(&self) -> RuleSet {
        self.context.configuration.compiler_identity().rules()
    }

    /// Clang's `Sema::CheckForIntOverflow` for the finished full expression
    /// `root`: reports the pending overflows from `start` on that one
    /// evaluation of it reaches. The check strips parentheses and casts,
    /// evaluates a unary or binary operator, and otherwise only looks into a
    /// call's arguments and a subscript's index, so a `?:` at the top reports
    /// nothing. A constant context evaluates what it is given.
    /// Clang: `Sema::CheckForIntOverflow` in `SemaChecking.cpp`.
    pub(super) fn finish_full_expression(&mut self, root: &'tu Expression<'tu>, start: usize) {
        if self.pending_overflows.len() <= start {
            return;
        }
        let pending = self
            .scratch
            .alloc_slice_copy(&self.pending_overflows[start..]);
        self.pending_overflows.truncate(start);
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((root, self.constant_contexts != 0));
        while let Some((e, evaluating)) = work.pop() {
            self.overflow_walk_step(e, evaluating, pending, &mut work);
        }
    }

    /// One step of [`Self::finish_full_expression`]'s walk.
    fn overflow_walk_step(
        &mut self,
        e: &'tu Expression<'tu>,
        evaluating: bool,
        pending: &[PendingOverflow<'tu>],
        work: &mut ArenaVec<'_, (&'tu Expression<'tu>, bool)>,
    ) {
        use BinaryOperator as B;
        use ExpressionType as E;
        match e.kind {
            | E::Parenthesized { expression } => work.push((expression, evaluating)),
            | E::Cast {
                operand_expression, ..
            } => work.push((operand_expression, evaluating)),
            | E::Generic(_) | E::Builtin(_)
                if self
                    .expression_info(e)
                    .selected_expression
                    .is_some_and(|selected| !std::ptr::eq(selected, e)) =>
                if let Some(selected) = self.expression_info(e).selected_expression {
                    work.push((selected, evaluating));
                },
            | E::Builtin(builtin) if !evaluating =>
                for operand in builtin.operands.iter().rev() {
                    if let super::SyntaxOperand::Expression(operand) = operand {
                        work.push((operand, false));
                    }
                },
            | E::Call { arguments, .. } if !evaluating =>
                for argument in arguments.as_slice().iter().rev() {
                    work.push((argument, false));
                },
            | E::Unary {
                operand_expression, ..
            } => {
                self.report_pending(e, pending);
                work.push((operand_expression, true));
            },
            | E::Binary {
                operator: B::Subscript,
                right_expression,
                left_expression,
            } => {
                work.push((right_expression, evaluating));
                if evaluating {
                    work.push((left_expression, true));
                }
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                self.report_pending(e, pending);
                let truth = Self::constant_truth(self.expression_info(left_expression));
                let skipped = match operator {
                    | B::LogicalAnd => truth == Some(false),
                    | B::LogicalOr => truth == Some(true),
                    | _ => false,
                };
                if !skipped {
                    work.push((right_expression, true));
                }
                work.push((left_expression, true));
            },
            | E::Conditional(c) | E::OmittedConditional(c) if evaluating => {
                match Self::constant_truth(self.expression_info(c.condition_expression)) {
                    | Some(true) => work.push((c.then_expression, true)),
                    | Some(false) => work.push((c.else_expression, true)),
                    | None => {
                        work.push((c.else_expression, true));
                        work.push((c.then_expression, true));
                    },
                }
                work.push((c.condition_expression, true));
            },
            | _ => {},
        }
    }

    /// Reports the pending overflow of the operator `e`, if it has one.
    fn report_pending(&mut self, e: &'tu Expression<'tu>, pending: &[PendingOverflow<'tu>]) {
        if let Some(overflow) = pending
            .iter()
            .find(|overflow| std::ptr::eq(overflow.expression, e))
        {
            self.exceptional_warning(
                DiagnosticGroup::IntegerOverflow,
                overflow.text,
                operator_source(e),
            );
        }
    }
}

/// Where both compilers point at an operator's warning.
fn operator_source(e: &Expression<'_>) -> SourceVectors {
    e.operator_source_vectors.unwrap_or(e.source_vectors)
}
