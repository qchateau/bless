use super::devec::DeVec;
use async_trait::async_trait;
use std::{
    cmp::min,
    fs::File,
    io::{self, Read, Seek},
    ops::Range,
};
use tokio::task::yield_now;

const BUFFER_SIZE: usize = 0xffff;

#[derive(Debug)]
pub struct FileBuffer {
    buffer_offset: u64,
    buffer: DeVec<u8>,
    file: File,
}

impl FileBuffer {
    pub async fn new(path: &str) -> Result<Self, io::Error> {
        let file = File::open(path)?;
        return Ok(Self {
            buffer_offset: 0,
            buffer: DeVec::new(),
            file,
        });
    }
}

#[async_trait]
impl super::FileBuffer for FileBuffer {
    fn data(&self) -> &[u8] {
        return self.buffer.as_slice();
    }
    fn range(&self) -> Range<u64> {
        return Range {
            start: self.buffer_offset,
            end: self.buffer_offset + self.buffer.len() as u64,
        };
    }
    fn jump(&mut self, bytes: u64) -> io::Result<u64> {
        self.buffer.clear();
        self.buffer_offset = bytes;
        return Ok(bytes);
    }
    async fn total_size(&self) -> u64 {
        return self.file.metadata().unwrap().len();
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        yield_now().await;

        let try_read_size = min(self.buffer_offset as usize, BUFFER_SIZE);
        self.buffer.resize_front(self.buffer.len() + try_read_size);

        let buf = self.buffer.as_mut_slice();
        let buf = &mut buf[..try_read_size];

        let read_offset = self.buffer_offset - try_read_size as u64;

        let read_size_res = match self.file.seek(io::SeekFrom::Start(read_offset)) {
            Ok(_) => self.file.read(buf),
            Err(e) => Err(e),
        };
        let read_size = *read_size_res.as_ref().unwrap_or(&0);

        let missing_bytes = try_read_size - read_size;
        buf.rotate_right(missing_bytes);
        self.buffer.resize_front(self.buffer.len() - missing_bytes);

        self.buffer_offset -= read_size as u64;
        return read_size_res;
    }
    async fn load_next(&mut self) -> std::io::Result<usize> {
        yield_now().await;

        let size_before = self.buffer.len();
        let read_offset = self.range().end;
        self.buffer.resize_back(size_before + BUFFER_SIZE);

        let buf = self.buffer.as_mut_slice();
        let buf_start = buf.len() - BUFFER_SIZE;
        let buf = &mut buf[buf_start..];

        let read_size_res = match self.file.seek(io::SeekFrom::Start(read_offset)) {
            Ok(_) => self.file.read(buf),
            Err(e) => Err(e),
        };
        let read_size = *read_size_res.as_ref().unwrap_or(&0);
        self.buffer.resize_back(size_before + read_size);
        return read_size_res;
    }
    fn shrink_to(&mut self, range: Range<u64>) -> Range<u64> {
        assert!(range.start <= range.end && range.end <= self.data().len() as u64);

        self.buffer.resize_back(range.end as usize);
        self.buffer
            .resize_front(self.buffer.len() - range.start as usize);
        self.buffer_offset += range.start;
        self.buffer.shrink_to(self.buffer.len());
        return range;
    }
}
