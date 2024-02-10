
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StackVec<T, const N: usize> where T: Copy + Default {
    data: [T; N],
    len: usize,
}

impl<T, const N: usize> StackVec<T, N> where T: Copy + Default {
    pub(crate) fn new() -> Self {
        Self {
            data: [Default::default(); N],
            len: 0,
        }
    }
}

impl<T, const N: usize> StackVec<T, N> where T: Copy + Default {
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

