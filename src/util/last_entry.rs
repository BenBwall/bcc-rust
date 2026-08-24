use std::{
    borrow::{
        Borrow,
        BorrowMut,
    },
    ops::{
        Deref,
        DerefMut,
    },
};

pub(crate) fn last_entry<T>(vector: &mut Vec<T>) -> Option<LastEntry<'_, T>> {
    let last_index = vector.len().checked_sub(1)?;

    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "The unsafe block is small and the operations are related."
    )]
    // SAFETY: `last_index` identifies `vector`'s final initialized element. The
    // entry owns the exclusive borrow of `vector` and exposes no operation that
    // can reallocate it or move that element while this reference exists.
    let last = unsafe { &mut *vector.as_mut_ptr().add(last_index) };
    Some(LastEntry { vector, last })
}

pub(crate) struct LastEntry<'vector, T> {
    vector: &'vector mut Vec<T>,
    last:   &'vector mut T,
}

impl<'vector, T> LastEntry<'vector, T> {
    pub(crate) fn get(&self) -> &T {
        self.last
    }

    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.last
    }

    pub(crate) fn insert(&mut self, value: T) -> T {
        std::mem::replace(self.last, value)
    }

    // Safety invariants:
    // - `last` uniquely references the final initialized element of `vector`.
    // - `vector` is non-empty, so `Vec::len` is guaranteed to return a non-zero
    //   value.
    // - This function must not panic.

    pub(crate) fn remove(self) -> T {
        let Self { vector, last } = self;
        // SAFETY: `last` uniquely references the final initialized element. Move
        // the value through that reference's provenance, then shorten the vector
        // so it no longer treats the moved-out slot as initialized.
        let value = unsafe { std::ptr::read(last) };
        // SAFETY: We check that the vector is non-empty when constructing the entry,
        // and no entry operation can change its length, so `Vec::len` is guaranteed to
        // return a non-zero value here.
        let new_length = unsafe { vector.len().unchecked_sub(1) };
        // SAFETY: construction proves the vector is non-empty, and no entry
        // operation can change its length, so `new_length` removes exactly the
        // moved-out final slot.
        unsafe {
            vector.set_len(new_length);
        }
        value
    }

    pub(crate) fn into_mut(self) -> &'vector mut T {
        let Self { vector: _, last } = self;
        last
    }
}

impl<T> Borrow<T> for LastEntry<'_, T> {
    fn borrow(&self) -> &T {
        self.get()
    }
}

impl<T> BorrowMut<T> for LastEntry<'_, T> {
    fn borrow_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> AsRef<T> for LastEntry<'_, T> {
    fn as_ref(&self) -> &T {
        self.get()
    }
}

impl<T> AsMut<T> for LastEntry<'_, T> {
    fn as_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> Deref for LastEntry<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for LastEntry<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::last_entry;

    #[test]
    fn empty_vector_has_no_last_entry() {
        let mut values: Vec<i32> = Vec::new();

        assert!(last_entry(&mut values).is_none());
        assert!(values.is_empty());
    }

    #[test]
    fn get_inspects_only_the_final_element() {
        let mut values = vec![10, 20, 30];

        assert_eq!(last_entry(&mut values).unwrap().get(), &30);
        assert_eq!(values, [10, 20, 30]);
    }

    #[test]
    fn get_mut_mutates_only_the_final_element() {
        let mut values = vec![10, 20, 30];

        *last_entry(&mut values).unwrap().get_mut() = 31;

        assert_eq!(values, [10, 20, 31]);
    }

    #[test]
    fn into_mut_mutates_only_the_final_element() {
        let mut values = vec![10, 20, 30];

        *last_entry(&mut values).unwrap().into_mut() = 31;

        assert_eq!(values, [10, 20, 31]);
    }

    #[test]
    fn insert_replaces_only_the_final_element_and_returns_the_old_value() {
        let mut values = vec![10, 20, 30];

        let old = last_entry(&mut values).unwrap().insert(31);

        assert_eq!(old, 30);
        assert_eq!(values, [10, 20, 31]);
    }

    #[test]
    fn final_element_address_is_stable_for_the_entry_lifetime() {
        let mut values = vec![10, 20, 30];
        let original_address = std::ptr::from_ref(values.last().unwrap());
        let mut entry = last_entry(&mut values).unwrap();

        assert_eq!(std::ptr::from_ref(entry.get()), original_address);
        assert_eq!(
            std::ptr::from_mut(entry.get_mut()),
            original_address.cast_mut()
        );
        assert_eq!(entry.insert(31), 30);
        assert_eq!(std::ptr::from_ref(entry.get()), original_address);
    }

    #[test]
    fn remove_removes_only_the_final_element() {
        let mut values = vec![10, 20, 30];

        let removed = last_entry(&mut values).unwrap().remove();

        assert_eq!(removed, 30);
        assert_eq!(values, [10, 20]);
    }
}
