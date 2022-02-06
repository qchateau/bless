pub struct InfiniteLoopBreaker<T> {
    count: u64,
    current_count: u64,
    error: T,
}

impl<T: Clone> InfiniteLoopBreaker<T> {
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
            return Err(self.error.clone());
        }
        return Ok(());
    }
}
