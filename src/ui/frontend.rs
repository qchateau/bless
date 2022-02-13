use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::{future::FutureExt, select, StreamExt};
use human_bytes::human_bytes;
use regex::Regex;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_async_std::Signals;
use std::{
    cell::RefCell,
    io::{self, Stdout},
};
use tokio::sync::{mpsc::UnboundedSender, watch::Receiver};
use tui::{
    backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::{
    errors::ViewError,
    ui::backend::{BackendState, Command},
};

const FAST_SCROLL_LINES: i64 = 5;

pub struct Frontend {
    terminal: Option<Terminal<backend::CrosstermBackend<Stdout>>>,
    command: String,
    errors: RefCell<Vec<String>>,
    search: Option<Regex>,
    wrap: bool,
    stop: bool,
    follow: bool,
    align_bottom: bool,
    allow_refresh_commands: bool,
    command_sender: RefCell<UnboundedSender<Command>>,
    state_receiver: Receiver<BackendState>,
}

impl Frontend {
    pub fn new(
        command_sender: UnboundedSender<Command>,
        state_receiver: Receiver<BackendState>,
    ) -> io::Result<Self> {
        let crossterm_backend = backend::CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(crossterm_backend)?;
        return Ok(Self {
            terminal: Some(terminal),
            command: String::new(),
            errors: RefCell::from(Vec::new()),
            search: None,
            wrap: true,
            stop: false,
            align_bottom: false,
            allow_refresh_commands: false,
            follow: false,
            command_sender: RefCell::from(command_sender),
            state_receiver,
        });
    }
    pub async fn run(&mut self) -> Result<(), ViewError> {
        let mut events_reader = EventStream::new();
        let mut signals_reader = Signals::new(TERM_SIGNALS)
            .map_err(|e| ViewError::from(format!("failed to install signal handler: {}", e)))?;

        while !self.stop {
            self.update()?;

            select! {
                maybe_event = events_reader.next().fuse() => match maybe_event {
                    Some(Ok(Event::Key(key))) => self.handle_key(key),
                    Some(Ok(Event::Resize(_, height))) => self.send_command(Command::Resize(height as u64)),
                    Some(Ok(_)) => {},
                    Some(Err(e)) => return Err(ViewError::from(format!("event error: {}", e))),
                    None => return Err(ViewError::from("end of event stream: {}")),
                },
                maybe_state = self.state_receiver.changed().fuse() => match maybe_state {
                    Ok(_) => (),
                    Err(e) => return Err(ViewError::from(format!("channel error: {}", e)))
                },
                maybe_signal = signals_reader.next().fuse() => match maybe_signal {
                    Some(signal) => {
                        eprintln!("received signal {}", signal);
                        return Ok(());
                    },
                    None => return Err(ViewError::from("signal handler interrupted"))
                },
            }
        }

        return Ok(());
    }

    fn update(&mut self) -> Result<(), ViewError> {
        let mut terminal = self.terminal.take().unwrap();
        terminal.draw(|f| self.refresh(f)).unwrap();
        self.terminal = Some(terminal);
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let height = self.terminal.as_ref().unwrap().size().unwrap().height as i64;
        let mut command_done = true;

        match key {
            KeyEvent {
                modifiers: KeyModifiers::CONTROL,
                code: KeyCode::Char('c'),
            } => {
                if self.command.is_empty() && self.search.is_none() {
                    self.stop = true;
                } else {
                    self.command.clear();
                    self.search = None;
                }
            }
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } => self.command.push(c),
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::SHIFT,
            } => {
                self.send_command(Command::MoveLine(FAST_SCROLL_LINES));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.send_command(Command::MoveLine(1));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::SHIFT,
            } => {
                self.send_command(Command::MoveLine(-FAST_SCROLL_LINES));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.send_command(Command::MoveLine(-1));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => {
                self.send_command(Command::MoveLine(height));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => {
                self.send_command(Command::MoveLine(-height));
                self.align_bottom = false;
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.command.clear();
                self.search = None;
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

        match self.command.as_str() {
            "q" => self.stop = true,
            "w" => {
                self.wrap = !self.wrap;
                self.allow_refresh_commands = true;
            }
            "f" => {
                self.follow = !self.follow;
                if self.follow {
                    self.align_bottom = true;
                    self.allow_refresh_commands = true;
                }
                self.send_command(Command::Follow(self.follow));
            }
            "n" => {
                if let Some(re) = self.search.as_ref() {
                    self.send_command(Command::SearchDownNext(re.as_str().to_owned()));
                } else {
                    self.push_error("nothing to search".to_owned());
                }
                self.align_bottom = false;
            }
            "N" => {
                if let Some(re) = self.search.as_ref() {
                    self.send_command(Command::SearchUp(re.as_str().to_owned()));
                } else {
                    self.push_error("nothing to search".to_owned());
                }
                self.align_bottom = false;
            }
            "gg" => {
                self.send_command(Command::JumpLine(0));
                self.align_bottom = false;
            }
            "GG" => {
                self.send_command(Command::JumpLine(-height));
                self.align_bottom = true;
                self.allow_refresh_commands = true;
            }
            "j" => {
                self.send_command(Command::MoveLine(1));
                self.align_bottom = false;
            }
            "J" => {
                self.send_command(Command::MoveLine(FAST_SCROLL_LINES));
                self.align_bottom = false;
            }
            "k" => {
                self.send_command(Command::MoveLine(-1));
                self.align_bottom = false;
            }
            "K" => {
                self.send_command(Command::MoveLine(-FAST_SCROLL_LINES));
                self.align_bottom = false;
            }
            x if x.to_lowercase().ends_with("gg") => {
                if let Ok(line) = x.get(..x.len() - 2).unwrap().parse::<i64>() {
                    self.send_command(Command::JumpLine(line));
                    self.align_bottom = false;
                } else {
                    self.push_error("not a number".to_owned());
                }
            }
            x if x.to_lowercase().ends_with("pp") => {
                if let Ok(jump_pos_percent) = x.get(..x.len() - 2).unwrap().parse::<f64>() {
                    self.send_command(Command::JumpFileRatio(jump_pos_percent / 100.0));
                    self.align_bottom = false;
                } else {
                    self.push_error("not a number".to_owned());
                }
            }
            x if x.starts_with("/") && x.ends_with("\n") => {
                let pattern = x.get(1..x.len() - 1).unwrap_or("");
                if pattern.is_empty() {
                    self.search = None;
                } else if let Ok(re) =
                    Regex::new(pattern).map_err(|_| ViewError::from("invalid regex"))
                {
                    self.search = Some(re);
                    self.send_command(Command::SearchDown(pattern.to_string()));
                } else {
                    self.push_error("invalid regex".to_owned());
                }
                self.align_bottom = false;
            }
            _ => command_done = self.command.ends_with("\n"),
        };

        if command_done {
            self.command.clear();
        }
    }

    fn refresh<B: backend::Backend>(&mut self, f: &mut Frame<B>) {
        let back = self.state_receiver.borrow();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
            .split(f.size());

        let text_width = chunks[1].width as usize;
        let text_height = chunks[1].height as usize;
        let mut view_lines_per_line = Vec::new();

        let text = {
            let mut lines = Vec::new();

            for mut line in back.text.lines() {
                let lines_before = lines.len();
                let mut spans = Vec::new();

                macro_rules! handle_line {
                    ($data:expr, $style:expr) => {
                        if self.wrap {
                            while let Some((char_pos, _)) = $data.char_indices().nth(
                                text_width - spans.iter().fold(0, |acc, x: &Span| acc + x.width()),
                            ) {
                                let (left, right) = $data.split_at(char_pos);
                                spans.push(Span::styled(left, $style));
                                lines.push(Spans::from(spans.clone()));
                                spans.clear();
                                $data = right;
                            }
                        }
                        spans.push(Span::styled($data, $style));
                    };
                }

                if let Some(re) = self.search.as_ref() {
                    while let Some(m) = re.find(line) {
                        let mut before = &line[..m.start()];
                        let mut match_content = m.as_str();

                        handle_line![before, Style::default()];
                        handle_line![match_content, Style::default().bg(Color::DarkGray)];

                        line = &line.get(m.end()..).unwrap_or("");
                    }
                }
                handle_line![line, Style::default()];

                lines.push(Spans::from(spans));
                view_lines_per_line.push(lines.len() - lines_before);
            }
            Text::from(lines)
        };

        // wait for the curent line to be negative, otherwise it means we did not jump to bottom
        if self.align_bottom && self.allow_refresh_commands && back.current_line.unwrap_or(0) < 0 {
            let mut extra_lines = text.height() as i64 - text_height as i64;
            let mut move_lines = 0;
            for &out_lines in view_lines_per_line.iter() {
                if extra_lines <= 0 {
                    break;
                }
                extra_lines -= out_lines as i64;
                move_lines += 1;
            }
            if move_lines > 0 {
                self.send_command(Command::JumpLine(back.current_line.unwrap() + move_lines));
            }

            let mut missing_lines = text_height as i64 - text.height() as i64;
            let mut move_lines = 0;
            for &out_lines in view_lines_per_line.iter() {
                missing_lines -= out_lines as i64;
                if missing_lines < 0 {
                    break;
                }
                move_lines += 1;
            }
            if move_lines > 0 {
                self.send_command(Command::JumpLine(back.current_line.unwrap() - move_lines));
            }

            self.allow_refresh_commands = false;
        }

        let mut flags = Vec::new();
        if back.follow {
            flags.push("Follow".to_owned())
        }
        if self.wrap {
            flags.push("Wrap".to_owned())
        }
        if let Some(re) = &self.search {
            flags.push(format!("/{}", re.to_string()));
        }

        let header = Text::from(format!(
            "Line {}, Offset {} ({:.1}%){}\n{}",
            back.current_line
                .map(|x| x.to_string())
                .unwrap_or("?".to_owned()),
            human_bytes(back.offset as f64),
            100.0 * back.offset as f64 / back.file_size as f64,
            if flags.is_empty() {
                "".to_owned()
            } else {
                format!(", {}", flags.join(", "))
            },
            if !back.errors.is_empty() {
                format!("Backend error: {}", back.errors.join(", "))
            } else if !self.errors.borrow().is_empty() {
                format!("Frontend error: {}", self.errors.borrow().join(", "))
            } else {
                format!("Command: {}", self.command)
            },
        ));

        let paragraph = Paragraph::new(header)
            .style(Style::default())
            .block(
                Block::default()
                    .title(format!(
                        "{} - {}",
                        back.file_path,
                        human_bytes(back.file_size as f64)
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

    fn send_command(&self, command: Command) {
        if let Err(e) = self.command_sender.borrow_mut().send(command) {
            self.push_error(format!("command channel error: {}", e));
        }
    }

    fn push_error(&self, error: String) {
        self.errors.borrow_mut().push(error);
    }
}
