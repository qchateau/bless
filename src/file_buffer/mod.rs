pub mod bzip2;
pub mod raw;

pub struct ShrinkResult {
    pub before: u64,
    pub after: u64,
}

pub trait FileBuffer {
    fn data(&self) -> &[u8];
    fn range(&self) -> std::ops::Range<u64>;
    fn total_size(&self) -> u64;
    fn jump(&mut self, bytes: u64);
    fn load_prev(&mut self) -> Option<usize>;
    fn load_next(&mut self) -> Option<usize>;
    fn shrink_around(&mut self, pos: u64) -> ShrinkResult;
}
