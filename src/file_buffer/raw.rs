use crate::file_buffer::FileBuffer;
use async_trait::async_trait;
use memmap2::{Advice, Mmap, MmapOptions};
use regex::bytes::Regex;
use std::{
    cmp::min,
    io::{self, ErrorKind},
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{fs::File, task::yield_now};

const BUFFER_SIZE: u64 = 0x10000;
const FIND_WINDOW: u64 = 0x100000;
const FIND_OVERLAP: u64 = 0x1000;

#[derive(Debug)]
pub struct RawFileBuffer {
    range: Range<u64>,
    path: String,
    file: File,
    mmap: Mmap,
}

impl RawFileBuffer {
    pub async fn new(path: &str) -> io::Result<Self> {
        let path = path.to_owned();
        let file = File::open(&path).await?;
        let mmap = unsafe { MmapOptions::new().map(&file) }?;
        mmap.advise(Advice::Sequential)?;
        return Ok(Self {
            range: Range { start: 0, end: 0 },
            path,
            file,
            mmap,
        });
    }

    fn remmap(&mut self) -> io::Result<()> {
        self.mmap = unsafe { MmapOptions::new().map(&self.file) }?;
        self.mmap.advise(Advice::Sequential)?;
        return Ok(());
    }

    async fn maybe_remmap(&mut self) -> io::Result<()> {
        if self.total_size().await != self.mmap.len() as u64 {
            self.remmap()?;
        }
        return Ok(());
    }
}

#[async_trait]
impl FileBuffer for RawFileBuffer {
    fn data(&self) -> &[u8] {
        return &self.mmap[(self.range.start as usize)..(self.range.end as usize)];
    }
    fn range(&self) -> Range<u64> {
        return Range {
            start: self.range.start,
            end: self.range.end,
        };
    }
    fn jump(&mut self, bytes: u64) -> io::Result<u64> {
        self.range.start = bytes;
        self.range.end = bytes;
        return Ok(bytes);
    }
    async fn total_size(&self) -> u64 {
        return self.file.metadata().await.unwrap().len();
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        let start_before = self.range.start;
        self.range.start = self.range.start.saturating_sub(BUFFER_SIZE as u64);
        return Ok((start_before - self.range.start) as usize);
    }
    async fn load_next(&mut self) -> std::io::Result<usize> {
        let end_before = self.range.end;
        self.range.end += BUFFER_SIZE;
        if self.range.end > self.mmap.len() as u64 {
            self.maybe_remmap().await?;
        }
        self.range.end = min(self.range.end, self.mmap.len() as u64);
        return Ok((self.range.end - end_before) as usize);
    }
    async fn seek_from(
        &mut self,
        re: &Regex,
        offset: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        let mut begin = self.range.start + offset;
        loop {
            let end = min(begin + FIND_WINDOW, self.mmap.len() as u64);
            if let Some(m) = re.find(&self.mmap[begin as usize..end as usize]) {
                self.range.start = begin + m.range().start as u64;
                self.range.end = begin + m.range().end as u64;
                return Ok(Some(Range {
                    start: 0,
                    end: m.range().len() as u64,
                }));
            }

            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::from(ErrorKind::Interrupted));
            }

            if end == self.mmap.len() as u64 {
                match self.load_next().await {
                    Ok(0) => break,
                    _ => (),
                }
            }
            begin = end - FIND_OVERLAP;
            yield_now().await;
        }
        return Ok(None);
    }
    async fn rseek_from(
        &mut self,
        re: &Regex,
        offset: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        let mut end = min(self.range.start + offset, self.mmap.len() as u64);
        loop {
            let begin = end.saturating_sub(FIND_WINDOW);
            if let Some(m) = re
                .find_iter(&self.mmap[begin as usize..end as usize])
                .last()
            {
                self.range.start = begin + m.range().start as u64;
                self.range.end = begin + m.range().end as u64;
                return Ok(Some(Range {
                    start: 0,
                    end: m.range().len() as u64,
                }));
            }

            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::from(ErrorKind::Interrupted));
            }

            if begin == 0 {
                break;
            }

            end = min(begin + FIND_OVERLAP, self.mmap.len() as u64);
            yield_now().await;
        }
        return Ok(None);
    }
}
