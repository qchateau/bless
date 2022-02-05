use std::{cmp::min, fs::File, io, ops::Range, os::unix::fs::FileExt};

const BUFFER_SIZE: usize = 0xffff;

pub struct FileBuffer {
    file: File,
    buffer_offset: u64,
    buffer: Vec<u8>,
}

impl FileBuffer {
    pub fn new(path: &str) -> Result<Self, io::Error> {
        let file = File::open(path)?;
        return Ok(Self {
            file,
            buffer_offset: 0,
            buffer: Vec::new(),
        });
    }
}

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
    fn total_size(&self) -> u64 {
        return self.file.metadata().unwrap().len();
    }
    fn jump(&mut self, bytes: u64) {
        self.buffer.clear();
        self.buffer_offset = bytes;
    }
    fn load_prev(&mut self) -> Option<usize> {
        let shift = min(self.buffer_offset, BUFFER_SIZE as u64);
        let mut prev_buffer = vec![0u8; BUFFER_SIZE];

        let read_bytes = loop {
            if self.buffer_offset == 0 {
                return None;
            }

            match self.file.read_at(
                prev_buffer.as_mut_slice(),
                (self.buffer_offset - shift) as u64,
            ) {
                Ok(0) => self.buffer_offset = self.file.metadata().unwrap().len(),
                Ok(read_size) => break read_size,
                _ => return None,
            }
        };
        prev_buffer.resize(read_bytes, 0);

        prev_buffer.append(&mut self.buffer);
        self.buffer = prev_buffer;
        self.buffer_offset -= read_bytes as u64;
        return Some(read_bytes);
    }
    fn load_next(&mut self) -> Option<usize> {
        let mut next_buffer = vec![0u8; BUFFER_SIZE];
        let read_bytes = match self.file.read_at(
            next_buffer.as_mut_slice(),
            (self.buffer_offset + self.buffer.len() as u64)
                .try_into()
                .unwrap(),
        ) {
            Ok(0) => return None,
            Ok(read_bytes) => read_bytes,
            _ => return None,
        };
        next_buffer.resize(read_bytes, 0);

        let read_size = next_buffer.len();
        self.buffer.append(&mut next_buffer);
        return Some(read_size);
    }
}
