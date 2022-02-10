use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use human_bytes::human_bytes;
use regex::{bytes, Regex};
use signal_hook::{consts::TERM_SIGNALS, iterator::Signals};
use std::{fmt::Display, io, str::from_utf8, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::{errors::ViewError, file_view::FileView, utils::wrap_text};

const FAST_SCROLL_LINES: u64 = 5;

struct BiRegex {
    str: Regex,
    bytes: bytes::Regex,
}

impl BiRegex {
    fn new(pattern: &str) -> Result<Self, ViewError> {
        return Ok(Self {
            str: Regex::new(pattern).map_err(|_| ViewError::from("invalid regex"))?,
            bytes: bytes::Regex::new(pattern).map_err(|_| ViewError::from("invalid regex"))?,
        });
    }
}

pub struct Ui {
    file_view: Box<dyn FileView>,
    command: String,
    status: String,
    search_pattern: Option<BiRegex>,
    wrap: bool,
    align_bottom: bool,
    follow: bool,
    stop: bool,
}

impl Ui {
    pub fn new(file_view: Box<dyn FileView>) -> Self {
        return Self {
            file_view,
            command: String::new(),
            status: String::new(),
            search_pattern: None,
            wrap: true,
            align_bottom: false,
            follow: false,
            stop: false,
        };
    }

    fn handle_key(&mut self, term_size: &Rect, key: KeyEvent) -> Result<(), ViewError> {
        match key {
            KeyEvent {
                modifiers: KeyModifiers::CONTROL,
                code: KeyCode::Char('c'),
            } => {
                if self.command.is_empty() && self.search_pattern.is_none() {
                    self.stop = true;
                } else {
                    self.command.clear();
                    self.search_pattern = None;
                }
            }
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } => self.command.push(c),
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::SHIFT,
            } => self.file_view.down(FAST_SCROLL_LINES)?,
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => self.file_view.down(1)?,
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::SHIFT,
            } => self.file_view.up(FAST_SCROLL_LINES)?,
            KeyEvent {
                code: KeyCode::Up, ..
            } => self.file_view.up(1)?,
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => self.file_view.down(term_size.height.into())?,
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => self.file_view.up(term_size.height.into())?,
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.command.clear();
                self.search_pattern = None;
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.command.push('\n'),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.command.pop();
            }
            _ => (),
        };
        Ok(())
    }

    fn handle_command(&mut self, term_size: &Rect) -> Result<(), ViewError> {
        let line_before = self.file_view.current_line();
        let mut align_bottom = false;
        let mut done = true;
        let command = self.command.as_str().to_owned();

        let res = match command.as_str() {
            "q" => {
                self.stop = true;
                Ok(())
            }
            "w" => {
                self.wrap = !self.wrap;
                Ok(())
            }
            "f" => {
                self.follow = !self.follow;
                Ok(())
            }
            "n" => {
                if let Some(re) = self.search_pattern.as_ref() {
                    self.file_view.down_to_line_matching(&re.bytes, true)
                } else {
                    Err(ViewError::from("nothing to search"))
                }
            }
            "N" => {
                if let Some(re) = self.search_pattern.as_ref() {
                    self.file_view.up_to_line_matching(&re.bytes)
                } else {
                    Err(ViewError::from("nothing to search"))
                }
            }
            "gg" => {
                self.file_view.top();
                Ok(())
            }
            "GG" => {
                align_bottom = true;
                self.file_view.bottom();
                self.file_view.up(term_size.height.into())
            }
            "j" => self.file_view.down(1),
            "J" => self.file_view.down(FAST_SCROLL_LINES),
            "k" => self.file_view.up(1),
            "K" => self.file_view.up(FAST_SCROLL_LINES),
            x if x.to_lowercase().ends_with("gg") => x
                .get(..x.len() - 2)
                .unwrap()
                .parse::<u64>()
                .map_err(|_| ViewError::from("not a number"))
                .and_then(|line| self.file_view.jump_to_line(line)),
            x if x.to_lowercase().ends_with("pp") => x
                .get(..x.len() - 2)
                .unwrap()
                .parse::<f64>()
                .map_err(|_| ViewError::from("not a number"))
                .map(|percent| {
                    self.file_view
                        .jump_to_byte((self.file_view.file_size() as f64 * percent / 100.0) as u64)
                }),
            x if x.starts_with("/") && x.ends_with("\n") => {
                let pattern = x.get(1..x.len() - 1).unwrap_or("");
                if pattern.is_empty() {
                    self.search_pattern = None;
                    Ok(())
                } else {
                    BiRegex::new(pattern).and_then(|re| {
                        self.search_pattern = Some(re);
                        self.file_view.down_to_line_matching(
                            &self.search_pattern.as_ref().unwrap().bytes,
                            false,
                        )
                    })
                }
            }
            _ => {
                done = self.command.ends_with("\n");
                Ok(())
            }
        };

        if done {
            self.command.clear();
        }

        if align_bottom {
            self.align_bottom = true;
        } else if self.file_view.current_line() != line_before {
            self.align_bottom = false;
        }

        return res;
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut wait_signal = Signals::new(TERM_SIGNALS)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        while !self.stop {
            for sig in wait_signal.pending() {
                eprintln!("received singal {}", sig);
                return Ok(());
            }

            terminal.draw(|f| self.refresh(f))?;

            if crossterm::event::poll(Duration::from_millis(500))? {
                if let Event::Key(key) = event::read()? {
                    let term_size = terminal.size()?;
                    self.status.clear();
                    self.handle_key(&term_size, key)
                        .and_then(|_| self.handle_command(&term_size))
                        .unwrap_or_else(|e| self.set_error(e));
                }
            }
        }

        return Ok(());
    }

    fn refresh<B: Backend>(&mut self, f: &mut Frame<B>) {
        if self.follow {
            self.file_view.bottom();
            self.file_view.up(f.size().height.into()).ok();
            self.align_bottom = true;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
            .split(f.size());

        let text = loop {
            let height: u64 = chunks[1].height.into();
            let text = match self.file_view.view(height) {
                Ok(x) => x,
                Err(e) => {
                    self.set_error(e);
                    b""
                }
            };
            let mut text = from_utf8(text)
                .map(|x| x.to_owned())
                .unwrap_or_else(|e| format!("invalid utf-8: {:?}", e));

            if self.wrap {
                text = wrap_text(text, chunks[1].width.into());
                if self.align_bottom && text.lines().count() > height as usize {
                    match self.file_view.down(1) {
                        Ok(_) => continue,
                        Err(e) => self.set_error(e),
                    }
                }
            }
            break text;
        };
        let text = match self.search_pattern.as_ref() {
            None => Text::from(text),
            Some(re) => {
                let match_style = Style::default().bg(Color::DarkGray);
                let mut lines = Vec::new();
                for mut line in text.lines() {
                    let mut spans = Vec::new();
                    while let Some(m) = re.str.find(line) {
                        let before = &line[..m.start()];
                        spans.push(Span::raw(before));
                        spans.push(Span::styled(m.as_str(), match_style));
                        line = &line.get(m.end()..).unwrap_or("");
                    }
                    spans.push(Span::raw(line));
                    lines.push(Spans::from(spans))
                }
                Text::from(lines)
            }
        };

        let mut flags = Vec::new();
        if self.follow {
            flags.push("Follow".to_owned())
        }
        if self.wrap {
            flags.push("Wrap".to_owned())
        }
        if let Some(re) = &self.search_pattern {
            flags.push(format!("/{}", re.str.to_string()));
        }

        let header = Text::from(format!(
            "Line {}, Offset {} ({:.1}%){}\n{}",
            self.file_view
                .current_line()
                .map(|x| x.to_string())
                .unwrap_or("?".to_owned()),
            human_bytes(self.file_view.offest() as f64),
            100.0 * self.file_view.offest() as f64 / self.file_view.file_size() as f64,
            if flags.is_empty() {
                "".to_owned()
            } else {
                format!(", {}", flags.join(", "))
            },
            if self.status.is_empty() {
                format!("Command: {}", self.command)
            } else {
                format!("Status: {}", self.status)
            },
        ));

        let paragraph = Paragraph::new(header)
            .style(Style::default())
            .block(
                Block::default()
                    .title(format!(
                        "{} - {}",
                        self.file_view.file_path(),
                        human_bytes(self.file_view.file_size() as f64)
                    ))
                    .borders(Borders::ALL),
            )
            .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[0]);

        let paragraph = Paragraph::new(text)
            .style(Style::default())
            .block(Block::default())
            .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[1]);
    }

    fn set_error<D: Display>(&mut self, e: D) {
        self.status = format!("{}", e);
    }
}
