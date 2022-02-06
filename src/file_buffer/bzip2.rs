use super::raw;
use bzip2::Decompress;
use regex::bytes::Regex;
use std::{fs::File, io, io::Read, ops::Range};

const BLOCK_MAGIC: &'static [u8] = &[0x31, 0x41, 0x59, 0x26, 0x53, 0x59];
const BUFFER_SIZE: usize = 0xffff;

fn lstrip_to_magic(mut data: &[u8]) -> &[u8] {
    while !data.is_empty() && !data.starts_with(BLOCK_MAGIC) {
        data = &data[1..]
    }
    return data;
}

#[derive(Debug)]
pub struct FileBuffer {
    raw_buffer: raw::FileBuffer,
    header: Vec<u8>,
    decoded: Vec<u8>,
}

impl FileBuffer {
    pub fn new(path: &str) -> Result<Self, io::Error> {
        let mut file = File::open(path)?;
        let mut header = vec![0u8; 4];
        file.read_exact(header.as_mut_slice()).unwrap();
        return Ok(Self {
            raw_buffer: raw::FileBuffer::new(path)?,
            header,
            decoded: Vec::new(),
        });
    }
    pub fn is_valid(&self) -> bool {
        return Regex::new("BZ[h0][1-9]").unwrap().is_match(&self.header);
    }

    fn decode(&mut self) -> io::Result<()> {
        let mut decoder = Decompress::new(false);
        let mut data = lstrip_to_magic(super::FileBuffer::data(&mut self.raw_buffer));
        if data.len() < BLOCK_MAGIC.len() {
            self.decoded.clear();
            return Ok(());
        }

        // remove the last char, it's irrelevant in most cases, and we *never*
        // want the decoder to encounter the end of the stream
        data = &data[..data.len() - 1];

        let mut buf = Vec::new();
        decoder.decompress_vec(self.header.as_slice(), &mut buf)?;
        loop {
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            buf.reserve(buf.len() + BUFFER_SIZE);
            decoder.decompress_vec(data, &mut buf)?;
            let consumed = decoder.total_in() - before_in;
            let produced = decoder.total_out() - before_out;

            data = &data[consumed as usize..];
            if produced == 0 && consumed == 0 {
                break;
            }
        }
        self.decoded = buf;
        return Ok(());
    }
}

impl super::FileBuffer for FileBuffer {
    fn data(&mut self) -> &[u8] {
        return self.decoded.as_slice();
    }
    fn range(&self) -> Range<u64> {
        return self.raw_buffer.range();
    }
    fn total_size(&self) -> u64 {
        return self.raw_buffer.total_size();
    }
    fn jump(&mut self, bytes: u64) {
        self.raw_buffer.jump(bytes);
        self.decoded.clear();
    }
    fn load_prev(&mut self) -> std::io::Result<usize> {
        let size_before = self.decoded.len();
        while self.decoded.len() == size_before {
            self.raw_buffer.load_prev()?;
            self.decode().ok();
        }
        return Ok(self.decoded.len() - size_before);
    }
    fn load_next(&mut self) -> std::io::Result<usize> {
        let size_before = self.decoded.len();
        while self.decoded.len() == size_before {
            self.raw_buffer.load_next()?;
            self.decode().ok();
        }
        return Ok(self.decoded.len() - size_before);
    }
    fn shrink_to(&mut self, range: Range<u64>) {
        return self.raw_buffer.shrink_to(range);
    }
}
