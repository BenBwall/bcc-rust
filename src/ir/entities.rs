//! Entity references and the dense tables that own them.
//!
//! Every IR object is named by a `u32` index into a table owned by its
//! function or module. Indices are dense and never reused, so side tables in
//! analyses are plain arrays keyed by the same index.

use std::{
    fmt,
    marker::PhantomData,
    num::NonZeroU32,
    ops::{
        Index,
        IndexMut,
    },
};

use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A dense index that names one entry of a [`Table`].
pub(crate) trait Entity: Copy + Eq {
    /// The entity at `index`. Panics if `index` does not fit in a `u32`.
    fn new(index: usize) -> Self;

    /// The entity's position in its table.
    fn index(self) -> usize;
}

macro_rules! entities {
    ($($(#[$meta:meta])* $name:ident => $prefix:literal;)+) => {$(
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub(crate) struct $name(u32);

        impl Entity for $name {
            fn new(index: usize) -> Self {
                Self(u32::try_from(index).expect(concat!("too many ", $prefix, " entities")))
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl $name {
            /// The raw index, as the textual form spells it.
            pub(crate) const fn as_u32(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }
    )+};
}

entities! {
    /// An SSA value: an instruction result or a block parameter.
    Value => "v";
    /// An instruction of one function.
    Inst => "inst";
    /// A basic block. Block 0 is the entry block; its parameters are the
    /// function's parameters.
    Block => "block";
    /// A fixed-size, fixed-alignment region of a function's frame.
    StackSlot => "slot";
    /// A wide constant (`i128`, `f80` or `f128` bits) in a function's pool.
    ConstId => "const";
    /// A function of a module, defined or declared.
    FuncId => "func";
    /// A global object of a module, defined or declared.
    GlobalId => "global";
    /// An interned call signature of a module.
    SigId => "sig";
}

/// A strict-aliasing access tag: an index into a per-module table of type
/// descriptors. The prototype emits none; the IR only carries them.
///
/// It stores the index plus one, so `Option<AccessTag>` stays four bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AccessTag(NonZeroU32);

impl AccessTag {
    /// The tag at `index`, or `None` for `u32::MAX`, which has no room for the
    /// stored offset.
    pub(crate) fn new(index: u32) -> Option<Self> {
        index.checked_add(1).and_then(NonZeroU32::new).map(Self)
    }

    pub(crate) const fn index(self) -> u32 {
        self.0.get() - 1
    }
}

/// A dense table of `V` keyed by the entity `K`, allocated in an arena.
pub(crate) struct Table<'a, K, V> {
    items: ArenaVec<'a, V>,
    keys:  PhantomData<K>,
}

impl<'a, K: Entity, V> Table<'a, K, V> {
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            items: ArenaVec::new_in(arena),
            keys:  PhantomData,
        }
    }

    /// Appends `value` and returns its key.
    pub(crate) fn push(&mut self, value: V) -> K {
        let key = K::new(self.items.len());
        self.items.push(value);
        key
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn get(&self, key: K) -> Option<&V> {
        self.items.get(key.index())
    }

    /// Every key, in index order.
    pub(crate) fn keys(&self) -> impl DoubleEndedIterator<Item = K> + use<K, V> {
        (0..self.items.len()).map(K::new)
    }

    /// Every key with its value, in index order.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = (K, &V)> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, value)| (K::new(index), value))
    }
}

impl<K: Entity, V> Index<K> for Table<'_, K, V> {
    type Output = V;

    fn index(&self, index: K) -> &V {
        &self.items[index.index()]
    }
}

impl<K: Entity, V> IndexMut<K> for Table<'_, K, V> {
    fn index_mut(&mut self, index: K) -> &mut V {
        &mut self.items[index.index()]
    }
}

impl<K, V: fmt::Debug> fmt::Debug for Table<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items.iter()).finish()
    }
}
