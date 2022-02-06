use crate::file_buffer::{bzip2, raw, FileBuffer};
use regex::bytes::Regex;
use std::io;

const MATCH_WINDOW: usize = 0xffff;

pub trait FileView {
    fn file_path(&self) -> &str;
    fn file_size(&self) -> u64;
    fn current_line(&self) -> Option<i64>;
    fn offest(&self) -> u64;
    fn view(&mut self, nlines: u64) -> Result<&str, ViewError>;
    fn up(&mut self, lines: u64) -> Result<(), ViewError>;
    fn up_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError>;
    fn down(&mut self, lines: u64) -> Result<(), ViewError>;
    fn down_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError>;
    fn jump_to_line(&mut self, line: u64) -> Result<(), ViewError>;
    fn jump_to_byte(&mut self, bytes: u64);
    fn top(&mut self);
    fn bottom(&mut self);
}

#[derive(Debug, Clone)]
pub struct ViewError {
    msg: String,
}

impl From<String> for ViewError {
    fn from(msg: String) -> Self {
        Self { msg }
    }
}

impl From<&str> for ViewError {
    fn from(msg: &str) -> Self {
        Self {
            msg: String::from(msg),
        }
    }
}

pub struct BufferedFileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<i64>,
}

impl BufferedFileView {
    pub fn new_plaintext(path: String) -> Result<Self, io::Error> {
        let buffer = raw::FileBuffer::new(path.as_str())?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(0),
        });
    }
    pub fn new_bzip2(path: String) -> Result<Self, io::Error> {
        let buffer = bzip2::FileBuffer::new(path.as_str())?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(0),
        });
    }
    fn current_view(&self) -> &[u8] {
        return self.buffer.data().get(self.view_offset..).unwrap_or(b"");
    }
    fn above_view(&self) -> &[u8] {
        return self.buffer.data().get(..self.view_offset).unwrap_or(b"");
    }
    fn shrink(&mut self) {
        let removed = self
            .buffer
            .shrink_around(self.view_offset as u64 + self.buffer.range().start);
        self.view_offset -= removed.before as usize;
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
    fn view(&mut self, nlines: u64) -> Result<&str, ViewError> {
        let mut i = 10;

        while !self
            .current_view()
            .iter()
            .filter(|&&x| x == b'\n')
            .nth(nlines as usize - 1)
            .is_some()
        {
            i -= 1;
            if i <= 0 {
                return Err(ViewError::from("exceeded max number of iterations"));
            }
            if self.buffer.load_next().is_none() {
                break;
            }
        }

        let string = std::str::from_utf8(self.current_view()).unwrap_or("invalid utf-8");
        return Ok(string);
    }
    fn up(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut i = 10;

        while lines > 0 {
            let above = self
                .buffer
                .data()
                .get(..self.view_offset.saturating_sub(1))
                .unwrap();
            match above.iter().rposition(|&x| x == b'\n') {
                Some(pos) => {
                    self.view_offset = pos + 1;
                    self.current_line = self.current_line.map(|x| x - 1);
                    lines -= 1;
                }
                None => match self.buffer.load_prev() {
                    Some(read_bytes) => {
                        self.view_offset += read_bytes;
                        self.shrink();
                        i -= 1;
                        if i <= 0 {
                            return Err(ViewError::from("exceeded max number of iterations"));
                        }
                    }
                    None => {
                        let was_at_top = self.view_offset == 0;
                        self.view_offset = 0;
                        self.current_line = Some(0);
                        if was_at_top {
                            return Err(ViewError::from("already at the top"));
                        } else {
                            lines -= 1;
                        }
                    }
                },
            }
        }
        return Ok(());
    }
    fn up_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError> {
        loop {
            let m = regex.find_iter(self.above_view()).last();
            if let Some(m) = m {
                self.view_offset = m.start();
                self.up(1).ok();
                break;
            }
            let loaded = self.buffer.load_prev();
            self.view_offset = loaded.unwrap_or(0) + MATCH_WINDOW;
            if loaded.is_none() {
                break;
            }
        }

        self.shrink();
        while !self
            .view(1)
            .map(|x| regex.is_match(x.lines().nth(0).unwrap().as_bytes()))
            .unwrap_or(false)
        {
            self.up(1)?;
        }
        return Ok(());
    }
    fn down(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut i = 10;

        while lines > 0 {
            match self
                .buffer
                .data()
                .get(self.view_offset..)
                .unwrap_or(b"")
                .iter()
                .position(|&x| x == b'\n')
            {
                Some(pos) => {
                    self.view_offset += pos + 1;
                    self.current_line = self.current_line.map(|x| x + 1);
                    lines -= 1;
                }
                None => match self.buffer.load_next() {
                    None => {
                        return Err(ViewError::from("already at the bottom"));
                    }
                    Some(_) => {
                        self.shrink();
                        i -= 1;
                        if i <= 0 {
                            return Err(ViewError::from("exceeded max number of iterations"));
                        }
                    }
                },
            }
        }
        return Ok(());
    }
    fn down_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError> {
        loop {
            let m = regex.find(self.current_view());
            if let Some(m) = m {
                self.view_offset = m.start();
                self.up(1).ok();
                break;
            }

            let loaded = self.buffer.load_next();
            self.view_offset = self
                .buffer
                .data()
                .len()
                .saturating_sub(loaded.unwrap_or(0) + MATCH_WINDOW);
            if loaded.is_none() {
                break;
            }
        }

        self.shrink();
        while !self
            .view(1)
            .map(|x| x.lines().nth(0))
            .map(|x| x.map(|x| regex.is_match(x.as_bytes())).unwrap_or(false))
            .unwrap_or(false)
        {
            self.down(1)?;
        }

        return Ok(());
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
