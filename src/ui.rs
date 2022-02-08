use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use human_bytes::human_bytes;
use regex::{bytes, Regex};
use signal_hook::{consts::TERM_SIGNALS, iterator::Signals};
use std::{fmt::Display, io, str::from_utf8, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::{errors::ViewError, file_view::FileView, utils::wrap_text};

pub struct Ui {
    file_view: Box<dyn FileView>,
    command: String,
    status: String,
    search_pattern: Option<Regex>,
    wrap: bool,
    align_bottom: bool,
    follow: bool,
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
        };
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut wait_signal = Signals::new(TERM_SIGNALS)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        loop {
            for sig in wait_signal.pending() {
                eprintln!("received singal {}", sig);
                return Ok(());
            }

            terminal.draw(|f| self.refresh(f))?;

            if crossterm::event::poll(Duration::from_millis(500))? {
                if let Event::Key(key) = event::read()? {
                    self.status.clear();
                    let line_before = self.file_view.current_line();
                    let mut align_bottom = false;
                    let mut reset = false;
                    let term_size = terminal.size().unwrap();
                    let lines = match key.modifiers {
                        KeyModifiers::SHIFT => 5,
                        _ => 1,
                    };

                    let res1 = match key {
                        KeyEvent {
                            modifiers: KeyModifiers::CONTROL,
                            code: KeyCode::Char('c'),
                        } => {
                            if self.command.is_empty() {
                                break;
                            } else {
                                reset = true;
                            }
                            Ok(())
                        }
                        KeyEvent {
                            code: KeyCode::Char(c),
                            ..
                        } => {
                            self.command.push(c);
                            Ok(())
                        }
                        KeyEvent {
                            code: KeyCode::Down,
                            ..
                        } => self.file_view.down(lines),
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => self.file_view.up(lines),
                        KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        } => self.file_view.down(term_size.height.into()),
                        KeyEvent {
                            code: KeyCode::PageUp,
                            ..
                        } => self.file_view.up(term_size.height.into()),
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        } => {
                            reset = true;
                            Ok(())
                        }
                        KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            self.command.push('\n');
                            Ok(())
                        }
                        KeyEvent {
                            code: KeyCode::Backspace,
                            ..
                        } => {
                            self.command.pop();
                            Ok(())
                        }
                        _ => Ok(()),
                    };

                    let mut done = true;
                    let res2 = match self.command.as_str() {
                        "q" => break,
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
                                self.file_view.down_to_line_matching(
                                    &bytes::Regex::new(re.as_str()).unwrap(),
                                    true,
                                )
                            } else {
                                Err(ViewError::from("nothing to search"))
                            }
                        }
                        "N" => {
                            if let Some(re) = self.search_pattern.as_ref() {
                                self.file_view
                                    .up_to_line_matching(&bytes::Regex::new(re.as_str()).unwrap())
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
                        x if x.to_lowercase() == "j" => self.file_view.down(lines),
                        x if x.to_lowercase() == "k" => self.file_view.up(lines),
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
                                self.file_view.jump_to_byte(
                                    (self.file_view.file_size() as f64 * percent / 100.0) as u64,
                                )
                            }),
                        x if x.starts_with("/") && x.ends_with("\n") => {
                            let pattern = x.get(1..x.len() - 1).unwrap_or("");
                            if pattern.is_empty() {
                                self.search_pattern = None;
                                break;
                            }
                            match (Regex::new(pattern), bytes::Regex::new(pattern)) {
                                (Ok(unicode_re), Ok(bytes_re)) => {
                                    self.search_pattern = Some(unicode_re);
                                    self.file_view.down_to_line_matching(&bytes_re, false)
                                }
                                _ => Err(ViewError::from("invalid regex")),
                            }
                        }
                        _ => {
                            done = self.command.ends_with("\n");
                            Ok(())
                        }
                    };

                    if reset {
                        self.command.clear();
                        self.search_pattern = None;
                    }

                    if done {
                        self.command.clear()
                    }

                    if align_bottom {
                        self.align_bottom = true;
                    } else if self.file_view.current_line() != line_before {
                        self.align_bottom = false;
                    }

                    match res1.and_then(|_| res2) {
                        Err(e) => {
                            self.set_error(e);
                            continue;
                        }
                        _ => (),
                    }
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
                    while let Some(m) = re.find(line) {
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
            flags.push(format!("/{}", re.to_string()));
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
