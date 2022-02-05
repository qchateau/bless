use crate::file_buffer::{bzip2, raw, FileBuffer};
use std::{io, str::Utf8Error};

pub trait FileView {
    fn file_path(&self) -> &str;
    fn file_size(&self) -> u64;
    fn current_line(&self) -> Option<i64>;
    fn offest(&self) -> u64;
    fn view(&mut self, nlines: u64) -> String;
    fn up(&mut self, lines: u64) -> Result<(), ViewError>;
    fn down(&mut self, lines: u64) -> Result<(), ViewError>;
    fn jump_to_line(&mut self, line: u64) -> Result<(), ViewError>;
    fn jump_to_byte(&mut self, bytes: u64);
    fn top(&mut self);
    fn bottom(&mut self);
}

pub struct BufferedFileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ViewError {
    msg: String,
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
    fn current_view(&self) -> Result<&str, Utf8Error> {
        return std::str::from_utf8(self.buffer.data().get(self.view_offset..).unwrap_or(b""));
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
    fn view(&mut self, nlines: u64) -> String {
        let mut is_end = false;
        let mut i = 10;

        return loop {
            let view = match self.current_view() {
                Err(err) => return format!("utf-8 error: {}", err),
                Ok(view) => view,
            };
            if view.lines().count() as u64 >= nlines {
                return view.to_owned();
            }

            i -= 1;
            if is_end || i <= 0 {
                break view.to_owned();
            }
            is_end = self.buffer.load_next().is_none();
        };
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
                        i -= 1;
                        if i <= 0 {
                            return Err(ViewError {
                                msg: format!(
                                    "exceeded max number of iterations, last read: {}",
                                    read_bytes
                                ),
                            });
                        }
                    }
                    None => {
                        self.view_offset = 0;
                        self.current_line = Some(0);
                        return Err(ViewError {
                            msg: "already at the top".to_owned(),
                        });
                    }
                },
            }
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
                        return Err(ViewError {
                            msg: "already at the bottom".to_owned(),
                        })
                    }
                    _ => {
                        i -= 1;
                        if i <= 0 {
                            return Err(ViewError {
                                msg: "exceeded max number of iterations".to_owned(),
                            });
                        }
                    }
                },
            }
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
