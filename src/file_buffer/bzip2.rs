use crate::utils::infinite_loop_breaker::InfiniteLoopBreaker;

use super::FileBuffer;
use async_trait::async_trait;
use bzip2::Decompress;
use human_bytes::human_bytes;
use log::{debug, info};
use memmap2::{Advice, Mmap, MmapOptions};
use regex::bytes::Regex;
use std::{
    collections::VecDeque,
    fmt,
    io::{self, ErrorKind},
    ops::Range,
    sync::atomic::AtomicBool,
    vec::Vec,
};
use tokio::{fs::File, io::AsyncReadExt, task::yield_now};

const ALLOC_SIZE: usize = 0x100000;
const MAGIC_RFIND_WINDOW: usize = 0x10000;
const MAGIC_RFIND_OVERLAP: usize = 8;
const MAX_INVALID_BLOCKS: u64 = 10;

struct Block {
    file_range: Range<usize>,
    data: Vec<u8>,
}

pub struct Bz2FileBuffer {
    file: File,
    header: Vec<u8>,
    decoded: Vec<u8>,
    blocks: VecDeque<Block>,
    magic_re: Regex,
}

impl fmt::Debug for Bz2FileBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Bz2FileBuffer")
            .field("header", &self.header)
            .field("blocks.len", &self.blocks.len())
            .field("decoded.len", &self.decoded.len())
            .finish()
    }
}

impl Bz2FileBuffer {
    pub async fn new(path: &str) -> io::Result<Self> {
        let mut file = File::open(path).await?;
        let mut header = vec![0u8; 4];
        let magic_re: Regex = Regex::new(r"\x31\x41\x59\x26\x53\x59").unwrap();
        file.read_exact(header.as_mut_slice()).await.unwrap();
        return Ok(Self {
            file,
            header,
            decoded: Vec::new(),
            blocks: VecDeque::new(),
            magic_re,
        });
    }
    pub fn is_valid(&self) -> bool {
        return Regex::new("BZ[h0][1-9]").unwrap().is_match(&self.header);
    }
    fn mmap(&self, advice: Advice) -> io::Result<Mmap> {
        let mmap = unsafe { MmapOptions::new().map(&self.file) }?;
        mmap.advise(advice)?;
        return Ok(mmap);
    }
    fn rebuild_data(&mut self) {
        self.decoded.clear();
        for block in &self.blocks {
            self.decoded.extend(block.data.iter());
        }
    }
    fn decode_block(&self, file_range: Range<usize>) -> io::Result<Block> {
        let mut block = Block {
            file_range,
            data: Vec::new(),
        };
        let mut decoder = Decompress::new(false);
        let mmap = self.mmap(Advice::Sequential)?;

        let mut in_data = &mmap[block.file_range.clone()];
        decoder.decompress(self.header.as_slice(), &mut block.data)?;

        info!("decoding {}", human_bytes(in_data.len() as f64));
        loop {
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            if block.data.capacity() - block.data.len() < ALLOC_SIZE {
                block.data.reserve(ALLOC_SIZE);
            }
            decoder.decompress_vec(in_data, &mut block.data)?;
            let consumed = decoder.total_in() - before_in;
            let produced = decoder.total_out() - before_out;

            in_data = &in_data[consumed as usize..];
            if produced == 0 && consumed == 0 {
                return Ok(block);
            }
        }
    }
    fn find_block_from(&self, byte: usize) -> io::Result<usize> {
        debug!("searching next block from {}", byte);
        let mmap = self.mmap(Advice::Sequential)?;
        if let Some(m) = self.magic_re.find(&mmap[byte..]) {
            debug!("found at {}", byte + m.range().start);
            return Ok(byte + m.range().start);
        } else {
            // Kind of a hack, but makes things easier
            return Ok(mmap.len() - 1);
        }
    }
    fn rfind_block_from(&self, byte: usize) -> io::Result<usize> {
        debug!("searching previous block from {}", byte);
        let mmap = self.mmap(Advice::Sequential)?;
        let mut end = byte;
        let mut start = end.saturating_sub(MAGIC_RFIND_WINDOW);
        loop {
            if let Some(m) = self.magic_re.find_iter(&mmap[start..end]).last() {
                debug!("found at {}", start + m.range().start);
                return Ok(start + m.range().start);
            }
            if start == 0 {
                break;
            }
            end = start + MAGIC_RFIND_OVERLAP;
            start = end.saturating_sub(MAGIC_RFIND_WINDOW);
        }
        return Ok(self.header.len());
    }
}

#[async_trait]
impl FileBuffer for Bz2FileBuffer {
    fn data(&self) -> &[u8] {
        return self.decoded.as_slice();
    }
    fn range(&self) -> Range<u64> {
        return Range {
            start: self
                .blocks
                .iter()
                .nth(0)
                .map(|x| x.file_range.start as u64)
                .unwrap_or(self.header.len() as u64),
            end: self
                .blocks
                .iter()
                .last()
                .map(|x| x.file_range.end as u64)
                .unwrap_or(self.header.len() as u64),
        };
    }
    fn jump(&mut self, byte: u64) -> io::Result<u64> {
        let mut breaker = InfiniteLoopBreaker::new(MAX_INVALID_BLOCKS);

        let mut start = byte as usize;
        let mut end = byte as usize;

        let block = loop {
            end = self.find_block_from(end + 1)?;
            start = self.rfind_block_from(start)?;

            let block_range = Range { start, end };
            match self.decode_block(block_range) {
                Ok(block) => break block,
                Err(err) => {
                    if let Err(_) = breaker.it() {
                        return Err(err);
                    }
                }
            }
        };

        info!("jump to {:?} (requested {})", block.file_range, byte);
        self.blocks.clear();
        self.blocks.push_back(block);
        self.rebuild_data();
        return Ok(self.blocks[0].file_range.start as u64);
    }
    async fn total_size(&self) -> u64 {
        return self.file.metadata().await.unwrap().len();
    }
    async fn load_next(&mut self) -> io::Result<usize> {
        debug!("load next");
        yield_now().await;

        let mut breaker = InfiniteLoopBreaker::new(MAX_INVALID_BLOCKS);
        let size_before = self.data().len();

        let start = self.range().end as usize;
        let mut end = start;

        let block = loop {
            end = self.find_block_from(end + 1)?;
            if end <= start {
                return Ok(0);
            }

            let block_range = Range { start, end };
            match self.decode_block(block_range) {
                Ok(block) => break block,
                Err(err) => {
                    if let Err(_) = breaker.it() {
                        return Err(err);
                    }
                }
            }
        };

        self.decoded.extend(block.data.iter());
        self.blocks.push_back(block);
        return Ok(self.data().len() - size_before);
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        debug!("load previous");
        yield_now().await;

        let mut breaker = InfiniteLoopBreaker::new(MAX_INVALID_BLOCKS);
        let size_before = self.data().len();

        let end = self.range().start as usize;
        let mut start = end;

        let block = loop {
            start = self.rfind_block_from(start)?;
            if start >= end {
                return Ok(0);
            }

            let block_range = Range { start, end };
            match self.decode_block(block_range) {
                Ok(block) => break block,
                Err(err) => {
                    if let Err(_) = breaker.it() {
                        return Err(err);
                    }
                }
            }
        };

        let mut new = block.data.clone();
        new.extend(self.decoded.iter());
        self.decoded = new;
        self.blocks.push_front(block);
        return Ok(self.data().len() - size_before);
    }
    async fn find(
        &mut self,
        _re: &Regex,
        _cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        return Err(io::Error::from(ErrorKind::Unsupported));
    }
    async fn rfind(
        &mut self,
        _re: &Regex,
        _cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        return Err(io::Error::from(ErrorKind::Unsupported));
    }
    fn shrink_to(&mut self, range: Range<u64>) -> Range<u64> {
        assert!(range.start <= range.end && range.end <= self.data().len() as u64);

        let mut len_front = 0;
        while !self.blocks.is_empty() {
            let block_len = self.blocks.front().unwrap().data.len();
            len_front += block_len;
            if len_front > range.start as usize {
                len_front -= block_len;
                break;
            }
            self.blocks.pop_front();
        }

        let mut len_range = 0;
        let mut idx = self.blocks.len();
        for (i, block) in self.blocks.iter().enumerate() {
            if len_range >= range.end as usize {
                idx = i;
                break;
            }
            len_range += block.data.len();
        }
        self.blocks.truncate(idx);

        self.decoded = (&self.decoded[len_front..(len_front + len_range)]).to_vec();
        let shrinked_range = Range {
            start: len_front as u64,
            end: (len_front + len_range) as u64,
        };
        info!("shrinked to {:?} (requested {:?})", shrinked_range, range);
        return shrinked_range;
    }
}
