use std::{cmp::min, fmt, fs::File, io, os::unix::fs::FileExt};

const BUFFER_SIZE: usize = 0xffff;

pub trait FileBuffer {
    fn data(&self) -> &[u8];
    fn offset(&self) -> u64;
    fn total_size(&self) -> u64;
    fn jump(&mut self, bytes: u64);
    fn load_prev(&mut self) -> usize;
    fn load_next(&mut self) -> usize;
}

pub struct PlaintextFileBuffer {
    file: File,
    buffer_offset: u64,
    buffer: Vec<u8>,
}

impl fmt::Debug for PlaintextFileBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaintextFileBuffer")
            .field("file", &self.file)
            .field("buffer_offset", &self.buffer_offset)
            .field("buffer", &format!("[size: {}]", self.buffer.len()))
            .finish()
    }
}

impl PlaintextFileBuffer {
    pub fn new(path: &str) -> Result<Self, io::Error> {
        let file = File::open(path)?;
        return Ok(Self {
            file,
            buffer_offset: 0,
            buffer: Vec::new(),
        });
    }
}

impl FileBuffer for PlaintextFileBuffer {
    fn data(&self) -> &[u8] {
        return self.buffer.as_slice();
    }
    fn offset(&self) -> u64 {
        return self.buffer_offset;
    }
    fn total_size(&self) -> u64 {
        return self.file.metadata().unwrap().len();
    }
    fn jump(&mut self, bytes: u64) {
        self.buffer = Vec::new();
        self.buffer_offset = bytes;
    }
    fn load_prev(&mut self) -> usize {
        let shift = min(self.buffer_offset, BUFFER_SIZE as u64);
        if shift == 0 {
            return 0;
        }

        let mut prev_buffer = vec![0u8; shift as usize];

        let read_bytes = loop {
            let read_bytes = self
                .file
                .read_at(
                    prev_buffer.as_mut_slice(),
                    (self.buffer_offset - shift).try_into().unwrap(),
                )
                .unwrap();
            if read_bytes > 0 {
                break read_bytes;
            }
            self.buffer_offset = self.file.metadata().unwrap().len();
            if self.buffer_offset == 0 {
                break 0;
            }
        };
        prev_buffer.resize(read_bytes, 0);

        prev_buffer.append(&mut self.buffer);
        self.buffer = prev_buffer;
        self.buffer_offset -= read_bytes as u64;
        return read_bytes;
    }
    fn load_next(&mut self) -> usize {
        let mut next_buffer = vec![0u8; BUFFER_SIZE];
        let read_bytes = self
            .file
            .read_at(
                next_buffer.as_mut_slice(),
                (self.buffer_offset + self.buffer.len() as u64)
                    .try_into()
                    .unwrap(),
            )
            .unwrap();
        next_buffer.resize(read_bytes, 0);

        let read_size = next_buffer.len();
        self.buffer.append(&mut next_buffer);
        return read_size;
    }
}
