//! Deferred call predicates substitute parameters by binding ordinal, then
//! evaluate immutable typed syntax on an explicit stack. Unknown runtime
//! values disable `enable_if` candidates and do not fire `diagnose_if`.
//! C99: §6.6p3-6, p. 95; PDF p. 107; §6.5.13-§6.5.15, pp. 89-90;
//! PDF pp. 101-102 (arithmetic and short circuit evaluation).
//! C99: §6.3.1.2p1 and §6.3.1.4p1, p. 43; PDF p. 55
//! (Boolean normalization and floating-to-integer parameter conversions).
//! C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57 (common real precision
//! preserves the real or complex domain of each operand).
//! <https://clang.llvm.org/docs/AttributeReference.html#diagnose-if>

use super::super::{
    Analyzer,
    BinaryOperator,
    Expression,
    ExpressionType,
    Floating,
    Integer,
    ObjectSize,
    Scalar,
    TypeId,
    TypeKind,
    UnaryOperator,
};
use crate::{
    builtins::BuiltinOperation,
    util::bump::ArenaVec,
};

enum Step<'tu> {
    Eval(&'tu Expression<'tu>),
    Unary(UnaryOperator),
    Binary(BinaryOperator, TypeId, u8),
    Logical(BinaryOperator, &'tu Expression<'tu>),
    Conditional(&'tu Expression<'tu>, &'tu Expression<'tu>, TypeId),
    Cast(TypeId),
    Constant,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn equivalent_predicate(
        &self,
        a: &super::super::CallPredicate<'tu>,
        b: &super::super::CallPredicate<'tu>,
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((a.expression, b.expression));
        while let Some((left, right)) = work.pop() {
            let left = self.selected_designator(left);
            let right = self.selected_designator(right);
            let li = self.expression_info(left);
            let ri = self.expression_info(right);
            match (left.kind, right.kind) {
                | (ExpressionType::Identifier(ln), ExpressionType::Identifier(rn)) => {
                    let lp = a
                        .parameters
                        .iter()
                        .position(|&p| p.is_some() && p == li.binding);
                    let rp = b
                        .parameters
                        .iter()
                        .position(|&p| p.is_some() && p == ri.binding);
                    if lp != rp || (lp.is_none() && ln.name != rn.name) {
                        return false;
                    }
                },
                | (ExpressionType::Constant(_), ExpressionType::Constant(_))
                | (ExpressionType::Boolean(_), ExpressionType::Boolean(_)) => {
                    if li.ty != ri.ty || li.integer != ri.integer {
                        return false;
                    }
                },
                | (
                    ExpressionType::Unary {
                        operator: lo,
                        operand_expression: le,
                    },
                    ExpressionType::Unary {
                        operator: ro,
                        operand_expression: re,
                    },
                ) if lo == ro => work.push((le, re)),
                | (
                    ExpressionType::Binary {
                        operator: lo,
                        left_expression: ll,
                        right_expression: lr,
                    },
                    ExpressionType::Binary {
                        operator: ro,
                        left_expression: rl,
                        right_expression: rr,
                    },
                ) if lo == ro => {
                    work.push((ll, rl));
                    work.push((lr, rr));
                },
                | (ExpressionType::Conditional(l), ExpressionType::Conditional(r)) => {
                    work.push((l.condition_expression, r.condition_expression));
                    work.push((l.then_expression, r.then_expression));
                    work.push((l.else_expression, r.else_expression));
                },
                | (
                    ExpressionType::Cast {
                        operand_expression: l,
                        ..
                    },
                    ExpressionType::Cast {
                        operand_expression: r,
                        ..
                    },
                ) if li.ty == ri.ty => work.push((l, r)),
                | (
                    ExpressionType::Call {
                        function_expression: lf,
                        arguments: la,
                    },
                    ExpressionType::Call {
                        function_expression: rf,
                        arguments: ra,
                    },
                ) if la.len() == ra.len() => {
                    work.push((lf, rf));
                    work.extend(la.iter().copied().zip(ra.iter().copied()));
                },
                | _ => return false,
            }
        }
        true
    }

    pub(super) fn predicate_value(
        &self,
        parameters: &[Option<usize>],
        expression: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) -> Option<Integer> {
        self.predicate_value_for(parameters, expression, arguments, None)
    }

    /// Cleanup supplies the address of an automatic object without adding a
    /// synthetic expression to the immutable syntax tree.
    pub(super) fn cleanup_predicate_value(
        &self,
        parameters: &[Option<usize>],
        expression: &'tu Expression<'tu>,
        variable: usize,
    ) -> Option<Integer> {
        self.predicate_value_for(parameters, expression, &[], Some(variable))
    }

    fn predicate_value_for(
        &self,
        parameters: &[Option<usize>],
        expression: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
        cleanup: Option<usize>,
    ) -> Option<Integer> {
        let mut work = ArenaVec::new_in(self.scratch);
        let mut values: ArenaVec<'_, Option<PredicateConstant>> = ArenaVec::new_in(self.scratch);
        work.push(Step::Eval(expression));
        while let Some(step) = work.pop() {
            match step {
                | Step::Eval(expression) => {
                    let info = self.expression_info(expression);
                    let converted = self.resolved.argument_type(expression, info.ty);
                    if converted != info.ty {
                        work.push(Step::Cast(converted));
                    }
                    let known = || self.predicate_known(info);
                    match expression.kind {
                        | ExpressionType::Identifier(_) => {
                            let value = if let Some(index) = parameters
                                .iter()
                                .position(|&binding| binding.is_some() && binding == info.binding)
                            {
                                let argument = if cleanup.is_some() && index == 0 {
                                    Some(PredicateConstant::NonNullAddress(false))
                                } else {
                                    arguments.get(index).and_then(|&argument| {
                                        self.predicate_known(self.expression_info(argument))
                                    })
                                };
                                self.predicate_convert(argument, info.ty)
                            } else {
                                known()
                            };
                            values.push(value);
                        },
                        | ExpressionType::Parenthesized { expression } =>
                            work.push(Step::Eval(expression)),
                        | ExpressionType::Unary {
                            operator: UnaryOperator::Extension,
                            operand_expression,
                        } => work.push(Step::Eval(operand_expression)),
                        | ExpressionType::Unary {
                            operator: UnaryOperator::AddressOf,
                            ..
                        } => values.push(known()),
                        | ExpressionType::Unary {
                            operator,
                            operand_expression,
                        } => {
                            work.push(Step::Unary(operator));
                            work.push(Step::Eval(operand_expression));
                        },
                        | ExpressionType::Binary {
                            operator:
                                operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                            left_expression,
                            right_expression,
                        } => {
                            work.push(Step::Logical(operator, right_expression));
                            work.push(Step::Eval(left_expression));
                        },
                        | ExpressionType::Binary {
                            operator,
                            left_expression,
                            right_expression,
                        } => {
                            let complex = |expression| {
                                matches!(
                                    self.types.nodes[self.expression_info(expression).ty.index],
                                    TypeKind::Scalar(s) if s.complex()
                                )
                            };
                            let domains = u8::from(complex(left_expression))
                                | (u8::from(complex(right_expression)) << 1);
                            work.push(Step::Binary(operator, info.ty, domains));
                            work.push(Step::Eval(right_expression));
                            work.push(Step::Eval(left_expression));
                        },
                        | ExpressionType::Conditional(conditional)
                        | ExpressionType::OmittedConditional(conditional) => {
                            work.push(Step::Conditional(
                                conditional.then_expression,
                                conditional.else_expression,
                                info.ty,
                            ));
                            work.push(Step::Eval(conditional.condition_expression));
                        },
                        | ExpressionType::Cast {
                            operand_expression, ..
                        } => {
                            work.push(Step::Cast(info.ty));
                            work.push(Step::Eval(operand_expression));
                        },
                        | ExpressionType::Call {
                            function_expression,
                            arguments: operands,
                        } => {
                            let operation = self
                                .expression_info(self.selected_designator(function_expression))
                                .binding
                                .and_then(|binding| self.bindings[binding].origin.descriptor())
                                .map(|descriptor| descriptor.operation);
                            match operation {
                                | Some(BuiltinOperation::Constant) if operands.len() == 1 => {
                                    work.push(Step::Constant);
                                    work.push(Step::Eval(operands[0]));
                                },
                                | Some(BuiltinOperation::ObjectSize(dynamic))
                                    if operands.len() == 2 =>
                                {
                                    let mode = self
                                        .expression_info(operands[1])
                                        .integer
                                        .and_then(Integer::to_u64)
                                        .and_then(|mode| u8::try_from(mode).ok())
                                        .unwrap_or(0);
                                    let size = if let Some(variable) = cleanup {
                                        self.cleanup_object_size(
                                            operands[0],
                                            mode,
                                            dynamic,
                                            parameters,
                                            variable,
                                        )
                                    } else {
                                        self.substituted_object_size(
                                            operands[0],
                                            mode,
                                            dynamic,
                                            parameters,
                                            arguments,
                                        )
                                    };
                                    let value = match size {
                                        | ObjectSize::Constant(value) =>
                                            Some(Integer::int(i128::from(value)).cast(64, false)),
                                        | ObjectSize::Unknown => Some(
                                            Integer::int(if mode >= 2 { 0 } else { -1 })
                                                .cast(64, false),
                                        ),
                                        | _ => None,
                                    };
                                    values.push(value.map(PredicateConstant::Integer));
                                },
                                | _ => values.push(known()),
                            }
                        },
                        | _ => values.push(known()),
                    }
                },
                | Step::Unary(operator) => {
                    let value = values.pop().flatten().and_then(|value| match value {
                        | PredicateConstant::Integer(value) =>
                            value.unary(operator).map(PredicateConstant::Integer),
                        | PredicateConstant::Floating(value) => match operator {
                            | UnaryOperator::LogicalNot => Some(PredicateConstant::Integer(
                                Integer::int(i128::from(!value.truth())),
                            )),
                            | UnaryOperator::Minus =>
                                Some(PredicateConstant::Floating(Self::floating_negate(value))),
                            | UnaryOperator::Plus => Some(PredicateConstant::Floating(value)),
                            | _ => None,
                        },
                        | PredicateConstant::NonNullAddress(_) => (operator
                            == UnaryOperator::LogicalNot)
                            .then_some(PredicateConstant::Integer(Integer::int(0))),
                    });
                    values.push(value);
                },
                | Step::Cast(ty) => {
                    let value = values.pop().flatten();
                    values.push(self.predicate_convert(value, ty));
                },
                | Step::Constant => {
                    let value = values.pop().flatten();
                    values.push(Some(PredicateConstant::Integer(Integer::int(i128::from(
                        value.is_some_and(|value| {
                            !matches!(value, PredicateConstant::NonNullAddress(false))
                        }),
                    )))));
                },
                | Step::Binary(operator, ty, domains) => {
                    let right = values.pop().flatten();
                    let left = values.pop().flatten();
                    let value = if operator == BinaryOperator::Comma {
                        right
                    } else {
                        left.zip(right).and_then(|(left, right)| {
                            self.predicate_binary(operator, ty, domains, left, right)
                        })
                    };
                    values.push(value);
                },
                | Step::Logical(operator, right) => match values.pop().flatten() {
                    | Some(left)
                        if (operator == BinaryOperator::LogicalAnd && !left.truth())
                            || (operator == BinaryOperator::LogicalOr && left.truth()) =>
                        values.push(Some(PredicateConstant::Integer(Integer::int(i128::from(
                            left.truth(),
                        ))))),
                    | Some(left) => {
                        values.push(Some(left));
                        work.push(Step::Binary(operator, self.expression_info(right).ty, 0));
                        work.push(Step::Eval(right));
                    },
                    | None => values.push(None),
                },
                | Step::Conditional(then, otherwise, ty) => {
                    if let Some(condition) = values.pop().flatten() {
                        work.push(Step::Cast(ty));
                        work.push(Step::Eval(if condition.truth() { then } else { otherwise }));
                    } else {
                        values.push(None);
                    }
                },
            }
        }
        values.pop().flatten().map(|value| match value {
            | PredicateConstant::Integer(value) => value,
            | PredicateConstant::Floating(value) => Integer::int(i128::from(value.truth())),
            | PredicateConstant::NonNullAddress(_) => Integer::int(1),
        })
    }

    fn predicate_known(
        &self,
        info: super::super::ExpressionInfo<'tu>,
    ) -> Option<PredicateConstant> {
        info.integer
            .map(PredicateConstant::Integer)
            .or_else(|| info.floating().map(PredicateConstant::Floating))
            .or_else(|| {
                self.address_parts(info.expression, false)
                    .and_then(|(base, _)| {
                        if matches!(base, super::super::expressions::AddressBase::Absolute) {
                            None
                        } else {
                            Some(PredicateConstant::NonNullAddress(info.static_address))
                        }
                    })
            })
    }

    fn predicate_convert(
        &self,
        value: Option<PredicateConstant>,
        ty: TypeId,
    ) -> Option<PredicateConstant> {
        let value = value?;
        if self.pointer_target(ty).is_some()
            || matches!(
                self.types.nodes[self.types.non_atomic(ty).index],
                TypeKind::Nullptr
            )
        {
            return match value {
                | PredicateConstant::Integer(value) =>
                    Some(PredicateConstant::Integer(value.cast(64, false))),
                | PredicateConstant::NonNullAddress(_) => Some(value),
                | PredicateConstant::Floating(_) => None,
            };
        }
        if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool)) {
            return Some(PredicateConstant::Integer(
                Integer::int(i128::from(value.truth())).cast(1, false),
            ));
        }
        if let Some((bits, signed)) = self.integer_type(ty) {
            return match value {
                | PredicateConstant::Integer(value) =>
                    Some(PredicateConstant::Integer(value.cast(bits, signed))),
                | PredicateConstant::Floating(value) => self
                    .float_cast(
                        ty,
                        crate::translation_phases::preprocessing::FloatTokenType::LongDouble(
                            value.real,
                        ),
                    )
                    .map(PredicateConstant::Integer),
                | PredicateConstant::NonNullAddress(_) => None,
            };
        }
        self.predicate_float(value)
            .and_then(|value| self.round_floating(value, ty))
            .map(PredicateConstant::Floating)
    }

    fn predicate_float(&self, value: PredicateConstant) -> Option<Floating> {
        match value {
            | PredicateConstant::Floating(value) => Some(value),
            | PredicateConstant::Integer(value) => {
                let text = crate::diagnostics::format_in!(self.scratch, "{}L\0", value);
                crate::float_parsing::string_to_long_double(text)
                    .ok()
                    .map(Floating::real)
            },
            | PredicateConstant::NonNullAddress(_) => None,
        }
    }

    fn predicate_binary(
        &self,
        operator: BinaryOperator,
        ty: TypeId,
        domains: u8,
        left: PredicateConstant,
        right: PredicateConstant,
    ) -> Option<PredicateConstant> {
        if matches!(
            operator,
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
        ) {
            return Some(PredicateConstant::Integer(Integer::int(i128::from(
                if operator == BinaryOperator::LogicalAnd {
                    left.truth() && right.truth()
                } else {
                    left.truth() || right.truth()
                },
            ))));
        }
        if let (PredicateConstant::NonNullAddress(_), PredicateConstant::Integer(value))
        | (PredicateConstant::Integer(value), PredicateConstant::NonNullAddress(_)) =
            (left, right)
        {
            return (value.value == 0
                && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual))
            .then(|| {
                PredicateConstant::Integer(Integer::int(i128::from(
                    operator == BinaryOperator::NotEqual,
                )))
            });
        }
        if let (PredicateConstant::Integer(left), PredicateConstant::Integer(right)) = (left, right)
        {
            return left
                .binary(operator, right)
                .or_else(|| left.sign_bit_shift(operator, right))
                .map(PredicateConstant::Integer);
        }
        let left = self.predicate_float(left)?;
        let right = self.predicate_float(right)?;
        Self::floating_comparison(operator, left, right)
            .map(PredicateConstant::Integer)
            .or_else(|| {
                let TypeKind::Scalar(scalar) = self.types.nodes[self.types.non_atomic(ty).index]
                else {
                    return None;
                };
                let precision = self.floating_precision(scalar)?;
                Self::floating_binary(operator, left, right, precision, domains)
                    .and_then(|value| self.round_floating(value, ty))
                    .map(PredicateConstant::Floating)
            })
    }
}

#[derive(Clone, Copy)]
enum PredicateConstant {
    Integer(Integer),
    Floating(Floating),
    NonNullAddress(bool),
}
impl PredicateConstant {
    fn truth(self) -> bool {
        match self {
            | Self::Integer(value) => value.value != 0,
            | Self::Floating(value) => value.truth(),
            | Self::NonNullAddress(_) => true,
        }
    }
}
