use super::devec::DeVec;
use async_trait::async_trait;
use memmap2::{Advice, Mmap, MmapOptions};
use regex::bytes::Regex;
use std::{
    cmp::min,
    io::{self, ErrorKind},
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    task::yield_now,
};

const BUFFER_SIZE: usize = 0x10000;
const FIND_WINDOW: usize = 0x100000;
const FIND_OVERLAP: usize = 0x1000;

#[derive(Debug)]
pub struct FileBuffer {
    buffer_offset: u64,
    buffer: DeVec<u8>,
    file: File,
}

impl FileBuffer {
    pub async fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path).await?;
        return Ok(Self {
            buffer_offset: 0,
            buffer: DeVec::new(),
            file,
        });
    }
    fn mmap(&self, advice: Advice) -> io::Result<Mmap> {
        let mmap = unsafe { MmapOptions::new().map(&self.file) }?;
        mmap.advise(advice)?;
        return Ok(mmap);
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
        return self.file.metadata().await.unwrap().len();
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        let try_read_size = min(self.buffer_offset as usize, BUFFER_SIZE);
        self.buffer.resize_front(self.buffer.len() + try_read_size);

        let buf = self.buffer.as_mut_slice();
        let buf = &mut buf[..try_read_size];

        let read_offset = self.buffer_offset - try_read_size as u64;

        let read_size_res = match self.file.seek(io::SeekFrom::Start(read_offset)).await {
            Ok(_) => self.file.read(buf).await,
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
        let size_before = self.buffer.len();
        let read_offset = self.range().end;
        self.buffer.resize_back(size_before + BUFFER_SIZE);

        let buf = self.buffer.as_mut_slice();
        let buf_start = buf.len() - BUFFER_SIZE;
        let buf = &mut buf[buf_start..];

        let read_size_res = match self.file.seek(io::SeekFrom::Start(read_offset)).await {
            Ok(_) => self.file.read(buf).await,
            Err(e) => Err(e),
        };
        let read_size = *read_size_res.as_ref().unwrap_or(&0);
        self.buffer.resize_back(size_before + read_size);
        return read_size_res;
    }
    async fn find(&mut self, re: &Regex, cancelled: &AtomicBool) -> io::Result<Option<Range<u64>>> {
        let mmap = self.mmap(Advice::Sequential)?;
        let mut begin = self.buffer_offset as usize;
        let mut end = min(begin + FIND_WINDOW, mmap.len());
        loop {
            if let Some(m) = re.find(&mmap[begin..end]) {
                return Ok(Some(Range {
                    start: (begin + m.range().start) as u64,
                    end: (begin + m.range().end) as u64,
                }));
            }
            if end == mmap.len() {
                break;
            }
            begin = end - FIND_OVERLAP;
            end = min(begin + FIND_WINDOW, mmap.len());
            yield_now().await;
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::from(ErrorKind::Interrupted));
            }
        }
        return Ok(None);
    }
    async fn rfind(
        &mut self,
        re: &Regex,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Range<u64>>> {
        let mmap = self.mmap(Advice::Sequential)?;
        let mut end = self.buffer_offset as usize;
        let mut begin = end.saturating_sub(FIND_WINDOW);
        loop {
            if let Some(m) = re.find_iter(&mmap[begin..end]).last() {
                return Ok(Some(Range {
                    start: (begin + m.range().start) as u64,
                    end: (begin + m.range().end) as u64,
                }));
            }
            if begin == 0 {
                break;
            }
            end = begin + FIND_OVERLAP;
            begin = end.saturating_sub(FIND_WINDOW);
            yield_now().await;
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::from(ErrorKind::Interrupted));
            }
        }
        return Ok(None);
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
