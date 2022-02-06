use super::devec::DeVec;
use std::{
    cmp::{max, min},
    fs::File,
    io,
    ops::Range,
    os::unix::fs::FileExt,
};

const BUFFER_SIZE: usize = 0xffff;

#[derive(Debug)]
pub struct FileBuffer {
    buffer_offset: u64,
    buffer: DeVec<u8>,
    file: File,
}

impl FileBuffer {
    pub fn new(path: &str) -> Result<Self, io::Error> {
        let file = File::open(path)?;
        return Ok(Self {
            buffer_offset: 0,
            buffer: DeVec::new(),
            file,
        });
    }
}

impl super::FileBuffer for FileBuffer {
    fn data(&mut self) -> &[u8] {
        return self.buffer.as_slice();
    }
    fn range(&self) -> Range<u64> {
        return Range {
            start: self.buffer_offset,
            end: self.buffer_offset + self.buffer.len() as u64,
        };
    }
    fn total_size(&self) -> u64 {
        return self.file.metadata().unwrap().len();
    }
    fn jump(&mut self, bytes: u64) {
        self.buffer.clear();
        self.buffer_offset = bytes;
    }
    fn load_prev(&mut self) -> io::Result<usize> {
        let try_read_size = min(self.buffer_offset as usize, BUFFER_SIZE);
        self.buffer.resize_front(try_read_size);

        let buf = self.buffer.as_mut_slice();
        let buf = &mut buf[..try_read_size];

        let read_offset = self.buffer_offset - try_read_size as u64;
        let read_size_res = self.file.read_at(buf, read_offset);
        let read_size = *read_size_res.as_ref().unwrap_or(&0);

        let missing_bytes = try_read_size - read_size;
        buf.rotate_right(missing_bytes);
        self.buffer.resize_front(self.buffer.len() - missing_bytes);

        self.buffer_offset -= read_size as u64;
        return read_size_res;
    }
    fn load_next(&mut self) -> std::io::Result<usize> {
        let size_before = self.buffer.len();
        let read_offset = self.range().end;
        self.buffer.resize(size_before + BUFFER_SIZE);

        let buf = self.buffer.as_mut_slice();
        let buf_start = buf.len() - BUFFER_SIZE;
        let buf = &mut buf[buf_start..];

        let read_size_res = self.file.read_at(buf, read_offset);
        let read_size = *read_size_res.as_ref().unwrap_or(&0);
        self.buffer.resize(size_before + read_size);
        return read_size_res;
    }
    fn shrink_to(&mut self, range: Range<u64>) {
        let inter = Range {
            start: max(self.range().start, range.start),
            end: min(self.range().end, range.end),
        };
        if inter.end <= inter.start {
            self.buffer_offset = inter.start;
            self.buffer.clear();
            return;
        }

        let extra_end = self.range().end.saturating_sub(inter.end) as usize;
        let extra_start = inter.start.saturating_sub(self.range().start) as usize;
        self.buffer.resize(self.buffer.len() - extra_end);
        self.buffer.resize_front(self.buffer.len() - extra_start);
        self.buffer_offset = inter.start;
        self.buffer.shrink_to(inter.count());
    }
}
