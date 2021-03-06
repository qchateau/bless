use std::{cmp::max, fmt};

pub struct DeVec<T> {
    data: Vec<T>,
    offset: usize,
}

impl<T> DeVec<T> {
    pub fn new() -> DeVec<T> {
        DeVec {
            data: Vec::new(),
            offset: 0,
        }
    }
    pub fn len(&self) -> usize {
        return self.data.len() - self.offset;
    }
    pub fn as_slice(&self) -> &[T] {
        return &self.data.as_slice()[self.offset..];
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        return &mut self.data.as_mut_slice()[self.offset..];
    }
    pub fn clear(&mut self) {
        self.data.truncate(self.offset)
    }
    pub fn shrink_to(&mut self, size: usize) {
        let size = max(size, self.len());
        let max_offset = size / 2;
        if self.offset > max_offset {
            let shift = self.offset - max_offset;
            self.data.rotate_left(shift);
            self.data.truncate(self.data.len() - shift);
            self.offset = max_offset;
        }
        self.data.shrink_to(size);
    }
}

impl<T: Default + Clone> DeVec<T> {
    pub fn resize_back(&mut self, size: usize) {
        self.data.resize(size + self.offset, T::default());
    }
    pub fn resize_front(&mut self, size: usize) {
        let len = self.len();
        if size > len {
            let extra = size - len;
            let missing = extra.saturating_sub(self.offset);
            self.offset = self.offset.saturating_sub(extra);
            if missing > 0 {
                let new_size =
                    2usize.pow(((self.data.len() + missing) as f64).log2().ceil() as u32);
                let extra_alloc = new_size - (self.data.len() + missing);

                let mut new_vec = Vec::new();
                new_vec.reserve(new_size);
                new_vec.resize(extra_alloc + missing, T::default());
                new_vec.append(&mut self.data);
                self.offset = extra_alloc;
                self.data = new_vec;
            }
        } else {
            self.offset += len - size;
        }
    }
}

impl<T> fmt::Debug for DeVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("DeVec")
            .field("data.len", &self.data.len())
            .field("offset", &self.offset)
            .finish()
    }
}
