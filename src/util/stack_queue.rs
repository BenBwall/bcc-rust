#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StackQueue<T, const N: usize>
where
    T: Copy + Default,
{
    data:  [T; N],
    len:   usize,
    start: usize,
}

impl<T, const N: usize> Default for StackQueue<T, N>
where
    T: Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> StackQueue<T, N>
where
    T: Copy + Default,
{
    pub(crate) fn new() -> Self {
        Self {
            data:  [Default::default(); N],
            len:   0,
            start: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.start = 0;
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn push_back(&mut self, value: T) {
        self.data[self.start + self.len] = value;
        self.len += 1;
    }

    pub(crate) fn pop_back(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            Some(self.data[self.len])
        }
    }

    pub(crate) fn push_front(&mut self, value: T) {
        self.start = self.start.wrapping_sub(1);
        self.len += 1;
        self.data[self.start] = value;
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            let old_start = self.start;
            self.start = self.start.wrapping_add(1);
            self.len -= 1;
            let value = self.data[old_start];
            Some(value)
        }
    }
}
