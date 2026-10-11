//! Runtime values: what an SSA value holds while the interpreter runs.
//!
//! The static type of every value is known from the IR, so a runtime value
//! carries only its contents. Integers of every width are `u128` bits masked
//! to the width; pointers carry an address and the object they may access.

use std::num::NonZeroU32;

use crate::ir::Type;

/// The contents of one SSA value.
///
/// An `Int` of type `iN` always has its bits above `N` clear. `f80` and
/// `f128` values are not supported yet; the interpreter traps on them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RuntimeValue {
    Int(u128),
    F32(f32),
    F64(f64),
    Ptr(Pointer),
    /// The result of an operation whose `nsw`, `nuw`, `exact` or
    /// `inbounds` fact was violated, of a shift by the width or more, of a
    /// load of bytes never written, or of a `poison` constant.
    Poison,
}

impl RuntimeValue {
    /// The null pointer.
    pub(crate) const NULL: Self = Self::Ptr(Pointer::NULL);

    /// The integer `bits` of type `ty`, masked to its width.
    pub(crate) const fn int(ty: Type, bits: u128) -> Self {
        Self::Int(bits & ty.mask())
    }

    /// The value `freeze` gives poison of type `ty`: zero, the deterministic
    /// choice among the arbitrary values the IR allows.
    pub(crate) const fn zero(ty: Type) -> Self {
        match ty {
            | Type::F32 => Self::F32(0.0),
            | Type::F64 => Self::F64(0.0),
            | Type::Ptr => Self::NULL,
            | _ => Self::Int(0),
        }
    }

    pub(crate) const fn is_poison(self) -> bool {
        matches!(self, Self::Poison)
    }

    /// The integer bits, if this is an integer.
    pub(crate) const fn as_int(self) -> Option<u128> {
        match self {
            | Self::Int(bits) => Some(bits),
            | _ => None,
        }
    }

    /// The pointer, if this is one.
    pub(crate) const fn as_pointer(self) -> Option<Pointer> {
        match self {
            | Self::Ptr(pointer) => Some(pointer),
            | _ => None,
        }
    }
}

/// A pointer: an address and, unless it was made from an integer that named
/// no object, the provenance of the object it may access.
///
/// Addresses are deterministic: every object gets a fresh base address when
/// it is allocated, and `ptrtoint` returns the address. Two pointers compare
/// by address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Pointer {
    pub(crate) address:    u64,
    pub(crate) provenance: Option<ObjectRef>,
}

impl Pointer {
    /// Address zero, which names no object.
    pub(crate) const NULL: Self = Self {
        address:    0,
        provenance: None,
    };
}

/// One allocation of a memory object. An object's record is reused after it
/// is freed, with its generation advanced, so a pointer into the old
/// allocation no longer matches.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ObjectRef {
    /// The object's index plus one.
    pub(super) index:      NonZeroU32,
    pub(super) generation: u32,
}

impl ObjectRef {
    pub(super) const fn new(index: usize, generation: u32) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The object table is far smaller than u32::MAX entries."
        )]
        let raw = index as u32 + 1;
        Self {
            index: NonZeroU32::new(raw).expect("an index plus one is nonzero"),
            generation,
        }
    }

    pub(super) const fn index(self) -> usize {
        (self.index.get() - 1) as usize
    }
}
