//! Phase-7 C23 named constants: `constexpr` objects and `constexpr`
//! compound literals.
//!
//! A `constexpr` object is an ordinary `const` object whose initializer
//! [`Analyzer::check_constexpr_initializer`] holds to the C23 constraints:
//! every value it stores is a constant of the kind its subobject's type
//! asks for, and converts to that type without changing. Its name, and a
//! `.` member of it, is then a named constant: [`Analyzer::constant_place`]
//! records which subobject of which initializer such an expression
//! designates, and [`Analyzer::read_named_constant`] gives a scalar one the
//! value its initializer stores there, so `constexpr int n = 4; int a[n];`
//! declares an ordinary array.
//!
//! For `constexpr struct s { int a; double b; } v = { 1, 2.5 };`, `v.a` is
//! the place 0 bytes into `v`'s initializer; the initializer's plan stores
//! the constant `1` there, which converted to `int` is `v.a`'s value, an
//! integer constant expression operand. `(int) v.b` is one too under GCC,
//! a cast of a named constant of arithmetic type; Clang folds it as a GNU
//! extension instead.
//!
//! C23 (N3220): §6.7.2 paragraphs 5-6, p. 99; PDF p. 112 and paragraphs
//! 16-17, p. 100; PDF p. 113 (`constexpr`); §6.6 paragraphs 7-8, p. 95;
//! PDF p. 108 and paragraphs 13-15, p. 96; PDF p. 109 (named constants);
//! §6.5.3.6 paragraphs 4-7, p. 79; PDF p. 92 (compound literals).

use super::{
    Analyzer,
    BitRange,
    ConstantClass,
    ExpressionInfo,
    InitOperation,
    SemanticErrorKind,
    errors::ConstexprFault,
    integer::Integer,
    types::{
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::RuleSet,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                Initializer,
                TypeQualifiers,
            },
            syntax::{
                Expression,
                ExpressionType,
                StorageClass,
            },
        },
        preprocessing::StringTokenType,
    },
    util::bump::ArenaVec,
};

/// The subobject of a named constant an expression designates: `offset`
/// bytes into the object `initializer` initializes, and for a bit-field
/// member its bits from there.
/// C23 (N3220): §6.6 paragraphs 7 and 15, pp. 95-96; PDF pp. 108-109.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConstantPlace<'tu> {
    pub(crate) initializer: &'tu Initializer<'tu>,
    pub(crate) offset:      u64,
    pub(crate) bit_field:   Option<BitRange>,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the form of a `constexpr` declaration before its parts are
    /// analyzed, and opens the region in which GCC rejects declaring a tag:
    /// it declares objects, beside `auto` only to infer their type, and
    /// under GCC exactly one.
    /// C23 (N3220): §6.7.2 paragraphs 2 and 4-5, p. 99; PDF p. 112;
    /// §6.7.1 paragraph 12, p. 98; PDF p. 111.
    pub(super) fn constexpr_declaration(&mut self, d: &'tu Declaration<'tu>) {
        let spec = d.declaration_specifiers;
        if !Self::constexpr_storage(spec.extensions)
            || spec.storage_class == Some(StorageClass::Typedef)
        {
            return;
        }
        let fault = |fault| SemanticErrorKind::InvalidConstexpr(fault);
        if d.init_declarators.is_empty() {
            self.error(
                fault(ConstexprFault::NotObject),
                spec.source_vectors,
                None,
                None,
            );
        } else if spec.storage_class == Some(StorageClass::Auto)
            && !super::inference::inferred(spec)
        {
            self.error(
                SemanticErrorKind::InvalidInference(
                    super::inference::InferenceFault::StorageWithoutInference,
                ),
                spec.source_vectors,
                None,
                None,
            );
        } else if self.context.configuration.compiler_identity().rules() == RuleSet::Gcc
            && let [_, second, ..] = d.init_declarators.as_slice()
        {
            self.error(
                fault(ConstexprFault::MultipleDeclarators),
                second.declarator.source_vectors,
                None,
                None,
            );
        }
        self.underspecified += 1;
        self.work.push(super::Work::EndUnderspecified);
    }

    /// Reports, under GCC's identity, a use of a `constexpr` object inside
    /// its own initializer, where C23 has not yet put it in scope.
    /// C23 (N3220): §6.2.1 paragraph 7, p. 36; PDF p. 49.
    pub(super) fn constexpr_self_reference(
        &mut self,
        binding: usize,
        name: crate::translation_phases::parsing::syntax::Identifier,
    ) {
        if self.constexpr_objects.contains_key(&binding)
            && self.context.configuration.compiler_identity().rules() == RuleSet::Gcc
        {
            self.error(
                SemanticErrorKind::InvalidConstexpr(ConstexprFault::SelfReference),
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
    }

    /// Checks the initializer of the `constexpr` object `binding` and, if
    /// it holds to the constraints, makes the object a named constant.
    /// C23 (N3220): §6.7.2 paragraphs 5-6, p. 99; PDF p. 112.
    pub(super) fn constexpr_initialized(
        &mut self,
        binding: usize,
        initializer: &'tu Initializer<'tu>,
        errors_before: usize,
    ) {
        let ty = self.bindings[binding].ty;
        let source = self.bindings[binding].name.source_vectors;
        if self.check_constexpr_initializer(ty, initializer, source, errors_before) {
            _ = self.named_objects.insert(binding, initializer);
        }
    }

    /// Checks a `constexpr` compound literal as a `constexpr` object and,
    /// if it holds to the constraints, makes it a named constant whose
    /// value `info` takes.
    /// C23 (N3220): §6.5.3.6 paragraphs 4 and 7, p. 79; PDF p. 92.
    pub(super) fn constexpr_literal(
        &mut self,
        info: &mut ExpressionInfo<'tu>,
        initializer: &'tu Initializer<'tu>,
        errors_before: usize,
    ) {
        let source = info.expression.source_vectors;
        if self.check_constexpr_initializer(info.ty, initializer, source, errors_before) {
            self.read_named_constant(
                info,
                ConstantPlace {
                    initializer,
                    offset: 0,
                    bit_field: None,
                },
            );
        }
    }

    /// Reports the first way a `constexpr` object of type `ty` or its
    /// initializer breaks the C23 constraints, unless analysis already
    /// reported an error since `errors_before`. Returns whether the object
    /// is a named constant.
    /// C23 (N3220): §6.7.2 paragraphs 5-6, p. 99; PDF p. 112.
    fn check_constexpr_initializer(
        &mut self,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        source: SourceVectors,
        errors_before: usize,
    ) -> bool {
        if self.types.unanalyzed(ty) || initializer.recovered {
            return false;
        }
        if let Some(fault) = self.constexpr_type_fault(ty) {
            self.error(
                SemanticErrorKind::InvalidConstexpr(fault),
                source,
                None,
                None,
            );
            return false;
        }
        if self.semantic_errors != errors_before {
            return false;
        }
        let Some(&plan) = self.initializers.plan(initializer) else {
            return false;
        };
        for operation in plan.operations {
            let fault = match *operation {
                | InitOperation::Store {
                    value,
                    destination,
                    bit_field,
                    ..
                } => {
                    let e = plan.values[value.0 as usize].expression;
                    self.constexpr_value_fault(destination.ty, e)
                        .or_else(|| {
                            bit_field.and_then(|bits| self.bit_field_fault(destination.ty, bits, e))
                        })
                        .map(|fault| (fault, e.source_vectors))
                },
                | InitOperation::Copy { value, .. } => {
                    let e = plan.values[value.0 as usize].expression;
                    self.constant_place(e)
                        .is_none()
                        .then_some((ConstexprFault::NotConstant, e.source_vectors))
                },
                | InitOperation::String {
                    literal,
                    destination,
                } => self
                    .string_fault(destination.ty, literal)
                    .then_some((ConstexprFault::NotRepresentable, literal.source_vectors)),
                | InitOperation::Repeat { .. } | InitOperation::Unmodeled { .. } =>
                    Some((ConstexprFault::NotConstant, initializer.source_vectors)),
                | InitOperation::Zero { .. }
                | InitOperation::Select { .. }
                | InitOperation::EmptyScalar { .. } => None,
            };
            if let Some((fault, source)) = fault {
                self.error(
                    SemanticErrorKind::InvalidConstexpr(fault),
                    source,
                    None,
                    None,
                );
                return false;
            }
        }
        true
    }

    /// Why a `constexpr` object cannot have type `ty`: an atomic, volatile
    /// or restrict-qualified type, its own or a member's (recursively), or
    /// a variably modified one.
    /// C23 (N3220): §6.7.2 paragraph 5, p. 99; PDF p. 112.
    fn constexpr_type_fault(&self, ty: TypeId) -> Option<ConstexprFault> {
        if self.variably_modified(ty) {
            return Some(ConstexprFault::VariablyModified);
        }
        let invalid = |ty: TypeId| {
            ty.qualifiers
                .intersects(TypeQualifiers::VOLATILE | TypeQualifiers::RESTRICT)
                || matches!(self.types.nodes[ty.index], TypeKind::Atomic(_))
        };
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push((ty, false));
        while let Some((mut ty, member)) = pending.pop() {
            if invalid(ty) {
                return Some(if member {
                    ConstexprFault::MemberQualifiers
                } else {
                    ConstexprFault::Qualifiers
                });
            }
            while let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
                ty = element;
                if invalid(ty) {
                    return Some(if member {
                        ConstexprFault::MemberQualifiers
                    } else {
                        ConstexprFault::Qualifiers
                    });
                }
            }
            if let TypeKind::Tag(id) = self.types.nodes[ty.index]
                && self.types.tags[id].kind != TagKind::Enum
            {
                for member in self.types.tags[id].members.get() {
                    pending.push((member.ty, true));
                }
            }
        }
        None
    }

    /// Why the value of `e` cannot initialize a scalar subobject of type
    /// `target` of a `constexpr` object: a pointer takes only a null
    /// pointer, an integer only an integer constant expression and a
    /// floating type only an arithmetic constant expression of a real type
    /// for a real one, and the value converted to `target` must be the
    /// value itself.
    /// C23 (N3220): §6.7.2 paragraphs 5-6, p. 99; PDF p. 112.
    fn constexpr_value_fault(
        &mut self,
        target: TypeId,
        e: &'tu Expression<'tu>,
    ) -> Option<ConstexprFault> {
        let value = self.expression_info(e);
        if self.types.unanalyzed(value.ty) {
            return None;
        }
        let constant = value.constant != ConstantClass::None || value.integer.is_some();
        if self.pointer_target(target).is_some() || self.nullptr_type(target) {
            return if self.nullptr_type(value.ty) || value.integer.is_some_and(Integer::is_zero) {
                None
            } else if constant {
                Some(ConstexprFault::PointerNotNull)
            } else {
                Some(ConstexprFault::NotConstant)
            };
        }
        if !self.arithmetic(target) {
            return (!constant).then_some(ConstexprFault::NotConstant);
        }
        let clang = self.context.configuration.compiler_identity().rules() == RuleSet::Clang;
        if self.integer_type(target).is_some() {
            // Clang takes any integer constant of integer type, a cast of a
            // floating one included, where GCC asks for an integer
            // constant expression.
            let integer = if clang {
                self.integer_type(self.types.non_atomic(value.ty)).is_some()
                    && value.integer.is_some()
                    && value.constant == ConstantClass::Arithmetic
            } else {
                value.ice && value.integer.is_some()
            };
            if !integer {
                return Some(if constant {
                    ConstexprFault::IntegerNotIce
                } else {
                    ConstexprFault::NotConstant
                });
            }
        } else if value.constant != ConstantClass::Arithmetic
            || (value.integer.is_none()
                && value.floating().is_none()
                && value.binary128().is_none())
        {
            return Some(if constant {
                ConstexprFault::ArithmeticNotConstant
            } else {
                ConstexprFault::NotConstant
            });
        } else if self.complex(value.ty) && !self.complex(target) {
            return Some(ConstexprFault::ComplexToReal);
        }
        // Clang checks no complex value.
        if clang && self.complex(target) {
            return None;
        }
        (!self.exactly_converts(target, value)).then_some(ConstexprFault::NotRepresentable)
    }

    /// Why, under GCC's identity, the integer constant `e` cannot initialize
    /// a bit-field of type `target` and width `bits.width`: its value
    /// converted to `target` does not fit the width. Clang takes the value
    /// truncated to the width, with a warning.
    /// C23 (N3220): §6.7.2 paragraph 6, p. 99; PDF p. 112; §6.7.3.2
    /// paragraph 12, p. 106; PDF p. 119.
    fn bit_field_fault(
        &self,
        target: TypeId,
        bits: BitRange,
        e: &'tu Expression<'tu>,
    ) -> Option<ConstexprFault> {
        if self.context.configuration.compiler_identity().rules() != RuleSet::Gcc {
            return None;
        }
        let (type_bits, signed) = self.integer_type(target)?;
        let integer = self.expression_info(e).integer?;
        // A bit-field wider than 128 bits keeps its value unchecked, as
        // [`Self::member_constant`] reads none from it.
        if type_bits > 128 || integer.wide.is_some() {
            return None;
        }
        let value = integer.cast(type_bits, signed);
        (value.cast(bits.width, signed).value != value.value)
            .then_some(ConstexprFault::NotRepresentable)
    }

    /// Whether the arithmetic constant `value` converted to `target` is the
    /// same value: an integer keeps its value, a floating value is exactly
    /// representable (converting it back gives it again), and a signaling
    /// NaN keeps its type.
    /// C23 (N3220): §6.7.2 paragraphs 5-6, p. 99; PDF p. 112.
    fn exactly_converts(&mut self, target: TypeId, value: ExpressionInfo<'tu>) -> bool {
        let mut converted = Self::expression_result(value.expression, target);
        self.fold_conversion(&mut converted, target, value);
        if let Some(integer) = value.integer
            && value.floating().is_none()
            && value.binary128().is_none()
        {
            if self.integer_type(target).is_some() {
                return converted.integer.is_some_and(|c| c.value == integer.value);
            }
            if let Some(converted) = converted.binary128() {
                // The value is exact when converting it back gives it again;
                // a value only the wide table holds is checked no further.
                return integer.wide.is_none()
                    && converted.imag.is_zero()
                    && converted.real.to_integer(integer.signed, 128)
                        == Some(integer.value.cast_unsigned());
            }
            return converted.floating().is_some_and(|floating| {
                floating.imag.is_zero()
                    && crate::float_parsing::bits::to_integer(&floating.real, 128, true)
                        .is_some_and(|bits| bits.cast_signed() == integer.value)
            });
        }
        let source = self.types.non_atomic(value.ty).unqualified();
        let mut back = Self::expression_result(value.expression, source);
        self.fold_conversion(&mut back, source, converted);
        if let (Some(a), Some(b)) = (value.binary128(), back.binary128()) {
            return a.real.to_bits() == b.real.to_bits() && a.imag.to_bits() == b.imag.to_bits();
        }
        let (Some(a), Some(b)) = (value.floating(), back.floating()) else {
            return false;
        };
        let real_target = self.real_part(target);
        let real_source = self.real_part(source);
        let same_type = self.types.composite(real_source, real_target).is_some();
        let same = |a: crate::float_parsing::LongDouble, b: crate::float_parsing::LongDouble| {
            // An x87 NaN has the largest exponent and a significand other
            // than the explicit integer bit; a signaling one has its
            // quiet bit (bit 62) clear.
            let nan = |x: crate::float_parsing::LongDouble| {
                x.x87_parts().and_then(|(_, exponent, significand)| {
                    (exponent == 0x7FFF && significand << 1 != 0)
                        .then_some(significand >> 62 & 1 == 0)
                })
            };
            match (nan(a), nan(b)) {
                | (Some(signaling), Some(_)) => !signaling || same_type,
                | _ => a.compare(b) == 0,
            }
        };
        same(a.real, b.real) && same(a.imag, b.imag)
    }

    /// The real type of an arithmetic type: the type itself, or a complex
    /// type's corresponding real type.
    fn real_part(&mut self, ty: TypeId) -> TypeId {
        match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Scalar(scalar) => self.types.scalar(scalar.real_type()),
            | _ => ty,
        }
    }

    /// Whether `ty` is a complex type.
    fn complex(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(ty).index],
            TypeKind::Scalar(scalar) if scalar.complex()
        )
    }

    /// Whether an element of the string literal `literal` changes value
    /// when an array of type `array` takes it: `"\xff"`, whose plain
    /// `char` element is -1 where `char` is signed, cannot initialize an
    /// `unsigned char` array.
    /// C23 (N3220): §6.7.2 paragraph 5 and NOTE 3, pp. 99-100; PDF
    /// pp. 112-113.
    fn string_fault(&mut self, array: TypeId, literal: &'tu Expression<'tu>) -> bool {
        let mut e = literal;
        while let ExpressionType::Parenthesized { expression } = e.kind {
            e = expression;
        }
        let ExpressionType::StringLiteral(string) = e.kind else {
            return false;
        };
        let TypeKind::Array(element, _) = self.types.nodes[array.index] else {
            return false;
        };
        // Clang reads a narrow string's elements as `signed char`, even
        // where `char` is unsigned.
        let clang = self.context.configuration.compiler_identity().rules() == RuleSet::Clang;
        let source = match string {
            | StringTokenType::String(_) if clang => Scalar::SignedChar,
            | StringTokenType::String(_) => Scalar::Char,
            | StringTokenType::WideString(_) => self.types.target.wchar_t,
            | StringTokenType::EncodedString(_, encoding) => self.encoded_character_type(encoding),
        };
        let source = self.types.scalar(source);
        let (Some((from_bits, from_signed)), Some((bits, signed))) =
            (self.integer_type(source), self.integer_type(element))
        else {
            return false;
        };
        self.literal_units(string).iter().any(|&unit| {
            let value = Integer::int(i128::from(unit)).cast(from_bits, from_signed);
            value.cast(bits, signed).value != value.value
        })
    }

    /// The named constant place `e` designates, through parentheses.
    pub(super) fn constant_place(&self, mut e: &'tu Expression<'tu>) -> Option<ConstantPlace<'tu>> {
        while let ExpressionType::Parenthesized { expression } = e.kind {
            e = expression;
        }
        self.named_constants
            .get(&std::ptr::from_ref(e).addr())
            .copied()
    }

    /// Records `place`, moved through the copies that give it its value, as
    /// the subobject `info`'s expression designates, and gives a scalar one
    /// the value the named constant's initializer stores there: an integer
    /// one is an integer constant expression operand, a bit-field's
    /// truncated to its width.
    /// C23 (N3220): §6.6 paragraphs 7-8 and 10, pp. 95-96; PDF
    /// pp. 108-109.
    pub(super) fn read_named_constant(
        &mut self,
        info: &mut ExpressionInfo<'tu>,
        place: ConstantPlace<'tu>,
    ) {
        let ty = self.types.non_atomic(info.ty).unqualified();
        let place = self.copied_place(place, ty);
        _ = self
            .named_constants
            .insert(std::ptr::from_ref(info.expression).addr(), place);
        let pointer = self.pointer_target(ty).is_some() || self.nullptr_type(ty);
        if !pointer && !self.arithmetic(ty) {
            return;
        }
        let Some(stored) = self.stored_value(place, ty) else {
            return;
        };
        if pointer {
            // A null pointer, but no null pointer constant (C23 §6.6
            // footnote 118).
            if match stored {
                | Stored::Zero => true,
                | Stored::Value(value) =>
                    self.nullptr_type(value.ty) || value.integer.is_some_and(Integer::is_zero),
            } {
                let bits = u32::try_from(self.types.target.pointer.size * 8).unwrap_or(64);
                info.integer = Some(Integer::int(0).cast(bits, false));
                info.constant = ConstantClass::Arithmetic;
            }
            return;
        }
        let value = match stored {
            | Stored::Value(value) => value,
            | Stored::Zero => {
                let mut zero =
                    Self::expression_result(info.expression, self.types.scalar(Scalar::Int));
                zero.integer = Some(Integer::int(0));
                zero.ice = true;
                zero.constant = ConstantClass::Arithmetic;
                zero
            },
        };
        let mut read = Self::expression_result(info.expression, ty);
        self.fold_conversion(&mut read, ty, value);
        if read.constant != ConstantClass::Arithmetic {
            return;
        }
        info.integer = match (read.integer, place.bit_field, self.integer_type(ty)) {
            | (Some(integer), Some(bits), Some((type_bits, signed))) =>
                Some(integer.cast(bits.width, signed).cast(type_bits, signed)),
            | (integer, ..) => integer,
        };
        info.folded = read.folded;
        info.unfolded_binary128 = read.unfolded_binary128;
        info.constant = ConstantClass::Arithmetic;
        info.ice = self.integer_type(ty).is_some() && info.integer.is_some();
    }

    /// The value the named constant's initializer stores in the scalar
    /// subobject of type `ty` at `place`, or `None` when the plan gives it
    /// no single constant value.
    fn stored_value(&self, mut place: ConstantPlace<'tu>, ty: TypeId) -> Option<Stored<'tu>> {
        let size = self.types.layout(ty)?.size;
        // Each copy of a whole structure or union moves to the named
        // constant it copies. Recorded places start past the copies that
        // hold them whole ([`Self::copied_place`]), so the walk takes a step
        // per copy nested in the type rather than per copy declared.
        loop {
            let plan = self.initializers.plan(place.initializer)?;
            let span = Span::new(place.offset, place.bit_field, size);
            let mut found = None;
            for operation in plan.operations {
                match *operation {
                    | InitOperation::Repeat { .. } | InitOperation::Unmodeled { .. } =>
                        return None,
                    | InitOperation::Store {
                        value,
                        destination,
                        bit_field,
                        ..
                    } if destination.offset == place.offset
                        && bit_field == place.bit_field
                        && destination.ty.index == ty.index =>
                        found = Some(Found::Value(plan.values[value.0 as usize].expression)),
                    | InitOperation::Zero { destination, .. }
                    | InitOperation::EmptyScalar { destination, .. }
                        if self.covers(destination, span) =>
                        found = Some(Found::Zero),
                    | InitOperation::Copy {
                        value, destination, ..
                    } if self.covers(destination, span) =>
                        found = Some(Found::Copy(
                            plan.values[value.0 as usize].expression,
                            destination.offset,
                        )),
                    | InitOperation::Store { .. } | InitOperation::String { .. }
                        if self
                            .written_span(operation)
                            .is_none_or(|written| written.overlaps(span)) =>
                        found = None,
                    | _ => {},
                }
            }
            match found? {
                | Found::Zero => return Some(Stored::Zero),
                | Found::Value(e) => return Some(Stored::Value(self.expression_info(e))),
                | Found::Copy(e, offset) => place = self.through_copy(place, e, offset)?,
            }
        }
    }

    /// `place`, of type `ty`, moved through each copy of a structure or
    /// union that last writes all of it, and has it as a subobject, to the
    /// named constant the copy reads. The copy's own place was recorded
    /// this way, so a chain of copies `s1 = s0`, `s2 = s1`, ... takes one
    /// step per copy declared. A union stays where a copy of its member
    /// covers it, as that copy does not tell which member is active.
    /// C23 (N3220): §6.6 paragraph 7, p. 95; PDF p. 108.
    fn copied_place(&self, mut place: ConstantPlace<'tu>, ty: TypeId) -> ConstantPlace<'tu> {
        let Some(size) = self.types.layout(ty).map(|layout| layout.size) else {
            return place;
        };
        loop {
            let Some(plan) = self.initializers.plan(place.initializer) else {
                return place;
            };
            let span = Span::new(place.offset, place.bit_field, size);
            let mut copy = None;
            for operation in plan.operations {
                match *operation {
                    | InitOperation::Repeat { .. } | InitOperation::Unmodeled { .. } =>
                        return place,
                    | InitOperation::Copy {
                        value, destination, ..
                    } if destination.offset <= place.offset
                        && self.contains_subobject(
                            destination.ty,
                            place.offset - destination.offset,
                            ty,
                        ) =>
                        copy = Some((plan.values[value.0 as usize].expression, destination.offset)),
                    | _ if self
                        .written_span(operation)
                        .is_none_or(|written| written.overlaps(span)) =>
                        copy = None,
                    | _ => {},
                }
            }
            let Some(next) = copy.and_then(|(e, offset)| self.through_copy(place, e, offset))
            else {
                return place;
            };
            place = next;
        }
    }

    /// `place` read through the copy of the named constant `e` placed at
    /// `offset`: the same subobject of the place `e` designates.
    fn through_copy(
        &self,
        place: ConstantPlace<'tu>,
        e: &'tu Expression<'tu>,
        offset: u64,
    ) -> Option<ConstantPlace<'tu>> {
        let copied = self.constant_place(e)?;
        Some(ConstantPlace {
            initializer: copied.initializer,
            offset:      copied.offset + (place.offset - offset),
            bit_field:   place.bit_field,
        })
    }

    /// Whether the subobject `destination` holds all of `span`.
    fn covers(&self, destination: super::Subobject, span: Span) -> bool {
        self.types
            .layout(destination.ty)
            .is_some_and(|layout| Span::new(destination.offset, None, layout.size).covers(span))
    }

    /// The bits `operation` writes, or `None` for a range repeated or not
    /// modeled, which may write anywhere.
    fn written_span(&self, operation: &InitOperation<'tu>) -> Option<Span> {
        let (destination, bit_field) = match *operation {
            | InitOperation::Zero { destination, .. }
            | InitOperation::Select { destination, .. }
            | InitOperation::Copy { destination, .. }
            | InitOperation::String { destination, .. }
            | InitOperation::EmptyScalar { destination, .. } => (destination, None),
            | InitOperation::Store {
                destination,
                bit_field,
                ..
            } => (destination, bit_field),
            | InitOperation::Repeat { .. } | InitOperation::Unmodeled { .. } => return None,
        };
        let size = self.types.layout(destination.ty)?.size;
        Some(Span::new(destination.offset, bit_field, size))
    }

    /// Records the place of `base.member` when `base` designates a named
    /// constant of structure or union type, and reads its value. A union
    /// constant's only named member is the one its initializer gives a
    /// value, the first for an empty initializer.
    /// C23 (N3220): §6.6 paragraphs 7 and 15, pp. 95-96; PDF pp. 108-109.
    pub(super) fn member_constant(
        &mut self,
        info: &mut ExpressionInfo<'tu>,
        base: &'tu Expression<'tu>,
        member: crate::translation_phases::parsing::syntax::Identifier,
    ) {
        let Some(place) = self.constant_place(base) else {
            return;
        };
        let base_ty = self.expression_info(base).ty;
        let TypeKind::Tag(id) = self.types.nodes[base_ty.index] else {
            return;
        };
        let Some(&index) = self.member_indices.get(&(id, member.name)) else {
            return;
        };
        let Some(field) = self.types.tags[id].fields.get().get(index).copied() else {
            return;
        };
        // A bit-field's value is the bits of its width (C23 §6.7.3.2p12),
        // read here for an integer type of at most 128 bits.
        let bit_field = field.width.map(|width| BitRange {
            bit_offset: field.bit_offset,
            width,
        });
        if bit_field.is_some()
            && self
                .integer_type(field.ty)
                .is_none_or(|(bits, _)| bits > 128)
        {
            return;
        }
        if self.types.tags[id].kind == TagKind::Union
            && self.active_member(place, base_ty) != Some(field.path.index)
        {
            return;
        }
        self.read_named_constant(
            info,
            ConstantPlace {
                initializer: place.initializer,
                offset: place.offset + field.offset,
                bit_field,
            },
        );
    }

    /// The member the initializer of a named constant gives a value in the
    /// union of type `union` at `place`.
    fn active_member(&self, mut place: ConstantPlace<'tu>, union: TypeId) -> Option<usize> {
        // As in [`Self::stored_value`], a step per nested copy.
        loop {
            let plan = self.initializers.plan(place.initializer)?;
            let mut active = None;
            for operation in plan.operations {
                match *operation {
                    | InitOperation::Select {
                        destination,
                        member,
                        ..
                    } if destination.offset == place.offset
                        && destination.ty.index == union.index =>
                        active = Some(Active::Member(member as usize)),
                    | InitOperation::Zero { destination, .. }
                        if destination.offset == place.offset
                            && destination.ty.index == union.index =>
                        active = Some(Active::Member(0)),
                    | InitOperation::Copy {
                        value, destination, ..
                    } if destination.offset <= place.offset
                        && self.contains_subobject(
                            destination.ty,
                            place.offset - destination.offset,
                            union,
                        ) =>
                        active = Some(Active::Copy(
                            plan.values[value.0 as usize].expression,
                            destination.offset,
                        )),
                    | _ => {},
                }
            }
            match active? {
                | Active::Member(member) => return Some(member),
                | Active::Copy(e, offset) => place = self.through_copy(place, e, offset)?,
            }
        }
    }

    /// Whether an object of type `outer` has a subobject of type `inner`,
    /// itself included, `offset` bytes into it: a copy of it then copies
    /// that subobject whole, where a copy of a member of `inner` does not.
    fn contains_subobject(&self, mut outer: TypeId, mut offset: u64, inner: TypeId) -> bool {
        loop {
            if offset == 0 && outer.index == inner.index {
                return true;
            }
            match self.types.nodes[outer.index] {
                | TypeKind::Array(element, _) => {
                    let Some(size) = self.types.layout(element).map(|layout| layout.size) else {
                        return false;
                    };
                    if size == 0 {
                        return false;
                    }
                    offset %= size;
                    outer = element;
                },
                | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Struct => {
                    let member = self.types.tags[id]
                        .members
                        .get()
                        .iter()
                        .rev()
                        .find(|member| {
                            member.width.is_none()
                                && member.offset <= offset
                                && self.types.layout(member.ty).is_some_and(|layout| {
                                    offset < member.offset + layout.size.max(1)
                                })
                        });
                    let Some(member) = member else {
                        return false;
                    };
                    offset -= member.offset;
                    outer = member.ty;
                },
                | _ => return false,
            }
        }
    }

    /// Whether a cast of `operand` reads a named constant of arithmetic
    /// type that the cast makes an integer constant expression operand:
    /// under GCC's identity; Clang folds the cast as a GNU extension.
    /// C23 (N3220): §6.6 paragraph 8, p. 95; PDF p. 108.
    pub(super) fn named_cast_operand(&self, operand: &'tu Expression<'tu>) -> bool {
        self.context.configuration.compiler_identity().rules() == RuleSet::Gcc
            && self.constant_place(operand).is_some()
    }
}

/// The value a named constant's initializer gives a scalar subobject.
#[derive(Clone, Copy)]
enum Stored<'tu> {
    /// Zero, from an omitted or empty initializer.
    Zero,
    /// The value of an expression, converted to the subobject's type.
    Value(ExpressionInfo<'tu>),
}

/// What a named constant's initializer last stored over a subobject.
#[derive(Clone, Copy)]
enum Found<'tu> {
    Zero,
    Value(&'tu Expression<'tu>),
    /// A copy of a whole structure or union placed at the offset.
    Copy(&'tu Expression<'tu>, u64),
}

/// Which member a union's initializer selects, or the union it copies.
#[derive(Clone, Copy)]
enum Active<'tu> {
    Member(usize),
    Copy(&'tu Expression<'tu>, u64),
}

/// The bits of an object a subobject or bit-field occupies, from the
/// object's start.
#[derive(Clone, Copy)]
struct Span {
    start: u64,
    end:   u64,
}

impl Span {
    /// The `size` bytes at `offset`, or the bits of `bit_field` from there.
    fn new(offset: u64, bit_field: Option<BitRange>, size: u64) -> Self {
        let start = offset.saturating_mul(8);
        match bit_field {
            | Some(bits) => {
                let start = start.saturating_add(u64::from(bits.bit_offset));
                Self {
                    start,
                    end: start.saturating_add(u64::from(bits.width)),
                }
            },
            | None => Self {
                start,
                end: start.saturating_add(size.saturating_mul(8)),
            },
        }
    }

    /// Whether `self` holds all of `other`.
    const fn covers(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Whether `self` and `other` share a bit.
    const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}
