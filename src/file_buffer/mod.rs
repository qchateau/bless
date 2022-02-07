pub mod bzip2;
mod devec;
pub mod raw;

use std::{fmt::Debug, io::Result};

pub trait FileBuffer: Debug {
    fn data(&self) -> &[u8];
    fn range(&self) -> std::ops::Range<u64>;
    fn total_size(&self) -> u64;
    fn jump(&mut self, bytes: u64);
    fn load_prev(&mut self) -> Result<usize>;
    fn load_next(&mut self) -> Result<usize>;
    fn shrink_to(&mut self, range: std::ops::Range<u64>);
}

pub fn make_file_buffer(path: &str) -> Result<Box<dyn FileBuffer>> {
    let bz = bzip2::FileBuffer::new(path)?;
    if bz.is_valid() {
        return Ok(Box::from(bz));
    }

    return Ok(Box::from(raw::FileBuffer::new(path)?));
}
