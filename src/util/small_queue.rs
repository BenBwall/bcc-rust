use std::collections::VecDeque;

use super::stack_queue::StackQueue;

#[expect(
    dead_code,
    reason = "This is currently unused, but I prove useful in the future."
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SmallQueue<T, const N: usize>
where
    T: Copy + Default,
{
    StackQueue(StackQueue<T, N>),
    HeapQueue(VecDeque<T>),
}

impl<T, const N: usize> Default for SmallQueue<T, N>
where
    T: Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> SmallQueue<T, N>
where
    T: Copy + Default,
{
    pub(crate) fn new() -> Self {
        Self::StackQueue(StackQueue::new())
    }

    pub(crate) fn clear(&mut self) {
        match self {
            | Self::StackQueue(queue) => queue.clear(),
            | Self::HeapQueue(queue) => queue.clear(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            | Self::StackQueue(queue) => queue.len(),
            | Self::HeapQueue(queue) => queue.len(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            | Self::StackQueue(queue) => queue.is_empty(),
            | Self::HeapQueue(queue) => queue.is_empty(),
        }
    }

    pub(crate) fn push_back(&mut self, value: T) {
        match self {
            | Self::StackQueue(queue) => queue.push_back(value),
            | Self::HeapQueue(queue) => queue.push_back(value),
        }
    }

    pub(crate) fn pop_back(&mut self) -> Option<T> {
        match self {
            | Self::StackQueue(queue) => queue.pop_back(),
            | Self::HeapQueue(queue) => queue.pop_back(),
        }
    }

    pub(crate) fn push_front(&mut self, value: T) {
        match self {
            | Self::StackQueue(queue) => queue.push_front(value),
            | Self::HeapQueue(queue) => queue.push_front(value),
        }
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        match self {
            | Self::StackQueue(queue) => queue.pop_front(),
            | Self::HeapQueue(queue) => queue.pop_front(),
        }
    }
}
