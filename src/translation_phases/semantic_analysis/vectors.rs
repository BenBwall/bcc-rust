//! GNU vector type construction and element-wise operators.
//! Extension contracts: GCC Vector Extensions and Clang Language Extensions,
//! Vectors and Extended Vectors. No backend instruction selection is performed.

use super::{
    Analyzer,
    ArenaVec,
    AttributeSpecifier,
    BinaryOperator,
    Expression,
    ExpressionInfo,
    Scalar,
    SemanticErrorKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    expressions::{
        ConversionKind,
        ValueCategory,
    },
};

pub(super) fn constructs_vector(a: &AttributeSpecifier<'_>, context: &super::Context<'_>) -> bool {
    a.tokens
        .iter()
        .any(|t| context.string_cache.at(t.contents).trim_matches('_') == "vector_size")
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Group vector construction before alignment-only attributes,
    /// independently of which syntax attribute list carries them.
    pub(super) fn vector_attribute_chain(
        &mut self,
        mut ty: TypeId,
        chain: Option<&'tu super::SpecifierExtension<'tu>>,
    ) -> TypeId {
        for construction in [true, false] {
            let mut current = chain;
            while let Some(item) = current {
                if let super::SpecifierExtensionKind::Attributes(a) = item.kind
                    && constructs_vector(a, self.context) == construction
                {
                    ty = if self.layout_attribute(a) {
                        self.types.unknown()
                    } else {
                        self.vector_attribute(ty, a)
                    };
                }
                current = item.next;
            }
        }
        ty
    }

    pub(super) fn vector(&self, ty: TypeId) -> Option<(TypeId, u64, u64)> {
        if let TypeKind::Vector {
            element,
            count,
            align,
        } = self.types.nodes[ty.index]
        {
            Some((element, count, align))
        } else {
            None
        }
    }

    /// GNU `vector_size` operates on the element even on a pointer, array or
    /// function declarator. Explicit stacks rebuild the derivations.
    pub(super) fn vector_attribute(
        &mut self,
        mut ty: TypeId,
        a: &AttributeSpecifier<'tu>,
    ) -> TypeId {
        let tokens = a.tokens.as_slice();
        let mut bytes = None;
        let mut alignment = None;
        for (i, token) in tokens.iter().enumerate() {
            let name = self
                .context
                .string_cache
                .at(token.contents)
                .trim_matches('_');
            if name == "vector_size" || name == "aligned" {
                let value = if tokens.get(i + 1).is_some_and(|t| {
                    self.context
                        .string_cache
                        .at(t.contents)
                        .trim_end_matches('\0')
                        == "("
                }) {
                    // The parser retains attribute arguments as tokens. Resolve
                    // integer literals and enum constants with a bounded
                    // reducer.
                    self.vector_attribute_integer(&tokens[i + 2..])
                } else if name == "aligned" {
                    Some(16)
                } else {
                    None
                };
                if name == "vector_size" {
                    bytes = Some(value);
                } else {
                    alignment = Some(value);
                }
            }
        }
        if bytes.is_none() && alignment.is_none() {
            return ty;
        }
        let mut derived = ArenaVec::new_in(self.scratch);
        loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Pointer(next) | TypeKind::Array(next, _) => {
                    derived.push((self.types.nodes[ty.index], ty.qualifiers));
                    ty = next;
                },
                | TypeKind::Function { result, .. } => {
                    derived.push((self.types.nodes[ty.index], ty.qualifiers));
                    ty = result;
                },
                | _ => break,
            }
        }
        let mut vector = self.vector(ty);
        if let Some(size) = bytes {
            let valid_element = matches!(self.types.nodes[ty.index], TypeKind::Scalar(s) if !matches!(s, Scalar::Void | Scalar::Bool | Scalar::ComplexFloat | Scalar::ComplexDouble | Scalar::ComplexLongDouble));
            let layout = self.types.layout(ty);
            if !valid_element
                || size.is_none()
                || !size.is_some_and(|n| {
                    layout
                        .is_some_and(|l| n >= l.size && n % l.size == 0 && i64::try_from(n).is_ok())
                })
            {
                self.error(
                    SemanticErrorKind::InvalidVectorAttribute,
                    a.source_vectors,
                    None,
                    None,
                );
                return self.types.unknown();
            }
            if let (Some(size), Some(layout)) = (size, layout) {
                vector = Some((
                    ty.unqualified(),
                    size / layout.size,
                    size.next_power_of_two(),
                ));
            }
        }
        if let Some((element, count, mut align)) = vector {
            if let Some(value) = alignment {
                if let Some(value) = value.filter(|n| n.is_power_of_two()) {
                    align = value;
                } else {
                    self.error(
                        SemanticErrorKind::InvalidVectorAttribute,
                        a.source_vectors,
                        None,
                        None,
                    );
                    return self.types.unknown();
                }
            }
            ty = self
                .types
                .intern(TypeKind::Vector {
                    element,
                    count,
                    align,
                })
                .qualified(ty.qualifiers);
        } else if alignment.is_some() {
            // Non-vector alignment remains an explicitly unanalyzed layout.
            return self.types.unknown();
        }
        while let Some((kind, q)) = derived.pop() {
            let kind = match kind {
                | TypeKind::Pointer(_) => TypeKind::Pointer(ty),
                | TypeKind::Array(_, bound) => TypeKind::Array(ty, bound),
                | TypeKind::Function {
                    parameters,
                    prototype,
                    variadic,
                    ..
                } => TypeKind::Function {
                    result: ty,
                    parameters,
                    prototype,
                    variadic,
                },
                | _ => return self.types.unknown(),
            };
            ty = self.types.intern(kind).qualified(q);
        }
        ty
    }

    fn vector_attribute_integer(
        &self,
        tokens: &[super::super::preprocessing::Token],
    ) -> Option<u64> {
        let mut values = ArenaVec::new_in(self.scratch);
        let mut ops = ArenaVec::new_in(self.scratch);
        let precedence = |op: &str| match op {
            | "|" => 1,
            | "^" => 2,
            | "&" => 3,
            | "<<" | ">>" => 4,
            | "+" | "-" => 5,
            | "*" | "/" | "%" => 6,
            | _ => 0,
        };
        let reduce = |values: &mut ArenaVec<'_, u64>, op: &str| -> Option<()> {
            let b = values.pop()?;
            let a = values.pop()?;
            let n = match op {
                | "+" => a.checked_add(b)?,
                | "-" => a.checked_sub(b)?,
                | "*" => a.checked_mul(b)?,
                | "/" => a.checked_div(b)?,
                | "%" => a.checked_rem(b)?,
                | "<<" => a.checked_shl(u32::try_from(b).ok()?)?,
                | ">>" => a.checked_shr(u32::try_from(b).ok()?)?,
                | "&" => a & b,
                | "|" => a | b,
                | "^" => a ^ b,
                | _ => return None,
            };
            values.push(n);
            Some(())
        };
        let mut depth = 0;
        for t in tokens {
            let text = self
                .context
                .string_cache
                .at(t.contents)
                .trim_end_matches('\0');
            if text == ")" {
                if depth == 0 {
                    break;
                }
                while let Some(op) = ops.pop() {
                    if op == "(" {
                        break;
                    }
                    reduce(&mut values, op)?;
                }
                depth -= 1;
            } else if text == "(" {
                depth += 1;
                ops.push(text);
            } else if precedence(text) != 0 {
                while ops
                    .last()
                    .is_some_and(|op| *op != "(" && precedence(op) >= precedence(text))
                {
                    reduce(&mut values, ops.pop()?)?;
                }
                ops.push(text);
            } else {
                let parsed = if let super::super::preprocessing::TokenType::Integer(value) = t.kind
                {
                    u64::try_from(i128::from(value)).ok()
                } else {
                    None
                };
                let n = parsed.or_else(|| {
                    self.lookup(super::Namespace::Ordinary, t.contents)
                        .and_then(|b| self.bindings[b.binding].value)
                        .and_then(super::Integer::to_u64)
                })?;
                values.push(n);
            }
        }
        while let Some(op) = ops.pop() {
            reduce(&mut values, op)?;
        }
        if values.len() == 1 {
            values.pop()
        } else {
            None
        }
    }

    pub(super) fn vector_width(&self, ty: TypeId) -> Option<u64> {
        if let Some((element, count, _)) = self.vector(ty) {
            self.types.layout(element)?.size.checked_mul(count)
        } else {
            self.types.layout(ty).map(|l| l.size)
        }
    }

    pub(super) fn vector_assignment(&self, to: TypeId, from: TypeId) -> bool {
        self.vector(to).is_some()
            && self.vector(from).is_some()
            && self.vector_width(to) == self.vector_width(from)
    }

    fn vector_splat(&self, element: TypeId, scalar: ExpressionInfo<'tu>) -> bool {
        if let Some((bits, signed)) = self.integer_type(element) {
            if let Some(value) = scalar.integer.filter(|_| scalar.ice) {
                return if signed {
                    value.to_i128().is_some_and(|n| {
                        bits == 128
                            || (n >= -(1_i128 << (bits - 1))
                                && n <= ((1_u128 << (bits - 1)) - 1) as i128)
                    })
                } else if value.signed && value.value < 0 {
                    self.integer_type(scalar.ty).is_some_and(|(b, _)| b <= bits)
                } else {
                    bits == 128 || (value.value as u128) < (1_u128 << bits)
                };
            }
            return self
                .integer_type(scalar.ty)
                .is_some_and(|(b, s)| b < bits || (b == bits && (s || !signed)));
        }
        let precision = match self.types.nodes[element.index] {
            | TypeKind::Scalar(Scalar::Float) => 24,
            | TypeKind::Scalar(Scalar::Double) => 53,
            | TypeKind::Scalar(Scalar::LongDouble) =>
                if self.types.target.long_double_is_double() {
                    53
                } else {
                    64
                },
            | _ => return false,
        };
        if scalar.constant == super::ConstantClass::Arithmetic
            && let Some(value) = self.floating_value(scalar)
        {
            return self.round_floating(value, element).is_some_and(|rounded| {
                rounded.real.compare(value.real) == 0 && value.imag.is_zero()
            });
        }
        if let Some((bits, signed)) = self.integer_type(scalar.ty) {
            return bits - u32::from(signed) <= precision as u32;
        }
        let source_precision = match self.types.nodes[scalar.ty.index] {
            | TypeKind::Scalar(Scalar::Float) => 24,
            | TypeKind::Scalar(Scalar::Double) => 53,
            | TypeKind::Scalar(Scalar::LongDouble) =>
                if self.types.target.long_double_is_double() {
                    53
                } else {
                    64
                },
            | _ => return false,
        };
        source_precision <= precision
    }

    pub(super) fn vector_binary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> Option<ExpressionInfo<'tu>> {
        use BinaryOperator as B;
        let lv = self.vector(left.ty);
        let rv = self.vector(right.ty);
        if lv.is_none() && rv.is_none() {
            return None;
        }
        if op == B::Comma {
            return Some(Self::expression_result(e, right.ty.unqualified()));
        }
        if op == B::Subscript {
            if let Some((element, _, _)) = lv
                && self.integer_type(right.ty).is_some()
            {
                let element = element.qualified(left.ty.qualifiers);
                let mut result = Self::expression_result(e, element);
                result.category = if left.category == ValueCategory::Rvalue {
                    ValueCategory::Rvalue
                } else if element.qualifiers.contains(TypeQualifiers::CONST) {
                    ValueCategory::Lvalue
                } else {
                    left.category
                };
                // Clang disallows taking the address of a vector element.
                result.register = left.register;
                return Some(result);
            }
            return Some(self.vector_error(e));
        }
        let assignment = matches!(
            op,
            B::Assignment
                | B::AdditionAssignment
                | B::SubtractionAssignment
                | B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseOrAssignment
                | B::BitwiseXorAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment
        );
        if assignment && left.category != ValueCategory::ModifiableLvalue {
            return Some(self.vector_error(e));
        }
        if op == B::Assignment {
            return Some(if self.vector_assignment(left.ty, right.ty) {
                Self::expression_result(e, left.ty.unqualified())
            } else {
                self.vector_error(e)
            });
        }
        let (element, count, align) = lv.or(rv)?;
        let valid = match (lv, rv) {
            | (Some(_), Some(_)) => self.vector_assignment(left.ty, right.ty),
            | (Some(_), None) => self.vector_splat(element, right),
            | (None, Some(_)) => !assignment && self.vector_splat(element, left),
            | _ => false,
        };
        let integer = self.integer_type(element).is_some();
        let allowed = match op {
            | B::Addition
            | B::Subtraction
            | B::Multiplication
            | B::Division
            | B::AdditionAssignment
            | B::SubtractionAssignment
            | B::MultiplicationAssignment
            | B::DivisionAssignment
            | B::Equal
            | B::NotEqual
            | B::LessThan
            | B::LessThanOrEqual
            | B::GreaterThan
            | B::GreaterThanOrEqual => true,
            | B::Modulo
            | B::LeftShift
            | B::RightShift
            | B::BitwiseAnd
            | B::BitwiseOr
            | B::BitwiseXor
            | B::ModuloAssignment
            | B::LeftShiftAssignment
            | B::RightShiftAssignment
            | B::BitwiseAndAssignment
            | B::BitwiseOrAssignment
            | B::BitwiseXorAssignment => integer,
            | _ => false,
        };
        if !valid || !allowed {
            return Some(self.vector_error(e));
        }
        let comparison = matches!(
            op,
            B::Equal
                | B::NotEqual
                | B::LessThan
                | B::LessThanOrEqual
                | B::GreaterThan
                | B::GreaterThanOrEqual
        );
        let result_element = if comparison {
            let size = self.types.layout(element)?.size;
            self.types.scalar(match size {
                | 1 => Scalar::SignedChar,
                | 2 => Scalar::Short,
                | 4 => Scalar::Int,
                | 8 => Scalar::LongLong,
                | 16 => Scalar::Int128,
                | _ => return Some(self.vector_error(e)),
            })
        } else {
            element
        };
        let ty = self.types.intern(TypeKind::Vector {
            element: result_element,
            count,
            align: if comparison {
                self.types
                    .layout(result_element)?
                    .size
                    .checked_mul(count)?
                    .checked_next_power_of_two()?
            } else {
                align
            },
        });
        let operand_ty = self.types.intern(TypeKind::Vector {
            element,
            count,
            align,
        });
        self.convert(left.expression, operand_ty, ConversionKind::Arithmetic);
        self.convert(right.expression, operand_ty, ConversionKind::Arithmetic);
        Some(Self::expression_result(
            e,
            if assignment {
                left.ty.unqualified()
            } else {
                ty
            },
        ))
    }

    pub(super) fn vector_error(&mut self, e: &'tu Expression<'tu>) -> ExpressionInfo<'tu> {
        self.error(
            SemanticErrorKind::InvalidVectorOperand,
            e.source_vectors,
            None,
            None,
        );
        Self::expression_result(e, self.types.unknown())
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn convert_vector_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &super::super::parsing::Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        let operands = b.operands.as_slice();
        if let [
            super::SyntaxOperand::Expression(input),
            super::SyntaxOperand::Type(name),
        ] = operands
        {
            let from = self.expression_info(input);
            let to = self
                .resolved_type_names
                .get(&name.source_vectors)
                .copied()
                .unwrap_or_else(|| self.types.unknown());
            if self.types.unanalyzed(from.ty) || self.types.unanalyzed(to) {
                return Self::expression_result(e, self.types.unknown());
            }
            if let (Some((_, n, _)), Some((_, m, _))) = (self.vector(from.ty), self.vector(to))
                && n == m
            {
                return Self::expression_result(e, to.unqualified());
            }
        }
        self.error(
            SemanticErrorKind::InvalidVectorBuiltin,
            e.source_vectors,
            None,
            None,
        );
        Self::expression_result(e, self.types.unknown())
    }

    pub(super) fn shuffle_vector_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        arguments: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        let args = arguments.as_slice();
        let mut result = None;
        if args.len() >= 3 {
            let a = self.expression_info(args[0]);
            let b = self.expression_info(args[1]);
            if let (Some((element, n, _)), Some((other, m, _))) =
                (self.vector(a.ty), self.vector(b.ty))
                && element == other
                && n == m
            {
                let valid = args[2..].iter().all(|arg| {
                    let info = self.expression_info(arg);
                    info.ice
                        && info
                            .integer
                            .and_then(super::Integer::to_i128)
                            .is_some_and(|v| v >= -1 && v < i128::from(n) * 2)
                });
                let count = (args.len() - 2) as u64;
                if valid {
                    let align = self.types.layout(element).map_or(1, |l| l.size) * count;
                    let align = align.next_power_of_two();
                    result = Some(self.types.intern(TypeKind::Vector {
                        element,
                        count,
                        align,
                    }));
                }
            }
        }
        if let Some(ty) = result {
            return Self::expression_result(e, ty);
        }
        self.error(
            SemanticErrorKind::InvalidVectorBuiltin,
            e.source_vectors,
            None,
            None,
        );
        Self::expression_result(e, self.types.unknown())
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn bit_cast_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &super::super::parsing::Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        if let [
            super::SyntaxOperand::Type(name),
            super::SyntaxOperand::Expression(input),
        ] = b.operands.as_slice()
        {
            let from = self.expression_info(input);
            let to = self
                .resolved_type_names
                .get(&name.source_vectors)
                .copied()
                .unwrap_or_else(|| self.types.unknown());
            if self.types.unanalyzed(from.ty) || self.types.unanalyzed(to) {
                return Self::expression_result(e, self.types.unknown());
            }
            if self.types.layout(to).is_some()
                && self.types.layout(to).map(|l| l.size)
                    == self.types.layout(from.ty).map(|l| l.size)
            {
                return Self::expression_result(e, to.unqualified());
            }
        }
        self.error(
            SemanticErrorKind::InvalidVectorBuiltin,
            e.source_vectors,
            None,
            None,
        );
        Self::expression_result(e, self.types.unknown())
    }
}
