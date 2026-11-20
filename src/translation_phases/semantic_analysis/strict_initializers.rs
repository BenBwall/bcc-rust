//! The NaNs of an automatic array or structure that Clang initializes under
//! strict exception behavior (`-ftrapping-math`, or where
//! `#pragma STDC FENV_ACCESS ON` is in effect). Clang copies such an
//! object's initial value from a constant image when its evaluator folds the
//! whole initializer (`EmitAutoVarInit`), with the positive NaN `APFloat`
//! gives an invalid operation; otherwise its code computes every member at
//! run time, where the hardware's NaN has the sign bit set. Under strict
//! exception behavior that evaluator refuses an operation or conversion
//! whose result is not a NaN but raises an exception, an unordered
//! comparison, and a builtin call other than the classification and sign
//! operations (`checkFloatingPointResult`). Semantic analysis folds the
//! initializer with the hardware's NaN, as for any other automatic object
//! ([`NanSource::Runtime`]); [`Analyzer::settle_strict_initializer`] then
//! folds its invalid operations again with the compiler's NaN when Clang's
//! evaluator folds the whole initializer. A case it cannot follow keeps the
//! run-time NaN. C99: §F.7.5 paragraph 1, p. 450; PDF p. 462 (automatic
//! initialization at execution time); §7.6.1 paragraph 2, p. 189; PDF
//! p. 201.

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    Duration,
    constants::NanSource,
    expressions::{
        ConstantClass,
        ConversionKind,
        ExpressionInfo,
    },
    facts::InitOperation,
    types::{
        TypeId,
        TypeKind,
    },
};
use crate::{
    float_parsing::LongDouble,
    translation_phases::parsing::{
        declaration_syntax::Initializer,
        syntax::{
            BinaryOperator,
            Expression,
            ExpressionType,
            StatementType,
            UnaryOperator,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
    },
};

/// The builtins Clang's evaluator folds under strict exception behavior,
/// which raise no exception: the constants, the sign operations, `fmax`,
/// `fmin`, `__builtin_complex`, `__builtin_expect`, and the classification
/// and quiet comparison macros (C99 §7.12.3, pp. 216-217;
/// PDF pp. 228-229; §7.12.14, pp. 240-242; PDF pp. 252-254).
const STRICT_BUILTINS: &[&str] = &[
    "__builtin_inf",
    "__builtin_inff",
    "__builtin_infl",
    "__builtin_huge_val",
    "__builtin_huge_valf",
    "__builtin_huge_vall",
    "__builtin_nan",
    "__builtin_nanf",
    "__builtin_nanl",
    "__builtin_nans",
    "__builtin_nansf",
    "__builtin_nansl",
    "__builtin_copysign",
    "__builtin_copysignf",
    "__builtin_copysignl",
    "__builtin_fabs",
    "__builtin_fabsf",
    "__builtin_fabsl",
    "__builtin_isnan",
    "__builtin_isinf",
    "__builtin_isfinite",
    "__builtin_isnormal",
    "__builtin_isinf_sign",
    "__builtin_fpclassify",
    "__builtin_issignaling",
    "__builtin_signbit",
    "__builtin_signbitf",
    "__builtin_signbitl",
    "__builtin_isgreater",
    "__builtin_isgreaterequal",
    "__builtin_isless",
    "__builtin_islessequal",
    "__builtin_islessgreater",
    "__builtin_isunordered",
    "__builtin_fmax",
    "__builtin_fmaxf",
    "__builtin_fmaxl",
    "__builtin_fmin",
    "__builtin_fminf",
    "__builtin_fminl",
    "__builtin_complex",
    "__builtin_expect",
];

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Starts the initializer of `binding`: under Clang's strict exception
    /// behavior, an automatic array or structure's initializer records
    /// where its conversions begin, for
    /// [`Self::settle_strict_initializer`].
    pub(super) fn begin_strict_initializer(&mut self, binding: usize) {
        let data = self.bindings[binding];
        if self.functions.current.is_some()
            && self.context.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang
            && self.clang_strict_exceptions()
            && data.duration == Duration::Automatic
            && matches!(
                self.types.nodes[data.ty.index],
                TypeKind::Array(..) | TypeKind::Tag(_)
            )
            && self.aggregate(data.ty)
        {
            self.functions
                .strict_initializers
                .push((binding, self.conversions.len()));
        }
    }

    /// Folds the invalid operations of `binding`'s finished initializer
    /// with the compiler's NaN when Clang's evaluator folds the whole
    /// initializer under strict exception behavior; they keep the
    /// hardware's NaN, as Clang's code computes them, when it does not, or
    /// when a value derived from one is one this cannot fold again.
    pub(super) fn settle_strict_initializer(
        &mut self,
        binding: usize,
        initializer: &'tu Initializer<'tu>,
    ) {
        let Some(&(owner, conversions)) = self.functions.strict_initializers.last() else {
            return;
        };
        if owner != binding {
            return;
        }
        _ = self.functions.strict_initializers.pop();
        let mut nodes = ArenaVec::new_in(self.scratch);
        let mut initializers = ArenaVec::new_in(self.scratch);
        if !self.strict_initializer_nodes(initializer, &mut nodes, &mut initializers)
            || !nodes.iter().any(|&e| self.invalid_operation(e))
            || !self.constant_automatic_aggregate(initializer)
            || !nodes.iter().all(|&e| self.strict_operation(e))
            || !initializers.iter().all(|&i| self.strict_stores(i))
            || !self.conversions[conversions..].iter().all(|conversion| {
                !matches!(
                    conversion.kind,
                    ConversionKind::Arithmetic | ConversionKind::Assignment
                ) || self
                    .strict_conversion(self.expression_info(conversion.expression), conversion.ty)
            })
        {
            return;
        }
        // The nodes whose value derives from an invalid operation's NaN, in
        // the order they were collected, which puts operands first.
        let mut derived = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut order = ArenaVec::new_in(self.scratch);
        for &e in &nodes {
            let is_derived = |child: &'tu Expression<'tu>| {
                derived.contains_key(&std::ptr::from_ref(child).addr())
            };
            let from_nan = match e.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Cast {
                    operand_expression: expression,
                    ..
                } => is_derived(expression),
                | ExpressionType::Unary {
                    operand_expression, ..
                } => is_derived(operand_expression),
                | ExpressionType::Binary {
                    operator: BinaryOperator::Comma,
                    right_expression,
                    ..
                } => is_derived(right_expression),
                | ExpressionType::Binary {
                    left_expression,
                    right_expression,
                    ..
                } =>
                    self.invalid_operation(e)
                        || is_derived(left_expression)
                        || is_derived(right_expression),
                | ExpressionType::Conditional(c) =>
                    is_derived(c.then_expression) || is_derived(c.else_expression),
                | _ => {
                    let mut any = false;
                    super::emission::expression_children(self.scratch, e, |child| {
                        any |= is_derived(child);
                    });
                    any
                },
            };
            if from_nan {
                if !self.refoldable(e) {
                    return;
                }
                _ = derived.insert(std::ptr::from_ref(e).addr(), ());
                order.push(e);
            }
        }
        for &e in &order {
            self.refold(e);
        }
    }

    /// Collects the expressions `initializer` evaluates, each after its
    /// operands, and the initializers of its compound literals; false for
    /// an expression this does not follow.
    fn strict_initializer_nodes(
        &self,
        initializer: &'tu Initializer<'tu>,
        nodes: &mut ArenaVec<'_, &'tu Expression<'tu>>,
        initializers: &mut ArenaVec<'_, &'tu Initializer<'tu>>,
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push(initializer);
        while let Some(initializer) = pending.pop() {
            let Some(plan) = self.initializers.plan(initializer) else {
                return false;
            };
            initializers.push(initializer);
            for value in plan.values.iter().rev() {
                work.push((value.expression, false));
            }
            while let Some((e, done)) = work.pop() {
                if done {
                    nodes.push(e);
                    continue;
                }
                work.push((e, true));
                let info = self.expression_info(e);
                let mut push = |child| work.push((child, false));
                match e.kind {
                    // Unevaluated operands. C99: §6.5.3.4 paragraph 2, p. 80;
                    // PDF p. 92.
                    | ExpressionType::SizeofExpr(_)
                    | ExpressionType::SizeofType(_)
                    | ExpressionType::AlignofExpr(_)
                    | ExpressionType::Countof(_) => {},
                    | ExpressionType::Conditional(c) => {
                        let condition = self.expression_info(c.condition_expression);
                        match Self::constant_truth(condition) {
                            | Some(true) => push(c.then_expression),
                            | Some(false) => push(c.else_expression),
                            | None => {
                                push(c.else_expression);
                                push(c.then_expression);
                            },
                        }
                        push(c.condition_expression);
                    },
                    | ExpressionType::Generic(_) | ExpressionType::Builtin(_)
                        if info.selected_expression.is_some() =>
                        push(info.selected_expression.expect("checked")),
                    | ExpressionType::StatementExpression(block) => {
                        if !matches!(block.kind, StatementType::Compound { items } if items.len() == 1)
                        {
                            return false;
                        }
                        let Some((_, tail)) = self.statement_expression_tail(block) else {
                            return false;
                        };
                        push(tail);
                    },
                    | ExpressionType::CompoundLiteral { initializer, .. } =>
                        pending.push(initializer),
                    | _ => {
                        let mut children = ArenaVec::new_in(self.scratch);
                        super::emission::expression_children(self.scratch, e, |child| {
                            children.push(child);
                        });
                        for &child in children.iter().rev() {
                            push(child);
                        }
                    },
                }
            }
        }
        true
    }

    /// Whether `e` is an invalid operation, whose NaN the hardware and the
    /// compiler give with different signs: a floating `+`, `-`, `*` or `/`
    /// that makes a NaN of operands that are none.
    fn invalid_operation(&self, e: &'tu Expression<'tu>) -> bool {
        let ExpressionType::Binary {
            operator:
                BinaryOperator::Addition
                | BinaryOperator::Subtraction
                | BinaryOperator::Multiplication
                | BinaryOperator::Division,
            left_expression,
            right_expression,
        } = e.kind
        else {
            return false;
        };
        let info = self.expression_info(e);
        let Some(value) = info.floating() else {
            return false;
        };
        let nan = |value: super::Floating| value.real.is_nan() || value.imag.is_nan();
        nan(value)
            && [left_expression, right_expression].iter().all(|&operand| {
                self.converted_floating(self.expression_info(operand), info.ty)
                    .is_some_and(|value| !nan(value))
            })
    }

    /// Whether Clang's evaluator folds `e` under strict exception behavior,
    /// given that its operands fold: no `_Float16`, `__bf16`, binary128,
    /// complex integer or vector value, no arithmetic whose result is not a
    /// NaN but raises an exception, no unordered comparison, no conversion
    /// that rounds or overflows, and no builtin call outside
    /// [`STRICT_BUILTINS`].
    fn strict_operation(&self, e: &'tu Expression<'tu>) -> bool {
        use BinaryOperator as B;
        let info = self.expression_info(e);
        // Clang's evaluator gives an address converted to an integer no
        // value.
        if !self.strict_type(info.ty)
            || (info.constant == ConstantClass::Address && self.integer_type(info.ty).is_some())
        {
            return false;
        }
        match e.kind {
            | ExpressionType::Binary {
                operator: op @ (B::Addition | B::Subtraction | B::Multiplication | B::Division),
                left_expression,
                right_expression,
            } if info.floating().is_some() => {
                let left = self.expression_info(left_expression);
                let right = self.expression_info(right_expression);
                self.strict_arithmetic(op, left, right, info)
            },
            | ExpressionType::Binary {
                operator:
                    B::LessThan
                    | B::GreaterThan
                    | B::LessThanOrEqual
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual,
                left_expression,
                right_expression,
            } => [left_expression, right_expression].iter().all(|&operand| {
                self.expression_info(operand)
                    .floating()
                    .is_none_or(|value| !value.real.is_nan() && !value.imag.is_nan())
            }),
            | ExpressionType::Cast {
                operand_expression, ..
            } => self.strict_conversion(self.expression_info(operand_expression), info.ty),
            | ExpressionType::Call {
                function_expression,
                ..
            } => {
                let callee = self.selected_designator(function_expression);
                self.expression_info(callee)
                    .binding
                    .and_then(|binding| self.bindings[binding].origin.descriptor())
                    .is_some_and(|descriptor| STRICT_BUILTINS.contains(&descriptor.spelling))
            },
            | ExpressionType::Unary {
                operator: UnaryOperator::Real | UnaryOperator::Imag,
                ..
            } => false,
            // An array compound literal in a function has automatic storage
            // (C99 §6.5.2.5 paragraph 6, p. 76; PDF p. 88), so the pointer it
            // decays to is no constant.
            | ExpressionType::CompoundLiteral { .. } => !matches!(
                self.types.nodes[self.types.non_atomic(info.ty).index],
                TypeKind::Array(..)
            ),
            | _ => true,
        }
    }

    /// Whether this module follows values of type `ty`.
    fn strict_type(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(scalar) =>
                !scalar.complex_integer()
                    && !self.half_type(ty)
                    && !self.binary128_type(ty)
                    && (!scalar.floating() || self.floating_precision(scalar).is_some()),
            | TypeKind::Vector { .. } => false,
            | _ => true,
        }
    }

    /// Whether `left op right`, which folds to `result`, raises no exception
    /// in Clang's evaluator, or gives a NaN, which it folds regardless. A
    /// sum or difference raises none, and neither does a product or
    /// quotient of two complex values, or a real dividend by a complex
    /// divisor; a product with one real operand, and a complex quotient by
    /// a real divisor, check each part.
    fn strict_arithmetic(
        &self,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        result: ExpressionInfo<'tu>,
    ) -> bool {
        use BinaryOperator as B;
        let (Some(l), Some(r), Some(value)) = (
            self.converted_floating(left, result.ty),
            self.converted_floating(right, result.ty),
            result.floating(),
        ) else {
            return false;
        };
        let lt = self.types.non_atomic(left.ty);
        let rt = self.types.non_atomic(right.ty);
        match (self.complex_type(lt), self.complex_type(rt)) {
            | (false, false) => exact_operation(op, l.real, r.real, value.real),
            | (true, true) => true,
            | (_, _) if matches!(op, B::Addition | B::Subtraction) => true,
            | (true, false) if op == B::Multiplication || op == B::Division =>
                exact_operation(op, l.real, r.real, value.real)
                    && exact_operation(op, l.imag, r.real, value.imag),
            | (false, true) if op == B::Multiplication =>
                exact_operation(op, l.real, r.real, value.real)
                    && exact_operation(op, l.real, r.imag, value.imag),
            | _ => true,
        }
    }

    /// Whether every value `initializer` stores converts exactly to its
    /// subobject's type.
    fn strict_stores(&self, initializer: &'tu Initializer<'tu>) -> bool {
        let Some(plan) = self.initializers.plan(initializer) else {
            return false;
        };
        plan.operations.iter().all(|operation| match *operation {
            | InitOperation::Store {
                value, destination, ..
            } => self.strict_conversion(
                self.expression_info(plan.values[value.0 as usize].expression),
                destination.ty,
            ),
            | _ => true,
        })
    }

    /// Whether Clang's evaluator converts the constant `info` to `ty`
    /// under strict exception behavior: a conversion to a floating type
    /// that neither rounds nor overflows, or one to another type.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5
    /// paragraph 2, p. 44; PDF p. 56.
    fn strict_conversion(&self, info: ExpressionInfo<'tu>, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return true;
        };
        if !scalar.floating() {
            return true;
        }
        if !self.strict_type(ty) || !self.strict_type(info.ty) || info.binary128().is_some() {
            return false;
        }
        let Some(converted) = self.converted_floating(info, ty) else {
            // Not a constant: the constant aggregate check refuses it.
            return info.integer.is_none() && info.floating().is_none();
        };
        if let Some(value) = info.floating() {
            exact_conversion(value.real, converted.real)
                && (!scalar.complex() || exact_conversion(value.imag, converted.imag))
        } else if let Some(integer) = info.integer {
            integer
                .to_i128()
                .is_some_and(|integer| Exact::of(converted.real) == Some(Exact::integer(integer)))
        } else {
            false
        }
    }

    /// Whether [`Self::refold`] folds `e` again.
    fn refoldable(&self, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        let ty = self.types.non_atomic(info.ty);
        if info.floating().is_none()
            || !self.strict_type(ty)
            || !matches!(self.types.nodes[ty.index], TypeKind::Scalar(s) if s.floating())
        {
            return false;
        }
        match e.kind {
            | ExpressionType::Parenthesized { .. }
            | ExpressionType::Cast { .. }
            | ExpressionType::Conditional(_) => true,
            | ExpressionType::Unary { operator, .. } => matches!(
                operator,
                UnaryOperator::Plus | UnaryOperator::Minus | UnaryOperator::BitwiseNot
            ),
            | ExpressionType::Binary { operator, .. } => matches!(
                operator,
                BinaryOperator::Comma
                    | BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
            ),
            | _ => false,
        }
    }

    /// Folds `e` again from its operands' values, as its analysis folded it
    /// but with the compiler's NaN for an invalid operation.
    fn refold(&mut self, e: &'tu Expression<'tu>) {
        let info = self.expression_info(e);
        let value = match e.kind {
            | ExpressionType::Parenthesized { expression } =>
                self.expression_info(expression).floating(),
            | ExpressionType::Cast {
                operand_expression, ..
            } => self.converted_floating(self.expression_info(operand_expression), info.ty),
            | ExpressionType::Conditional(c) => {
                let condition = self.expression_info(c.condition_expression);
                let selected = if Self::constant_truth(condition).expect("folded") {
                    c.then_expression
                } else {
                    c.else_expression
                };
                self.converted_floating(self.expression_info(selected), info.ty)
            },
            | ExpressionType::Unary {
                operator,
                operand_expression,
            } => self.unary_floating(
                operator,
                operator == UnaryOperator::BitwiseNot,
                self.expression_info(operand_expression),
                info.ty,
            ),
            | ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                right_expression,
                ..
            } => self.expression_info(right_expression).floating(),
            | ExpressionType::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                let left = self.expression_info(left_expression);
                let right = self.expression_info(right_expression);
                self.converted_floating(left, info.ty)
                    .zip(self.converted_floating(right, info.ty))
                    .and_then(|(l, r)| {
                        self.floating_operation(
                            operator,
                            [l, r],
                            [
                                self.types.non_atomic(left.ty),
                                self.types.non_atomic(right.ty),
                            ],
                            info.ty,
                            NanSource::Compiler,
                        )
                    })
                    .and_then(|value| self.round_floating(value, info.ty))
            },
            | _ => unreachable!("checked by refoldable"),
        };
        let index = self.expression_indices[&std::ptr::from_ref(e).addr()];
        self.expressions[index].set_floating(value);
    }
}

/// Whether `left op right`, which IEC 60559 arithmetic gives as `result`,
/// raises no exception or gives a NaN: an infinite operand raises none, an
/// infinite result of finite operands overflowed or divided by zero, and a
/// finite one raised "inexact", or "underflow", unless it is the exact
/// value. C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.
fn exact_operation(
    op: BinaryOperator,
    left: LongDouble,
    right: LongDouble,
    result: LongDouble,
) -> bool {
    use BinaryOperator as B;
    if result.is_nan() || left.is_infinite() || right.is_infinite() {
        return true;
    }
    if result.is_infinite() {
        return false;
    }
    let (Some(a), Some(b), Some(c)) = (Exact::of(left), Exact::of(right), Exact::of(result)) else {
        return false;
    };
    match op {
        | B::Addition => a.sum(b) == Some(c),
        | B::Subtraction => a.sum(b.negated()) == Some(c),
        | B::Multiplication => a.product(b) == Some(c),
        | B::Division => c.product(b) == Some(a),
        | _ => false,
    }
}

/// Whether converting `value` gives `converted` without raising an
/// exception: a NaN or an infinity converts exactly, and a finite value
/// must keep its value.
fn exact_conversion(value: LongDouble, converted: LongDouble) -> bool {
    value.is_nan()
        || value.is_infinite()
        || Exact::of(value).is_some_and(|value| Exact::of(converted) == Some(value))
}

/// A finite value as `mantissa × 2^exponent`, negated if `negative`, with an
/// odd mantissa or a zero one, so equal values compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Exact {
    negative: bool,
    mantissa: u128,
    exponent: i32,
}

impl Exact {
    /// A finite x87 carrier value; `None` for an infinity, a NaN or another
    /// carrier.
    fn of(value: LongDouble) -> Option<Self> {
        let (negative, exponent, significand) = value.x87_parts()?;
        if exponent == 0x7FFF {
            return None;
        }
        // An x87 denormal has the smallest normal exponent.
        let exponent = i32::from(exponent.max(1)) - 16383 - 63;
        Some(Self::normalized(
            negative,
            u128::from(significand),
            exponent,
        ))
    }

    fn integer(value: i128) -> Self {
        Self::normalized(value < 0, value.unsigned_abs(), 0)
    }

    fn normalized(negative: bool, mantissa: u128, exponent: i32) -> Self {
        if mantissa == 0 {
            return Self {
                negative: false,
                mantissa: 0,
                exponent: 0,
            };
        }
        let shift = mantissa.trailing_zeros();
        Self {
            negative,
            mantissa: mantissa >> shift,
            exponent: exponent + shift.cast_signed(),
        }
    }

    const fn negated(self) -> Self {
        Self {
            negative: !self.negative,
            ..self
        }
    }

    /// The exact product, of two carrier values; `None` if it is wider
    /// than 128 bits.
    fn product(self, other: Self) -> Option<Self> {
        Some(Self::normalized(
            self.negative != other.negative,
            self.mantissa.checked_mul(other.mantissa)?,
            self.exponent.checked_add(other.exponent)?,
        ))
    }

    /// The exact sum; `None` if it spans more than 126 bits, which no
    /// carrier value does.
    fn sum(self, other: Self) -> Option<Self> {
        if self.mantissa == 0 {
            return Some(other);
        }
        if other.mantissa == 0 {
            return Some(self);
        }
        let low = self.exponent.min(other.exponent);
        let aligned = |value: Self| {
            let shift = u32::try_from(value.exponent - low).ok()?;
            if shift + value.mantissa.bit_width() > 126 {
                return None;
            }
            let magnitude = i128::try_from(value.mantissa << shift).ok()?;
            Some(if value.negative {
                -magnitude
            } else {
                magnitude
            })
        };
        let total = aligned(self)? + aligned(other)?;
        Some(Self::normalized(total < 0, total.unsigned_abs(), low))
    }
}
