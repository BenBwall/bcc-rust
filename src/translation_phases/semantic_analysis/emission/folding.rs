//! The values Clang's code generation folds without optimizing: the
//! condition of an `if`, a `switch`, a scalar `?:`, `&&` and `||` folds when
//! `CodeGenFunction::ConstantFoldsToSimpleInteger` succeeds, which is
//! `Expr::EvaluateAsInt` without side effects, refused when the condition
//! holds a label. A scalar `?:` whose operands are `Expr::isEvaluatable`
//! becomes a `select`, with no blocks. This is the one folding rule of the
//! dead flag; loops instead test the value IR emission produces, which
//! `functions.rs` models on its own (`EmitWhileStmt`).
//!
//! The evaluator starts from the constants semantic analysis already
//! computed and extends them as Clang's evaluator does in C: a comma whose
//! left operand evaluates, a conditional, `&&` and `||` that skip what they
//! do not take, a statement expression of null and side-effect-free
//! expression statements, a `const` non-volatile scalar object with a
//! constant initializer, the address of an object or function that is not
//! weak (non-null, and comparable within one object), `__builtin_expect`,
//! `__builtin_constant_p` and `__builtin_choose_expr`. A call, an
//! assignment, an increment, a volatile or other object read, a declaration
//! in a statement expression and anything this model does not know do not
//! fold.
//! C99: §6.6 paragraphs 2-3, 6 and 10, pp. 95-96; PDF pp. 107-108;
//! §6.5.13-§6.5.15, pp. 89-91; PDF pp. 101-103; §6.5.17 paragraph 2, p. 94;
//! PDF p. 106.
//! GNU extension: GCC manual, "Other Builtins" and "Statement Exprs".

use super::super::{
    Analyzer,
    BindingKind,
    ConstantClass,
    ExpressionType,
    Floating,
    Integer,
    TypeKind,
    types::Scalar,
};
use crate::{
    builtins::BuiltinId,
    translation_phases::parsing::{
        declaration_syntax::{
            Initializer,
            InitializerType,
            TypeQualifiers,
        },
        syntax::{
            BinaryOperator,
            BlockItem,
            Expression,
            ExpressionSlot,
            StatementType,
            UnaryOperator,
        },
    },
};

/// How deep the evaluator follows operands that semantic analysis did not
/// fold itself. Deeper conditions are left unfolded, which the iterative
/// traversal tolerates; Clang would rarely fold them either.
const DEPTH: u32 = 256;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang's `ConstantFoldsToSimpleInteger` for `condition`: its integer
    /// value, when it evaluates without side effects and holds no label.
    pub(in crate::translation_phases::semantic_analysis) fn folded_integer(
        &mut self,
        condition: &'tu Expression<'tu>,
    ) -> Option<Integer> {
        let Some(Value::Integer(value)) = self.evaluate_value(condition, 0) else {
            return None;
        };
        (!self.expression_contains_label(condition)).then_some(value)
    }

    /// The truth value code generation folds `condition` to: see
    /// [`Self::folded_integer`]. A bare floating condition does not fold,
    /// as `EvaluateAsInt` needs an integer.
    pub(in crate::translation_phases::semantic_analysis) fn folded_condition(
        &mut self,
        condition: &'tu Expression<'tu>,
    ) -> Option<bool> {
        self.folded_integer(condition).map(|value| value.value != 0)
    }

    /// Clang's `Expr::isEvaluatable`: whether `expression` evaluates to a
    /// constant of any type without side effects.
    pub(super) fn evaluatable(&mut self, expression: &'tu Expression<'tu>) -> bool {
        self.evaluate_value(expression, 0).is_some()
    }

    /// The value of `expression` as Clang's evaluator computes it, if it
    /// does; memoized, since conditions nest.
    fn evaluate_value(&mut self, expression: &'tu Expression<'tu>, depth: u32) -> Option<Value> {
        let key = std::ptr::from_ref(expression).addr();
        if let Some(&value) = self.functions.emission.folded.get(&key) {
            return value;
        }
        let value = self.compute_value(expression, depth);
        _ = self.functions.emission.folded.insert(key, value);
        value
    }

    fn compute_value(&mut self, expression: &'tu Expression<'tu>, depth: u32) -> Option<Value> {
        let info = self.expression_info(expression);
        // Semantic analysis folds `__builtin_constant_p` as GCC's front end
        // does, which differs from Clang's evaluator; recompute around it.
        if info.constant == ConstantClass::Arithmetic
            && !info.unfolded_binary128
            && !self.holds_constant_p(expression)
        {
            if let Some(integer) = info.integer {
                return Some(Value::Integer(integer));
            }
            if let Some(floating) = info.floating {
                return Some(Value::Floating(floating));
            }
        }
        if depth > DEPTH {
            return None;
        }
        let depth = depth + 1;
        let value = match expression.kind {
            | ExpressionType::Parenthesized { expression } =>
                self.evaluate_value(expression, depth),
            | ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                left_expression,
                right_expression,
            } => {
                _ = self.evaluate_value(left_expression, depth)?;
                self.evaluate_value(right_expression, depth)
            },
            | ExpressionType::Binary {
                operator: operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                left_expression,
                right_expression,
            } => {
                let left = self.evaluate_value(left_expression, depth)?.truth()?;
                if left == (operator == BinaryOperator::LogicalOr) {
                    Some(Value::Integer(Integer::int(i128::from(left))))
                } else {
                    let right = self.evaluate_value(right_expression, depth)?.truth()?;
                    Some(Value::Integer(Integer::int(i128::from(right))))
                }
            },
            | ExpressionType::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                let left = self.evaluate_value(left_expression, depth)?;
                let right = self.evaluate_value(right_expression, depth)?;
                Self::binary_value(operator, left, right)
            },
            | ExpressionType::Conditional(conditional) => {
                let taken = if self
                    .evaluate_value(conditional.condition_expression, depth)?
                    .truth()?
                {
                    conditional.then_expression
                } else {
                    conditional.else_expression
                };
                let value = self.evaluate_value(taken, depth)?;
                Some(self.converted_value(value, expression))
            },
            | ExpressionType::Unary {
                operator,
                operand_expression,
            } => match operator {
                | UnaryOperator::LogicalNot => {
                    let truth = self.evaluate_value(operand_expression, depth)?.truth()?;
                    Some(Value::Integer(Integer::int(i128::from(!truth))))
                },
                | UnaryOperator::AddressOf => self.address_of_value(operand_expression),
                | UnaryOperator::Extension => self.evaluate_value(operand_expression, depth),
                | UnaryOperator::Plus | UnaryOperator::Minus | UnaryOperator::BitwiseNot => {
                    let Value::Integer(value) = self.evaluate_value(operand_expression, depth)?
                    else {
                        return None;
                    };
                    value
                        .unary(operator)
                        .map(|value| self.converted_value(Value::Integer(value), expression))
                },
                | _ => None,
            },
            | ExpressionType::Cast {
                operand_expression, ..
            } => {
                let value = self.evaluate_value(operand_expression, depth)?;
                self.cast_value(value, expression)
            },
            | ExpressionType::Identifier(_) => self.identifier_value(expression, depth),
            | ExpressionType::StringLiteral(_) => Some(Value::Address(None)),
            | ExpressionType::StatementExpression(block) => {
                let StatementType::Compound { items } = block.kind else {
                    return None;
                };
                let (last, rest) = items.as_slice().split_last()?;
                for item in rest {
                    match *item {
                        | BlockItem::Statement(statement) => match statement.kind {
                            | StatementType::Null => {},
                            | StatementType::Expression(ExpressionSlot::Parsed(expression)) =>
                                _ = self.evaluate_value(expression, depth)?,
                            | _ => return None,
                        },
                        | _ => return None,
                    }
                }
                match *last {
                    | BlockItem::Statement(statement) => match statement.kind {
                        | StatementType::Expression(ExpressionSlot::Parsed(value)) =>
                            self.evaluate_value(value, depth),
                        | _ => None,
                    },
                    | _ => None,
                }
            },
            | ExpressionType::Call {
                function_expression,
                arguments,
            } => self.builtin_value(function_expression, arguments.as_slice(), depth),
            | ExpressionType::Generic(_) | ExpressionType::Builtin(_) => info
                .selected_expression
                .filter(|selected| !std::ptr::eq(*selected, expression))
                .and_then(|selected| self.evaluate_value(selected, depth)),
            | _ => None,
        };
        value.map(|value| match value {
            | Value::Integer(integer) => self.converted_value(Value::Integer(integer), expression),
            | other => other,
        })
    }

    /// The integer value `value` takes in `expression`'s type.
    fn converted_value(&self, value: Value, expression: &'tu Expression<'tu>) -> Value {
        let Value::Integer(integer) = value else {
            return value;
        };
        let ty = self.expression_info(expression).ty;
        if matches!(
            self.types.nodes[self.types.non_atomic(ty).index],
            TypeKind::Scalar(Scalar::Bool)
        ) {
            return Value::Integer(Integer::int(i128::from(integer.value != 0)).cast(8, false));
        }
        self.integer_type(ty).map_or(value, |(bits, signed)| {
            Value::Integer(integer.cast(bits, signed))
        })
    }

    /// A binary operator applied to evaluated operands: integer arithmetic
    /// and comparison, and the comparison of addresses within one object
    /// or with null.
    fn binary_value(operator: BinaryOperator, left: Value, right: Value) -> Option<Value> {
        use BinaryOperator as B;
        match (left, right) {
            | (Value::Integer(left), Value::Integer(right)) =>
                left.binary(operator, right).map(Value::Integer),
            | (Value::Address(left), Value::Address(right))
                if matches!(operator, B::Equal | B::NotEqual) =>
            {
                let equal = match (left, right) {
                    | (Some((a, x)), Some((b, y))) if a == b => x == y,
                    | (Some(_), Some(_)) | (None, None) => return None,
                    // String literal addresses are not null either.
                    | _ => false,
                };
                Some(Value::Integer(Integer::int(i128::from(
                    equal == (operator == B::Equal),
                ))))
            },
            | (Value::Address(_), Value::Integer(zero))
            | (Value::Integer(zero), Value::Address(_))
                if zero.value == 0 && matches!(operator, B::Equal | B::NotEqual) =>
                Some(Value::Integer(Integer::int(i128::from(
                    operator == B::NotEqual,
                )))),
            | (Value::Address(Some((base, offset))), Value::Integer(step))
                if matches!(operator, B::Addition | B::Subtraction) =>
            {
                let step = step.to_i128()?;
                let offset = if operator == B::Addition {
                    offset.checked_add(step)?
                } else {
                    offset.checked_sub(step)?
                };
                Some(Value::Address(Some((base, offset))))
            },
            | _ => None,
        }
    }

    /// A cast of an evaluated operand to `expression`'s type: integers,
    /// truth values and real floating values convert to integers, a null or
    /// object address stays an address; a converted floating value is only
    /// known to evaluate.
    fn cast_value(&self, value: Value, expression: &'tu Expression<'tu>) -> Option<Value> {
        let ty = self.types.non_atomic(self.expression_info(expression).ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Void) => Some(Value::Other),
            | TypeKind::Scalar(Scalar::Bool) => value
                .truth()
                .map(|truth| Value::Integer(Integer::int(i128::from(truth)).cast(8, false))),
            | TypeKind::Pointer(_) => match value {
                | Value::Integer(zero) if zero.value == 0 => Some(Value::Integer(zero)),
                | Value::Address(address) => Some(Value::Address(address)),
                | _ => None,
            },
            | TypeKind::Scalar(
                Scalar::Float
                | Scalar::Double
                | Scalar::LongDouble
                | Scalar::Float128
                | Scalar::ComplexFloat
                | Scalar::ComplexDouble
                | Scalar::ComplexLongDouble
                | Scalar::ComplexFloat128,
            ) => match value {
                | Value::Floating(floating) => Some(Value::Floating(floating)),
                | Value::Integer(_) | Value::Other => Some(Value::Other),
                | Value::Address(_) => None,
            },
            | _ => {
                let (bits, signed) = self.integer_type(ty)?;
                match value {
                    | Value::Integer(integer) => Some(Value::Integer(integer.cast(bits, signed))),
                    | Value::Floating(floating) =>
                        floating_integer(floating, bits, signed).map(Value::Integer),
                    | _ => None,
                }
            },
        }
    }

    /// The value of a named object Clang's evaluator may read: a `const`,
    /// non-volatile integer object whose initializer evaluates. A function
    /// or array names its address.
    fn identifier_value(&mut self, expression: &'tu Expression<'tu>, depth: u32) -> Option<Value> {
        let binding = self.expression_info(expression).binding?;
        let data = self.bindings[binding];
        match data.kind {
            | BindingKind::Function => self.address_of_value(expression),
            | BindingKind::Object | BindingKind::Parameter => {
                if matches!(self.types.nodes[data.ty.index], TypeKind::Array(..)) {
                    return self.address_of_value(expression);
                }
                if !data.ty.qualifiers.contains(TypeQualifiers::CONST)
                    || data.ty.qualifiers.contains(TypeQualifiers::VOLATILE)
                {
                    return None;
                }
                let initializer = self
                    .functions
                    .emission
                    .constant_initializers
                    .get(&binding)
                    .copied()?;
                match self.evaluate_value(initializer, depth)? {
                    | Value::Floating(floating) => Some(Value::Floating(floating)),
                    | value => self.cast_value(value, expression),
                }
            },
            | _ => None,
        }
    }

    /// Whether `expression` calls `__builtin_constant_p`, memoized.
    fn holds_constant_p(&mut self, expression: &'tu Expression<'tu>) -> bool {
        let mut work = crate::util::bump::ArenaVec::new_in(self.scratch);
        work.push((expression, false));
        while let Some((current, children_done)) = work.pop() {
            let key = std::ptr::from_ref(current).addr();
            if self.functions.emission.constant_p.contains_key(&key) {
                continue;
            }
            if children_done {
                let mut holds = matches!(current.kind, ExpressionType::Call { function_expression, .. }
                if self.expression_info(function_expression).binding.is_some_and(|binding| {
                    self.bindings[binding].origin
                        == super::super::BindingOrigin::Intrinsic(BuiltinId::ConstantP)
                }));
                super::labels::expression_children(self.scratch, current, |child| {
                    holds |= self
                        .functions
                        .emission
                        .constant_p
                        .get(&std::ptr::from_ref(child).addr())
                        .copied()
                        .unwrap_or(false);
                });
                _ = self.functions.emission.constant_p.insert(key, holds);
            } else {
                work.push((current, true));
                super::labels::expression_children(self.scratch, current, |child| {
                    work.push((child, false));
                });
            }
        }
        self.functions
            .emission
            .constant_p
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
            .unwrap_or(false)
    }

    /// The address of `operand`, when it designates an object or function
    /// that is not weak: Clang's evaluator knows it is not null.
    fn address_of_value(&mut self, operand: &'tu Expression<'tu>) -> Option<Value> {
        let mut operand = operand;
        while let ExpressionType::Parenthesized { expression } = operand.kind {
            operand = expression;
        }
        if !matches!(operand.kind, ExpressionType::Identifier(_)) {
            return None;
        }
        let binding = self.expression_info(operand).binding?;
        let data = self.bindings[binding];
        if !matches!(
            data.kind,
            BindingKind::Function | BindingKind::Object | BindingKind::Parameter
        ) || self.declared_weak(binding)
        {
            return None;
        }
        Some(Value::Address(Some((self.entity_of(binding).0, 0))))
    }

    /// The builtins Clang's evaluator folds that a condition may call:
    /// `__builtin_expect` takes its first operand; `__builtin_constant_p` is
    /// 1 for an operand that folds and 0 for one with side effects.
    fn builtin_value(
        &mut self,
        callee: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
        depth: u32,
    ) -> Option<Value> {
        let binding = self.expression_info(callee).binding?;
        let super::super::BindingOrigin::Intrinsic(id) = self.bindings[binding].origin else {
            return None;
        };
        match id {
            | BuiltinId::Expect | BuiltinId::ExpectWithProbability =>
                self.evaluate_value(arguments.first()?, depth),
            | BuiltinId::ConstantP => {
                let operand = *arguments.first()?;
                let constant = !matches!(
                    self.evaluate_value(operand, depth),
                    Some(Value::Address(Some(_))) | None
                );
                if constant {
                    Some(Value::Integer(Integer::int(1)))
                } else if self.has_side_effects(operand) {
                    Some(Value::Integer(Integer::int(0)))
                } else {
                    None
                }
            },
            | _ => None,
        }
    }

    /// Clang's `Expr::HasSideEffects`, syntactically: an assignment, an
    /// increment or decrement, a call or a volatile read.
    fn has_side_effects(&self, expression: &'tu Expression<'tu>) -> bool {
        let mut work = crate::util::bump::ArenaVec::new_in(self.scratch);
        work.push(expression);
        while let Some(expression) = work.pop() {
            let effect = match expression.kind {
                | ExpressionType::Call { .. }
                | ExpressionType::StatementExpression(_)
                | ExpressionType::Binary {
                    operator:
                        BinaryOperator::Assignment
                        | BinaryOperator::MultiplicationAssignment
                        | BinaryOperator::DivisionAssignment
                        | BinaryOperator::ModuloAssignment
                        | BinaryOperator::AdditionAssignment
                        | BinaryOperator::SubtractionAssignment
                        | BinaryOperator::LeftShiftAssignment
                        | BinaryOperator::RightShiftAssignment
                        | BinaryOperator::BitwiseAndAssignment
                        | BinaryOperator::BitwiseXorAssignment
                        | BinaryOperator::BitwiseOrAssignment,
                    ..
                }
                | ExpressionType::Unary {
                    operator:
                        UnaryOperator::PreIncrement
                        | UnaryOperator::PreDecrement
                        | UnaryOperator::PostIncrement
                        | UnaryOperator::PostDecrement,
                    ..
                } => true,
                | ExpressionType::Identifier(_) => self
                    .expression_info(expression)
                    .ty
                    .qualifiers
                    .contains(TypeQualifiers::VOLATILE),
                | ExpressionType::SizeofExpr(_) | ExpressionType::AlignofExpr(_) => false,
                | _ => {
                    super::labels::expression_children(self.scratch, expression, |child| {
                        work.push(child);
                    });
                    false
                },
            };
            if effect {
                return true;
            }
        }
        false
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Keeps the initializer of `binding` when it is a `const`,
    /// non-volatile object, whose value Clang's evaluator may read.
    /// C99: §6.7.3 paragraph 3, p. 108; PDF p. 120.
    pub(in crate::translation_phases::semantic_analysis) fn record_constant_initializer(
        &mut self,
        binding: usize,
        initializer: &'tu Initializer<'tu>,
    ) {
        let data = self.bindings[binding];
        if data.kind == BindingKind::Object
            && data.ty.qualifiers.contains(TypeQualifiers::CONST)
            && !data.ty.qualifiers.contains(TypeQualifiers::VOLATILE)
            && let InitializerType::AssignmentExpression(expression) = initializer.kind
        {
            _ = self
                .functions
                .emission
                .constant_initializers
                .insert(binding, expression);
        }
    }
}

/// A value Clang's evaluator computed.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) enum Value {
    Integer(Integer),
    /// The address of an entity with an element offset, or of a string
    /// literal (`None`).
    Address(Option<(usize, i128)>),
    Floating(Floating),
    /// A converted floating or a void value, which only matters as
    /// evaluated.
    Other,
}

impl Value {
    /// The truth of a scalar value, if known.
    fn truth(self) -> Option<bool> {
        match self {
            | Self::Integer(integer) => Some(integer.value != 0),
            | Self::Address(_) => Some(true),
            | Self::Floating(floating) => Some(floating.truth()),
            | Self::Other => None,
        }
    }
}

/// The integer a real floating value converts to, truncated toward zero:
/// none when it does not fit, which is undefined.
/// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
fn floating_integer(value: Floating, bits: u32, signed: bool) -> Option<Integer> {
    let (negative, exponent, mantissa) = value.real.x87_parts()?;
    if exponent == 0x7FFF {
        return None;
    }
    // The x87 significand holds its integer bit: value = m * 2^(e - 16383 -
    // 63).
    let shift = i32::from(exponent) - 16383 - 63;
    let magnitude = if mantissa == 0 || shift <= -64 {
        0
    } else if shift < 0 {
        u128::from(mantissa >> (-shift))
    } else if shift <= 64 {
        u128::from(mantissa) << shift
    } else {
        return None;
    };
    let limit = if signed {
        (1_u128 << (bits - 1)) - u128::from(!negative)
    } else if negative {
        0
    } else {
        (1_u128 << bits) - 1
    };
    if magnitude > limit {
        return None;
    }
    let magnitude = i128::try_from(magnitude).ok()?;
    Some(Integer::int(if negative { -magnitude } else { magnitude }).cast(bits, signed))
}
