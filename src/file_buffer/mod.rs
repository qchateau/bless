pub mod bzip2;
pub mod raw;

use crate::errors::Result;
use async_trait::async_trait;
use regex::bytes::Regex;
use std::{fmt::Debug, io, ops::Range, sync::atomic::AtomicBool};

#[async_trait]
pub trait FileBuffer: Debug {
    // slice to the file data
    fn data(&self) -> &[u8];
    // range of the data on file, the size may be different
    // from the data size for compressed files
    fn range(&self) -> Range<u64>;
    // jump to a byte offset in the file
    // the actual jump position may be diffent, and is returned
    fn jump(&mut self, bytes: u64) -> io::Result<u64>;
    // total size of the file
    async fn total_size(&self) -> u64;
    // load more data at the front
    async fn load_prev(&mut self) -> io::Result<usize>;
    // load more data at the back
    async fn load_next(&mut self) -> io::Result<usize>;
    // find a pattern forward
    async fn seek_from(
        &mut self,
        re: &Regex,
        offset: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>>;
    // find a pattern backwards
    async fn rseek_from(
        &mut self,
        re: &Regex,
        offset: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>>;
}

pub async fn make_file_buffer(path: &str) -> Result<Box<dyn FileBuffer>> {
    let bz = bzip2::Bz2FileBuffer::new(path).await?;
    if bz.is_valid() {
        return Ok(Box::from(bz));
    }

    return Ok(Box::from(raw::RawFileBuffer::new(path).await?));
}
