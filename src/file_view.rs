use crate::{
    errors::ViewError,
    file_buffer::{make_file_buffer, FileBuffer},
    utils::{nth_or_last, InfiniteLoopBreaker},
};
use human_bytes::human_bytes;
use num_integer::div_ceil;
use regex::{bytes, Regex};
use std::{
    io,
    io::ErrorKind,
    ops::Range,
    str::{from_utf8, from_utf8_unchecked},
    sync::atomic::{AtomicBool, Ordering},
};

const MATCH_WINDOW: usize = 0x1000;
const SHRINK_THRESHOLD: usize = 1_000_000;

#[derive(Debug)]
pub struct ViewState {
    view_offset: usize,
    buffer_pos: u64,
    current_line: Option<i64>,
}

#[derive(Debug)]
pub struct FileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<i64>,
}

impl FileView {
    pub async fn new(path: String) -> io::Result<Self> {
        let buffer = make_file_buffer(&path).await?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(1),
        });
    }
    pub async fn file_size(&self) -> u64 {
        return self.buffer.total_size().await;
    }
    pub fn file_path(&self) -> &str {
        return self.file_path.as_str();
    }
    pub fn current_line(&self) -> Option<i64> {
        return self.current_line;
    }
    pub fn offset(&self) -> u64 {
        let buffer_size = self.buffer.range().count();
        let data_size = self.buffer.data().len();
        return self.buffer.range().start
            + (self.view_offset as f64 * buffer_size as f64 / data_size as f64) as u64;
    }
    pub async fn view(
        &mut self,
        nlines: usize,
        ncols: Option<usize>,
    ) -> Result<Vec<&str>, ViewError> {
        eprintln!("building view for {}x{}", nlines, ncols.unwrap_or(0));

        loop {
            let mut in_lines = 0;
            let mut out_lines = 0;

            for line in self.current_view().lines() {
                if ncols.is_some() {
                    out_lines += div_ceil(line.chars().count(), ncols.unwrap());
                } else {
                    out_lines += 1;
                }

                if out_lines > nlines {
                    return Ok(self.current_view().lines().take(in_lines).collect());
                }

                in_lines += 1;
                if out_lines == nlines {
                    return Ok(self.current_view().lines().take(in_lines).collect());
                }
            }

            match self.load_next().await {
                Ok(0) => break,
                Ok(_) => (),
                Err(e) => return Err(ViewError::Other(e.to_string())),
            }
        }

        loop {
            if self.up(1).await.is_err() {
                return Ok(self.current_view().lines().collect());
            }
            let out_lines = self.current_view().lines().fold(0, |acc, line| {
                if ncols.is_some() {
                    acc + div_ceil(line.chars().count(), ncols.unwrap())
                } else {
                    acc + 1
                }
            });
            if out_lines >= nlines {
                if out_lines > nlines {
                    self.down(1).await.ok();
                }
                return Ok(self.current_view().lines().collect());
            }
        }
    }
    pub async fn up(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::Other("exceeded max number of iterations trying to go up".to_string()),
        );

        eprintln!("up {}", lines);
        while lines > 0 {
            breaker.it()?;

            let view = self.above_view();
            let view = &view[..view.len().saturating_sub(1)]; // skip backline
            match nth_or_last(view.rmatch_indices('\n'), lines.saturating_sub(1) as usize) {
                Some(((pos, _), nth)) => {
                    self.view_offset = pos + 1;
                    self.current_line = self.current_line.map(|x| x - 1 - nth as i64);
                    lines -= 1 + nth as u64;
                    breaker.reset();
                }
                None => match self.load_prev().await {
                    Ok(0) => {
                        let was_at_top = self.view_offset == 0;
                        self.view_offset = 0;
                        self.current_line = Some(1);
                        if was_at_top {
                            return Err(ViewError::BOF);
                        } else {
                            lines -= 1;
                        }
                    }
                    Ok(_) => (),
                    Err(e) => {
                        return Err(ViewError::Other(e.to_string()));
                    }
                },
            }
        }
        return Ok(());
    }
    pub async fn up_to_line_matching(
        &mut self,
        regex: &Regex,
        cancelled: &AtomicBool,
    ) -> Result<(), ViewError> {
        eprintln!("up to line matching {}", regex.as_str());

        let state = self.save_state();

        let bytes_re = bytes::Regex::new(regex.as_str()).unwrap();
        match self.buffer.rfind(&bytes_re, cancelled).await {
            // fast path: the buffer implements find
            Ok(maybe_match) => {
                if let Some(m) = maybe_match {
                    return self.jump_to_byte(m.start).await;
                } else {
                    self.load_state(&state)?;
                    return Err(ViewError::NoMatchFound);
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                return Err(ViewError::Cancelled);
            }
            // the buffer doesn't implement find, do it ourself
            Err(e) if e.kind() == ErrorKind::Unsupported => {
                eprintln!("no fast implementation, using default impl");
            }
            // the buffer does implement find, but encountered and error
            Err(e) => {
                return Err(ViewError::Other(e.to_string()));
            }
        }

        let err = loop {
            let m = regex.find_iter(self.above_view()).last();
            if let Some(m) = m {
                // align view_offset to a line start, use self.up
                // add 1 to the view offset in case the match is start of line already
                self.view_offset = m.start() + 1;
                self.up(1).await.ok();
                self.current_line = None;
                return Ok(());
            }

            self.view_offset = MATCH_WINDOW;

            match self.load_prev().await {
                Ok(0) => break ViewError::NoMatchFound,
                Err(e) => break ViewError::Other(e.to_string()),
                Ok(_) => (),
            }

            if cancelled.load(Ordering::Acquire) {
                break ViewError::Cancelled;
            }
        };

        self.load_state(&state)?;
        return Err(err);
    }
    pub async fn down(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::Other("exceeded max number of iterations trying to go down".to_string()),
        );

        eprintln!("down {}", lines);
        while lines > 0 {
            breaker.it()?;
            match nth_or_last(
                self.current_view().match_indices('\n'),
                lines.saturating_sub(1) as usize,
            ) {
                Some(((pos, _), nth)) => {
                    self.view_offset += pos + 1;
                    self.current_line = self.current_line.map(|x| x + 1 + nth as i64);
                    lines -= 1 + nth as u64;
                    breaker.reset();
                }
                None => match self.load_next().await {
                    Ok(0) => return Err(ViewError::EOF),
                    Ok(_) => (),
                    Err(e) => return Err(ViewError::Other(e.to_string())),
                },
            }
        }
        return Ok(());
    }
    pub async fn down_to_line_matching(
        &mut self,
        regex: &Regex,
        skip_current: bool,
        cancelled: &AtomicBool,
    ) -> Result<(), ViewError> {
        eprintln!("down to line matching {}", regex.as_str());

        let state = self.save_state();
        if skip_current {
            self.down(1).await.ok();
        }

        let bytes_re = bytes::Regex::new(regex.as_str()).unwrap();
        match self.buffer.find(&bytes_re, cancelled).await {
            // fast path: the buffer implements find
            Ok(maybe_match) => {
                if let Some(m) = maybe_match {
                    return self.jump_to_byte(m.start).await;
                } else {
                    self.load_state(&state)?;
                    return Err(ViewError::NoMatchFound);
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                return Err(ViewError::Cancelled);
            }
            // the buffer doesn't implement find, do it ourself
            Err(e) if e.kind() == ErrorKind::Unsupported => {
                eprintln!("no fast implementation, using default impl");
            }
            // the buffer does implement find, but encountered and error
            Err(e) => {
                return Err(ViewError::Other(e.to_string()));
            }
        }

        let err = loop {
            let m = regex.find(self.current_view());
            if let Some(m) = m {
                // align view_offset to a line start, use self.up
                // add 1 to the view offset in case the match is start of line already
                self.view_offset += m.start() + 1;
                self.up(1).await.ok();
                self.current_line = None;
                return Ok(());
            }

            self.view_offset += self.current_view().len().saturating_sub(MATCH_WINDOW);

            match self.load_next().await {
                Ok(0) => break ViewError::NoMatchFound,
                Err(e) => break ViewError::Other(e.to_string()),
                Ok(_) => (),
            }

            if cancelled.load(Ordering::Acquire) {
                break ViewError::Cancelled;
            }
        };

        self.load_state(&state)?;
        return Err(err);
    }
    pub async fn jump_to_line(&mut self, line: i64) -> Result<(), ViewError> {
        eprintln!("jump to line {}", line);

        //  move to the right "side" of the file
        if line > 0 && (self.current_line.is_none() || self.current_line.unwrap() <= 0) {
            self.top().await?
        } else if line <= 0 && (self.current_line.is_none() || self.current_line.unwrap() > 0) {
            self.bottom().await?
        }

        let mut offset = line - self.current_line.unwrap();
        if offset.abs() > line.abs() {
            // shotcut, easier to reset the cursor
            if line > 0 {
                self.top().await?;
            } else {
                self.bottom().await?;
            }
            offset = line - self.current_line.unwrap();
        }

        return if offset > 0 {
            self.down(offset.abs() as u64).await
        } else if offset < 0 {
            self.up(offset.abs() as u64).await
        } else {
            Ok(())
        };
    }
    pub async fn jump_to_byte(&mut self, bytes: u64) -> Result<(), ViewError> {
        eprintln!("jump to byte {}", bytes);

        self.buffer
            .jump(bytes)
            .map_err(|e| ViewError::Other(e.to_string()))?;
        self.view_offset = 0;

        if bytes == 0 {
            self.current_line = Some(1);
            Ok(())
        } else {
            self.current_line = None;
            self.up(1).await
        }
    }
    pub async fn top(&mut self) -> Result<(), ViewError> {
        eprintln!("jump to top");

        self.jump_to_byte(0).await
    }
    pub async fn bottom(&mut self) -> Result<(), ViewError> {
        eprintln!("jump to bottom");

        self.buffer
            .jump(self.buffer.total_size().await - 1)
            .map_err(|e| ViewError::Other(e.to_string()))?;
        self.view_offset = self.buffer.data().len();
        self.current_line = Some(0);
        Ok(())
    }
    pub fn save_state(&self) -> ViewState {
        return ViewState {
            view_offset: self.view_offset,
            current_line: self.current_line,
            buffer_pos: self.buffer.range().start,
        };
    }
    pub fn load_state(&mut self, state: &ViewState) -> Result<(), ViewError> {
        self.view_offset = state.view_offset;
        self.current_line = state.current_line;
        self.buffer
            .jump(state.buffer_pos)
            .map_err(|e| ViewError::Other(e.to_string()))?;
        Ok(())
    }
    fn current_view(&self) -> &str {
        let slice = self.buffer.data().get(self.view_offset..).unwrap_or(b"");
        match from_utf8(slice) {
            Ok(string) => string,
            Err(e) => unsafe { from_utf8_unchecked(&slice[..e.valid_up_to()]) },
        }
    }
    fn above_view(&self) -> &str {
        let slice = self.buffer.data().get(..self.view_offset).unwrap_or(b"");
        match from_utf8(slice) {
            Ok(string) => string,
            Err(e) => unsafe { from_utf8_unchecked(&slice[..e.valid_up_to()]) },
        }
    }
    fn maybe_shrink(&mut self, range: Range<u64>) {
        let size_before = self.buffer.data().len();
        if size_before < SHRINK_THRESHOLD {
            return;
        }
        let shrinked = self.buffer.shrink_to(range.clone());
        self.view_offset = (range.start - shrinked.start) as usize;
        eprintln!(
            "shriked: {}, size: {}, new offset: {}",
            human_bytes((size_before - self.buffer.data().len()) as f64),
            human_bytes(self.buffer.data().len() as f64),
            self.view_offset
        );
    }
    fn maybe_shrink_left(&mut self) {
        let start = self.view_offset as u64;
        let end = self.buffer.data().len() as u64;
        self.maybe_shrink(Range { start, end });
    }
    fn maybe_shrink_right(&mut self) {
        let start = 0;
        let end = self.view_offset as u64;
        self.maybe_shrink(Range { start, end });
    }
    async fn load_next(&mut self) -> io::Result<usize> {
        let load_size = self.buffer.load_next().await?;
        self.maybe_shrink_left();
        eprintln!("loaded {} next bytes", load_size);
        return Ok(load_size);
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        let load_size = self.buffer.load_prev().await?;
        self.view_offset += load_size;
        self.maybe_shrink_right();
        eprintln!("loaded {} previous bytes", load_size);
        return Ok(load_size);
    }
}
