//! Retained initializer placement plans. The current-object checker in
//! `initializers.rs` emits one plan for each initializer it accepts, keyed by
//! the initializer's syntax identity, so that automatic and static lowering
//! store into the subobjects semantic analysis chose instead of walking the
//! braces again.
//!
//! A plan is a source-ordered list of operations. Offsets are absolute byte
//! offsets from the start of the initialized object; types are the completed,
//! qualified destination types. A brace pair opens a [`InitOperation::Zero`]
//! scope for the bytes its elements omit; a union member chosen by an element
//! is a [`InitOperation::Select`]; an expression initializes a scalar
//! ([`InitOperation::Store`]) or a whole structure or union
//! ([`InitOperation::Copy`]) from an [`InitValue`] that belongs to this
//! initializer occurrence; a string literal fills a character array
//! ([`InitOperation::String`]). A GNU range designator is one
//! [`InitOperation::Repeat`] over the operations of its element, stored at the
//! range's first element: the plan never expands a range, so its size follows
//! the written initializer, not the object's extent, and a value stored by a
//! range is evaluated once.
//!
//! For `struct { int x, y; } a[4] = { [1 ... 2] = { 5 }, [3].y = n }` the
//! plan is `Zero(a, end 5)`, `Repeat(2 × 8 bytes, 2 operations)`,
//! `Zero(a[1], end 4)`, `Store(a[1].x, value 0)`, `Store(a[3].y, value 1)`.
//!
//! C99: §6.7.8 paragraphs 9-22, pp. 126-127; PDF pp. 138-139.
//! GNU extension: GCC manual, "Designated Initializers" (ranges).
//! <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "M8 switches lowering to retained plans in its next step"
    )
)]

use rustc_hash::FxBuildHasher;

use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::Initializer,
            syntax::Expression,
        },
        semantic_analysis::TypeId,
    },
    util::bump::{
        ArenaMap,
        Bump,
    },
};

/// Every retained plan, by the address of the initializer it places.
#[derive(Debug)]
pub(crate) struct InitializerFacts<'tu> {
    pub(in crate::translation_phases::semantic_analysis) plans:
        ArenaMap<'tu, usize, InitializerPlan<'tu>>,
}

impl<'tu> InitializerFacts<'tu> {
    pub(in crate::translation_phases::semantic_analysis) fn new(arena: &'tu Bump) -> Self {
        Self {
            plans: ArenaMap::with_hasher_in(FxBuildHasher, arena),
        }
    }

    /// The plan semantic analysis accepted for `initializer`, an object
    /// definition's, an automatic declaration's or a compound literal's.
    /// `None` when the initializer was rejected or is unmodeled.
    pub(crate) fn plan(&self, initializer: &Initializer<'tu>) -> Option<&InitializerPlan<'tu>> {
        self.plans.get(&std::ptr::from_ref(initializer).addr())
    }

    /// The number of retained plans and their operations, for measurement.
    #[cfg(test)]
    pub(crate) fn totals(&self) -> (usize, usize) {
        (
            self.plans.len(),
            self.plans.values().map(|plan| plan.operations.len()).sum(),
        )
    }
}

/// The placement of one accepted initializer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InitializerPlan<'tu> {
    /// The completed type of the initialized object: an array of unknown
    /// size has the extent its initializer gives it (§6.7.8p22).
    pub(crate) ty:         TypeId,
    pub(crate) operations: &'tu [InitOperation<'tu>],
    /// Indexed by [`InitValueId`].
    pub(crate) values:     &'tu [InitValue<'tu>],
}

/// One placement step, in source order.
/// C99: §6.7.8 paragraphs 17-21, pp. 126-127; PDF pp. 138-139.
#[derive(Clone, Copy, Debug)]
pub(crate) enum InitOperation<'tu> {
    /// A brace-enclosed list initializes `destination`, an aggregate: its
    /// bytes that no operation before `end` (an exclusive operation index)
    /// stores are zero (§6.7.8p21), and it replaces whatever earlier
    /// operations stored there (§6.7.8p19). `source` is the list's element.
    Zero {
        destination: Subobject,
        end:         u32,
        source:      SourceVectors,
    },
    /// An element initializes member `member` of the union `destination`
    /// (§6.7.8p17). Selecting a member other than the one an earlier
    /// operation selected replaces that member's bytes; `scope` indexes the
    /// [`InitOperation::Zero`] of the list containing the element.
    Select {
        destination: Subobject,
        member:      u32,
        scope:       u32,
        source:      SourceVectors,
    },
    /// A scalar subobject receives `value` converted to its type, as by
    /// assignment (§6.7.8p11). `source` is the expression, or the outer
    /// brace pair of a braced scalar.
    Store {
        value:       InitValueId,
        destination: Subobject,
        bit_field:   Option<BitRange>,
        source:      SourceVectors,
    },
    /// A structure or union receives `value`, an expression of a compatible
    /// type (§6.7.8p13). Union selections inside it are forgotten; those of
    /// the enclosing unions `enclosing_unions` are not.
    Copy {
        value:            InitValueId,
        destination:      Subobject,
        enclosing_unions: &'tu [TypeId],
    },
    /// A character or wide array receives the elements of a string literal,
    /// and zeroes after them (§6.7.8p14-15, p21).
    String {
        literal:     &'tu Expression<'tu>,
        destination: Subobject,
    },
    /// The next `length` operations initialize the first of `count`
    /// subobjects `stride` bytes apart, and are repeated for each in order;
    /// a value they store is evaluated once (GNU range designator).
    Repeat {
        count:  u64,
        stride: u64,
        length: u32,
    },
    /// An empty brace pair initializes the scalar `destination`: a C23
    /// empty initializer, which C99 §6.7.8p1 does not allow.
    EmptyScalar {
        destination: Subobject,
        source:      SourceVectors,
    },
    /// An element initializes inside `destination`, whose layout the plan
    /// does not describe: a vector's lanes, or a type analysis could not
    /// model. Operations inside it are still listed.
    Unmodeled {
        destination: Subobject,
        source:      SourceVectors,
    },
}

/// A subobject at a byte offset from the start of the initialized object,
/// with its type qualified by its containers' qualifiers (§6.7.3p8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Subobject {
    pub(crate) offset: u64,
    pub(crate) ty:     TypeId,
}

/// The bits of a bit-field member, from its subobject's byte offset.
/// C99: §6.7.2.1 paragraph 10, p. 102; PDF p. 114.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BitRange {
    pub(crate) bit_offset: u32,
    pub(crate) width:      u32,
}

/// An expression of one initializer occurrence whose value an operation
/// stores. C11 §6.7.9p23 leaves the order of these evaluations unspecified;
/// lowering evaluates each at its first store, in plan order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InitValue<'tu> {
    pub(crate) expression: &'tu Expression<'tu>,
    /// A range stores the value into more than one subobject, so lowering
    /// keeps the converted value instead of evaluating it again.
    pub(crate) repeated:   bool,
}

/// Index of an [`InitValue`] in its plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InitValueId(pub(crate) u32);
