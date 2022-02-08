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

pub fn wrap_text(text: String, width: usize) -> String {
    assert!(width > 0);
    let mut lines = Vec::new();
    for mut line in text.lines() {
        while line.len() > width {
            lines.push(line.get(..width).unwrap());
            line = &line[width..];
        }
        lines.push(line);
    }
    return lines.join("\n");
}
