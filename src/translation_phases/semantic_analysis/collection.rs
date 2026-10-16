//! Scratch cons lists accumulate parameter, member and scope-entry sequences.
//! The completed list is copied once into its destination arena. This file owns
//! storage, not the semantic constraints on those elements.
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22 (translation phase 7).

use std::cell::Cell;

use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A scratch cons list builds nested parameter/member lists in linear space.
pub(super) struct Collection<'s, T: Copy> {
    pub(super) head: Cell<Option<&'s Link<'s, T>>>,
}

impl<'s, T: Copy> Collection<'s, T> {
    pub(super) fn push(&self, scratch: &'s Bump, value: T) {
        self.head.set(Some(scratch.alloc(Link {
            value,
            next: self.head.get(),
        })));
    }

    pub(super) fn finish<'tu>(&self, tu: &'tu Bump, scratch: &Bump) -> &'tu [T] {
        let mut items = ArenaVec::new_in(scratch);
        let mut next = self.head.get();
        while let Some(link) = next {
            items.push(link.value);
            next = link.next;
        }
        items.reverse();
        tu.alloc_slice_copy(&items)
    }
}

pub(super) struct Link<'s, T: Copy> {
    pub(super) value: T,
    pub(super) next:  Option<&'s Link<'s, T>>,
}

impl<T: Copy> Collection<'_, T> {
    pub(super) fn new() -> Self {
        Self {
            head: Cell::new(None),
        }
    }
}
