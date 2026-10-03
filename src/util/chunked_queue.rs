//! A first-in, first-out queue stored in fixed-size chunks.
//!
//! Unlike a `Vec`, the queue grows without reallocating and copying what it
//! already holds, and it frees each chunk as soon as every value in it has
//! been taken, so a large buffer that is filled once and then drained shrinks
//! while it is read.

use std::{
    collections::VecDeque,
    vec::IntoIter,
};

/// Bytes per chunk; a chunk holds at least one value.
const CHUNK_BYTES: usize = 64 << 10;

#[derive(Debug)]
pub(crate) struct ChunkedQueue<T> {
    /// The chunk being drained, holding the oldest values.
    front:  IntoIter<T>,
    /// Newer chunks, oldest first; only the last one may have room.
    chunks: VecDeque<Vec<T>>,
    len:    usize,
}

impl<T> Default for ChunkedQueue<T> {
    fn default() -> Self {
        Self {
            front:  Vec::new().into_iter(),
            chunks: VecDeque::new(),
            len:    0,
        }
    }
}

impl<T> ChunkedQueue<T> {
    const CHUNK_LEN: usize = if size_of::<T>() == 0 {
        usize::MAX
    } else {
        let len = CHUNK_BYTES / size_of::<T>();
        if len == 0 { 1 } else { len }
    };

    pub(crate) fn push_back(&mut self, value: T) {
        match self.chunks.back_mut() {
            | Some(chunk) if chunk.len() < Self::CHUNK_LEN => chunk.push(value),
            | _ => {
                let mut chunk = Vec::with_capacity(Self::CHUNK_LEN.min(CHUNK_BYTES));
                chunk.push(value);
                self.chunks.push_back(chunk);
            },
        }
        self.len += 1;
    }
}

impl<T> Iterator for ChunkedQueue<T> {
    type Item = T;

    /// Takes the oldest value, freeing its chunk once the chunk is empty.
    fn next(&mut self) -> Option<T> {
        let value = match self.front.next() {
            | Some(value) => value,
            | None => {
                self.front = self.chunks.pop_front()?.into_iter();
                self.front.next()?
            },
        };
        self.len -= 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<T> ExactSizeIterator for ChunkedQueue<T> {}

#[cfg(test)]
mod tests {
    use super::ChunkedQueue;

    #[test]
    fn values_leave_in_insertion_order_across_chunks() {
        let mut queue = ChunkedQueue::default();
        let count = ChunkedQueue::<u64>::CHUNK_LEN * 3 + 7;
        for value in 0..count as u64 {
            queue.push_back(value);
        }
        assert_eq!(queue.len(), count);
        assert!(queue.by_ref().take(10).eq(0..10));
        queue.push_back(u64::MAX);
        assert_eq!(queue.len(), count - 10 + 1);
        assert!(queue.eq((10..count as u64).chain([u64::MAX])));
    }

    #[test]
    fn an_empty_queue_yields_nothing() {
        let mut queue = ChunkedQueue::<String>::default();
        assert_eq!(queue.next(), None);
        queue.push_back("only".to_owned());
        assert_eq!(queue.next().as_deref(), Some("only"));
        assert_eq!(queue.next(), None);
        assert_eq!(queue.len(), 0);
    }
}
