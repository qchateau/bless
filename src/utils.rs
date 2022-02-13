use std::fmt::Display;

pub struct InfiniteLoopBreaker<T> {
    count: u64,
    current_count: u64,
    error: T,
}

impl<T: Clone + Display> InfiniteLoopBreaker<T> {
    pub fn new(count: u64, error: T) -> Self {
        return Self {
            count,
            current_count: count,
            error,
        };
    }
    pub fn reset(&mut self) {
        self.current_count = self.count;
    }
    pub fn it(&mut self) -> Result<(), T> {
        self.current_count -= 1;
        if self.current_count == 0 {
            eprintln!("loop break: {}", self.error);
            return Err(self.error.clone());
        }
        return Ok(());
    }
}

pub fn nth_or_last<I: Iterator>(mut iter: I, nth: usize) -> Option<(I::Item, usize)> {
    let mut cnt = 0;
    let mut res = None;
    while cnt <= nth {
        match iter.next() {
            Some(item) => {
                res = Some(item);
                cnt += 1;
            }
            None => break,
        }
    }

    return match res {
        Some(item) => Some((item, cnt - 1)),
        None => None,
    };
}
