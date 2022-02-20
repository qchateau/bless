use log::info;
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

#[derive(Debug, Clone)]
pub struct InfiniteLoopError;

impl Display for InfiniteLoopError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "infinite loop")
    }
}

impl Error for InfiniteLoopError {}

pub struct InfiniteLoopBreaker {
    count: u64,
    current_count: u64,
}

impl InfiniteLoopBreaker {
    pub fn new(count: u64) -> Self {
        return Self {
            count,
            current_count: count,
        };
    }
    pub fn reset(&mut self) {
        self.current_count = self.count;
    }
    pub fn it(&mut self) -> Result<(), InfiniteLoopError> {
        self.current_count -= 1;
        if self.current_count == 0 {
            info!("loop break");
            return Err(InfiniteLoopError);
        }
        return Ok(());
    }
}
