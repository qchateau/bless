use crate::{
    errors::ViewError,
    file_buffer::{make_file_buffer, FileBuffer},
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
pub struct FileView {
    file_path: String,
    buffer: Box<dyn FileBuffer>,
    view_offset: usize,
    current_line: Option<i64>,
}

impl FileView {
    pub async fn new(path: String) -> Result<Self, io::Error> {
        let buffer = make_file_buffer(&path).await?;
        return Ok(Self {
            file_path: path,
            buffer: Box::from(buffer),
            view_offset: 0,
            current_line: Some(0),
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
    pub async fn view(&mut self, nlines: u64) -> Result<&[u8], ViewError> {
        eprintln!("building view for {} lines", nlines);
        while !self
            .current_view()
            .iter()
            .filter(|&&x| x == b'\n')
            .nth(nlines.saturating_sub(1) as usize)
            .is_some()
        {
            match self.load_next().await {
                Ok(0) => break,
                Ok(_) => (),
                Err(e) => return Err(ViewError::from(e.to_string())),
            }
        }
        return Ok(self.current_view());
    }
    pub async fn up(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::from("exceeded max number of iterations trying to go up"),
        );

        eprintln!("up {}", lines);
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
                None => match self.load_prev().await {
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
    pub async fn up_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError> {
        eprintln!("up to line matching {}", regex.as_str());

        let state = self.save_state();

        loop {
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
    pub async fn down(&mut self, mut lines: u64) -> Result<(), ViewError> {
        let mut breaker = InfiniteLoopBreaker::new(
            10,
            ViewError::from("exceeded max number of iterations trying to go down"),
        );

        eprintln!("down {}", lines);
        while lines > 0 {
            breaker.it()?;
            match self.current_view().iter().position(|&x| x == b'\n') {
                Some(pos) => {
                    self.view_offset += pos + 1;
                    self.current_line = self.current_line.map(|x| x + 1);
                    lines -= 1;
                    breaker.reset();
                }
                None => match self.load_next().await {
                    Ok(0) => return Err(ViewError::from("already at the bottom")),
                    Ok(_) => (),
                    Err(e) => return Err(ViewError::from(e.to_string())),
                },
            }
        }
        return Ok(());
    }
    pub async fn down_to_line_matching(
        &mut self,
        regex: &Regex,
        skip_current: bool,
    ) -> Result<(), ViewError> {
        eprintln!("down to line matching {}", regex.as_str());

        let state = self.save_state();
        if skip_current {
            self.down(1).await.ok();
        }

        loop {
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
    pub async fn jump_to_line(&mut self, line: i64) -> Result<(), ViewError> {
        eprintln!("jump to line {}", line);

        if line >= 0 && (self.current_line.is_none() || self.current_line.unwrap() < 0) {
            self.top().await?
        } else if line < 0 && (self.current_line.is_none() || self.current_line.unwrap() >= 0) {
            self.bottom().await
        }

        let offset = line - self.current_line().unwrap();
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

        self.buffer.jump(bytes);
        self.view_offset = 0;

        if bytes == 0 {
            self.current_line = Some(0);
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
    pub async fn bottom(&mut self) {
        eprintln!("jump to bottom");

        self.buffer.jump(self.buffer.total_size().await - 1);
        self.view_offset = 0;
        self.current_line = Some(0);
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
    async fn load_next(&mut self) -> io::Result<usize> {
        let load_size = self.buffer.load_next().await?;
        self.maybe_shrink();
        eprintln!("loaded {} next bytes", load_size);
        return Ok(load_size);
    }
    async fn load_prev(&mut self) -> io::Result<usize> {
        let load_size = self.buffer.load_prev().await?;
        self.view_offset += load_size;
        self.maybe_shrink();
        eprintln!("loaded {} previous bytes", load_size);
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
