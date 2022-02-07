use crate::{
    errors::ViewError,
    file_buffer::{make_file_buffer, FileBuffer},
    file_view::FileView,
    utils::InfiniteLoopBreaker,
};
use regex::bytes::Regex;
use std::{io, ops::Range};

const MATCH_WINDOW: usize = 0x1000;
const SHRINK_BUFFER_SIZE: usize = 0x100000;
const MAYBE_SHRINK_THRESHOLD: usize = 2 * SHRINK_BUFFER_SIZE;

#[derive(Debug)]
struct ViewState {
    view_offset: usize,
    buffer_pos: u64,
    current_line: Option<i64>,
}

#[derive(Debug)]
pub struct BufferedFileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<i64>,
}

impl BufferedFileView {
    pub fn new(path: String) -> Result<Self, io::Error> {
        let buffer = make_file_buffer(&path)?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(0),
        });
    }
    fn current_view(&mut self) -> &[u8] {
        return self.buffer.data().get(self.view_offset..).unwrap_or(b"");
    }
    fn above_view(&mut self) -> &[u8] {
        return self.buffer.data().get(..self.view_offset).unwrap_or(b"");
    }
    fn maybe_shrink(&mut self) {
        if self.buffer.range().count() > MAYBE_SHRINK_THRESHOLD {
            self.shrink()
        }
    }
    fn shrink(&mut self) {
        let start = self.view_offset as u64 + self.buffer.range().start;
        let end = start + SHRINK_BUFFER_SIZE as u64;
        self.buffer.shrink_to(Range { start, end });
        self.view_offset = 0;
    }
    fn load_next(&mut self) -> io::Result<usize> {
        let res = self.buffer.load_next()?;
        self.maybe_shrink();
        return Ok(res);
    }
    fn load_prev(&mut self) -> io::Result<usize> {
        let load_size = self.buffer.load_prev()?;
        self.view_offset += load_size;
        self.maybe_shrink();
        return Ok(load_size);
    }
    fn save_state(&self) -> ViewState {
        return ViewState {
            view_offset: self.view_offset,
            current_line: self.current_line,
            buffer_pos: self.buffer.range().start,
        };
    }
    fn load_state(&mut self, state: &ViewState) {
        self.view_offset = state.view_offset;
        self.current_line = state.current_line;
        self.buffer.jump(state.buffer_pos);
    }
}

impl FileView for BufferedFileView {
    fn file_size(&self) -> u64 {
        return self.buffer.total_size();
    }
    fn file_path(&self) -> &str {
        return self.file_path.as_str();
    }
    fn current_line(&self) -> Option<i64> {
        return self.current_line;
    }
    fn offest(&self) -> u64 {
        let buffer_size = self.buffer.range().count();
        let data_size = self.buffer.data().len();
        return self.buffer.range().start
            + (self.view_offset as f64 * buffer_size as f64 / data_size as f64) as u64;
    }
    fn view(&mut self, nlines: u64) -> Result<&[u8], ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::from("exceeded max number of iterations building the current view"),
        );

        while !self
            .current_view()
            .iter()
            .filter(|&&x| x == b'\n')
            .nth(nlines as usize - 1)
            .is_some()
        {
            breaker.it()?;
            match self.load_next() {
                Ok(0) => break,
                Ok(_) => (),
                Err(e) => return Err(ViewError::from(e.to_string())),
            }
        }
        return Ok(self.current_view());
    }
    fn up(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::from("exceeded max number of iterations trying to go up"),
        );

        while lines > 0 {
            breaker.it()?;
            let view = self.above_view();
            let view = &view[..view.len().saturating_sub(1)]; // skip backline
            match view.iter().rposition(|&x| x == b'\n') {
                Some(pos) => {
                    self.view_offset = pos + 1;
                    self.current_line = self.current_line.map(|x| x - 1);
                    lines -= 1;
                    breaker.reset();
                }
                None => match self.load_prev() {
                    Ok(0) => {
                        let was_at_top = self.view_offset == 0;
                        self.view_offset = 0;
                        self.current_line = Some(0);
                        if was_at_top {
                            return Err(ViewError::from("already at the top"));
                        } else {
                            lines -= 1;
                        }
                    }
                    Ok(_) => (),
                    Err(e) => {
                        return Err(ViewError::from(e.to_string()));
                    }
                },
            }
        }
        return Ok(());
    }
    fn up_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError> {
        let state = self.save_state();

        loop {
            let m = regex.find_iter(self.above_view()).last();
            if let Some(m) = m {
                // align view_offset to a line start, use self.up
                // add 1 to the view offset in case the match is start of line already
                self.view_offset = m.start() + 1;
                self.up(1).ok();
                self.current_line = None;
                return Ok(());
            }

            self.view_offset = MATCH_WINDOW;

            match self.load_prev() {
                Ok(0) => {
                    self.load_state(&state);
                    return Err(ViewError::from("no match found"));
                }
                Err(e) => {
                    self.load_state(&state);
                    return Err(ViewError::from(e.to_string()));
                }
                Ok(_) => (),
            }
        }
    }
    fn down(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::from("exceeded max number of iterations trying to go down"),
        );

        while lines > 0 {
            breaker.it()?;
            match self.current_view().iter().position(|&x| x == b'\n') {
                Some(pos) => {
                    self.view_offset += pos + 1;
                    self.current_line = self.current_line.map(|x| x + 1);
                    lines -= 1;
                    breaker.reset();
                }
                None => match self.load_next() {
                    Ok(0) => return Err(ViewError::from("already at the bottom")),
                    Ok(_) => (),
                    Err(e) => return Err(ViewError::from(e.to_string())),
                },
            }
        }
        return Ok(());
    }
    fn down_to_line_matching(
        &mut self,
        regex: &Regex,
        skip_current: bool,
    ) -> Result<(), ViewError> {
        let state = self.save_state();
        if skip_current {
            self.down(1).ok();
        }

        loop {
            let m = regex.find(self.current_view());
            if let Some(m) = m {
                // align view_offset to a line start, use self.up
                // add 1 to the view offset in case the match is start of line already
                self.view_offset += m.start() + 1;
                self.up(1).ok();
                self.current_line = None;
                return Ok(());
            }

            self.view_offset += self.current_view().len().saturating_sub(MATCH_WINDOW);

            match self.load_next() {
                Ok(0) => {
                    self.load_state(&state);
                    return Err(ViewError::from("no match found"));
                }
                Err(e) => {
                    self.load_state(&state);
                    return Err(ViewError::from(e.to_string()));
                }
                Ok(_) => (),
            }
        }
    }
    fn jump_to_line(&mut self, line: u64) -> Result<(), ViewError> {
        if self.current_line.is_none() {
            self.top()
        }

        let offset = line as i64 - self.current_line().unwrap() as i64;
        return if offset > 0 {
            self.down(offset.abs() as u64)
        } else {
            self.up(offset.abs() as u64)
        };
    }
    fn jump_to_byte(&mut self, bytes: u64) {
        self.buffer.jump(bytes);
        self.view_offset = 0;
        self.current_line = None;
        self.up(1).ok();
    }
    fn top(&mut self) {
        self.jump_to_byte(0);
    }
    fn bottom(&mut self) {
        self.buffer.jump(self.buffer.total_size() - 1);
        self.view_offset = 0;
        self.current_line = Some(0);
    }
}
