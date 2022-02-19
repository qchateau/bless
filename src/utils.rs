use std::{
    borrow::Cow,
    fmt::Display,
    str::{from_utf8, from_utf8_unchecked},
};

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

pub fn decode_utf8(data: &[u8]) -> Cow<str> {
    match from_utf8(data) {
        Ok(string) => Cow::Borrowed(string),
        Err(e) => {
            if e.valid_up_to() > data.len() - 4 {
                Cow::Borrowed(unsafe { from_utf8_unchecked(&data[..e.valid_up_to()]) })
            } else {
                String::from_utf8_lossy(data)
            }
        }
    }
}
