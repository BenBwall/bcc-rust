//! Phase-7 expression typing and conversions, using postorder continuations.
//! C99: §6.3, pp. 42-50; PDF pp. 54-62; §6.5, pp. 67-94; PDF pp. 79-106.
//! Statement control flow and return conversion are Stage 3 responsibilities.

use super::{
    super::preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        LiteralUnit,
        StringTokenType,
    },
    Analyzer,
    ArenaList,
    ArenaVec,
    ArrayBound,
    BinaryOperator,
    BindingKind,
    CStandard,
    ConditionalExpression,
    Constant,
    Duration,
    Expression,
    ExpressionSlot,
    ExpressionType,
    Identifier,
    Integer,
    Layout,
    Linkage,
    Namespace,
    Scalar,
    SemanticErrorKind,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    UnaryOperator,
    constants::Floating,
};
use crate::float_parsing::LongDouble;

/// The category before contextual lvalue conversion or decay.
/// C99: §6.3.2.1p1-4, pp. 46-47; PDF pp. 58-59.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueCategory {
    Lvalue,
    ModifiableLvalue,
    FunctionDesignator,
    Rvalue,
}

/// Constant-expression classes; integer values use the shared ICE arithmetic.
/// Address expressions retain their syntax and resolved bindings for lowering.
/// C99: §6.6p6-9, pp. 95-96; PDF pp. 107-108.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstantClass {
    None,
    Arithmetic,
    Address,
}

/// Clang does not ICE-evaluate or GNU-fold evaluated atomic value casts.
/// Initializer constant eligibility is tracked separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstantFolding {
    Permitted,
    AtomicCast,
}

/// Retained result keyed by immutable expression identity. The sequence is
/// deterministic postorder; identities are never printed as host addresses.
/// C99: §6.5p1, p. 67; PDF p. 79; §6.3.2.1, pp. 46-47; PDF pp. 58-59.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent retained facts describe category, address eligibility and constant \
              evaluation; none is a state transition."
)]
pub(crate) struct ExpressionInfo<'tu> {
    pub(crate) expression:         &'tu Expression<'tu>,
    pub(crate) ty:                 TypeId,
    /// Arithmetic type before the final compound-assignment conversion.
    pub(crate) operation_type:     Option<TypeId>,
    pub(crate) category:           ValueCategory,
    pub(crate) binding:            Option<usize>,
    pub(crate) bit_field:          Option<u32>,
    pub(crate) register:           bool,
    /// Whether designation can form an address constant without reading an
    /// object.
    pub(crate) static_address:     bool,
    /// An arithmetic constant containing binary128, retained without
    /// approximate folding.
    pub(crate) unfolded_binary128: bool,
    pub(crate) floating:           Option<Floating>,
    pub(crate) integer:            Option<Integer>,
    pub(crate) ice:                bool,
    pub(crate) constant:           ConstantClass,
    pub(crate) folding:            ConstantFolding,
}

impl ExpressionInfo<'_> {
    pub(super) fn atomic_cast(self) -> bool {
        self.folding == ConstantFolding::AtomicCast
    }
}

/// An explicit contextual conversion for a particular operand occurrence.
/// C99: §6.3, pp. 42-50; PDF pp. 54-62.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversionKind {
    Lvalue,
    ArrayDecay,
    FunctionDecay,
    Arithmetic,
    Assignment,
    DefaultArgument,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Conversion<'tu> {
    pub(crate) expression: &'tu Expression<'tu>,
    pub(crate) ty:         TypeId,
    pub(crate) kind:       ConversionKind,
}

/// The object an address constant is based on. C99: §6.6p9, p. 96; PDF
/// p. 108.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AddressBase<'tu> {
    /// An integer-valued pointer constant such as a null pointer.
    Absolute,
    Binding(usize),
    String(&'tu Expression<'tu>),
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn expression_info(&self, e: &'tu Expression<'tu>) -> ExpressionInfo<'tu> {
        self.expression_indices
            .get(&std::ptr::from_ref(e).addr())
            .map_or_else(
                || Self::expression_result(e, self.types.unknown()),
                |&i| self.expressions[i],
            )
    }

    pub(super) fn expression_result(e: &'tu Expression<'tu>, ty: TypeId) -> ExpressionInfo<'tu> {
        ExpressionInfo {
            expression: e,
            ty,
            operation_type: None,
            category: ValueCategory::Rvalue,
            binding: None,
            bit_field: None,
            register: false,
            static_address: false,
            unfolded_binary128: false,
            floating: None,
            integer: None,
            ice: false,
            constant: ConstantClass::None,
            folding: ConstantFolding::Permitted,
        }
    }

    /// C99: §6.3.2.1p1, p. 46; PDF p. 58. A structure/union containing
    /// const members is not modifiable; arrays and incomplete types are
    /// excluded.
    fn object_category(&mut self, ty: TypeId) -> ValueCategory {
        if self.complete_object(ty)
            && !matches!(self.types.nodes[ty.index], TypeKind::Array(..))
            && !self.contains_const(ty)
        {
            ValueCategory::ModifiableLvalue
        } else {
            ValueCategory::Lvalue
        }
    }

    fn contains_const(&mut self, ty: TypeId) -> bool {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push((ty, false));
        while let Some((current, ready)) = pending.pop() {
            if !ready && self.const_members.contains_key(&current.index) {
                continue;
            }
            if ready {
                let value = match self.types.nodes[current.index] {
                    | TypeKind::Array(element, _) =>
                        element.qualifiers.contains(TypeQualifiers::CONST)
                            || self
                                .const_members
                                .get(&element.index)
                                .copied()
                                .unwrap_or(false),
                    | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum =>
                        self.types.tags[id].members.get().iter().any(|m| {
                            m.ty.qualifiers.contains(TypeQualifiers::CONST)
                                || self
                                    .const_members
                                    .get(&m.ty.index)
                                    .copied()
                                    .unwrap_or(false)
                        }),
                    | _ => false,
                };
                _ = self.const_members.insert(current.index, value);
            } else {
                // A provisional answer ends a cycle through an invalid
                // self-containing record.
                _ = self.const_members.insert(current.index, false);
                pending.push((current, true));
                match self.types.nodes[current.index] {
                    | TypeKind::Array(element, _) => pending.push((element, false)),
                    | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum => {
                        for member in self.types.tags[id].members.get() {
                            pending.push((member.ty, false));
                        }
                    },
                    | _ => {},
                }
            }
        }
        ty.qualifiers.contains(TypeQualifiers::CONST)
            || self.const_members.get(&ty.index).copied().unwrap_or(false)
    }

    pub(super) fn complete_object(&self, mut ty: TypeId) -> bool {
        loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Array(element, bound) => {
                    if bound == ArrayBound::Incomplete {
                        return false;
                    }
                    ty = element;
                },
                | TypeKind::Tag(id) => return self.types.tags[id].complete.get(),
                | TypeKind::Scalar(Scalar::Void)
                | TypeKind::Function { .. }
                | TypeKind::Unknown => return false,
                | _ => return true,
            }
        }
    }

    pub(super) fn arithmetic(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        self.integer_type(ty).is_some()
            || matches!(self.types.nodes[ty.index], TypeKind::Scalar(s) if s != Scalar::Void)
    }

    fn real(&self, ty: TypeId) -> bool {
        self.integer_type(ty).is_some()
            || matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(
                    Scalar::Float | Scalar::Double | Scalar::LongDouble | Scalar::Float128
                )
            )
    }

    pub(super) fn scalar_type(&self, ty: TypeId) -> bool {
        self.arithmetic(ty) || self.pointer_target(ty).is_some()
    }

    /// Pointer arithmetic needs a complete object type (§6.5.6p2); an
    /// unanalyzed target, such as a GNU vector, is not checked.
    fn pointer_arithmetic_target(&self, target: TypeId) -> bool {
        self.complete_object(target) || self.types.unanalyzed(target)
    }

    fn pointer_target(&self, ty: TypeId) -> Option<TypeId> {
        let ty = self.types.non_atomic(ty);
        if let TypeKind::Pointer(target) = self.types.nodes[ty.index] {
            Some(target)
        } else {
            None
        }
    }

    pub(super) fn convert(&mut self, e: &'tu Expression<'tu>, ty: TypeId, kind: ConversionKind) {
        self.conversions.push(Conversion {
            expression: e,
            ty,
            kind,
        });
    }

    /// C99: §6.3.2.1p2-4, pp. 46-47; PDF pp. 58-59.
    pub(super) fn converted(&mut self, info: ExpressionInfo<'tu>) -> TypeId {
        let (ty, kind) = match self.types.nodes[info.ty.index] {
            | TypeKind::Array(element, _) => (
                self.types.intern(TypeKind::Pointer(element)),
                Some(ConversionKind::ArrayDecay),
            ),
            | TypeKind::Function { .. } => (
                self.types.intern(TypeKind::Pointer(info.ty)),
                Some(ConversionKind::FunctionDecay),
            ),
            | _ if matches!(
                info.category,
                ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
            ) =>
                (
                    self.types.non_atomic(info.ty).unqualified(),
                    Some(ConversionKind::Lvalue),
                ),
            | _ => (info.ty.unqualified(), None),
        };
        if let Some(kind) = kind {
            self.convert(info.expression, ty, kind);
        }
        ty
    }

    /// C99: §6.3.1.1p2, p. 42; PDF p. 54. Bit-field width can promote an
    /// unsigned int field to int when all its values fit.
    pub(super) fn promote(&mut self, info: ExpressionInfo<'tu>, ty: TypeId) -> TypeId {
        // An enumeration's compatible type has at least the rank of int, so
        // the enumeration promotes to that type unless it is a narrow
        // bit-field (§6.3.1.1p1-2).
        if let TypeKind::Tag(id) = self.types.nodes[ty.index]
            && self.types.tags[id].kind == TagKind::Enum
        {
            let promoted = if info.bit_field.is_some_and(|width| width < 32) {
                Scalar::Int
            } else {
                self.types.tags[id].compatible.get()
            };
            return self.types.scalar(promoted);
        }
        // Clang's implementation-defined extended bit-field promotions.
        // C99: §6.3.1.1p2, p. 42; PDF p. 54; §6.7.2.1p4, p. 101; PDF p. 113.
        if let Some(width) = info.bit_field
            && width <= 32
            && matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(Scalar::Int128 | Scalar::UnsignedInt128)
            )
        {
            let unsigned = width == 32
                && matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(Scalar::UnsignedInt128)
                );
            return self.types.scalar(if unsigned {
                Scalar::UnsignedInt
            } else {
                Scalar::Int
            });
        }
        if self.integer_type(ty).is_some_and(|(bits, _)| bits < 32)
            || (info.bit_field.is_some_and(|width| width < 32)
                && matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(Scalar::Int | Scalar::UnsignedInt | Scalar::Bool)
                ))
        {
            self.types.scalar(Scalar::Int)
        } else {
            ty.unqualified()
        }
    }

    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57. Preserve ranks even
    /// when long and long long have the same LP64 width.
    fn common_arithmetic(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        lt: TypeId,
        rt: TypeId,
    ) -> TypeId {
        let lt = self.promote(left, lt);
        let rt = self.promote(right, rt);
        let ls = self.scalar_representation(lt);
        let rs = self.scalar_representation(rt);
        let (Some(ls), Some(rs)) = (ls, rs) else {
            return self.types.unknown();
        };
        let float_rank = |s| match s {
            | Scalar::Float | Scalar::ComplexFloat => 1,
            | Scalar::Double | Scalar::ComplexDouble => 2,
            | Scalar::LongDouble | Scalar::ComplexLongDouble => 3,
            | Scalar::Float128 | Scalar::ComplexFloat128 => 4,
            | _ => 0,
        };
        let rank = float_rank(ls).max(float_rank(rs));
        let complex = matches!(
            ls,
            Scalar::ComplexFloat
                | Scalar::ComplexDouble
                | Scalar::ComplexLongDouble
                | Scalar::ComplexFloat128
        ) || matches!(
            rs,
            Scalar::ComplexFloat
                | Scalar::ComplexDouble
                | Scalar::ComplexLongDouble
                | Scalar::ComplexFloat128
        );
        let scalar = if rank != 0 {
            match (rank, complex) {
                | (1, false) => Scalar::Float,
                | (2, false) => Scalar::Double,
                | (4, false) => Scalar::Float128,
                | (_, false) => Scalar::LongDouble,
                | (1, true) => Scalar::ComplexFloat,
                | (2, true) => Scalar::ComplexDouble,
                | (4, true) => Scalar::ComplexFloat128,
                | (_, true) => Scalar::ComplexLongDouble,
            }
        } else {
            let rank = |s| match s {
                | Scalar::Long | Scalar::UnsignedLong => 2,
                | Scalar::LongLong | Scalar::UnsignedLongLong => 3,
                | Scalar::Int128 | Scalar::UnsignedInt128 => 4,
                | _ => 1,
            };
            let unsigned = |s| {
                matches!(
                    s,
                    Scalar::UnsignedInt
                        | Scalar::UnsignedLong
                        | Scalar::UnsignedLongLong
                        | Scalar::UnsignedInt128
                )
            };
            let (high, low) = if rank(ls) >= rank(rs) {
                (ls, rs)
            } else {
                (rs, ls)
            };
            if unsigned(high) == unsigned(low)
                || unsigned(high)
                || self.types.target.integer(high).map(|m| m.0)
                    > self.types.target.integer(low).map(|m| m.0)
            {
                high
            } else {
                match high {
                    | Scalar::Long => Scalar::UnsignedLong,
                    | Scalar::LongLong => Scalar::UnsignedLongLong,
                    | Scalar::Int128 => Scalar::UnsignedInt128,
                    | _ => Scalar::UnsignedInt,
                }
            }
        };
        let ty = self.types.scalar(scalar);
        self.convert(left.expression, ty, ConversionKind::Arithmetic);
        self.convert(right.expression, ty, ConversionKind::Arithmetic);
        ty
    }

    fn scalar_representation(&self, ty: TypeId) -> Option<Scalar> {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(s) => Some(s),
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum =>
                Some(self.types.tags[id].compatible.get()),
            | _ => None,
        }
    }

    /// C99: §6.3.2.3p3, p. 48; PDF p. 60.
    pub(super) fn null_pointer_constant(&self, info: ExpressionInfo<'tu>) -> bool {
        (info.ice && info.integer.is_some_and(|v| v.value == 0))
            || (matches!(self.types.nodes[info.ty.index], TypeKind::Pointer(target) if matches!(self.types.nodes[target.index], TypeKind::Scalar(Scalar::Void)))
                && info.constant == ConstantClass::Address
                && info.integer.is_some_and(|v| v.value == 0))
    }

    /// C99: §6.5.16.1p1, p. 92; PDF p. 104. Qualifier inclusion applies
    /// only to the immediate targets, never recursively to int **.
    pub(super) fn assignment_compatible(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> bool {
        let target = self.types.non_atomic(target);
        let from = self.converted(source);
        if self.types.unanalyzed(target) || self.types.unanalyzed(from) {
            return true;
        }
        if self.vector_assignment(target, from) {
            return true;
        }
        if self.arithmetic(target) && self.arithmetic(from) {
            return true;
        }
        if matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Bool)
        ) && self.pointer_target(from).is_some()
        {
            return true;
        }
        if let Some(to) = self.pointer_target(target) {
            if self.null_pointer_constant(source) {
                return true;
            }
            if let Some(from) = self.pointer_target(from) {
                if !to.qualifiers.contains(from.qualifiers) {
                    return false;
                }
                return self.pointer_compatible(to, from, true);
            }
            return false;
        }
        matches!(self.types.nodes[target.index], TypeKind::Tag(_))
            && self
                .types
                .composite(target.unqualified(), from.unqualified())
                .is_some()
    }

    fn pointer_compatible(&mut self, left: TypeId, right: TypeId, void: bool) -> bool {
        if self.types.unanalyzed(left) || self.types.unanalyzed(right) {
            return true;
        }
        if void {
            let is_void =
                |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void));
            let is_function =
                |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Function { .. });
            if (is_void(left) && !is_function(right)) || (is_void(right) && !is_function(left)) {
                return true;
            }
        }
        self.types
            .composite(left.unqualified(), right.unqualified())
            .is_some()
    }

    /// C89 §3.3.2.2 implicit function declarations; GNU modes retain the
    /// removed feature. Strict C99 and later require a declaration (§6.5.1p2,
    /// p. 69; PDF p. 81).
    pub(super) fn implicit_function(&mut self, mut e: &'tu Expression<'tu>) {
        if self.tainted {
            return;
        }
        while let ExpressionType::Parenthesized { expression } = e.kind {
            e = expression;
        }
        if let ExpressionType::Identifier(name) = e.kind
            && self.lookup(Namespace::Ordinary, name.name).is_none()
            && self.builtin_name(name)
        {
            // Unmodeled GNU compiler intrinsics do not have the C89 implicit
            // int signature. Preserve an opaque result rather than rejecting
            // valid pointer/aggregate returns based on an invented int type.
            let ty = if let Some(ty) = self.x86_builtin_type(name) {
                ty
            } else {
                let result = self.types.unknown();
                self.types.intern(TypeKind::Function {
                    result,
                    parameters: &[],
                    prototype: false,
                    variadic: false,
                })
            };
            self.bind(
                name,
                ty,
                BindingKind::Function,
                Linkage::External,
                Duration::None,
                None,
            );
            return;
        }
        if let ExpressionType::Identifier(name) = e.kind
            && self.lookup(Namespace::Ordinary, name.name).is_none()
            && (self.context.configuration.standard() < CStandard::C99
                || self.context.configuration.gnu_extensions())
        {
            let result = self.types.scalar(Scalar::Int);
            let ty = self.types.intern(TypeKind::Function {
                result,
                parameters: &[],
                prototype: false,
                variadic: false,
            });
            self.bind(
                name,
                ty,
                BindingKind::Function,
                Linkage::External,
                Duration::None,
                None,
            );
            self.context.report_extension(
                crate::configuration::Feature::ImplicitFunctionDeclaration,
                "implicit function declaration",
                name.source_vectors,
            );
        }
    }

    /// GCC builtins are reserved identifiers (C99 §7.1.3p1, p. 165; PDF p.
    /// 177) that the implementation declares; an undeclared one is an
    /// unmodeled function rather than an implicit `int` declaration.
    pub(super) fn builtin_name(&self, name: Identifier) -> bool {
        super::atomics::modeled(self.context.string_cache.at(name.name))
            || self
                .context
                .string_cache
                .at(name.name)
                .starts_with("__builtin_")
    }

    /// GCC's `__builtin_constant_p` folds to whether its operand is an
    /// arithmetic constant; it is valid where a constant is required.
    fn constant_p(
        &self,
        e: &'tu Expression<'tu>,
        function: &'tu Expression<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        let mut callee = function;
        while let ExpressionType::Parenthesized { expression } = callee.kind {
            callee = expression;
        }
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let &[argument] = arguments.as_slice() else {
            return None;
        };
        // Undeclared builtins are bound as opaque functions; a user
        // declaration with a known result type is an ordinary function.
        if self.context.string_cache.at(name.name) != "__builtin_constant_p"
            || self
                .lookup(Namespace::Ordinary, name.name)
                .is_some_and(|entry| {
                    !matches!(
                        self.types.nodes[self.bindings[entry.binding].ty.index],
                        TypeKind::Function { result, .. } if result == self.types.unknown()
                    )
                })
        {
            return None;
        }
        let operand = self.expression_info(argument);
        let constant = operand.constant == ConstantClass::Arithmetic
            && (operand.integer.is_some() || operand.floating.is_some());
        let mut info = Self::expression_result(e, self.types.unknown());
        info.integer = Some(Integer::int(i128::from(constant)));
        info.ice = true;
        info.constant = ConstantClass::Arithmetic;
        Some(info)
    }

    /// The integer that the address of an lvalue reached from an
    /// integer-valued pointer constant folds to, as in the classic `offsetof`
    /// macro.
    fn integer_address(&self, e: &'tu Expression<'tu>) -> Option<i128> {
        match self.address_parts(e, true)? {
            | (AddressBase::Absolute, offset) => Some(offset),
            | _ => None,
        }
    }

    /// Splits a pointer value (or, with `lvalue`, an lvalue's address) into
    /// the object it is based on and a byte offset, following member,
    /// subscript, cast and integer-offset steps. GCC folds differences of
    /// such addresses, which §6.6p10 lets an implementation accept as
    /// constant expressions. Each step has one address operand, so this is a
    /// loop rather than recursion over syntax.
    pub(super) fn address_parts(
        &self,
        mut e: &'tu Expression<'tu>,
        mut lvalue: bool,
    ) -> Option<(AddressBase<'tu>, i128)> {
        let mut offset = 0_i128;
        loop {
            if let ExpressionType::Parenthesized { expression } = e.kind {
                e = expression;
                continue;
            }
            let info = self.expression_info(e);
            if !lvalue {
                if matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ) {
                    // The decayed value addresses the designated object.
                    lvalue = true;
                    continue;
                }
                match e.kind {
                    | ExpressionType::Cast {
                        operand_expression, ..
                    } => {
                        let operand = self.expression_info(operand_expression);
                        if operand.ice
                            && let Some(value) = operand.integer
                        {
                            return Some((
                                AddressBase::Absolute,
                                offset.checked_add(if self.pointer_target(info.ty).is_some() {
                                    info.integer?.to_i128()?
                                } else {
                                    value.to_i128()?
                                })?,
                            ));
                        }
                        e = operand_expression;
                    },
                    | ExpressionType::Unary {
                        operator: UnaryOperator::AddressOf,
                        operand_expression,
                    } => {
                        lvalue = true;
                        e = operand_expression;
                    },
                    | ExpressionType::Binary {
                        operator:
                            operator @ (BinaryOperator::Addition | BinaryOperator::Subtraction),
                        left_expression,
                        right_expression,
                    } => {
                        let left = self.expression_info(left_expression);
                        let right = self.expression_info(right_expression);
                        let (pointer, index) = if right.ice {
                            (left, right)
                        } else {
                            (right, left)
                        };
                        let index = index.integer.filter(|_| index.ice)?.to_i128()?;
                        let target = self.pointer_target(info.ty)?;
                        let size = i128::from(self.types.layout(target)?.size);
                        let step = index.checked_mul(size)?;
                        offset = if operator == BinaryOperator::Addition {
                            offset.checked_add(step)?
                        } else if std::ptr::eq(pointer.expression, left_expression) {
                            offset.checked_sub(step)?
                        } else {
                            return None;
                        };
                        e = pointer.expression;
                    },
                    | _ => {
                        let value = Self::pointer_value(info)?;
                        return Some((AddressBase::Absolute, offset.checked_add(value)?));
                    },
                }
                continue;
            }
            match e.kind {
                | ExpressionType::Identifier(_) =>
                    return Some((AddressBase::Binding(info.binding?), offset)),
                | ExpressionType::StringLiteral(_) =>
                    return Some((AddressBase::String(e), offset)),
                | ExpressionType::DirectMember {
                    base_expression,
                    member,
                }
                | ExpressionType::IndirectMember {
                    base_expression,
                    member,
                } => {
                    let base = self.expression_info(base_expression);
                    let indirect = matches!(e.kind, ExpressionType::IndirectMember { .. });
                    let record = if indirect {
                        self.pointer_target(base.ty)?
                    } else {
                        base.ty
                    };
                    let TypeKind::Tag(id) = self.types.nodes[record.index] else {
                        return None;
                    };
                    let index = *self.member_indices.get(&(id, member.name))?;
                    let field = self.types.tags[id].fields.get().get(index)?;
                    offset = offset.checked_add(i128::from(field.offset))?;
                    lvalue = !indirect;
                    e = base_expression;
                },
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let left = self.expression_info(left_expression);
                    let right = self.expression_info(right_expression);
                    let (pointer, index) = if right.ice {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    let index = index.integer.filter(|_| index.ice)?.to_i128()?;
                    let size = i128::from(self.types.layout(info.ty)?.size);
                    offset = offset.checked_add(index.checked_mul(size)?)?;
                    lvalue = false;
                    e = pointer.expression;
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } => {
                    lvalue = false;
                    e = operand_expression;
                },
                | _ => return None,
            }
        }
    }

    /// Whether two address bases designate the same object; identical string
    /// literals are merged, as GCC does.
    fn same_address_base(&self, left: AddressBase<'tu>, right: AddressBase<'tu>) -> bool {
        match (left, right) {
            | (AddressBase::Absolute, AddressBase::Absolute) => true,
            | (AddressBase::Binding(l), AddressBase::Binding(r)) => l == r,
            | (AddressBase::String(l), AddressBase::String(r)) => match (l.kind, r.kind) {
                | (
                    ExpressionType::StringLiteral(StringTokenType::String(l)),
                    ExpressionType::StringLiteral(StringTokenType::String(r)),
                )
                | (
                    ExpressionType::StringLiteral(StringTokenType::WideString(l)),
                    ExpressionType::StringLiteral(StringTokenType::WideString(r)),
                ) => self.context.literal_units(l) == self.context.literal_units(r),
                | _ => false,
            },
            | _ => false,
        }
    }

    /// The element distance between two addresses of one object.
    /// C99: §6.5.6p9, pp. 83-84; PDF pp. 95-96.
    fn address_difference(
        &self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        target: TypeId,
    ) -> Option<Integer> {
        let (left_base, left_offset) = self.address_parts(left.expression, false)?;
        let (right_base, right_offset) = self.address_parts(right.expression, false)?;
        let size = self.types.layout(target)?.size;
        let bytes = left_offset.checked_sub(right_offset)?;
        // Divide 64-bit magnitudes; i128 division is unavailable (see
        // integer.rs).
        let magnitude = u64::try_from(bytes.unsigned_abs()).ok()?;
        if !self.same_address_base(left_base, right_base) || size == 0 || magnitude % size != 0 {
            return None;
        }
        let elements = i128::from(magnitude / size);
        Some(Integer::int(if bytes < 0 { -elements } else { elements }).cast(64, true))
    }

    fn pointer_value(info: ExpressionInfo<'tu>) -> Option<i128> {
        (info.constant == ConstantClass::Address)
            .then_some(info.integer)
            .flatten()
            .map(|v| v.value)
    }

    pub(super) fn check_condition(&mut self, mut slot: ExpressionSlot<'tu>, integer: bool) {
        while let ExpressionSlot::Selection(header) = slot {
            let Some(e) = header.expression else {
                return;
            };
            slot = e;
        }
        if let ExpressionSlot::Parsed(e) = slot {
            let info = self.expression_info(e);
            let ty = self.converted(info);
            if !self.types.unanalyzed(ty)
                && if integer {
                    self.integer_type(ty).is_none()
                } else {
                    !self.scalar_type(ty)
                }
            {
                self.error(
                    if integer {
                        SemanticErrorKind::InvalidSwitchExpression
                    } else {
                        SemanticErrorKind::InvalidCondition
                    },
                    e.source_vectors,
                    None,
                    None,
                );
            }
        }
    }

    /// Postorder reduction never evaluates an operand through Rust recursion.
    /// C99: §6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106.
    pub(super) fn expression_done(&mut self, e: &'tu Expression<'tu>) {
        use ExpressionType as E;
        let mut info = Self::expression_result(e, self.types.unknown());
        match e.kind {
            | E::Identifier(name) => {
                if let Some(entry) = self.lookup(Namespace::Ordinary, name.name) {
                    let binding = self.bindings[entry.binding];
                    info.ty = binding.ty;
                    info.binding = Some(entry.binding);
                    self.record_binding_use(entry.binding, name);
                    info.register = self.register_bindings.contains_key(&entry.binding);
                    info.static_address = binding.duration == Duration::Static
                        || binding.kind == BindingKind::Function;
                    info.category = match binding.kind {
                        | BindingKind::Function => ValueCategory::FunctionDesignator,
                        | BindingKind::Enumerator => ValueCategory::Rvalue,
                        | BindingKind::Typedef => {
                            info.ty = self.types.unknown();
                            ValueCategory::Rvalue
                        },
                        | _ => self.object_category(info.ty),
                    };
                    info.ice = binding.kind == BindingKind::Enumerator;
                    if info.ice {
                        info.integer = binding.value;
                        info.constant = ConstantClass::Arithmetic;
                    } else if binding.kind == BindingKind::Object
                        && binding.value.is_some()
                        && self.context.configuration.gnu_extensions()
                    {
                        // Not a C99 constant expression (§6.6p6-8); GCC folds
                        // it, as §6.6p10 allows.
                        info.integer = binding.value;
                        info.constant = ConstantClass::Arithmetic;
                    }
                } else if !e.recovered && !self.builtin_name(name) {
                    self.error(
                        SemanticErrorKind::UndeclaredIdentifier,
                        name.source_vectors,
                        Some(name.name),
                        None,
                    );
                }
            },
            | E::Constant(Constant::Integer(value)) => {
                let scalar = match value {
                    | IntegerTokenType::Int(_) => Scalar::Int,
                    | IntegerTokenType::UnsignedInt(_) => Scalar::UnsignedInt,
                    | IntegerTokenType::Long(_) => Scalar::Long,
                    | IntegerTokenType::UnsignedLong(_) => Scalar::UnsignedLong,
                    | IntegerTokenType::LongLong(_) => Scalar::LongLong,
                    | IntegerTokenType::UnsignedLongLong(_) => Scalar::UnsignedLongLong,
                    | _ => {
                        self.retain_expression(info);
                        return;
                    },
                };
                info.ty = self.types.scalar(scalar);
                if let Some((bits, signed)) = self.integer_type(info.ty) {
                    info.integer = Some(Integer {
                        value: value.into(),
                        bits,
                        signed,
                    });
                }
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Char(value)) => {
                let scalar = if matches!(value, CharacterTokenType::WideChar(_)) {
                    self.types.target.wchar_t
                } else {
                    Scalar::Int
                };
                info.ty = self.types.scalar(scalar);
                let mut value = i128::from(value.target_value(&self.types.target));
                if matches!(
                    e.kind,
                    E::Constant(Constant::Char(CharacterTokenType::Char(_)))
                ) && self.types.target.char_signed
                    && (128..=255).contains(&value)
                {
                    value -= 256;
                }
                let (bits, signed) = self
                    .integer_type(info.ty)
                    .expect("character type is integer");
                info.integer = Some(Integer {
                    value,
                    bits,
                    signed,
                });
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Boolean(value) => {
                info.ty = self.types.scalar(Scalar::Int);
                info.integer = Some(Integer::int(i128::from(value)));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(
                FloatTokenType::Float128(_) | FloatTokenType::ImaginaryFloat128(_),
            )) => {
                info.ty = self.types.scalar(
                    if matches!(
                        e.kind,
                        E::Constant(Constant::Float(FloatTokenType::ImaginaryFloat128(_)))
                    ) {
                        Scalar::ComplexFloat128
                    } else {
                        Scalar::Float128
                    },
                );
                info.unfolded_binary128 = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(value)) => {
                info.ty = self.types.scalar(match value {
                    | FloatTokenType::Float(_) => Scalar::Float,
                    | FloatTokenType::Double(_) => Scalar::Double,
                    | FloatTokenType::LongDouble(_) => Scalar::LongDouble,
                    | FloatTokenType::ImaginaryFloat(_) => Scalar::ComplexFloat,
                    | FloatTokenType::ImaginaryDouble(_) => Scalar::ComplexDouble,
                    | FloatTokenType::ImaginaryLongDouble(_) => Scalar::ComplexLongDouble,
                    | _ => {
                        self.retain_expression(info);
                        return;
                    },
                });
                info.floating = Some(Floating::real(match value {
                    | FloatTokenType::Float(value) => LongDouble::from_double(f64::from(value)),
                    | FloatTokenType::Double(value) => LongDouble::from_double(value.get()),
                    | FloatTokenType::LongDouble(value) => value,
                    | _ => LongDouble::ZERO,
                }));
                info.floating = match value {
                    | FloatTokenType::ImaginaryFloat(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(f64::from(value)),
                    }),
                    | FloatTokenType::ImaginaryDouble(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(value.get()),
                    }),
                    | FloatTokenType::ImaginaryLongDouble(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: value,
                    }),
                    | _ => info.floating,
                };
                info.constant = ConstantClass::Arithmetic;
            },
            | E::StringLiteral(value) =>
                if let Some((element, count)) = self.string_type(value) {
                    info.ty = self
                        .types
                        .intern(TypeKind::Array(element, ArrayBound::Constant(count)));
                    info.category = ValueCategory::Lvalue;
                    info.static_address = true;
                },
            | E::Parenthesized { expression } => {
                info = ExpressionInfo {
                    expression: e,
                    ..self.expression_info(expression)
                };
            },
            | E::Generic(generic) => info = self.type_generic(e, generic),
            | E::Unary {
                operator,
                operand_expression,
            } => {
                info = self.type_unary(e, operator, self.expression_info(operand_expression));
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                info = self.type_binary(
                    e,
                    operator,
                    self.expression_info(left_expression),
                    self.expression_info(right_expression),
                );
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                info = self.type_conditional(e, c);
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                let ty = self
                    .resolved_type_names
                    .get(&name.source_vectors)
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                info = self.type_sizeof(e, ty, None, matches!(e.kind, E::AlignofType(_)));
            },
            | E::SizeofExpr(operand) | E::AlignofExpr(operand) => {
                let operand = self.expression_info(operand);
                info = self.type_sizeof(
                    e,
                    operand.ty,
                    operand.bit_field,
                    matches!(e.kind, E::AlignofExpr(_)),
                );
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                let target = self
                    .resolved_type_names
                    .get(&target_type.source_vectors)
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                info = self.type_cast(e, target, self.expression_info(operand_expression));
            },
            | E::DirectMember {
                base_expression,
                member,
            }
            | E::IndirectMember {
                base_expression,
                member,
            } => {
                info = self.type_member(
                    e,
                    self.expression_info(base_expression),
                    member,
                    matches!(e.kind, E::IndirectMember { .. }),
                );
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                info = if let ExpressionType::Identifier(name) = function_expression.kind
                    && self.context.string_cache.at(name.name) == "__builtin_shufflevector"
                {
                    self.context.report_extension(
                        crate::configuration::Feature::VectorBuiltins,
                        "__builtin_shufflevector",
                        name.source_vectors,
                    );
                    self.shuffle_vector_builtin(e, arguments)
                } else if let Some(info) = self.vector_overload(e, function_expression, arguments) {
                    info
                } else if let Some(info) = self.atomic_call(e, function_expression, arguments) {
                    info
                } else if self.classify_type_callee(function_expression) {
                    self.classify_type(e, &arguments)
                } else if let Some(info) = self.math128_builtin(e, function_expression, &arguments)
                {
                    info
                } else if let Some(info) = self.constant_p(e, function_expression, arguments) {
                    info
                } else {
                    self.type_call(e, self.expression_info(function_expression), arguments)
                };
            },
            | E::Builtin(b) => {
                info = self.type_builtin(e, b);
            },
            | E::CompoundLiteral {
                type_name,
                initializer,
            } => {
                let ty = self
                    .resolved_type_names
                    .get(&type_name.source_vectors)
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                // §6.5.2.5p1 excludes variable length array types; a pointer
                // to one is a valid compound literal type.
                if !self.types.unanalyzed(ty)
                    && ((matches!(self.types.nodes[ty.index], TypeKind::Array(..))
                        && self.variably_modified(ty))
                        || (!self.complete_object(ty)
                            && !matches!(
                                self.types.nodes[ty.index],
                                TypeKind::Array(_, ArrayBound::Incomplete)
                            )))
                {
                    info = self.invalid_expression(e, SemanticErrorKind::InvalidCompoundLiteral);
                } else {
                    info.ty = self.check_initializer(ty, initializer, self.scope == 0);
                }
                info.category = self.object_category(info.ty);
                info.static_address = self.scope == 0;
            },
            // Unmodeled extensions retain unknown results and suppress constraints.
            | _ => {},
        }
        if info.unfolded_binary128 {
            info.integer = None;
            info.floating = None;
            info.ice = false;
        }
        if e.recovered {
            info.unfolded_binary128 = false;
            info.ty = self.types.unknown();
            info.integer = None;
            info.floating = None;
            info.ice = false;
            info.constant = ConstantClass::None;
        }
        // Size/alignment and generic controlling operands are unevaluated.
        // Generic selection already copied only its selected expression.
        let atomic_cast = match e.kind {
            | E::Unary {
                operand_expression, ..
            } => self.expression_info(operand_expression).atomic_cast(),
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } =>
                self.expression_info(left_expression).atomic_cast()
                    || self.expression_info(right_expression).atomic_cast(),
            | E::Conditional(c) | E::OmittedConditional(c) =>
                self.expression_info(c.condition_expression).atomic_cast()
                    || self.expression_info(c.then_expression).atomic_cast()
                    || self.expression_info(c.else_expression).atomic_cast(),
            | E::Cast {
                operand_expression, ..
            } =>
                matches!(self.types.nodes[info.ty.index], TypeKind::Atomic(_))
                    || self.expression_info(operand_expression).atomic_cast(),
            | _ => false,
        };
        if info.atomic_cast() || atomic_cast {
            info.folding = ConstantFolding::AtomicCast;
            info.ice = false;
        }
        self.retain_expression(info);
    }

    fn retain_expression(&mut self, info: ExpressionInfo<'tu>) {
        let key = std::ptr::from_ref(info.expression).addr();
        if self.expression_indices.contains_key(&key) {
            return;
        }
        _ = self.expression_indices.insert(key, self.expressions.len());
        self.expressions.push(info);
    }

    /// C99: §6.4.5p5-6, p. 63; PDF p. 75. The count includes the final
    /// zero; narrow source characters contribute their UTF-8 code units.
    pub(super) fn string_type(&mut self, value: StringTokenType) -> Option<(TypeId, u64)> {
        let (id, wide) = match value {
            | StringTokenType::String(id) => (id, false),
            | StringTokenType::WideString(id) => (id, true),
            | _ => return None,
        };
        let count = if wide {
            self.context.wide_literal_units(id).count() as u64
        } else {
            self.context
                .literal_units(id)
                .iter()
                .map(|u| match u {
                    | LiteralUnit::Character(c) => c.len_utf8() as u64,
                    | LiteralUnit::Numeric(_) => 1,
                })
                .sum::<u64>()
        } + 1;
        Some((
            self.types.scalar(if wide {
                self.types.target.wchar_t
            } else {
                Scalar::Char
            }),
            count,
        ))
    }

    pub(super) fn invalid_expression(
        &mut self,
        e: &'tu Expression<'tu>,
        kind: SemanticErrorKind,
    ) -> ExpressionInfo<'tu> {
        if !e.recovered {
            self.error(
                kind,
                e.operator_source_vectors.unwrap_or(e.source_vectors),
                None,
                None,
            );
        }
        Self::expression_result(e, self.types.unknown())
    }

    /// C99: §6.5.3.4p1-5, pp. 80-81; PDF pp. 92-93.
    fn type_sizeof(
        &mut self,
        e: &'tu Expression<'tu>,
        ty: TypeId,
        bit_field: Option<u32>,
        align: bool,
    ) -> ExpressionInfo<'tu> {
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let gnu = self.context.configuration.gnu_extensions()
            && matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(Scalar::Void) | TypeKind::Function { .. }
            );
        if bit_field.is_some() || (!gnu && !self.complete_object(ty)) {
            return self.invalid_expression(e, SemanticErrorKind::InvalidSizeof);
        }
        let result = self.types.scalar(self.types.target.size_t);
        let mut info = Self::expression_result(e, result);
        if let Some(layout) = self
            .types
            .layout(ty)
            .or_else(|| gnu.then_some(Layout { size: 1, align: 1 }))
        {
            info.integer = Some(Integer {
                value:  i128::from(if align { layout.align } else { layout.size }),
                bits:   64,
                signed: false,
            });
            info.ice = true;
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    /// C99: §6.5.3.1-§6.5.3.3, pp. 78-80; PDF pp. 90-92;
    /// postfix increments §6.5.2.4, p. 75; PDF p. 87.
    fn type_unary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: UnaryOperator,
        operand: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        use UnaryOperator as U;
        if self.types.unanalyzed(operand.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        if op == U::Extension {
            return ExpressionInfo {
                expression: e,
                ..operand
            };
        }
        if op == U::AddressOf {
            let mut immediate = operand.expression;
            while let ExpressionType::Parenthesized { expression } = immediate.kind {
                immediate = expression;
            }
            if let ExpressionType::Binary {
                operator: BinaryOperator::Subscript,
                left_expression,
                ..
            } = immediate.kind
                && self
                    .vector(self.expression_info(left_expression).ty)
                    .is_some()
            {
                return self.vector_error(e);
            }
            let indirect = matches!(
                operand.expression.kind,
                ExpressionType::Unary {
                    operator: U::Indirection,
                    ..
                } | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    ..
                }
            );
            if operand.bit_field.is_some()
                || operand.register
                || (!indirect
                    && !matches!(
                        operand.category,
                        ValueCategory::Lvalue
                            | ValueCategory::ModifiableLvalue
                            | ValueCategory::FunctionDesignator
                    ))
            {
                return self.invalid_expression(e, SemanticErrorKind::InvalidAddressOperand);
            }
            let ty = self.types.intern(TypeKind::Pointer(operand.ty));
            let mut info = Self::expression_result(e, ty);
            if operand.static_address {
                info.constant = ConstantClass::Address;
                info.integer = self
                    .integer_address(operand.expression)
                    .map(|value| Integer::int(value).cast(64, false));
            }
            return info;
        }
        let ty = self.converted(operand);
        if let Some((element, _, _)) = self.vector(ty) {
            if matches!(op, U::Plus | U::Minus)
                || (op == U::BitwiseNot && self.integer_type(element).is_some())
            {
                return Self::expression_result(e, ty);
            }
            return self.vector_error(e);
        }
        if op == U::Indirection {
            let Some(target) = self.pointer_target(ty) else {
                return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
            };
            let mut info = Self::expression_result(e, target);
            info.category = if matches!(self.types.nodes[target.index], TypeKind::Function { .. }) {
                ValueCategory::FunctionDesignator
            } else {
                self.object_category(target)
            };
            info.static_address = self.address_value(operand);
            return info;
        }
        if matches!(
            op,
            U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement
        ) {
            if operand.category != ValueCategory::ModifiableLvalue {
                return self.invalid_expression(e, SemanticErrorKind::ExpectedModifiableLvalue);
            }
            if !(self.real(ty)
                || self
                    .pointer_target(ty)
                    .is_some_and(|t| self.pointer_arithmetic_target(t)))
            {
                return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
            }
            return Self::expression_result(e, ty);
        }
        if !match op {
            | U::Plus | U::Minus => self.arithmetic(ty),
            | U::BitwiseNot => self.integer_type(ty).is_some(),
            | U::LogicalNot => self.scalar_type(ty),
            | _ => true,
        } {
            return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
        }
        if matches!(op, U::Real | U::Imag) {
            return self.real_imag(e, op, operand);
        }
        let ty = if op == U::LogicalNot {
            self.types.scalar(Scalar::Int)
        } else {
            self.promote(operand, ty)
        };
        self.convert(operand.expression, ty, ConversionKind::Arithmetic);
        let mut info = Self::expression_result(e, ty);
        info.unfolded_binary128 = operand.unfolded_binary128;
        info.integer = operand.integer.and_then(|v| v.unary(op));
        if op == U::LogicalNot {
            info.integer =
                Self::constant_truth(operand).map(|truth| Integer::int(i128::from(!truth)));
        }
        if self.integer_type(ty).is_none() {
            info.floating = self
                .floating_value(operand)
                .map(|value| {
                    if op == U::Minus {
                        Self::floating_negate(value)
                    } else {
                        value
                    }
                })
                .and_then(|value| self.round_floating(value, ty));
        }
        info.ice = operand.ice;
        if operand.constant == ConstantClass::Arithmetic {
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    pub(super) fn address_value(&self, info: ExpressionInfo<'tu>) -> bool {
        info.constant == ConstantClass::Address
            || (info.static_address
                && matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ))
    }

    /// C99: §6.5.4p2-4, p. 81; PDF p. 93.
    fn type_cast(
        &mut self,
        e: &'tu Expression<'tu>,
        target: TypeId,
        operand: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        let from = self.converted(operand);
        if matches!(self.types.nodes[target.index], TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Union)
        {
            return Self::expression_result(e, self.types.unknown());
        }
        if self.types.unanalyzed(target) || self.types.unanalyzed(from) {
            return Self::expression_result(e, self.types.unknown());
        }
        if self.vector(target).is_some() || self.vector(from).is_some() {
            let permitted = |ty| self.vector(ty).is_some() || self.integer_type(ty).is_some();
            if permitted(target)
                && permitted(from)
                && self.vector_width(target) == self.vector_width(from)
            {
                return Self::expression_result(e, target.unqualified());
            }
            return self.vector_error(e);
        }
        let void = matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Void)
        );
        if !void
            && (!self.scalar_type(target)
                || !self.scalar_type(from)
                || (self.pointer_target(target).is_some()
                    && self.arithmetic(from)
                    && self.integer_type(from).is_none())
                || (self.pointer_target(from).is_some()
                    && self.arithmetic(target)
                    && self.integer_type(target).is_none()))
        {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCast);
        }
        self.convert(operand.expression, target, ConversionKind::Assignment);
        let mut info = Self::expression_result(e, target.unqualified());
        info.unfolded_binary128 = operand.unfolded_binary128
            || (self.binary128_type(target) && operand.constant == ConstantClass::Arithmetic);
        let mut immediate = operand.expression;
        while let ExpressionType::Parenthesized { expression } = immediate.kind {
            immediate = expression;
        }
        if let Some((bits, signed)) = self.integer_type(target) {
            info.integer = if let ExpressionType::Constant(Constant::Float(value)) = immediate.kind
            {
                self.float_cast(target, value)
            } else {
                operand.integer.map(|v| {
                    if matches!(
                        self.types.nodes[target.index],
                        TypeKind::Scalar(Scalar::Bool)
                    ) {
                        Integer::int(i128::from(v.value != 0)).cast(1, false)
                    } else {
                        v.cast(bits, signed)
                    }
                })
            };
            info.ice = operand.ice
                || matches!(immediate.kind, ExpressionType::Constant(Constant::Float(_)));
        }
        if let Some(value) = operand.floating
            && self.integer_type(target).is_some()
        {
            info.integer = if matches!(
                self.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Bool)
            ) {
                Some(Integer::int(i128::from(value.truth())).cast(1, false))
            } else {
                self.float_cast(target, FloatTokenType::LongDouble(value.real))
            };
        }
        if self.arithmetic(target) && self.integer_type(target).is_none() {
            info.floating = self
                .floating_value(operand)
                .and_then(|value| self.round_floating(value, target));
        }
        if self.arithmetic(target) && operand.constant == ConstantClass::Arithmetic {
            info.constant = ConstantClass::Arithmetic;
        }
        if info.unfolded_binary128 {
            info.integer = None;
            info.floating = None;
            info.ice = false;
        }
        // Known pointer truth is an accepted arithmetic constant form, but
        // not an integer constant expression. C99 §6.3.1.2p1, p. 43;
        // PDF p. 55; implementation latitude §6.6p10, p. 96; PDF p. 108.
        if matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Bool)
        ) && self.pointer_target(from).is_some()
            && self.address_value(operand)
        {
            let truth = Self::constant_truth(operand).unwrap_or(true);
            info.integer = Some(Integer::int(i128::from(truth)).cast(1, false));
            info.constant = ConstantClass::Arithmetic;
            info.ice = false;
        }
        if self.pointer_target(target).is_some()
            && (self.address_value(operand) || (operand.ice && operand.integer.is_some()))
        {
            info.constant = ConstantClass::Address;
            info.integer = operand.integer.map(|v| v.cast(64, false));
        }
        // GNU modes fold an integer-valued address constant converted to an
        // integer type, which C99 does not count as arithmetic (§6.6p8).
        if self.context.configuration.gnu_extensions()
            && self.integer_type(target).is_some()
            && Self::pointer_value(operand).is_some()
        {
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    /// C99: §6.5.2.3p1-4, pp. 72-73; PDF pp. 84-85.
    fn type_member(
        &mut self,
        e: &'tu Expression<'tu>,
        base: ExpressionInfo<'tu>,
        member: Identifier,
        indirect: bool,
    ) -> ExpressionInfo<'tu> {
        if self.types.unanalyzed(base.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let mut ty = base.ty;
        if indirect {
            let converted = self.converted(base);
            let Some(target) = self.pointer_target(converted) else {
                return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
            };
            // An unanalyzed pointee may be a record; its members are unknown.
            if self.types.unanalyzed(target) {
                return Self::expression_result(e, self.types.unknown());
            }
            ty = target;
        }
        let TypeKind::Tag(id) = self.types.nodes[ty.index] else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
        };
        let Some(&index) = self.member_indices.get(&(id, member.name)) else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
        };
        let Some(field) = self.types.tags[id].fields.get().get(index).copied() else {
            return Self::expression_result(e, self.types.unknown());
        };
        let mut field_ty = field.ty;
        let mut arrays = ArenaVec::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = self.types.nodes[field_ty.index] {
            arrays.push(bound);
            field_ty = element;
        }
        // Qualifiers of the record and of any anonymous members on the path
        // reach the member, and an array's element (§6.5.2.3p3, §6.7.3p8).
        field_ty = field_ty.qualified(ty.qualifiers | field.qualifiers);
        while let Some(bound) = arrays.pop() {
            field_ty = self.types.intern(TypeKind::Array(field_ty, bound));
        }
        let mut info = Self::expression_result(e, field_ty);
        if indirect
            || matches!(
                base.category,
                ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
            )
        {
            info.category = self.object_category(field_ty);
        }
        info.bit_field = field.width;
        info.register = !indirect && base.register;
        info.static_address = if indirect {
            self.address_value(base)
        } else {
            base.static_address
        };
        info
    }

    /// C99: §6.5.2.2p1-7, pp. 71-72; PDF pp. 83-84.
    fn type_call(
        &mut self,
        e: &'tu Expression<'tu>,
        function: ExpressionInfo<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        self.x86_immediates(function, arguments);
        let ty = self.converted(function);
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let Some(target) = self.pointer_target(ty) else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        };
        let TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } = self.types.nodes[target.index]
        else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        };
        if prototype
            && (arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len()))
        {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
        }
        let mut failed = prototype
            && (arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len()));
        for (index, &argument) in arguments.iter().enumerate() {
            let info = self.expression_info(argument);
            failed |= self.types.unanalyzed(info.ty);
            if prototype && let Some(&parameter) = parameters.get(index) {
                if !self.assignment_compatible(parameter, info) {
                    failed = true;
                    self.error(
                        SemanticErrorKind::InvalidArgumentType,
                        argument.source_vectors,
                        None,
                        None,
                    );
                }
                self.convert(argument, parameter, ConversionKind::Assignment);
            } else {
                let ty = self.converted(info);
                let ty = if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Float)) {
                    self.types.scalar(Scalar::Double)
                } else {
                    self.promote(info, ty)
                };
                self.convert(argument, ty, ConversionKind::DefaultArgument);
            }
            if !self.types.unanalyzed(info.ty)
                && matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::Void)
                )
            {
                failed = true;
                self.error(
                    SemanticErrorKind::InvalidArgumentType,
                    argument.source_vectors,
                    None,
                    None,
                );
            }
        }
        if failed {
            return Self::expression_result(e, self.types.unknown());
        }
        if !matches!(
            self.types.nodes[result.index],
            TypeKind::Scalar(Scalar::Void)
        ) && !self.types.unanalyzed(result)
            && !self.complete_object(result)
        {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        }
        Self::expression_result(e, result.unqualified())
    }

    /// C99: §6.5.5-§6.5.17, pp. 82-94; PDF pp. 94-106.
    fn type_binary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        use BinaryOperator as B;
        if self.types.unanalyzed(left.ty) || self.types.unanalyzed(right.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        if let Some(result) = self.vector_binary(e, op, left, right) {
            return result;
        }
        let compound = match op {
            | B::MultiplicationAssignment => Some(B::Multiplication),
            | B::DivisionAssignment => Some(B::Division),
            | B::ModuloAssignment => Some(B::Modulo),
            | B::AdditionAssignment => Some(B::Addition),
            | B::SubtractionAssignment => Some(B::Subtraction),
            | B::LeftShiftAssignment => Some(B::LeftShift),
            | B::RightShiftAssignment => Some(B::RightShift),
            | B::BitwiseAndAssignment => Some(B::BitwiseAnd),
            | B::BitwiseXorAssignment => Some(B::BitwiseXor),
            | B::BitwiseOrAssignment => Some(B::BitwiseOr),
            | _ => None,
        };
        if op == B::Assignment || compound.is_some() {
            if left.category != ValueCategory::ModifiableLvalue {
                return self.invalid_expression(e, SemanticErrorKind::ExpectedModifiableLvalue);
            }
            if let Some(operation) = compound {
                // This is one bounded delegation, never a traversal of syntax.
                if matches!(operation, B::Addition | B::Subtraction) {
                    // Compound addition is directional, unlike ordinary +.
                    // C99 §6.5.16.2p1, p. 93; PDF p. 105.
                    let valid = if self.pointer_target(left.ty).is_some() {
                        self.integer_type(right.ty).is_some()
                    } else {
                        self.arithmetic(left.ty) && self.arithmetic(right.ty)
                    };
                    if !valid {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                }
            } else if !self.assignment_compatible(left.ty, right) {
                return self.invalid_expression(e, SemanticErrorKind::InvalidAssignment);
            }
            if compound.is_none() {
                self.convert(
                    right.expression,
                    self.types.non_atomic(left.ty).unqualified(),
                    ConversionKind::Assignment,
                );
                return Self::expression_result(e, self.types.non_atomic(left.ty).unqualified());
            }
        }
        let op = compound.unwrap_or(op);
        let lt = self.converted(left);
        let rt = self.converted(right);
        let int = self.types.scalar(Scalar::Int);
        if op == B::Comma {
            return Self::expression_result(e, rt);
        }
        let mut info = Self::expression_result(e, self.types.unknown());
        let mut arithmetic_operands = None;
        match op {
            | B::Subscript => {
                let pair = if self.integer_type(rt).is_some() {
                    self.pointer_target(lt).map(|t| (t, left))
                } else if self.integer_type(lt).is_some() {
                    self.pointer_target(rt).map(|t| (t, right))
                } else {
                    None
                };
                let Some((target, pointer)) = pair else {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidSubscript);
                };
                if !self.pointer_arithmetic_target(target) {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidSubscript);
                }
                info.ty = target;
                info.category = self.object_category(target);
                info.static_address = self.address_value(pointer)
                    && if std::ptr::eq(pointer.expression, left.expression) {
                        right.ice && right.integer.is_some()
                    } else {
                        left.ice && left.integer.is_some()
                    };
                // Array decay retains the designated register object;
                // reading a register pointer does not transfer its storage.
                // C99 §6.5.3.2p1, p. 78; PDF p. 90.
                info.register = pointer.register
                    && matches!(self.types.nodes[pointer.ty.index], TypeKind::Array(..));
            },
            | B::Multiplication | B::Division | B::Modulo => {
                if !(self.arithmetic(lt) && self.arithmetic(rt))
                    || (op == B::Modulo
                        && (self.integer_type(lt).is_none() || self.integer_type(rt).is_none()))
                {
                    return self
                        .invalid_expression(e, SemanticErrorKind::InvalidArithmeticOperands);
                }
                info.ty = self.common_arithmetic(left, right, lt, rt);
            },
            | B::Addition | B::Subtraction =>
                if self.arithmetic(lt) && self.arithmetic(rt) {
                    info.ty = self.common_arithmetic(left, right, lt, rt);
                } else if let Some(target) = self.pointer_target(lt) {
                    if !self.pointer_arithmetic_target(target) {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                    if self.integer_type(rt).is_some() {
                        info.ty = lt;
                        if self.address_value(left) && right.ice && right.integer.is_some() {
                            info.constant = ConstantClass::Address;
                        }
                    } else if op == B::Subtraction
                        && self.pointer_target(rt).is_some_and(|t| {
                            self.pointer_arithmetic_target(t)
                                && self.pointer_compatible(target, t, false)
                        })
                    {
                        info.ty = self.types.scalar(self.types.target.ptrdiff_t);
                        info.integer = self.address_difference(left, right, target);
                        if info.integer.is_some() {
                            info.constant = ConstantClass::Arithmetic;
                        }
                    } else {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                } else if op == B::Addition
                    && self.integer_type(lt).is_some()
                    && self
                        .pointer_target(rt)
                        .is_some_and(|t| self.pointer_arithmetic_target(t))
                {
                    info.ty = rt;
                    if self.address_value(right) && left.ice && left.integer.is_some() {
                        info.constant = ConstantClass::Address;
                    }
                } else {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                },
            | B::LeftShift | B::RightShift | B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor => {
                if self.integer_type(lt).is_none() || self.integer_type(rt).is_none() {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidIntegerOperands);
                }
                info.ty = if matches!(op, B::LeftShift | B::RightShift) {
                    let ty = self.promote(right, rt);
                    self.convert(right.expression, ty, ConversionKind::Arithmetic);
                    let ty = self.promote(left, lt);
                    self.convert(left.expression, ty, ConversionKind::Arithmetic);
                    ty
                } else {
                    self.common_arithmetic(left, right, lt, rt)
                };
            },
            | B::LogicalAnd | B::LogicalOr => {
                if !self.scalar_type(lt) || !self.scalar_type(rt) {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidLogicalOperands);
                }
                info.ty = int;
            },
            | B::LessThan
            | B::LessThanOrEqual
            | B::GreaterThan
            | B::GreaterThanOrEqual
            | B::Equal
            | B::NotEqual => {
                let equality = matches!(op, B::Equal | B::NotEqual);
                let arithmetic = if equality {
                    self.arithmetic(lt) && self.arithmetic(rt)
                } else {
                    self.real(lt) && self.real(rt)
                };
                if arithmetic {
                    arithmetic_operands = Some(self.common_arithmetic(left, right, lt, rt));
                } else {
                    let null_pair = equality
                        && ((self.pointer_target(lt).is_some()
                            && self.null_pointer_constant(right))
                            || (self.pointer_target(rt).is_some()
                                && self.null_pointer_constant(left)));
                    // C99 §6.5.9p2, p. 86; PDF p. 98: null alternatives
                    // apply even when the null constant itself is void *.
                    let valid = if null_pair {
                        true
                    } else if let (Some(lp), Some(rp)) =
                        (self.pointer_target(lt), self.pointer_target(rt))
                    {
                        self.pointer_compatible(lp, rp, equality)
                            && (equality
                                || !matches!(self.types.nodes[lp.index], TypeKind::Function { .. }))
                    } else {
                        false
                    };
                    if !valid {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidComparisonOperands);
                    }
                }
                info.ty = int;
            },
            | _ => {},
        }
        if self.arithmetic(info.ty) {
            info.unfolded_binary128 = left.unfolded_binary128
                || right.unfolded_binary128
                || (self.binary128_type(info.ty)
                    && left.constant == ConstantClass::Arithmetic
                    && right.constant == ConstantClass::Arithmetic);
            let floating = left.floating.is_some() || right.floating.is_some();
            if floating
                && let Some((left, right)) = self
                    .floating_value(left)
                    .and_then(|v| self.round_floating(v, arithmetic_operands.unwrap_or(info.ty)))
                    .zip(self.floating_value(right).and_then(|v| {
                        self.round_floating(v, arithmetic_operands.unwrap_or(info.ty))
                    }))
            {
                info.floating = self
                    .floating_binary(op, left, right)
                    .and_then(|value| self.round_floating(value, info.ty));
                info.integer = Self::floating_comparison(op, left, right);
            }
            info.ice = left.ice && right.ice;
            if !floating && let Some((l, r)) = left.integer.zip(right.integer) {
                info.integer = l.binary(op, r);
                if info.integer.is_none()
                    && let Some(value) = l.sign_bit_shift(op, r)
                {
                    info.integer = Some(value);
                    if !self.tainted {
                        self.context.report_extension(
                            crate::configuration::Feature::SignBitShifts,
                            "left shift into the sign bit",
                            e.operator_source_vectors.unwrap_or(e.source_vectors),
                        );
                    }
                }
            }
            if matches!(op, B::LogicalAnd | B::LogicalOr) {
                info.integer = Self::constant_truth(left)
                    .zip(Self::constant_truth(right))
                    .map(|(l, r)| {
                        Integer::int(i128::from(if op == B::LogicalAnd {
                            l && r
                        } else {
                            l || r
                        }))
                    });
            }
            if let Some((bits, signed)) = self.integer_type(info.ty) {
                info.integer = info.integer.map(|v| v.cast(bits, signed));
            }
            if left.constant == ConstantClass::Arithmetic
                && right.constant == ConstantClass::Arithmetic
            {
                info.constant = ConstantClass::Arithmetic;
            }
            if matches!(op, B::LogicalAnd | B::LogicalOr)
                && Self::constant_truth(left)
                    .is_some_and(|v| (op == B::LogicalAnd && !v) || (op == B::LogicalOr && v))
            {
                info.integer = Self::constant_truth(left).map(|v| Integer::int(i128::from(v)));
                if left.constant == ConstantClass::Arithmetic {
                    info.constant = ConstantClass::Arithmetic;
                }
            }
        }
        if compound.is_some() {
            let ty = self.types.non_atomic(left.ty).unqualified();
            self.convert(e, ty, ConversionKind::Assignment);
            let mut result = Self::expression_result(e, ty);
            result.operation_type = Some(info.ty);
            return result;
        }
        info
    }

    /// C99: §6.5.15p2-6, pp. 90-91; PDF pp. 102-103.
    fn type_conditional(
        &mut self,
        e: &'tu Expression<'tu>,
        c: &'tu ConditionalExpression<'tu>,
    ) -> ExpressionInfo<'tu> {
        let condition = self.expression_info(c.condition_expression);
        let left = self.expression_info(c.then_expression);
        let right = self.expression_info(c.else_expression);
        let ct = self.converted(condition);
        let lt = self.converted(left);
        let rt = self.converted(right);
        if self.types.unanalyzed(ct) || self.types.unanalyzed(lt) || self.types.unanalyzed(rt) {
            return Self::expression_result(e, self.types.unknown());
        }
        if !self.scalar_type(ct) {
            return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
        }
        let ty = if self.arithmetic(lt) && self.arithmetic(rt) {
            self.common_arithmetic(left, right, lt, rt)
        } else if (matches!(self.types.nodes[lt.index], TypeKind::Scalar(Scalar::Void))
            && matches!(self.types.nodes[rt.index], TypeKind::Scalar(Scalar::Void)))
            || (self.pointer_target(lt).is_some() && self.null_pointer_constant(right))
        {
            lt
        } else if self.pointer_target(rt).is_some() && self.null_pointer_constant(left) {
            rt
        } else if let (Some(lp), Some(rp)) = (self.pointer_target(lt), self.pointer_target(rt)) {
            if !self.pointer_compatible(lp, rp, true) {
                return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
            }
            let target = if matches!(self.types.nodes[lp.index], TypeKind::Scalar(Scalar::Void)) {
                lp
            } else if matches!(self.types.nodes[rp.index], TypeKind::Scalar(Scalar::Void)) {
                rp
            } else {
                self.types
                    .composite(lp.unqualified(), rp.unqualified())
                    .unwrap_or(lp)
            };
            self.types.intern(TypeKind::Pointer(
                target.qualified(lp.qualifiers | rp.qualifiers),
            ))
        } else if matches!(self.types.nodes[lt.index], TypeKind::Tag(_))
            && self
                .types
                .composite(lt.unqualified(), rt.unqualified())
                .is_some()
        {
            lt.unqualified()
        } else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
        };
        self.convert(left.expression, ty, ConversionKind::Arithmetic);
        self.convert(right.expression, ty, ConversionKind::Arithmetic);
        let mut info = Self::expression_result(e, ty);
        if self.arithmetic(ty)
            && condition.constant == ConstantClass::Arithmetic
            && left.constant == ConstantClass::Arithmetic
            && right.constant == ConstantClass::Arithmetic
            && (condition.unfolded_binary128
                || left.unfolded_binary128
                || right.unfolded_binary128
                || self.binary128_type(ty))
        {
            info.unfolded_binary128 = true;
            info.constant = ConstantClass::Arithmetic;
        }
        if let Some(truth) = Self::constant_truth(condition) {
            let selected = if truth { left } else { right };
            info.unfolded_binary128 = selected.unfolded_binary128
                || (self.binary128_type(ty) && selected.constant == ConstantClass::Arithmetic);
            info.integer = selected.integer;
            if self.arithmetic(ty) && self.integer_type(ty).is_none() {
                info.floating = self
                    .floating_value(selected)
                    .and_then(|v| self.round_floating(v, ty));
            }
            if let Some((bits, signed)) = self.integer_type(ty) {
                info.integer = info.integer.map(|v| v.cast(bits, signed));
            }
            if condition.constant == ConstantClass::Arithmetic {
                info.constant = if self.address_value(selected) {
                    ConstantClass::Address
                } else {
                    selected.constant
                };
            }
        }
        info.ice = condition.ice && left.ice && right.ice;
        info
    }
}
