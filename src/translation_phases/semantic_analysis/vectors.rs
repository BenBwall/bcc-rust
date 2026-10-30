//! GNU vector type construction and element-wise operators.
//! Extension contracts: GCC Vector Extensions and Clang Language Extensions,
//! Vectors and Extended Vectors. No backend instruction selection is performed.

use super::{
    Analyzer,
    SemanticErrorKind,
    expressions::{
        ConversionKind,
        ExpressionInfo,
        ValueCategory,
    },
    types::{
        Scalar,
        TypeId,
        TypeKind,
        UnmodeledType,
    },
};
use crate::{
    configuration::RuleSet,
    translation_phases::parsing::{
        AttributeSpecifier,
        declaration_syntax::TypeQualifiers,
        syntax::{
            BinaryOperator,
            Expression,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Group vector construction before alignment-only attributes,
    /// independently of which syntax attribute list carries them.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    pub(super) fn vector_attribute_chain(
        &mut self,
        mut ty: TypeId,
        chain: Option<&'tu super::SpecifierExtension<'tu>>,
        storage_alignment: bool,
    ) -> TypeId {
        let mut current = chain;
        while let Some(item) = current {
            if let super::SpecifierExtensionKind::Attributes(a) = item.kind {
                ty = self.mode_attribute(ty, a);
            }
            current = item.next;
        }
        for construction in [true, false] {
            let mut current = chain;
            while let Some(item) = current {
                if let super::SpecifierExtensionKind::Attributes(a) = item.kind
                    && constructs_vector(a, self.context) == construction
                {
                    ty = if self.layout_attribute(a) {
                        self.types.unmodeled(UnmodeledType::ExtendedVector)
                    } else {
                        self.vector_attribute(ty, a, storage_alignment)
                    };
                }
                current = item.next;
            }
        }
        ty
    }

    /// Checks and types element-wise vector operators and comparisons.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    pub(super) fn vector_binary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> Option<ExpressionInfo<'tu>> {
        use BinaryOperator as B;
        // C11 §6.3.2.1p2, p. 54; PDF p. 72: value conversion removes
        // atomicity. Keep the original left category for assignment checks.
        let lt = self.types.non_atomic(left.ty);
        let rt = self.types.non_atomic(right.ty);
        let lv = self.vector(lt);
        let rv = self.vector(rt);
        if lv.is_none() && rv.is_none() {
            return None;
        }
        if op == B::Comma {
            let rt = self.converted(right);
            return Some(Self::expression_result(e, rt));
        }
        if op == B::Subscript {
            if let Some((element, _, _)) = lv
                && self.integer_type(rt).is_some()
            {
                // C99 §6.3.2.1p2: the index is read; the vector stays an
                // lvalue whose lane is designated.
                _ = self.converted(right);
                let atomic = matches!(self.types.nodes[left.ty.index], TypeKind::Atomic(_));
                let rules = self.context.configuration.compiler_identity().rules();
                if atomic && rules == RuleSet::Gcc && left.category != ValueCategory::Rvalue {
                    // GCC designates an atomic lane of an atomic vector
                    // lvalue: `a[1] = 2` stores it atomically, and `&a[1]`
                    // has type `_Atomic int *`.
                    let lane = self
                        .types
                        .intern(TypeKind::Atomic(element.unqualified()))
                        .qualified(element.qualifiers | left.ty.qualifiers);
                    let mut result = Self::expression_result(e, lane);
                    result.category = self.object_category(lane);
                    result.register = left.register;
                    return Some(result);
                }
                let element = element.qualified(left.ty.qualifiers);
                let mut result = Self::expression_result(e, element);
                result.category = if left.category == ValueCategory::Rvalue {
                    ValueCategory::Rvalue
                } else if atomic {
                    // C11 §6.3.2.1p2: a lane of an atomic vector is read
                    // from one atomic load of the whole.
                    _ = self.converted(left);
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
            return Some(if self.vector_value_assignment(lt, right, rt) {
                _ = self.converted(right);
                self.convert(
                    right.expression,
                    lt.unqualified(),
                    ConversionKind::Assignment,
                );
                Self::expression_result(e, lt.unqualified())
            } else {
                self.vector_error(e)
            });
        }
        let rules = self.context.configuration.compiler_identity().rules();
        let shift = matches!(
            op,
            B::LeftShift | B::RightShift | B::LeftShiftAssignment | B::RightShiftAssignment
        );
        // The vector operand fixes the operation's type; with two, the
        // left one does, as GCC and Clang type `a + b`.
        let (vector_ty, (element, count, align)) = match (lv, rv) {
            | (Some(v), _) => (lt, v),
            | (None, Some(v)) => (rt, v),
            | (None, None) => return None,
        };
        // How each operand becomes a vector of the operation's type.
        let mut conversions = [Some(ConversionKind::Arithmetic); 2];
        let valid = match (lv, rv) {
            // GNU lane-wise extension of C99 §6.5.7p2: a vector shifts by a
            // vector of as many integer lanes, whatever their width; the
            // count keeps its type.
            // GCC's enumeration lanes shift and count nothing.
            | (Some((_, n, _)), Some((other, m, _))) if shift => {
                conversions[1] = None;
                n == m
                    && self.integer_type(other).is_some()
                    && !self.gcc_enumeration_lanes(element, rules)
                    && !self.gcc_enumeration_lanes(other, rules)
            },
            // Clang refuses two atomic operands of an arithmetic operator,
            // though not of a comparison.
            | (Some(_), Some(_)) =>
                self.vector_operands(lt, rt, rules)
                    && !(rules == RuleSet::Clang
                        && !compares(op)
                        && matches!(self.types.nodes[left.ty.index], TypeKind::Atomic(_))
                        && matches!(self.types.nodes[right.ty.index], TypeKind::Atomic(_))),
            // A vector shifts by any integer count, which each lane takes;
            // GCC's enumeration lanes take no scalar.
            | (Some(_), None) if shift =>
                self.integer_type(rt).is_some() && !self.gcc_enumeration_lanes(element, rules),
            // Clang's scalar splat wants an ordinary vector; GCC reads an
            // atomic one first, and converts the scalar operand of a
            // compound assignment to it as a run-time value.
            | (Some(_), None) => {
                let atomic = matches!(self.types.nodes[left.ty.index], TypeKind::Atomic(_));
                let scalar = if atomic && assignment {
                    run_time(ExpressionInfo { ty: rt, ..right })
                } else {
                    ExpressionInfo { ty: rt, ..right }
                };
                (rules == RuleSet::Gcc || !atomic)
                    && match self.scalar_to_vector(element, lt, scalar, assignment, rules) {
                        | Some(kind) => {
                            conversions[1] = Some(kind);
                            true
                        },
                        | None => false,
                    }
            },
            // A scalar shifted by a vector: Clang converts any integer, GCC
            // only one the lanes hold.
            | (None, Some(_)) if shift =>
                !assignment
                    && self.integer_type(lt).is_some()
                    && (rules == RuleSet::Clang
                        || self.splat_operand(element, ExpressionInfo { ty: lt, ..left }, rules)),
            | (None, Some(_)) =>
                !assignment
                    && (rules == RuleSet::Gcc
                        || !matches!(self.types.nodes[right.ty.index], TypeKind::Atomic(_)))
                    && match self.scalar_to_vector(
                        element,
                        rt,
                        ExpressionInfo { ty: lt, ..left },
                        false,
                        rules,
                    ) {
                        | Some(kind) => {
                            conversions[0] = Some(kind);
                            true
                        },
                        | None => false,
                    },
            | (None, None) => false,
        };
        // GNU lane-wise extension of C99 §6.5.5p2, §6.5.7p2 and §6.5.10p2:
        // both vector operands must have integer elements.
        let integer = self.integer_type(element).is_some()
            && lv.is_none_or(|v| self.integer_type(v.0).is_some())
            && rv.is_none_or(|v| self.integer_type(v.0).is_some());
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
        let comparison = compares(op);
        let ty = if comparison {
            let size = self.types.layout(element)?.size;
            let mask = self.types.scalar(match size {
                | 1 => Scalar::SignedChar,
                | 2 => Scalar::Short,
                | 4 => Scalar::Int,
                | 8 => Scalar::LongLong,
                | 16 => Scalar::Int128,
                | _ => return Some(self.vector_error(e)),
            });
            self.types.intern(TypeKind::Vector {
                element: mask,
                count,
                align: self
                    .types
                    .layout(mask)?
                    .size
                    .checked_mul(count)?
                    .checked_next_power_of_two()?,
            })
        } else {
            vector_ty.unqualified()
        };
        let operand_ty = self.types.intern(TypeKind::Vector {
            element,
            count,
            align,
        });
        // C99 §6.3.2.1p2: each operand is read before it converts.
        _ = self.converted(left);
        _ = self.converted(right);
        if let Some(kind) = conversions[0] {
            self.convert(left.expression, operand_ty, kind);
        }
        if let Some(kind) = conversions[1] {
            self.convert(right.expression, operand_ty, kind);
        }
        let mut result = Self::expression_result(e, if assignment { lt.unqualified() } else { ty });
        result.opaque_vector = comparison && rules == RuleSet::Gcc;
        // C99 §6.5.16.2p3: a compound assignment computes in the
        // operation's type.
        if assignment {
            result.operation_type = Some(operand_ty);
        }
        Some(result)
    }

    /// Whether two vectors may be the operands of one operator. GCC wants
    /// as many lanes of one kind and width, signed or not (its
    /// `vector_types_compatible_elements_p`); Clang's default lax vector
    /// conversions want only equal sizes, the right operand then reading
    /// as the left's type.
    /// GNU extension: GCC manual, "Vector Extensions": "the operation is
    /// only possible on vectors of the same number of elements".
    fn vector_operands(&self, left: TypeId, right: TypeId, rules: RuleSet) -> bool {
        match rules {
            | RuleSet::Clang => self.vector_width(left) == self.vector_width(right),
            | RuleSet::Gcc => {
                let (Some((a, n, _)), Some((b, m, _))) = (self.vector(left), self.vector(right))
                else {
                    return false;
                };
                n == m
                    && signed_kind(self.types.nodes[a.index])
                        == signed_kind(self.types.nodes[b.index])
            },
        }
    }

    /// How the scalar operand `scalar` of an operator with a vector of
    /// lane type `element` (and type `vector`) becomes that vector: by
    /// [`ConversionKind::Arithmetic`], which converts it to the lane type
    /// and fills every lane, or, under Clang's lax vector conversions, by
    /// [`ConversionKind::Assignment`], which reads a scalar as large as
    /// the whole vector as its bits. `None` if it cannot.
    fn scalar_to_vector(
        &self,
        element: TypeId,
        vector: TypeId,
        scalar: ExpressionInfo<'tu>,
        compound: bool,
        rules: RuleSet,
    ) -> Option<ConversionKind> {
        if self.splat_operand(element, scalar, rules) {
            return Some(ConversionKind::Arithmetic);
        }
        // Clang `CheckVectorOperands`: "If we're allowing lax vector
        // conversions, only the total (data) size needs to be the same", for
        // a real scalar outside a compound assignment.
        (rules == RuleSet::Clang
            && !compound
            && self.scalar_class(scalar.ty).is_some()
            && self.types.layout(scalar.ty).map(|l| l.size) == self.vector_width(vector))
        .then_some(ConversionKind::Assignment)
    }

    /// Whether GCC steps an atomic vector of lane type `element` with `++`
    /// or `--`: it adds the `int` one as a run-time scalar, which only
    /// lanes that hold every `int` take ("conversion of scalar 'int' to
    /// vector ... involves truncation" otherwise).
    pub(super) fn gcc_steps_atomic_vector(
        &mut self,
        element: TypeId,
        operand: ExpressionInfo<'tu>,
    ) -> bool {
        let ty = self.types.scalar(Scalar::Int);
        let one = run_time(ExpressionInfo { ty, ..operand });
        self.splat_operand(element, one, RuleSet::Gcc)
    }

    /// Whether GCC's rules give a vector of lane type `element` lanes of an
    /// enumeration, which no scalar operand converts to ("cannot convert a
    /// value of type 'int' to vector type ... which has different size").
    fn gcc_enumeration_lanes(&self, element: TypeId, rules: RuleSet) -> bool {
        rules == RuleSet::Gcc
            && matches!(
                self.types.nodes[element.unqualified().index],
                TypeKind::Tag(id) if self.types.tags[id].kind == super::TagKind::Enum
            )
    }

    /// Whether a vector of type `from` converts implicitly to `to`, as in
    /// assignment, initialization, argument passing and `return`. Clang's
    /// default lax vector conversions accept equal sizes; GCC wants as many
    /// lanes of compatible types, whatever their alignment (its
    /// `vector_types_convertible_p` without `-flax-vector-conversions`).
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    pub(super) fn vector_assignment(&self, to: TypeId, from: TypeId) -> bool {
        let (Some((a, n, _)), Some((b, m, _))) = (self.vector(to), self.vector(from)) else {
            return false;
        };
        match self.context.configuration.compiler_identity().rules() {
            | RuleSet::Clang => self.vector_width(to) == self.vector_width(from),
            | RuleSet::Gcc => n == m && a.unqualified() == b.unqualified(),
        }
    }

    /// [`Self::vector_assignment`] for the value `source` of type `from`:
    /// GCC also converts a comparison's opaque mask to any vector of its
    /// size (`vector_types_convertible_p`, "`TYPE_VECTOR_OPAQUE`").
    pub(super) fn vector_value_assignment(
        &self,
        to: TypeId,
        source: ExpressionInfo<'tu>,
        from: TypeId,
    ) -> bool {
        self.vector_assignment(to, from)
            || (source.opaque_vector
                && self.vector(to).is_some()
                && self.vector_width(to) == self.vector_width(from))
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
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    pub(super) fn vector_attribute(
        &mut self,
        mut ty: TypeId,
        a: &AttributeSpecifier<'tu>,
        storage_alignment: bool,
    ) -> TypeId {
        let tokens = a.tokens;
        let mut bytes = None;
        let mut alignment = None;
        for (i, name) in super::attributes::known_names(a, self.context) {
            if name == "vector_size" || name == "aligned" {
                let value = if tokens.get(i + 1).is_some_and(|t| {
                    self.context
                        .string_cache
                        .at(t.contents)
                        .trim_end_matches('\0')
                        == "("
                }) {
                    if name == "aligned" {
                        self.attribute_alignment(a, i)
                    } else {
                        // The analyzed argument, an integer constant
                        // expression (`sizeof (int) * 4`); the bounded token
                        // reducer covers an argument analysis did not keep.
                        self.attribute_alignment(a, i)
                            .or_else(|| self.vector_attribute_integer(&tokens[i + 2..]))
                    }
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
        // GNU vector_size extends C99 §6.2.5p20 derived types. Alignment
        // alone applies only to this type; non-vector layout stays unanalyzed.
        let mut derived = ArenaVec::new_in(self.scratch);
        while bytes.is_some() {
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
        let gcc = self.context.configuration.compiler_identity().rules() == RuleSet::Gcc;
        if let Some(size) = bytes {
            // GNU vector elements extend the real/integer types of C99 §6.2.5:
            // both compilers take the half-precision types; Clang also a
            // bit-precise integer of a power-of-two width of at least 8 bits,
            // GCC an enumeration. Clang applies the attribute to the declared
            // type itself, so a pointer, array or function element is
            // invalid; GCC applies it to their innermost element.
            let valid_element = (gcc || derived.is_empty())
                && (matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(Scalar::Float16 | Scalar::BFloat16)
                ) || matches!(
                    self.types.nodes[ty.index],
                    TypeKind::BitInt { width, .. }
                        if !gcc && (8..=128).contains(&width) && width.is_power_of_two()
                ) || matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Tag(id)
                        if gcc && self.types.tags[id].kind == super::TagKind::Enum
                            && self.types.tags[id].complete.get()
                ) || matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(
                        Scalar::Char
                            | Scalar::SignedChar
                            | Scalar::UnsignedChar
                            | Scalar::Short
                            | Scalar::UnsignedShort
                            | Scalar::Int
                            | Scalar::UnsignedInt
                            | Scalar::Long
                            | Scalar::UnsignedLong
                            | Scalar::LongLong
                            | Scalar::UnsignedLongLong
                            | Scalar::Int128
                            | Scalar::UnsignedInt128
                            | Scalar::Float
                            | Scalar::Double
                            | Scalar::LongDouble
                            | Scalar::Float128
                    )
                ));
            let layout = self.types.layout(ty);
            // GCC also wants a power-of-two lane count ("number of vector
            // components ... not a power of two"); Clang rounds the object
            // up instead.
            if !valid_element
                || size.is_none()
                || !size.is_some_and(|n| {
                    layout.is_some_and(|l| {
                        n >= l.size
                            && n % l.size == 0
                            && i64::try_from(n).is_ok()
                            && (!gcc || (n / l.size).is_power_of_two())
                    })
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
        } else if alignment.is_some() && storage_alignment {
            let attributes = self.layout_attributes(a);
            ty = self.align_declared_type(ty, attributes.alignment);
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

    /// Evaluates integer attribute arguments using the supported
    /// constant-expression operators.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    /// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
    pub(super) fn vector_attribute_integer(
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

    /// Whether a scalar operand converts to the lane type `element` and
    /// fills the lanes, under the claimed compiler's rules. Both accept a
    /// constant the lane type holds and a variable whose type the lane type
    /// holds; they differ at the edges:
    ///
    /// - GCC (`scalar_to_vector` and `unsafe_conversion_p`) checks an integer
    ///   variable by width and accepts a sign change, an integer constant by
    ///   value, letting a negative one into unsigned lanes and an unsigned one
    ///   into signed lanes; it accepts an integer for floating lanes whose
    ///   format holds its type's range, never a floating scalar for integer
    ///   lanes, and no `_Bool` or enumeration.
    /// - Clang (`tryGCCVectorConvertAndSplat`) checks an integer variable by
    ///   rank, refusing a sign change into lanes of equal rank, and an integer
    ///   constant by its significant bits; it converts a floating scalar to
    ///   integer lanes of its size, an integer to floating lanes whose
    ///   significand is at least its width, and accepts `_Bool` and
    ///   enumerations as integers.
    ///
    /// GNU extension: GCC manual, "Vector Extensions"; Clang Language
    /// Extensions, "Vectors and Extended Vectors".
    fn splat_operand(&self, element: TypeId, scalar: ExpressionInfo<'tu>, rules: RuleSet) -> bool {
        if self.gcc_enumeration_lanes(element, rules) {
            return false;
        }
        let (Some(lane), Some(class)) = (self.scalar_class(element), self.scalar_class(scalar.ty))
        else {
            return false;
        };
        let constant = scalar.integer.filter(|_| scalar.ice);
        match (lane, class, rules) {
            | (
                ScalarClass::Integer { .. },
                ScalarClass::Integer { plain: false, .. },
                RuleSet::Gcc,
            )
            | (
                ScalarClass::Floating { .. },
                ScalarClass::Integer { plain: false, .. },
                RuleSet::Gcc,
            )
            | (ScalarClass::Integer { .. }, ScalarClass::Floating { .. }, RuleSet::Gcc) => false,
            | (
                ScalarClass::Integer { bits, signed, .. },
                ScalarClass::Integer {
                    bits: from,
                    signed: from_signed,
                    ..
                },
                RuleSet::Gcc,
            ) => match constant {
                | Some(value) =>
                    fits(value, bits, signed)
                        || (!signed && value.signed && value.value < 0)
                        || (signed && !from_signed),
                | None => from <= bits,
            },
            | (
                lane @ ScalarClass::Integer { bits, signed, .. },
                class @ ScalarClass::Integer { .. },
                RuleSet::Clang,
            ) => {
                // An enumeration of the lanes' own rank falls through to
                // Clang's final "enumeral type" refusal.
                let order = clang_integer_order(lane, class);
                if order == 0 {
                    return matches!(class, ScalarClass::Integer { plain: true, .. });
                }
                let ScalarClass::Integer {
                    signed: from_signed,
                    ..
                } = class
                else {
                    return false;
                };
                match constant {
                    | Some(value) => {
                        let needed = significant_bits(value, from_signed);
                        !(bits < needed && (order < 0 || from_signed != signed))
                    },
                    | None => order > 0,
                }
            },
            | (ScalarClass::Integer { .. }, ScalarClass::Floating { .. }, RuleSet::Clang) =>
                self.types.layout(element).map(|l| l.size)
                    == self.types.layout(scalar.ty).map(|l| l.size),
            | (
                ScalarClass::Floating { precision, .. },
                ScalarClass::Integer { bits, signed, .. },
                rules,
            ) => match constant {
                | Some(value) => exact_in(value, precision),
                | None if rules == RuleSet::Gcc => bits - u32::from(signed) <= precision,
                | None => bits <= precision,
            },
            | (
                ScalarClass::Floating { precision, .. },
                ScalarClass::Floating {
                    precision: from, ..
                },
                _,
            ) => {
                // GNU binary128 widening uses type precision, not the
                // long-double rounding bridge, which cannot represent a
                // binary128 destination.
                if precision != 113
                    && scalar.constant == super::ConstantClass::Arithmetic
                    && scalar.floating().is_some()
                    && let Some(value) = self.floating_value(scalar)
                {
                    return self.round_floating(value, element).is_some_and(|rounded| {
                        rounded.real.compare(value.real) == 0 && value.imag.is_zero()
                    });
                }
                from <= precision
            },
        }
    }

    /// What the vector rules ask of an arithmetic scalar type: an
    /// integer's width, signedness and Clang rank, and whether it is an
    /// ordinary integer type rather than `_Bool` or an enumeration; or a
    /// real floating type's significand precision.
    fn scalar_class(&self, ty: TypeId) -> Option<ScalarClass> {
        let ty = self.types.non_atomic(ty);
        let (scalar, plain) = match self.types.nodes[ty.index] {
            | TypeKind::Scalar(scalar) => (scalar.representation(), true),
            // Clang ranks a bit-precise integer below the standard integer
            // of its width (`getIntegerRank`).
            | TypeKind::BitInt { width, signed } =>
                return Some(ScalarClass::Integer {
                    bits: width,
                    signed,
                    rank: match width {
                        | 0..=8 => 1,
                        | 9..=16 => 2,
                        | 17..=32 => 3,
                        | 33..=64 => 5,
                        | _ => 6,
                    },
                    plain: true,
                }),
            | TypeKind::Tag(id) => {
                let tag = self.types.tags[id];
                if tag.kind != super::TagKind::Enum {
                    return None;
                }
                (tag.compatible.get().representation(), false)
            },
            | _ => return None,
        };
        let precision = match scalar {
            | Scalar::Float16 | Scalar::Fp16 => Some(11),
            | Scalar::BFloat16 => Some(8),
            | Scalar::Float => Some(24),
            | Scalar::Double => Some(53),
            | Scalar::Float128 => Some(113),
            | Scalar::LongDouble => Some(if self.types.target.long_double_is_double() {
                53
            } else if self.types.target.long_double_is_binary128() {
                113
            } else {
                64
            }),
            | _ => None,
        };
        if let Some(precision) = precision {
            return Some(ScalarClass::Floating { precision });
        }
        let rank = match scalar {
            | Scalar::Bool => 1,
            | Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar => 2,
            | Scalar::Short | Scalar::UnsignedShort => 3,
            | Scalar::Int | Scalar::UnsignedInt => 4,
            | Scalar::Long | Scalar::UnsignedLong => 5,
            | Scalar::LongLong | Scalar::UnsignedLongLong => 6,
            | Scalar::Int128 | Scalar::UnsignedInt128 => 7,
            | _ => return None,
        };
        let (bits, signed) = self.types.target.integer(scalar)?;
        Some(ScalarClass::Integer {
            bits,
            signed: signed && scalar != Scalar::Bool,
            rank,
            plain: plain && scalar != Scalar::Bool,
        })
    }

    #[cold]
    #[inline(never)]
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
    /// Checks equal lane counts and converts each lane to the requested vector
    /// element type.
    /// Clang extension: Clang Language Extensions, "`__builtin_convertvector`".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-convertvector>
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
            // C99 §6.3.2.1p2: the operand is read.
            let from = ExpressionInfo {
                ty: self.converted(from),
                ..from
            };
            let to = self
                .resolved_type_names
                .get(&super::type_name_identity(name))
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

    /// Checks vector inputs and constant lane indices for a shuffle.
    /// Clang extension: Clang Language Extensions, "`__builtin_shufflevector`".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-shufflevector>
    pub(super) fn shuffle_vector_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        arguments: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        let args = arguments.as_slice();
        // GNU builtin constraints depend on every operand having a known type.
        if args
            .iter()
            .any(|&arg| self.types.unanalyzed(self.expression_info(arg).ty))
        {
            return Self::expression_result(e, self.types.unknown());
        }
        let mut result = None;
        if args.len() >= 2 {
            let a = self.expression_info(args[0]);
            let b = self.expression_info(args[1]);
            // Clang's two-operand form shuffles by a run-time mask of
            // integer lanes, as GCC's `__builtin_shuffle (a, mask)` does,
            // and keeps the first vector type; GCC has no such form.
            if args.len() == 2 {
                if self.context.configuration.compiler_identity().rules() == RuleSet::Clang
                    && let Some((_, n, _)) = self.vector(a.ty)
                    && let Some((mask, m, _)) = self.vector(b.ty)
                    && n == m
                    && self.integer_type(mask).is_some()
                {
                    return Self::expression_result(e, a.ty.unqualified());
                }
            } else if let (Some((element, n, _)), Some((other, m, _))) =
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
    /// Checks GCC's run-time shuffle: one or two vectors of one type and a
    /// mask whose lanes are integers as wide as theirs, as many as theirs.
    /// The result has the first vector's type.
    /// GNU extension: GCC manual, "Vector Extensions": "`__builtin_shuffle
    /// (vec, mask)` and `__builtin_shuffle (vec0, vec1, mask)` ... The
    /// vector mask must have the same number of elements as vec0 and vec1
    /// and its element type must be an integral type of the same size as
    /// the element type of vec0 and vec1".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    pub(super) fn gnu_shuffle_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        arguments: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        let args = arguments.as_slice();
        if args
            .iter()
            .any(|&arg| self.types.unanalyzed(self.expression_info(arg).ty))
        {
            return Self::expression_result(e, self.types.unknown());
        }
        if let [first, inputs @ .., mask] = args
            && inputs.len() <= 1
        {
            let a = self.types.non_atomic(self.expression_info(first).ty);
            let same = inputs.iter().all(|&input| {
                let b = self.types.non_atomic(self.expression_info(input).ty);
                b.unqualified() == a.unqualified()
            });
            let mask = self.types.non_atomic(self.expression_info(mask).ty);
            if same
                && let Some((element, n, _)) = self.vector(a)
                && let Some((selector, m, _)) = self.vector(mask)
                && n == m
                && self.integer_type(selector).is_some()
                && self.types.layout(selector).map(|l| l.size)
                    == self.types.layout(element).map(|l| l.size)
            {
                return Self::expression_result(e, a.unqualified());
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

    /// Checks equal source/destination sizes for a builtin bit cast.
    /// Clang extension: Clang `SemaCast.cpp`, "`BuildBuiltinBitCastExpr`".
    /// <https://clang.llvm.org/doxygen/SemaCast_8cpp_source.html>
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
            // C99 §6.3.2.1p2: the operand is read; an array or function
            // decays as Clang's `__builtin_bit_cast` operand does.
            let from = ExpressionInfo {
                ty: self.converted(from),
                ..from
            };
            let to = self
                .resolved_type_names
                .get(&super::type_name_identity(name))
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

/// GNU construction and alignment suffixes apply to the declared type.
/// C99: §6.7, p. 97; PDF p. 109 (GNU attribute extension).
/// GNU extension: GCC manual, "Vector Extensions".
/// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
pub(super) fn declared_type_attribute(
    a: &AttributeSpecifier<'_>,
    context: &super::Context<'_>,
) -> bool {
    super::attributes::known_names(a, context)
        .any(|(_, name)| matches!(name, "vector_size" | "aligned" | "mode"))
}

pub(super) fn constructs_vector(a: &AttributeSpecifier<'_>, context: &super::Context<'_>) -> bool {
    super::attributes::known_names(a, context).any(|(_, name)| name == "vector_size")
}

/// What [`Analyzer::splat_operand`] asks of an arithmetic type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScalarClass {
    Integer {
        bits:   u32,
        signed: bool,
        /// Clang's integer conversion rank (`ASTContext::getIntegerRank`).
        rank:   u8,
        /// Not `_Bool` or an enumeration, which GCC does not splat.
        plain:  bool,
    },
    Floating {
        precision: u32,
    },
}

/// `scalar` as GCC sees an operand it converts at run time: no constant.
fn run_time(scalar: ExpressionInfo<'_>) -> ExpressionInfo<'_> {
    ExpressionInfo {
        ice: false,
        integer: None,
        constant: super::ConstantClass::None,
        ..scalar
    }
}

/// Clang's `ASTContext::getIntegerTypeOrder(lane, scalar)`: the sign of
/// the comparison of their ranks, where an unsigned type of a rank at
/// least the signed one's is the greater.
/// C99: §6.3.1.1 paragraph 1, p. 42; PDF p. 54 (integer conversion rank).
fn clang_integer_order(lane: ScalarClass, scalar: ScalarClass) -> i32 {
    let (
        ScalarClass::Integer {
            signed: a_signed,
            rank: a,
            ..
        },
        ScalarClass::Integer {
            signed: b_signed,
            rank: b,
            ..
        },
    ) = (lane, scalar)
    else {
        return 0;
    };
    if a_signed == b_signed {
        return (i32::from(a) - i32::from(b)).signum();
    }
    if !a_signed {
        if a >= b { 1 } else { -1 }
    } else if b >= a {
        -1
    } else {
        1
    }
}

/// Whether the integer constant `value` lies in the range of an integer
/// type `bits` wide, signed or not.
/// C99: §6.2.6.2 paragraphs 1-2, pp. 38-39; PDF pp. 50-51.
fn fits(value: super::Integer, bits: u32, signed: bool) -> bool {
    let Some(n) = value.to_i128() else {
        return !signed && bits == 128 && !value.signed;
    };
    if signed {
        bits >= 128 || (n >= -(1_i128 << (bits - 1)) && n < (1_i128 << (bits - 1)))
    } else {
        n >= 0 && (bits >= 127 || n < (1_i128 << bits))
    }
}

/// The bits a constant of a signed or unsigned type needs, as Clang's
/// `canConvertIntToOtherIntTy` counts them: its significant bits, with the
/// sign, if it is negative, else its active bits.
fn significant_bits(value: super::Integer, signed: bool) -> u32 {
    let n = value.value;
    if signed && n < 0 {
        129 - (!n).leading_zeros()
    } else {
        128 - (n as u128).leading_zeros()
    }
}

/// Whether the integer constant `value` is exact in a binary format with a
/// significand of `precision` bits (every format here has the exponent
/// range for any 128-bit integer).
fn exact_in(value: super::Integer, precision: u32) -> bool {
    let magnitude = if value.signed {
        value.value.unsigned_abs()
    } else {
        value.value as u128
    };
    magnitude == 0 || (magnitude >> magnitude.trailing_zeros()) >> precision == 0
}

/// The lane kind GCC's `vector_types_compatible_elements_p` compares: the
/// type with its signedness removed (`c_common_signed_type`), so plain,
/// signed and unsigned `char` match, as do `int` and `unsigned`, but not
/// `long` and `long long`.
fn signed_kind(kind: TypeKind<'_>) -> Option<Scalar> {
    let TypeKind::Scalar(scalar) = kind else {
        return None;
    };
    Some(match scalar {
        | Scalar::Char | Scalar::UnsignedChar => Scalar::SignedChar,
        | Scalar::UnsignedShort => Scalar::Short,
        | Scalar::UnsignedInt => Scalar::Int,
        | Scalar::UnsignedLong => Scalar::Long,
        | Scalar::UnsignedLongLong => Scalar::LongLong,
        | Scalar::UnsignedInt128 => Scalar::Int128,
        | other => other,
    })
}

/// Whether `op` compares, giving a mask.
const fn compares(op: BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::LessThan
            | BinaryOperator::LessThanOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterThanOrEqual
    )
}
