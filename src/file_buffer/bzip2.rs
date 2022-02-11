use super::raw;
use async_trait::async_trait;
use bzip2::Decompress;
use regex::bytes::Regex;
use std::{fmt, fs::File, io, io::Read, ops::Range};

const BLOCK_MAGIC: &'static [u8] = &[0x31, 0x41, 0x59, 0x26, 0x53, 0x59];
const BUFFER_SIZE: usize = 0xffff;
const BACKWARD_LOAD_SIZE: usize = 900_000; // One bzip block

fn lstrip_to_magic(mut data: &[u8]) -> &[u8] {
    while !data.is_empty() && !data.starts_with(BLOCK_MAGIC) {
        data = &data[1..]
    }
    return data;
}

pub struct FileBuffer {
    raw_buffer: raw::FileBuffer,
    header: Vec<u8>,
    decoded: Vec<u8>,
    decoder: Decompress,
}

impl fmt::Debug for FileBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FileBuffer")
            .field("header", &self.header)
            .field("raw_buffer", &self.raw_buffer)
            .field("decoded.len", &self.decoded.len())
            .finish()
    }
}

impl FileBuffer {
    pub async fn new(path: &str) -> Result<Self, io::Error> {
        let mut file = File::open(path)?;
        let mut header = vec![0u8; 4];
        file.read_exact(header.as_mut_slice()).unwrap();
        return Ok(Self {
            raw_buffer: raw::FileBuffer::new(path).await?,
            header,
            decoded: Vec::new(),
            decoder: Decompress::new(false),
        });
    }
    pub fn is_valid(&self) -> bool {
        return Regex::new("BZ[h0][1-9]").unwrap().is_match(&self.header);
    }

    fn reset_decoder(&mut self) {
        eprintln!("resetting decoder");
        self.decoder = Decompress::new(false);
        self.decoded.clear();
    }

    fn incremental_decode(&mut self) -> io::Result<()> {
        if self.decoder.total_in() == 0 {
            eprintln!("initializing decoder");
            self.decoder
                .decompress(self.header.as_slice(), &mut self.decoded[0..0])?;
        }

        let mut data = lstrip_to_magic(super::FileBuffer::data(&mut self.raw_buffer));
        let offset = self.decoder.total_in() as usize - self.header.len();
        data = &data[offset..];

        // remove the last char, it's irrelevant in most cases, and we *never*
        // want the decoder to encounter the end of the stream
        if !data.is_empty() {
            data = &data[..data.len() - 1];
        }

        eprintln!("decoding {} bytes", data.len());
        loop {
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            self.decoded.reserve(self.decoded.len() + BUFFER_SIZE);
            self.decoder.decompress_vec(data, &mut self.decoded)?;
            let consumed = self.decoder.total_in() - before_in;
            let produced = self.decoder.total_out() - before_out;

            data = &data[consumed as usize..];
            if produced == 0 && consumed == 0 {
                break;
            }
        }

        return Ok(());
    }
}

#[async_trait]
impl super::FileBuffer for FileBuffer {
    fn data(&self) -> &[u8] {
        return self.decoded.as_slice();
    }
    fn range(&self) -> Range<u64> {
        return self.raw_buffer.range();
    }
    fn jump(&mut self, bytes: u64) {
        self.raw_buffer.jump(bytes);
        self.reset_decoder();
    }
    async fn total_size(&self) -> u64 {
        return self.raw_buffer.total_size().await;
    }
    async fn load_prev(&mut self) -> std::io::Result<usize> {
        eprintln!("load previous");
        let size_before = self.decoded.len();
        while self.decoded.len() <= size_before {
            self.reset_decoder();

            let mut loaded = 0;
            while loaded < BACKWARD_LOAD_SIZE {
                // go back at least one block size, otherwise
                // we'll keep resetting the decoder
                loaded += match self.raw_buffer.load_prev().await? {
                    0 => {
                        // We reached the beginning, bail out no matter what
                        self.incremental_decode()?;
                        return Ok(self.decoded.len() - size_before);
                    }
                    x => x,
                };
            }

            self.incremental_decode()?;
        }
        return Ok(self.decoded.len() - size_before);
    }
    async fn load_next(&mut self) -> std::io::Result<usize> {
        eprintln!("load next");
        let size_before = self.decoded.len();
        while self.decoded.len() <= size_before {
            if self.raw_buffer.load_next().await? == 0 {
                return Ok(0);
            }
            self.incremental_decode()?;
        }
        return Ok(self.decoded.len() - size_before);
    }
    fn shrink_to(&mut self, range: Range<u64>) {
        self.reset_decoder();
        return self.raw_buffer.shrink_to(range);
    }
}
