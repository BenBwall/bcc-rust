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
    ArrayBound,
    BindingKind,
    ConstantClass,
    ExpressionType,
    Floating,
    InitOperation,
    Integer,
    TagKind,
    TypeId,
    TypeKind,
    types::Scalar,
};
use crate::{
    builtins::{
        BitOperation,
        BuiltinId,
        BuiltinOperation,
    },
    float_parsing::LongDouble,
    translation_phases::{
        parsing::{
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
        preprocessing::{
            LiteralEncoding,
            StringTokenType,
        },
    },
    util::string_cache::StringCacheId,
};

/// How deep the evaluator follows operands that semantic analysis did not
/// fold itself. Deeper conditions are left unfolded, which the iterative
/// traversal tolerates; Clang would rarely fold them either.
const DEPTH: u32 = 256;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang's `ConstantFoldsToSimpleInteger` for `condition`: its integer
    /// value, when it has an integer or enumerated type, evaluates without
    /// side effects and holds no label. `EvaluateAsInt` refuses any other
    /// type, so a null pointer condition does not fold.
    /// C99: §6.2.5 paragraph 17, p. 35; PDF p. 47.
    pub(in crate::translation_phases::semantic_analysis) fn folded_integer(
        &mut self,
        condition: &'tu Expression<'tu>,
    ) -> Option<Integer> {
        let ty = self.expression_info(condition).ty;
        _ = self.integer_type(ty)?;
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
    /// constant of any type without side effects. A `void` expression has
    /// no value, so it is not evaluatable.
    pub(super) fn evaluatable(&mut self, expression: &'tu Expression<'tu>) -> bool {
        let ty = self.expression_info(expression).ty;
        !matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void))
            && self.evaluate_value(expression, 0).is_some()
    }

    /// Whether `expression` evaluates to a number, which code generation
    /// emits as a constant rather than an instruction.
    pub(super) fn evaluates_to_number(&mut self, expression: &'tu Expression<'tu>) -> bool {
        matches!(
            self.evaluate_value(expression, 0),
            Some(Value::Integer(_) | Value::Floating(_))
        )
    }

    /// Clang's constant-context query value: the semantic integer that
    /// integer constant expressions, case labels, static assertions and
    /// static initializer data read. An unknown query is zero, even when
    /// `EvaluateAsInt` cannot fold it as a control condition.
    /// GNU extension: GCC manual, "Other Builtins", `__builtin_constant_p`.
    pub(in crate::translation_phases::semantic_analysis) fn constant_query_integer(
        &mut self,
        operand: &'tu Expression<'tu>,
    ) -> Integer {
        Integer::int(i128::from(self.constant_query_value(operand, 0, false)))
    }

    /// Whether function code emits one for a query whose constant-context
    /// value is zero: Clang's evaluator outside a constant context reads an
    /// array compound literal, and `llvm.is.constant` sees the arithmetic
    /// result of a comma whose left operand it cannot evaluate. Control
    /// folding and lowering then use one where integer constant expressions
    /// use zero. GNU extension: `__builtin_constant_p`.
    pub(in crate::translation_phases::semantic_analysis) fn constant_query_differs_at_runtime(
        &mut self,
        operand: &'tu Expression<'tu>,
    ) -> bool {
        !self.constant_query_value(operand, 0, false) && self.constant_query_value(operand, 0, true)
    }

    fn is_constant_query(&self, mut expression: &'tu Expression<'tu>) -> bool {
        while let ExpressionType::Parenthesized { expression: child } = expression.kind {
            expression = child;
        }
        let ExpressionType::Call {
            function_expression,
            ..
        } = expression.kind
        else {
            return false;
        };
        self.expression_info(function_expression)
            .binding
            .is_some_and(|binding| {
                self.bindings[binding].origin
                    == super::super::BindingOrigin::Intrinsic(BuiltinId::ConstantP)
            })
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
        // An unknown constant query emits zero, but Clang's evaluator may
        // leave the control condition unfolded. Recompute around queries.
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
                operator: BinaryOperator::Subscript,
                left_expression,
                right_expression,
            } => self.object_value(expression, depth).or_else(|| {
                let left = self.evaluate_value(left_expression, depth)?;
                let right = self.evaluate_value(right_expression, depth)?;
                match (left, right) {
                    | (Value::String(literal, offset), Value::Integer(index))
                    | (Value::Integer(index), Value::String(literal, offset)) =>
                        self.string_element(literal, offset.checked_add(index.to_i128()?)?),
                    | _ => None,
                }
            }),
            | ExpressionType::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                let left = self.evaluate_value(left_expression, depth)?;
                let right = self.evaluate_value(right_expression, depth)?;
                self.binary_value(operator, left, right)
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
                | UnaryOperator::Indirection =>
                    self.object_value(expression, depth).or_else(|| {
                        match self.evaluate_value(operand_expression, depth)? {
                            | Value::String(literal, offset) =>
                                self.string_element(literal, offset),
                            | _ => None,
                        }
                    }),
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
            | ExpressionType::DirectMember { .. } | ExpressionType::CompoundLiteral { .. } =>
                self.object_value(expression, depth),
            | ExpressionType::StringLiteral(literal) => Some(Value::String(literal, 0)),
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
    fn binary_value(&self, operator: BinaryOperator, left: Value, right: Value) -> Option<Value> {
        use BinaryOperator as B;
        match (left, right) {
            // Within the literal and one past its end, as Clang's evaluator
            // requires of pointer arithmetic.
            | (Value::String(literal, offset), Value::Integer(step))
            | (Value::Integer(step), Value::String(literal, offset))
                if operator == B::Addition
                    || (operator == B::Subtraction && matches!(left, Value::String(..))) =>
            {
                let step = step.to_i128()?;
                let offset = if operator == B::Addition {
                    offset.checked_add(step)?
                } else {
                    offset.checked_sub(step)?
                };
                let length = i128::from(self.string_length(literal));
                (0..=length + 1)
                    .contains(&offset)
                    .then_some(Value::String(literal, offset))
            },
            | (Value::String(..), Value::Integer(zero))
            | (Value::Integer(zero), Value::String(..))
                if zero.value == 0 && matches!(operator, B::Equal | B::NotEqual) =>
                Some(Value::Integer(Integer::int(i128::from(
                    operator == B::NotEqual,
                )))),
            // Distinct objects never compare equal; two literals may share
            // storage, so Clang does not fold their comparison.
            | (Value::String(..), Value::Address(Some(_)))
            | (Value::Address(Some(_)), Value::String(..))
                if matches!(operator, B::Equal | B::NotEqual) =>
                Some(Value::Integer(Integer::int(i128::from(
                    operator == B::NotEqual,
                )))),
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
        self.cast_to(value, self.expression_info(expression).ty)
    }

    /// [`Self::cast_value`] to type `ty`, as a conversion by assignment.
    fn cast_to(&self, value: Value, ty: TypeId) -> Option<Value> {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Void) => Some(Value::Other),
            | TypeKind::Scalar(Scalar::Bool) => value
                .truth()
                .map(|truth| Value::Integer(Integer::int(i128::from(truth)).cast(8, false))),
            | TypeKind::Pointer(_) => match value {
                | Value::Integer(zero) if zero.value == 0 => Some(Value::Integer(zero)),
                | Value::Address(address) => Some(Value::Address(address)),
                // A conversion to a pointer to another type invalidates the
                // evaluator's designator: the literal is no longer read
                // through.
                | Value::String(literal, offset) => Some(if self.string_pointer(literal, ty) {
                    Value::String(literal, offset)
                } else {
                    Value::Address(None)
                }),
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
                | Value::Address(_) | Value::String(..) => None,
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

    /// The number of elements of string literal `literal` before its null
    /// terminator.
    fn string_length(&self, literal: StringTokenType) -> u64 {
        let count = match literal {
            | StringTokenType::String(id) => self
                .context
                .encoded_literal_units(id, LiteralEncoding::Utf8)
                .count(),
            | StringTokenType::WideString(id) => self.context.wide_literal_units(id).count(),
            | StringTokenType::EncodedString(id, encoding) =>
                self.context.encoded_literal_units(id, encoding).count(),
        };
        count as u64
    }

    /// Whether `pointer` points to the element type of string literal
    /// `literal`, qualified or not, which keeps the literal readable.
    fn string_pointer(&self, literal: StringTokenType, pointer: TypeId) -> bool {
        let TypeKind::Pointer(target) = self.types.nodes[pointer.index] else {
            return false;
        };
        let element = match literal {
            | StringTokenType::String(_) => Scalar::Char,
            | StringTokenType::WideString(_) => self.types.target.wchar_t,
            | StringTokenType::EncodedString(_, encoding) => self.encoded_character_type(encoding),
        };
        matches!(self.types.nodes[target.index], TypeKind::Scalar(scalar) if scalar == element)
    }

    /// Element `index` of string literal `literal`, its terminator
    /// included, as Clang's evaluator reads it; none outside the array.
    /// C99: §6.4.5 paragraph 5, pp. 62-63; PDF pp. 74-75.
    fn string_element(&self, literal: StringTokenType, index: i128) -> Option<Value> {
        let index = u64::try_from(index).ok()?;
        let length = self.string_length(literal);
        let unit = match index.cmp(&length) {
            | std::cmp::Ordering::Less => {
                let index = usize::try_from(index).ok()?;
                match literal {
                    | StringTokenType::String(id) => self
                        .context
                        .encoded_literal_units(id, LiteralEncoding::Utf8)
                        .nth(index)?,
                    | StringTokenType::WideString(id) =>
                        self.context.wide_literal_units(id).nth(index)?,
                    | StringTokenType::EncodedString(id, encoding) => self
                        .context
                        .encoded_literal_units(id, encoding)
                        .nth(index)?,
                }
            },
            | std::cmp::Ordering::Equal => 0,
            | std::cmp::Ordering::Greater => return None,
        };
        Some(Value::Integer(Integer::int(i128::from(unit))))
    }

    /// The value Clang's evaluator reads from scalar subobject `expression`
    /// of a fresh compound literal or a `const`, non-volatile array or
    /// structure whose initializer
    /// evaluates whole (`VarDecl::evaluateValue`): what the retained
    /// initializer plan stores there, zero where it stores nothing. A
    /// bit-field, a union, a weak object and a plan with a GNU range are
    /// left unread.
    /// C99: §6.7.8 paragraphs 10 and 19-21, pp. 126-127; PDF pp. 138-139;
    /// §6.5.2.5 paragraph 4, p. 75; PDF p. 87 (compound literal values).
    fn object_value(&mut self, expression: &'tu Expression<'tu>, depth: u32) -> Option<Value> {
        let (initializer, offset, ty) = self.object_place(expression, depth)?;
        self.place_value(initializer, offset, ty, depth)
    }

    /// The value of the scalar of type `ty` at byte `offset` supplied by
    /// `initializer`: see [`Self::object_value`].
    fn place_value(
        &mut self,
        initializer: &'tu Initializer<'tu>,
        offset: u64,
        ty: TypeId,
        depth: u32,
    ) -> Option<Value> {
        let ty = self.types.non_atomic(ty);
        if !matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(_) | TypeKind::Pointer(_)
        ) || ty.qualifiers.contains(TypeQualifiers::VOLATILE)
        {
            return None;
        }
        let plan = *self.initializers.plan(initializer)?;
        for value in plan.values {
            _ = self.evaluate_value(value.expression, depth)?;
        }
        let mut read = None;
        for operation in plan.operations {
            let (destination, size) = match *operation {
                | InitOperation::Zero { destination, .. }
                | InitOperation::Select { destination, .. }
                | InitOperation::Store { destination, .. }
                | InitOperation::Copy { destination, .. }
                | InitOperation::String { destination, .. }
                | InitOperation::EmptyScalar { destination, .. }
                | InitOperation::Unmodeled { destination, .. } =>
                    (destination, self.types.layout(destination.ty)?.size),
                | InitOperation::Repeat { .. } => return None,
            };
            let covers = destination.offset <= offset && offset < destination.offset + size;
            if !covers {
                continue;
            }
            let exact = destination.offset == offset
                && self.types.non_atomic(destination.ty).unqualified() == ty.unqualified();
            read = Some(match *operation {
                | InitOperation::Zero { .. } => Read::Zero,
                | InitOperation::EmptyScalar { .. } if exact => Read::Zero,
                | InitOperation::Store {
                    value,
                    bit_field: None,
                    ..
                } if exact => Read::Value(plan.values.get(value.0 as usize)?.expression),
                | InitOperation::String { literal, .. } => {
                    let ExpressionType::StringLiteral(literal) = unparenthesized(literal).kind
                    else {
                        return None;
                    };
                    let element = self.types.layout(ty)?.size;
                    let index = (offset - destination.offset) / element;
                    // Elements past the literal's are zero.
                    if index > self.string_length(literal) {
                        Read::Zero
                    } else {
                        Read::String(literal, i128::from(index))
                    }
                },
                | _ => return None,
            });
        }
        let value = match read? {
            | Read::Zero => match self.types.nodes[ty.index] {
                | TypeKind::Scalar(
                    Scalar::Float
                    | Scalar::Double
                    | Scalar::LongDouble
                    | Scalar::Float128
                    | Scalar::ComplexFloat
                    | Scalar::ComplexDouble
                    | Scalar::ComplexLongDouble
                    | Scalar::ComplexFloat128,
                ) => Value::Floating(Floating {
                    real: LongDouble::ZERO,
                    imag: LongDouble::ZERO,
                }),
                | _ => Value::Integer(Integer::int(0)),
            },
            | Read::String(literal, index) => self.string_element(literal, index)?,
            | Read::Value(value) => self.evaluate_value(value, depth)?,
        };
        self.cast_to(value, ty)
    }

    /// The `const` object, byte offset and type that `expression`
    /// designates through names, subscripts with constant indices within
    /// the array, `.` members and `*` of an array or of an array plus a
    /// constant: Clang's evaluator reads such a subobject of a `const`
    /// object's value. The object must be `const` at its top level, or in
    /// its innermost element type for an array (`QualType::isConstant`),
    /// and not weak. A compound literal supplies its initializer directly:
    /// its fresh value has not been modified through an alias.
    fn object_place(
        &mut self,
        expression: &'tu Expression<'tu>,
        depth: u32,
    ) -> Option<(&'tu Initializer<'tu>, u64, TypeId)> {
        // The array expression and the element index of a subscript or an
        // indirection still to apply, innermost last.
        let mut expression = unparenthesized(expression);
        let mut steps = crate::util::bump::ArenaVec::new_in(self.scratch);
        let (initializer, ty) = loop {
            match expression.kind {
                | ExpressionType::Identifier(_) => {
                    let binding = self.expression_info(expression).binding?;
                    let data = self.bindings[binding];
                    if data.kind != BindingKind::Object
                        || !self.constant_object(data.ty)
                        || self.declared_weak(binding)
                    {
                        return None;
                    }
                    let initializer = self
                        .functions
                        .emission
                        .constant_aggregates
                        .get(&binding)
                        .copied()?;
                    break (initializer, data.ty);
                },
                | ExpressionType::CompoundLiteral { initializer, .. } =>
                    break (initializer, self.expression_info(expression).ty),
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let (array, index) = if self.array_typed(left_expression) {
                        (left_expression, right_expression)
                    } else {
                        (right_expression, left_expression)
                    };
                    steps.push(Step::Index(index));
                    expression = unparenthesized(array);
                },
                | ExpressionType::DirectMember {
                    base_expression,
                    member,
                } => {
                    steps.push(Step::Member(member.name));
                    expression = unparenthesized(base_expression);
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } => {
                    let operand = unparenthesized(operand_expression);
                    match operand.kind {
                        | ExpressionType::Binary {
                            operator: BinaryOperator::Addition,
                            left_expression,
                            right_expression,
                        } => {
                            let (array, index) = if self.array_typed(left_expression) {
                                (left_expression, right_expression)
                            } else {
                                (right_expression, left_expression)
                            };
                            steps.push(Step::Index(index));
                            expression = unparenthesized(array);
                        },
                        | _ => {
                            steps.push(Step::First);
                            expression = operand;
                        },
                    }
                },
                | _ => return None,
            }
        };
        let mut offset = 0_u64;
        let mut ty = ty;
        while let Some(step) = steps.pop() {
            let qualifiers = ty.qualifiers;
            match (step, self.types.nodes[ty.index]) {
                | (
                    Step::Index(_) | Step::First,
                    TypeKind::Array(element, ArrayBound::Constant(n)),
                ) => {
                    let index = match step {
                        | Step::Index(index) => match self.evaluate_value(index, depth)? {
                            | Value::Integer(index) => u64::try_from(index.to_i128()?).ok()?,
                            | _ => return None,
                        },
                        | _ => 0,
                    };
                    if index >= n {
                        return None;
                    }
                    offset =
                        offset.checked_add(index.checked_mul(self.types.layout(element)?.size)?)?;
                    ty = element.qualified(qualifiers);
                },
                | (Step::Member(name), TypeKind::Tag(id))
                    if self.types.tags[id].kind == TagKind::Struct =>
                {
                    let field = self.types.tags[id]
                        .fields
                        .get()
                        .iter()
                        .find(|field| field.name.name == name)?;
                    if field.width.is_some() {
                        return None;
                    }
                    offset = offset.checked_add(field.offset)?;
                    ty = field.ty.qualified(qualifiers | field.qualifiers);
                },
                | _ => return None,
            }
        }
        Some((initializer, offset, ty))
    }

    /// Whether an object of type `ty` is `const` for Clang's evaluator: at
    /// its top level, or in the innermost element type of an array.
    fn constant_object(&self, mut ty: TypeId) -> bool {
        loop {
            if ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
                return false;
            }
            if ty.qualifiers.contains(TypeQualifiers::CONST) {
                return true;
            }
            match self.types.nodes[ty.index] {
                | TypeKind::Array(element, _) => ty = element,
                | _ => return false,
            }
        }
    }

    fn array_typed(&self, expression: &'tu Expression<'tu>) -> bool {
        let ty = self.expression_info(expression).ty;
        matches!(self.types.nodes[ty.index], TypeKind::Array(..))
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
            return self
                .library_strlen(binding)
                .then(|| self.strlen_value(*arguments.first()?, depth))
                .flatten();
        };
        match id {
            | BuiltinId::Expect | BuiltinId::ExpectWithProbability =>
                self.evaluate_value(arguments.first()?, depth),
            | BuiltinId::ConstantP => {
                let operand = *arguments.first()?;
                let constant = self.constant_query_value(operand, depth, true);
                if constant {
                    Some(Value::Integer(Integer::int(1)))
                } else if self.has_side_effects(operand) {
                    Some(Value::Integer(Integer::int(0)))
                } else {
                    None
                }
            },
            // `strlen` folds like `__builtin_strlen` once it is the
            // library builtin; `abs` does not, unlike `__builtin_abs`.
            | BuiltinId::LibraryStrlen => self.strlen_value(*arguments.first()?, depth),
            | BuiltinId::LibraryAbs
                if self
                    .context
                    .string_cache
                    .at(self.bindings[binding].name.name)
                    == id.descriptor().spelling =>
            {
                let (bits, signed) = self.types.target.integer(Scalar::Int)?;
                let Value::Integer(value) = self.evaluate_value(*arguments.first()?, depth)? else {
                    return None;
                };
                let value = value.cast(bits, signed).value;
                // The minimum overflows, which Clang does not fold.
                (value != -(1_i128 << (bits - 1)))
                    .then(|| Value::Integer(Integer::int(value.abs())))
            },
            | _ => match id.descriptor().operation {
                | BuiltinOperation::Bit(operation, scalar) => {
                    let Value::Integer(value) = self.evaluate_value(*arguments.first()?, depth)?
                    else {
                        return None;
                    };
                    let bits = self.types.target.integer(scalar)?.0;
                    bit_count(operation, value, bits)
                        .map(|count| Value::Integer(Integer::int(count)))
                },
                | BuiltinOperation::Bswap(bits) => {
                    let Value::Integer(value) = self.evaluate_value(*arguments.first()?, depth)?
                    else {
                        return None;
                    };
                    let raw = value.cast(bits, false).value as u128;
                    let swapped = raw.swap_bytes() >> (128 - bits);
                    Some(Value::Integer(
                        Integer::int(swapped as i128).cast(bits, false),
                    ))
                },
                | _ => None,
            },
        }
    }

    /// Whether `binding` is the hosted library's `strlen`, which Clang folds
    /// as `__builtin_strlen`: an external function declared with the
    /// builtin's type, `size_t (const char *)`. Another type makes it an
    /// ordinary function, as `unsigned long` does on the LLP64 targets.
    /// C99: §7.21.6.3 paragraph 1, p. 334; PDF p. 346.
    fn library_strlen(&self, binding: usize) -> bool {
        let data = self.bindings[binding];
        let TypeKind::Function {
            result,
            parameters: [parameter],
            prototype: true,
            variadic: false,
        } = self.types.nodes[data.ty.index]
        else {
            return false;
        };
        let TypeKind::Pointer(target) = self.types.nodes[parameter.index] else {
            return false;
        };
        data.kind == BindingKind::Function
            && data.linkage == super::super::Linkage::External
            && self.context.configuration.hosted()
            && self.context.string_cache.at(data.name.name) == "strlen"
            && self.types.nodes[result.index] == TypeKind::Scalar(self.types.target.size_t)
            && self.types.nodes[target.index] == TypeKind::Scalar(Scalar::Char)
            && target.qualifiers == TypeQualifiers::CONST
    }

    /// The length `strlen` gives for `argument`, as Clang's evaluator
    /// computes it (`EvaluateBuiltinStrLen`): of a string literal from an
    /// element, or of a `const` character array whose initializer
    /// evaluates, up to its first null element within the array.
    fn strlen_value(&mut self, argument: &'tu Expression<'tu>, depth: u32) -> Option<Value> {
        if let Some((initializer, offset, ty)) = self.object_place(argument, depth)
            && let TypeKind::Array(element, ArrayBound::Constant(count)) =
                self.types.nodes[ty.index]
        {
            let size = self.types.layout(element)?.size;
            for index in 0..count {
                let Value::Integer(unit) =
                    self.place_value(initializer, offset + index * size, element, depth)?
                else {
                    return None;
                };
                if unit.value == 0 {
                    return Some(Value::Integer(Integer::int(i128::from(index))));
                }
            }
            return None;
        }
        match self.evaluate_value(argument, depth)? {
            | Value::String(literal, offset) => self.string_strlen(literal, offset),
            | _ => None,
        }
    }

    /// The length `strlen` gives for element `offset` of string literal
    /// `literal`: the elements before the first null one.
    fn string_strlen(&self, literal: StringTokenType, offset: i128) -> Option<Value> {
        let mut index = offset;
        loop {
            let Value::Integer(unit) = self.string_element(literal, index)? else {
                return None;
            };
            if unit.value == 0 {
                return Some(Value::Integer(Integer::int(index - offset)));
            }
            index += 1;
        }
    }

    /// Whether a query operand has a constant value: in Clang's evaluator in
    /// a constant context, or, `at_runtime`, in the value function code
    /// emits. The start of a string literal counts, while an interior
    /// pointer or another non-null object's address does not. Only at run
    /// time does an element read through a non-`const` array compound
    /// literal count, or a comma that discards an ordinary nonvolatile read
    /// before an arithmetic constant. GNU extension: `__builtin_constant_p`.
    /// C99: §6.3.2.3 paragraph 6, p. 47; PDF p. 59 (pointer-to-integer casts).
    fn constant_query_value(
        &mut self,
        operand: &'tu Expression<'tu>,
        depth: u32,
        at_runtime: bool,
    ) -> bool {
        if depth >= DEPTH {
            return false;
        }
        self.is_constant_query(operand)
            || self.constant_query_string_address(operand)
            || (at_runtime && self.discarded_comma_constant(operand, depth))
            || ((at_runtime || !self.reads_array_literal(operand))
                && match self.evaluate_value(operand, depth) {
                    | Some(Value::Address(Some(_))) | None => false,
                    | Some(Value::String(_, offset)) => offset == 0,
                    | Some(_) => true,
                })
    }

    /// A comma whose left operand is an ordinary nonvolatile read, before an
    /// arithmetic constant: function code emits the read, and
    /// `llvm.is.constant` sees only the constant result. A pointer result is
    /// never constant there. GNU extension: `__builtin_constant_p`;
    /// C99 §6.5.17p2, p. 94; PDF p. 106.
    fn discarded_comma_constant(&mut self, mut operand: &'tu Expression<'tu>, depth: u32) -> bool {
        if depth >= DEPTH {
            return false;
        }
        while let ExpressionType::Parenthesized { expression } = operand.kind {
            operand = expression;
        }
        let ExpressionType::Binary {
            operator: BinaryOperator::Comma,
            left_expression,
            right_expression,
        } = operand.kind
        else {
            return false;
        };
        !self.has_side_effects(left_expression)
            && (self.discarded_comma_constant(right_expression, depth + 1)
                || matches!(
                    self.evaluate_value(right_expression, depth + 1),
                    Some(Value::Integer(_) | Value::Floating(_))
                ))
    }

    /// Whether evaluating `operand` reads through a non-`const` array
    /// compound literal, outside `sizeof` and `_Alignof`.
    /// C99: §6.5.2.5 paragraph 4, p. 75; PDF p. 87.
    fn reads_array_literal(&self, operand: &'tu Expression<'tu>) -> bool {
        let mut work = crate::util::bump::ArenaVec::new_in(self.scratch);
        work.push(operand);
        while let Some(expression) = work.pop() {
            match expression.kind {
                | ExpressionType::SizeofExpr(_)
                | ExpressionType::SizeofType(_)
                | ExpressionType::AlignofExpr(_)
                | ExpressionType::AlignofType(_) => {},
                | ExpressionType::CompoundLiteral { .. }
                    if self.array_typed(expression)
                        && !self.constant_object(self.expression_info(expression).ty) =>
                    return true,
                | _ => super::labels::expression_children(self.scratch, expression, |child| {
                    work.push(child);
                }),
            }
        }
        false
    }

    /// Clang's `Expr::HasSideEffects`, syntactically: an assignment, an
    /// increment or decrement, a call or a volatile read: a volatile lvalue
    /// that is converted to its value, which an object named or reached
    /// through `*`, `[]`, `.` or `->` is, unless it is an array or the
    /// operand of `&`, or a member's structure.
    /// C99: §6.3.2.1 paragraph 2, p. 46; PDF p. 58; §5.1.2.3 paragraph 2,
    /// p. 13; PDF p. 25.
    fn has_side_effects(&self, expression: &'tu Expression<'tu>) -> bool {
        let mut work = crate::util::bump::ArenaVec::new_in(self.scratch);
        // Each expression, with whether its value is read if it is an
        // lvalue.
        work.push((expression, true));
        while let Some((expression, read)) = work.pop() {
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
                | ExpressionType::Identifier(_) => read && self.volatile_lvalue(expression),
                | ExpressionType::Parenthesized { expression: inner } => {
                    work.push((inner, read));
                    false
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::AddressOf,
                    operand_expression,
                } => {
                    work.push((operand_expression, false));
                    false
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression: base_expression,
                }
                | ExpressionType::IndirectMember {
                    base_expression, ..
                } => {
                    work.push((base_expression, true));
                    read && self.volatile_lvalue(expression)
                },
                | ExpressionType::DirectMember {
                    base_expression, ..
                } => {
                    work.push((base_expression, false));
                    read && self.volatile_lvalue(expression)
                },
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    work.push((left_expression, true));
                    work.push((right_expression, true));
                    read && self.volatile_lvalue(expression)
                },
                | ExpressionType::SizeofExpr(_) | ExpressionType::AlignofExpr(_) => false,
                | _ => {
                    super::labels::expression_children(self.scratch, expression, |child| {
                        work.push((child, true));
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

    /// Whether lvalue `expression` has a volatile-qualified type that is
    /// not an array, whose value a read would load.
    fn volatile_lvalue(&self, expression: &'tu Expression<'tu>) -> bool {
        let ty = self.expression_info(expression).ty;
        ty.qualifiers.contains(TypeQualifiers::VOLATILE)
            && !matches!(
                self.types.nodes[ty.index],
                TypeKind::Array(..) | TypeKind::Function { .. }
            )
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
        if data.kind != BindingKind::Object {
            return;
        }
        if matches!(
            self.types.nodes[data.ty.index],
            TypeKind::Array(..) | TypeKind::Tag(_)
        ) {
            if self.constant_object(data.ty) {
                _ = self
                    .functions
                    .emission
                    .constant_aggregates
                    .insert(binding, initializer);
            }
        } else if data.ty.qualifiers.contains(TypeQualifiers::CONST)
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
    /// The address of an entity with an element offset, or a non-null
    /// address the evaluator cannot read through (`None`): a string literal
    /// cast to another pointer type.
    Address(Option<(usize, i128)>),
    /// A pointer to element `offset` of a string literal.
    String(StringTokenType, i128),
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
            | Self::Address(_) | Self::String(..) => Some(true),
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

/// One step from a named object to the subobject an expression designates,
/// in [`Analyzer::object_place`].
#[derive(Clone, Copy)]
enum Step<'tu> {
    /// An array element at the index this expression evaluates to.
    Index(&'tu Expression<'tu>),
    /// The first element of an array (`*array`).
    First,
    Member(StringCacheId),
}

/// What the last operation of an initializer plan covering a subobject
/// put there, in [`Analyzer::object_value`].
#[derive(Clone, Copy)]
enum Read<'tu> {
    Zero,
    Value(&'tu Expression<'tu>),
    /// An element of a string literal.
    String(StringTokenType, i128),
}

fn unparenthesized<'tu>(mut expression: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let ExpressionType::Parenthesized { expression: inner } = expression.kind {
        expression = inner;
    }
    expression
}

/// The count bit builtin `operation` gives for `value` converted to its
/// `bits`-wide operand type, as Clang's evaluator computes it: none for
/// `__builtin_clz` and `__builtin_ctz` of zero, which are undefined.
/// GNU extension: GCC manual, "Other Builtins".
fn bit_count(operation: BitOperation, value: Integer, bits: u32) -> Option<i128> {
    let mask = if bits >= 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    };
    let raw = (value.value as u128) & mask;
    let leading = |pattern: u128| i128::from(pattern.leading_zeros()) - i128::from(128 - bits);
    Some(match operation {
        | BitOperation::Clz if raw != 0 => leading(raw),
        | BitOperation::Ctz if raw != 0 => i128::from(raw.trailing_zeros()),
        | BitOperation::Clz | BitOperation::Ctz => return None,
        | BitOperation::Popcount => i128::from(raw.count_ones()),
        | BitOperation::Parity => i128::from(raw.count_ones() % 2),
        | BitOperation::Ffs if raw == 0 => 0,
        | BitOperation::Ffs => i128::from(raw.trailing_zeros()) + 1,
        | BitOperation::Clrsb => {
            let negative = (raw >> (bits - 1)) & 1 == 1;
            leading(if negative { !raw & mask } else { raw }) - 1
        },
    })
}
