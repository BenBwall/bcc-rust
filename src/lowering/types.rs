//! C types as lowering holds them: which become IR values and of what type,
//! which live only in memory, and their sizes and alignments from the
//! target's data model.

use super::{
    Construct,
    LoweringErrorKind,
};
use crate::{
    configuration::RuleSet,
    ir::{
        AggregateFlags,
        AggregateLayout,
        AggregateScalar,
        Align,
        Extension,
        FirstMember,
        Type,
    },
    target::{
        Layout,
        RecordPassing,
        Scalar,
        Target,
    },
    translation_phases::semantic_analysis::{
        ArrayBound,
        TagKind,
        TypeId,
        TypeKind,
        Types,
    },
    util::bump::{
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// How lowering holds a value of a C type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Repr {
    Void,
    /// An integer type other than `_Bool`, with its C signedness and its
    /// width, which is its IR type's but for a bit-precise type narrower
    /// than that type. Such a value is kept extended to its IR type from
    /// its width, by its signedness ([`Repr::narrow`]).
    /// C99: §6.2.5 paragraphs 4-8, pp. 33-34; PDF pp. 45-46.
    /// C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52.
    Int {
        ty:     Type,
        signed: bool,
        width:  u8,
    },
    /// `_Bool`: an `i8` holding 0 or 1.
    /// C99: §6.2.5 paragraph 2, p. 33; PDF p. 45.
    Bool,
    /// A real floating type: `f32`, `f64`, or the x87 `f80` that `long
    /// double` is on the x86 GNU targets (it is `f64` on MSVC).
    /// C99: §6.2.5 paragraphs 10-11, p. 34; PDF p. 46.
    Float {
        ty: Type,
    },
    /// A C scalar held as two real SSA components.
    /// C99: §6.2.5p13, p. 34; PDF p. 46.
    Complex {
        component: Type,
    },
    Pointer,
    /// An array, structure or union: an object in memory, never a value.
    Aggregate,
    Function,
}

/// The representation of `ty`, or why lowering cannot handle it.
pub(super) fn repr(types: &Types<'_>, mut ty: TypeId) -> Result<Repr, LoweringErrorKind> {
    let unsupported = |construct| Err(LoweringErrorKind::Unsupported(construct));
    while let TypeKind::Atomic(inner) = types.kind(ty) {
        if matches!(
            types.kind(inner),
            TypeKind::Scalar(s) if s.complex_integer() || matches!(
                s.representation(),
                Scalar::ComplexFloat16
                    | Scalar::ComplexFloat
                    | Scalar::ComplexDouble
                    | Scalar::ComplexLongDouble
            )
        ) {
            return unsupported(Construct::ComplexAtomic);
        }
        ty = inner;
    }
    match types.kind(ty) {
        | TypeKind::Unknown => Err(super::LoweringError::unanalyzed_kind(types, ty)),
        | TypeKind::Unmodeled(reason) => Err(LoweringErrorKind::UnmodeledType(reason)),
        | TypeKind::Scalar(scalar) => scalar_repr(types, scalar),
        | TypeKind::Pointer(_) | TypeKind::Nullptr => Ok(Repr::Pointer),
        | TypeKind::BitInt { width, signed } => Ok(bit_int_repr(width, signed)),
        | TypeKind::Array(element, _) =>
            if types.unanalyzed(element) {
                Err(super::LoweringError::unanalyzed_kind(types, ty))
            } else {
                Ok(Repr::Aggregate)
            },
        | TypeKind::Function { .. } => Ok(Repr::Function),
        | TypeKind::Tag(_) => {
            let tag = types.tag(ty).expect("a tag type names its tag");
            if tag.tainted.get() {
                return Err(super::LoweringError::unanalyzed_kind(types, ty));
            }
            match tag.kind {
                | TagKind::Enum => scalar_repr(types, tag.compatible.get()),
                | TagKind::Struct | TagKind::Union => Ok(Repr::Aggregate),
            }
        },
        | TypeKind::Atomic(_) => unreachable!("atomic wrappers were removed above"),
        // A GNU vector lives in memory like an array of its lanes; its
        // operators work lane by lane (`vectors.rs`).
        | TypeKind::Vector { .. } => Ok(Repr::Aggregate),
    }
}

/// C99: §6.2.5 paragraphs 2-11, pp. 33-34; PDF pp. 45-46.
/// An interchange or extended floating type lowers as the type whose
/// representation it shares (C23 §H.2.2 paragraph 3, p. 553; PDF p. 566).
/// The IR type of `long double`: `f64` on MSVC, `f128` on WebAssembly and
/// the x87 `f80` elsewhere.
/// C99: §6.2.5 paragraph 10, p. 34; PDF p. 46.
fn long_double_type(types: &Types<'_>) -> Type {
    if types.target.long_double_is_double() {
        Type::F64
    } else if types.target.long_double_is_binary128() {
        Type::F128
    } else {
        Type::F80
    }
}

pub(super) fn scalar_repr(types: &Types<'_>, scalar: Scalar) -> Result<Repr, LoweringErrorKind> {
    // GNU complex integers: two parts of the real integer type.
    if scalar.complex_integer() {
        let Repr::Int { ty, .. } = scalar_repr(types, scalar.real_type())? else {
            return Err(LoweringErrorKind::MissingFact(
                "a complex integer type has no integer parts",
            ));
        };
        return Ok(Repr::Complex { component: ty });
    }
    match scalar.representation() {
        | Scalar::Void => Ok(Repr::Void),
        | Scalar::Bool => Ok(Repr::Bool),
        | Scalar::Float16 | Scalar::Fp16 => Ok(Repr::Float { ty: Type::F16 }),
        | Scalar::BFloat16 => Ok(Repr::Float { ty: Type::BF16 }),
        | Scalar::ComplexFloat16 => Ok(Repr::Complex {
            component: Type::F16,
        }),
        | Scalar::Float => Ok(Repr::Float { ty: Type::F32 }),
        | Scalar::Double => Ok(Repr::Float { ty: Type::F64 }),
        | Scalar::LongDouble => Ok(Repr::Float {
            ty: long_double_type(types),
        }),
        | Scalar::Float128 => Ok(Repr::Float { ty: Type::F128 }),
        | Scalar::ComplexFloat128 => Ok(Repr::Complex {
            component: Type::F128,
        }),
        | Scalar::ComplexFloat => Ok(Repr::Complex {
            component: Type::F32,
        }),
        | Scalar::ComplexDouble => Ok(Repr::Complex {
            component: Type::F64,
        }),
        | Scalar::ComplexLongDouble => Ok(Repr::Complex {
            component: long_double_type(types),
        }),
        | _ => {
            let (bits, signed) =
                types
                    .target
                    .integer(scalar)
                    .ok_or(LoweringErrorKind::MissingFact(
                        "an integer type has no width",
                    ))?;
            Ok(Repr::Int {
                ty: int_type(bits),
                signed,
                width: u8::try_from(int_type(bits).bits()).unwrap_or(u8::MAX),
            })
        },
    }
}

/// A bit-precise type of up to 128 bits is held in the IR integer type of
/// its storage, the type its layout gives
/// ([`crate::target::Layout`] of `bit_int_layout`); a wider one is not yet
/// lowered.
/// C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52.
fn bit_int_repr(width: u32, signed: bool) -> Repr {
    // A wider one lives in memory as its limbs (`wide_bit_int.rs`).
    let Some(width) = u8::try_from(width).ok().filter(|&width| width <= 128) else {
        return Repr::Aggregate;
    };
    Repr::Int {
        ty: int_type(u32::from(width)),
        signed,
        width,
    }
}

/// The IR integer type of a C integer `bits` wide.
pub(super) fn int_type(bits: u32) -> Type {
    match bits {
        | 0..=8 => Type::I8,
        | 9..=16 => Type::I16,
        | 17..=32 => Type::I32,
        | 33..=64 => Type::I64,
        | _ => Type::I128,
    }
}

impl Repr {
    /// The IR type of a value of this representation, if it has values.
    pub(super) const fn value_type(self) -> Option<Type> {
        match self {
            | Self::Int { ty, .. } | Self::Float { ty } => Some(ty),
            | Self::Bool => Some(Type::I8),
            | Self::Pointer => Some(Type::Ptr),
            | Self::Void | Self::Aggregate | Self::Complex { .. } | Self::Function => None,
        }
    }

    /// Whether it is an integer, `_Bool`, floating or pointer type: a C
    /// scalar type that lowering supports.
    /// C99: §6.2.5 paragraph 21, p. 36; PDF p. 48.
    pub(super) const fn is_scalar(self) -> bool {
        matches!(
            self,
            Self::Int { .. }
                | Self::Bool
                | Self::Float { .. }
                | Self::Complex { .. }
                | Self::Pointer
        )
    }

    pub(super) const fn signed(self) -> bool {
        matches!(self, Self::Int { signed: true, .. })
    }

    /// A bit-precise integer narrower than its IR type: the type, the
    /// signedness and the width its values are extended from.
    /// C23: §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56.
    pub(super) const fn narrow(self) -> Option<(Type, bool, u32)> {
        match self {
            | Self::Int { ty, signed, width } if (width as u32) < ty.bits() =>
                Some((ty, signed, width as u32)),
            | _ => None,
        }
    }

    /// What a parameter or result of this representation records for ABI
    /// lowering: the signedness of an integer type narrower than `int`
    /// (an `i8` or `i16`), or that it is `_Bool`. Conventions differ in
    /// whether such a value arrives widened, so ABI lowering decides.
    /// C99: §6.2.5 paragraphs 2-6, pp. 33-34; PDF pp. 45-46.
    pub(super) const fn extension(self) -> Option<Extension> {
        match self {
            | Self::Int {
                ty: Type::I8 | Type::I16,
                signed,
                ..
            } => Some(if signed {
                Extension::Signed
            } else {
                Extension::Unsigned
            }),
            | Self::Bool => Some(Extension::Bool),
            | _ => None,
        }
    }
}

/// Whether a value of `ty` crosses a call as an aggregate: a bit-precise
/// integer of 65 to 128 bits, on the x86-64 targets, whose calling
/// conventions classify it as an array of 64-bit limbs (x86-64 psABI
/// §3.2.3; Clang's `X86_64ABIInfo` and `WinX86_64ABIInfo`). The
/// WebAssembly C ABI passes it as an `i128` (`bit_int.rs`).
/// C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52.
pub(super) fn limb_argument(types: &Types<'_>, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::BitInt {
            width: 65..=128,
            ..
        }
    ) && types.target.record_passing == RecordPassing::Classified
}

/// The size and alignment of a complete object type; a variable length
/// array has none.
/// C99: §6.5.3.4 paragraph 2, p. 80; PDF p. 92.
pub(super) fn layout(types: &Types<'_>, ty: TypeId) -> Result<Layout, LoweringErrorKind> {
    types.layout(ty).ok_or_else(|| {
        let mut element = ty;
        while let TypeKind::Array(inner, bound) = types.kind(element) {
            if matches!(bound, ArrayBound::Variable(_)) {
                return LoweringErrorKind::Unsupported(Construct::VariableLengthArray);
            }
            element = inner;
        }
        LoweringErrorKind::MissingFact("an object type has no layout")
    })
}

pub(super) fn align(layout: Layout) -> Align {
    Align::from_bytes(layout.align).unwrap_or(Align::BYTE)
}

pub(super) fn is_volatile(ty: TypeId) -> bool {
    ty.is_volatile()
}

/// The facts calling conventions classify a structure or union by, for a
/// signature that passes or returns it by value: its size and alignment,
/// whether a member is misaligned or it ends in a flexible array member,
/// and, when it is small enough to travel in registers, its scalars
/// flattened by offset. The scalars land in `scratch`; interning the
/// signature copies them into the module.
///
/// A member is misaligned when its offset from the start of the whole
/// aggregate is not a multiple of its type's natural alignment, at any level of
/// nesting, which is how Clang applies the System V rule for unaligned
/// fields. A named bit-field contributes one `i8` scalar for each byte its
/// bits reach.
/// Equivalent (type, offset) visits are expanded only once: nested unions
/// may reach the same subobject through exponentially many member paths.
/// Different types at the same offset and the same type at different offsets
/// remain separate because they can contribute different ABI classes.
/// C99: §6.2.5 paragraph 20, pp. 35-36; PDF pp. 47-48 (structures, unions
/// and arrays); §6.7.2.1 paragraphs 12-16, pp. 102-103; PDF pp. 114-115
/// (member offsets, unions and flexible array members).
pub(super) fn aggregate_layout<'s>(
    types: &Types<'_>,
    ty: TypeId,
    rules: RuleSet,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    let whole = types
        .aggregate_value_layout(ty)
        .ok_or(LoweringErrorKind::MissingFact(
            "an aggregate value has no nominal layout",
        ))?;
    flattened_layout(types, ty, whole, rules, scratch)
}

/// The facts an argument of type `ty` is passed by: those of
/// [`aggregate_layout`], except that on System V a GNU transparent union
/// takes its first member's registers, as GCC and Clang pass it (Clang's
/// `X86_64ABIInfo::classifyArgumentType` calls
/// `useFirstFieldIfTransparentUnion`): `union { struct { float x, y; } s;
/// struct { int a, b; } t; }` arrives in `%xmm0` rather than a general
/// register, and a `signed char` first member is sign-extended. The layout
/// keeps the union's size and alignment, since without free registers the
/// whole union goes on the stack (`byval` of the union), and records the
/// first member's size, extension, flags and scalars ([`FirstMember`]). The
/// Microsoft x64 convention classifies the whole union, as Clang's
/// `WinX86_64ABIInfo` does. A result is always the whole union.
/// GNU extension: GCC manual, "Common Type Attributes", `transparent_union`.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
pub(super) fn argument_layout<'s>(
    types: &Types<'_>,
    target: Target,
    ty: TypeId,
    rules: RuleSet,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    let union = object_layout(types, ty, rules, scratch)?;
    let first = types
        .tag(ty)
        .filter(|tag| {
            tag.kind == TagKind::Union
                && tag.transparent.get()
                && matches!(target, Target::LinuxGnu | Target::LinuxMusl)
        })
        .and_then(|tag| tag.members.get().first());
    let Some(first) = first else {
        return Ok(union);
    };
    let member = layout(types, first.ty)?;
    let classified = flattened_layout(types, first.ty, member, rules, scratch)?;
    Ok(AggregateLayout {
        size: union.size,
        align: union.align,
        first: Some(FirstMember {
            size:      member.size,
            extension: repr(types, first.ty)?.extension(),
        }),
        ..classified
    })
}

/// [`aggregate_layout`] of a value of type `ty` whose size and alignment are
/// `whole`.
fn flattened_layout<'s>(
    types: &Types<'_>,
    ty: TypeId,
    whole: Layout,
    rules: RuleSet,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    let mut flags = AggregateFlags::empty();
    if types.tag(ty).is_some_and(|tag| tag.contains_flexible.get()) {
        flags |= AggregateFlags::FLEXIBLE;
    }
    let mut scalars = ArenaVec::new_in(scratch);
    if whole.size <= AggregateLayout::MAX_SCALAR_BYTES {
        // Pending (type, offset) pairs, pushed in reverse so that they pop
        // in member order and the scalars come out by offset.
        let mut work = ArenaVec::new_in(scratch);
        let mut visited = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, scratch);
        work.push((ty, 0_u64));
        while let Some((ty, offset)) = work.pop() {
            // C99 §6.7.2.1p14: union members share their object's address.
            // Check before scheduling children so repeated member paths do
            // not multiply the remaining traversal. Qualifiers do not affect
            // layout or ABI classes.
            if !visited.insert((ty.unqualified(), offset)) {
                continue;
            }
            if types
                .natural_alignment(ty)
                .is_some_and(|align| align > 0 && offset % align != 0)
            {
                flags |= AggregateFlags::UNALIGNED;
            }
            match types.kind(ty) {
                | TypeKind::Unknown =>
                    return Err(super::LoweringError::unanalyzed_kind(types, ty)),
                | TypeKind::Unmodeled(reason) =>
                    return Err(LoweringErrorKind::UnmodeledType(reason)),
                | TypeKind::Scalar(scalar) => scalar_leaves(types, scalar, offset, &mut scalars)?,
                // A bit-precise integer of more than 64 bits is an array of
                // 64-bit limbs to the calling conventions (x86-64 psABI
                // §3.2.3).
                | TypeKind::BitInt { width, .. } =>
                    if width <= 64 {
                        push_scalar(&mut scalars, offset, int_type(width))?;
                    } else {
                        for limb in 0..u64::from(width.div_ceil(64)) {
                            push_scalar(&mut scalars, offset + limb * 8, Type::I64)?;
                        }
                    },
                // An ILP32 pointer object is accessed as `i32` (`emit.rs`).
                | TypeKind::Pointer(_) | TypeKind::Nullptr => push_scalar(
                    &mut scalars,
                    offset,
                    if types.target.pointer.size == 4 {
                        Type::I32
                    } else {
                        Type::Ptr
                    },
                )?,
                | TypeKind::Array(element, bound) => {
                    let count = match bound {
                        | ArrayBound::Constant(count) => count,
                        | ArrayBound::Incomplete => 0,
                        | ArrayBound::Variable(_) | ArrayBound::Star =>
                            return Err(LoweringErrorKind::Unsupported(
                                Construct::VariableLengthArray,
                            )),
                    };
                    let size = layout(types, element)?.size;
                    for index in (0..count).rev() {
                        work.push((element, offset + index * size));
                    }
                },
                | TypeKind::Tag(_) => {
                    let tag = types.tag(ty).expect("a tag type names its tag");
                    if tag.tainted.get() {
                        return Err(super::LoweringError::unanalyzed_kind(types, ty));
                    }
                    if tag.contains_flexible.get() {
                        flags |= AggregateFlags::FLEXIBLE;
                    }
                    if tag.kind == TagKind::Enum {
                        scalar_leaves(types, tag.compatible.get(), offset, &mut scalars)?;
                        continue;
                    }
                    for member in tag.members.get().iter().rev() {
                        if member.invalid {
                            continue;
                        }
                        if let Some(width) = member.width {
                            // A named bit-field's bytes are INTEGER data, as
                            // Clang classifies them; an unnamed one is
                            // padding.
                            if member.name.is_some() && width != 0 {
                                let span = (member.bit_offset + width).div_ceil(8);
                                for byte in 0..u64::from(span) {
                                    push_scalar(
                                        &mut scalars,
                                        offset + member.offset + byte,
                                        Type::I8,
                                    )?;
                                }
                            }
                            continue;
                        }
                        work.push((member.ty, offset + member.offset));
                    }
                },
                | TypeKind::Function { .. } =>
                    return Err(LoweringErrorKind::MissingFact(
                        "an aggregate has a function member",
                    )),
                | TypeKind::Atomic(value) => work.push((value, offset)),
                // A vector member is classified as one unit, as Clang's
                // `X86_64ABIInfo::classify` treats a `VectorType` field.
                | TypeKind::Vector { .. } => match super::vectors::member_leaf(types, ty, rules)? {
                    | Some(leaf) => push_scalar(&mut scalars, offset, leaf)?,
                    | None => flags |= AggregateFlags::MEMORY,
                },
            }
        }
    }
    if types.target.record_passing == RecordPassing::SingleElement
        && single_element(types, ty, scratch)?
    {
        flags |= AggregateFlags::SINGLE_ELEMENT;
    }
    Ok(AggregateLayout {
        size: whole.size,
        align: align(whole),
        flags,
        scalars: scratch.alloc_slice_copy(&scalars),
        first: None,
    })
}

/// Whether `ty` is a structure or union whose one non-empty member, looking
/// through one-element arrays and nested records, is a scalar (a pointer,
/// an enumeration, a real arithmetic type or a bit-field's declared type)
/// with the size of every record on the way, as Clang's
/// `isSingleElementStruct` decides. A member is empty if it is an unnamed
/// bit-field, an array with a zero bound, or a record of empty members;
/// a complex value, a longer array or a flexible array member ends the
/// search.
/// Clang `isSingleElementStruct` and `isEmptyField` in
/// `clang/lib/CodeGen/ABIInfoImpl.cpp`; C99 §6.7.2.1 paragraphs 9-16,
/// pp. 102-103; PDF pp. 114-115.
fn single_element(
    types: &Types<'_>,
    ty: TypeId,
    scratch: &Bump,
) -> Result<bool, LoweringErrorKind> {
    let mut record = ty;
    let mut sizes = ArenaVec::new_in(scratch);
    loop {
        let Some(tag) = types.tag(record).filter(|tag| tag.kind != TagKind::Enum) else {
            return Ok(false);
        };
        if tag.contains_flexible.get() {
            return Ok(false);
        }
        sizes.push(layout(types, record)?.size);
        let mut found = None;
        for member in tag.members.get() {
            if member.invalid || (member.width.is_some() && member.name.is_none()) {
                continue;
            }
            let mut element = types.non_atomic(member.ty);
            if empty_member(types, element, scratch) {
                continue;
            }
            if found.is_some() {
                return Ok(false);
            }
            while let TypeKind::Array(inner, ArrayBound::Constant(1)) = types.kind(element) {
                element = types.non_atomic(inner);
            }
            found = Some(element);
        }
        let Some(element) = found else {
            return Ok(false);
        };
        match types.kind(element) {
            | TypeKind::Tag(_)
                if types
                    .tag(element)
                    .is_some_and(|tag| tag.kind != TagKind::Enum) =>
                record = element,
            | TypeKind::Scalar(scalar) if scalar.complex() => return Ok(false),
            | TypeKind::Scalar(_) | TypeKind::Pointer(_) | TypeKind::Nullptr | TypeKind::Tag(_) => {
                let size = layout(types, element)?.size;
                return Ok(sizes.iter().all(|&record| record == size));
            },
            | _ => return Ok(false),
        }
    }
}

/// Whether a member of type `ty` is empty for Clang's single-element rule:
/// an array with a zero bound at any level, or a record whose members are
/// all empty.
fn empty_member(types: &Types<'_>, ty: TypeId, scratch: &Bump) -> bool {
    let mut pending = ArenaVec::new_in(scratch);
    pending.push(ty);
    'members: while let Some(mut ty) = pending.pop() {
        while let TypeKind::Array(element, bound) = types.kind(ty) {
            if matches!(bound, ArrayBound::Constant(0)) {
                // This member is empty; the others still decide.
                continue 'members;
            }
            ty = types.non_atomic(element);
        }
        let Some(tag) = types.tag(ty).filter(|tag| tag.kind != TagKind::Enum) else {
            return false;
        };
        for member in tag.members.get() {
            if member.invalid || (member.width.is_some() && member.name.is_none()) {
                continue;
            }
            pending.push(types.non_atomic(member.ty));
        }
    }
    true
}

/// A complex scalar's two components, retaining its scalar ABI identity.
/// C99: §6.2.5p13, p. 34; PDF p. 46.
pub(super) fn complex_layout<'s>(
    types: &Types<'_>,
    ty: TypeId,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    let TypeKind::Scalar(scalar) = types.kind(types.non_atomic(ty)) else {
        return Err(LoweringErrorKind::MissingFact(
            "a complex type has no scalar",
        ));
    };
    complex_scalar_layout(types, scalar, scratch)
}

/// The ABI layout of the complex scalar type `scalar`.
pub(super) fn complex_scalar_layout<'s>(
    types: &Types<'_>,
    scalar: Scalar,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    // Calling conventions classify the nominal scalar, not a typedef's storage
    // override.
    let whole = types
        .target
        .scalar(scalar)
        .ok_or(LoweringErrorKind::MissingFact(
            "a complex scalar has no target layout",
        ))?;
    let mut scalars = ArenaVec::new_in(scratch);
    scalar_leaves(types, scalar, 0, &mut scalars)?;
    Ok(AggregateLayout {
        size:    whole.size,
        align:   align(whole),
        flags:   AggregateFlags::COMPLEX,
        scalars: scratch.alloc_slice_copy(&scalars),
        first:   None,
    })
}

/// The ABI object layout, preserving nominal aggregate layout and complex
/// identity.
pub(super) fn object_layout<'s>(
    types: &Types<'_>,
    ty: TypeId,
    rules: RuleSet,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    if matches!(repr(types, ty)?, Repr::Complex { .. }) {
        complex_layout(types, ty, scratch)
    } else if matches!(types.kind(types.non_atomic(ty)), TypeKind::Vector { .. })
        || matches!(types.kind(ty), TypeKind::BitInt { .. })
    {
        flattened_layout(types, ty, layout(types, ty)?, rules, scratch)
    } else {
        aggregate_layout(types, ty, rules, scratch)
    }
}

/// The scalars of a C scalar type at `offset`: one for a real type and two
/// for a complex one. A record containing a complex member retains these
/// leaves without acquiring top-level complex scalar ABI identity.
/// C99: §6.2.5 paragraphs 10-13, pp. 34-35; PDF pp. 46-47.
fn scalar_leaves(
    types: &Types<'_>,
    scalar: Scalar,
    offset: u64,
    scalars: &mut ArenaVec<'_, AggregateScalar>,
) -> Result<(), LoweringErrorKind> {
    let long_double = || long_double_type(types);
    if scalar.complex_integer() {
        let (bits, _) =
            types
                .target
                .integer(scalar.real_type())
                .ok_or(LoweringErrorKind::MissingFact(
                    "an integer type has no width",
                ))?;
        let ty = int_type(bits);
        push_scalar(scalars, offset, ty)?;
        return push_scalar(scalars, offset + u64::from(ty.bytes()), ty);
    }
    let (ty, parts) = match scalar.representation() {
        | Scalar::Void =>
            return Err(LoweringErrorKind::MissingFact(
                "an aggregate has a void member",
            )),
        | Scalar::Bool => (Type::I8, 1),
        | Scalar::Float16 | Scalar::Fp16 => (Type::F16, 1),
        | Scalar::BFloat16 => (Type::BF16, 1),
        | Scalar::ComplexFloat16 => (Type::F16, 2),
        | Scalar::Float => (Type::F32, 1),
        | Scalar::Double => (Type::F64, 1),
        | Scalar::LongDouble => (long_double(), 1),
        | Scalar::Float128 => (Type::F128, 1),
        | Scalar::ComplexFloat => (Type::F32, 2),
        | Scalar::ComplexDouble => (Type::F64, 2),
        | Scalar::ComplexLongDouble => (long_double(), 2),
        | Scalar::ComplexFloat128 => (Type::F128, 2),
        | _ => {
            let (bits, _) = types
                .target
                .integer(scalar)
                .ok_or(LoweringErrorKind::MissingFact(
                    "an integer type has no width",
                ))?;
            (int_type(bits), 1)
        },
    };
    let stride = types
        .target
        .scalar(scalar)
        .map_or(u64::from(ty.bytes()), |layout| layout.size / parts);
    for part in 0..parts {
        push_scalar(scalars, offset + part * stride, ty)?;
    }
    Ok(())
}

fn push_scalar(
    scalars: &mut ArenaVec<'_, AggregateScalar>,
    offset: u64,
    ty: Type,
) -> Result<(), LoweringErrorKind> {
    let offset = u32::try_from(offset).map_err(|_| {
        LoweringErrorKind::MissingFact("a small aggregate has a member beyond 4 GiB")
    })?;
    scalars.push(AggregateScalar { offset, ty });
    Ok(())
}

/// The pointed-to or element type of a pointer or array type.
pub(super) fn target_type(types: &Types<'_>, ty: TypeId) -> Option<TypeId> {
    match types.kind(ty) {
        | TypeKind::Pointer(target) | TypeKind::Array(target, _) => Some(target),
        | _ => None,
    }
}

/// The size pointer arithmetic steps by for an element type: its layout's
/// size, or 1 for void/function targets, which semantic analysis accepts only
/// as the GNU extension. GNU extension: GCC "Pointer Arith".
pub(super) fn arithmetic_size(types: &Types<'_>, element: TypeId) -> Option<u64> {
    if matches!(
        types.kind(element),
        TypeKind::Scalar(Scalar::Void) | TypeKind::Function { .. }
    ) {
        return Some(1);
    }
    types.layout(element).map(|layout| layout.size)
}

/// The LLVM data layout of a target: little-endian x86-64 with ELF or COFF
/// symbol mangling, 64-bit `long long`, 16-byte `__int128` and `long
/// double` storage, and a 16-byte aligned stack. The IR records it so the
/// LLVM back end can emit it unchanged.
pub(super) fn data_layout(target: Target) -> &'static str {
    match target {
        | Target::LinuxGnu | Target::LinuxMusl =>
            "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
        | Target::WindowsGnu | Target::WindowsMsvc =>
            "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
        | Target::Wasm32Unknown | Target::Wasm32Wasi =>
            "e-m:e-p:32:32-p10:8:8-p20:8:8-i64:64-i128:128-n32:64-S128-ni:1:10:20",
    }
}
