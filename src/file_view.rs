use crate::file_buffer::{FileBuffer, PlaintextFileBuffer};
use std::io;

pub trait FileView {
    fn file_path(&self) -> &str;
    fn file_size(&self) -> u64;
    fn current_line(&self) -> Option<u64>;
    fn offest(&self) -> u64;
    fn view(&mut self, nlines: u64) -> String;
    fn up(&mut self, lines: u64);
    fn down(&mut self, lines: u64);
    fn jump_to_line(&mut self, line: u64);
    fn jump_to_byte(&mut self, bytes: u64);
    fn top(&mut self);
    fn bottom(&mut self);
}

pub struct BufferedFileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<u64>,
}

impl BufferedFileView {
    pub fn new_plaintext(path: String) -> Result<Self, io::Error> {
        let buffer = PlaintextFileBuffer::new(path.as_str())?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(0),
        });
    }
}

impl FileView for BufferedFileView {
    fn file_size(&self) -> u64 {
        return self.buffer.total_size();
    }
    fn file_path(&self) -> &str {
        return self.file_path.as_str();
    }
    fn current_line(&self) -> Option<u64> {
        return self.current_line;
    }
    fn offest(&self) -> u64 {
        return self.buffer.offset() + self.view_offset as u64;
    }
    fn view(&mut self, nlines: u64) -> String {
        let mut is_end = false;
        loop {
            let view = self.buffer.data().get(self.view_offset..).unwrap_or(b"");
            let view = std::str::from_utf8(view);
            if view.is_err() {
                return format!("utf-8 error: {}", view.unwrap_err());
            }
            let view = view.unwrap();
            if is_end || view.lines().count() as u64 >= nlines {
                return view.to_owned();
            }
            is_end = self.buffer.load_next() == 0;
        }
    }
    fn up(&mut self, mut lines: u64) {
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
                    read_bytes if read_bytes > 0 => {
                        self.view_offset += read_bytes;
                    }
                    _ => {
                        self.view_offset = 0;
                        self.current_line = Some(0);
                        return;
                    }
                },
            }
        }
    }
    fn down(&mut self, mut lines: u64) {
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
                None => {
                    if self.buffer.load_next() == 0 {
                        return;
                    }
                }
            }
        }
    }
    fn jump_to_line(&mut self, line: u64) {
        if self.current_line.is_none() {
            self.top()
        }

        let offset = line as i64 - self.current_line().unwrap() as i64;
        if offset > 0 {
            self.down(offset.abs() as u64)
        } else {
            self.up(offset.abs() as u64)
        }
    }
    fn jump_to_byte(&mut self, bytes: u64) {
        self.buffer.jump(bytes);
        self.view_offset = 0;
        self.current_line = None;
        self.up(1);
    }
    fn top(&mut self) {
        self.jump_to_byte(0);
    }
    fn bottom(&mut self) {
        self.buffer.jump(self.buffer.total_size() - 1);
        self.view_offset = 0;
        self.current_line = None;
    }
}
