#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StackStack<T, const N: usize> {
    data:  [T; N],
    len:   usize,
}

impl<T, const N: usize> Default for StackStack<T, N> where T: Copy + Default{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> StackStack where T: Copy + Default {
    pub(crate) fn new() -> Self {
        Self {
            data:  [Default::default(); N],
            len:   0,
        }
    }
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(crate) fn push(&mut self, value: T) {
        self.data[self.len] = value;
        self.len += 1;
    }
    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            Some(self.data[self.len])
        }
    }
}