pub mod bzip2;
mod devec;
pub mod raw;

use async_trait::async_trait;
use std::{fmt::Debug, io::Result};

#[async_trait]
pub trait FileBuffer: Debug {
    fn data(&self) -> &[u8];
    fn range(&self) -> std::ops::Range<u64>;
    fn jump(&mut self, bytes: u64);
    async fn total_size(&self) -> u64;
    async fn load_prev(&mut self) -> Result<usize>;
    async fn load_next(&mut self) -> Result<usize>;
    fn shrink_to(&mut self, range: std::ops::Range<u64>);
}

pub async fn make_file_buffer(path: &str) -> Result<Box<dyn FileBuffer>> {
    let bz = bzip2::FileBuffer::new(path).await?;
    if bz.is_valid() {
        return Ok(Box::from(bz));
    }

    return Ok(Box::from(raw::FileBuffer::new(path).await?));
}
