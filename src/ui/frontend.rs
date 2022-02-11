use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::{future::FutureExt, select, StreamExt};
use human_bytes::human_bytes;
use regex::Regex;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_async_std::Signals;
use std::io::{self, Stdout};
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
    terminal: Terminal<backend::CrosstermBackend<Stdout>>,
    state: FrontendState,
    command_sender: UnboundedSender<Command>,
    state_receiver: Receiver<BackendState>,
}

#[derive(Clone)]
struct FrontendState {
    command: String,
    error: Option<String>,
    search: Option<Regex>,
    wrap: bool,
    follow: bool,
    stop: bool,
}

impl FrontendState {
    fn new() -> Self {
        return Self {
            command: String::new(),
            error: None,
            search: None,
            wrap: true,
            follow: false,
            stop: false,
        };
    }
}

impl Frontend {
    pub fn new(
        command_sender: UnboundedSender<Command>,
        state_receiver: Receiver<BackendState>,
    ) -> io::Result<Self> {
        let crossterm_backend = backend::CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(crossterm_backend)?;
        return Ok(Self {
            state: FrontendState::new(),
            terminal,
            command_sender,
            state_receiver,
        });
    }
    pub async fn run(&mut self) -> Result<(), ViewError> {
        let mut events_reader = EventStream::new();
        let mut signals_reader = Signals::new(TERM_SIGNALS)
            .map_err(|e| ViewError::from(format!("failed to install signal handler: {}", e)))?;

        while !self.state.stop {
            self.update();

            select! {
                maybe_event = events_reader.next().fuse() => match maybe_event {
                    Some(Ok(Event::Key(key))) => match self.handle_key(key) {
                        Err(e) => self.state.error = Some(format!("{}", e)),
                        Ok(_) => (),
                    },
                    Some(Err(e)) => return Err(ViewError::from(format!("event error: {}", e))),
                    _ => return Err(ViewError::from("end of event stream: {}")),
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

    fn update(&mut self) {
        let backend_state = self.state_receiver.borrow();
        let frontend_state = &self.state;
        self.terminal
            .draw(|f| Frontend::refresh(f, &frontend_state, &backend_state))
            .unwrap();
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<(), ViewError> {
        let commands = self.key_to_commands(key)?;
        for command in commands {
            self.command_sender
                .send(command)
                .map_err(|e| ViewError::from(format!("command channel error: {}", e)))?;
        }
        Ok(())
    }

    fn key_to_commands(&mut self, key: KeyEvent) -> Result<Vec<Command>, ViewError> {
        let height = self.terminal.size().unwrap().height as i64;

        let mut commands = Vec::new();
        let mut command_done = true;
        let mut align_bottom = false;

        match key {
            KeyEvent {
                modifiers: KeyModifiers::CONTROL,
                code: KeyCode::Char('c'),
            } => {
                if self.state.command.is_empty() && self.state.search.is_none() {
                    self.state.stop = true;
                } else {
                    self.state.command.clear();
                    self.state.search = None;
                }
            }
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } => self.state.command.push(c),
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::SHIFT,
            } => commands.push(Command::MoveLine(FAST_SCROLL_LINES)),
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => commands.push(Command::MoveLine(1)),
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::SHIFT,
            } => commands.push(Command::MoveLine(-FAST_SCROLL_LINES)),
            KeyEvent {
                code: KeyCode::Up, ..
            } => commands.push(Command::MoveLine(-1)),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => commands.push(Command::MoveLine(height)),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => commands.push(Command::MoveLine(-height)),
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.state.command.clear();
                self.state.search = None;
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => self.state.command.push('\n'),
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.state.command.pop();
            }
            _ => (),
        };

        match self.state.command.as_str() {
            "q" => self.state.stop = true,
            "w" => self.state.wrap = !self.state.wrap,
            "f" => {
                self.state.follow = !self.state.follow;
                commands.push(Command::Follow(self.state.follow));
            }
            "n" => {
                if let Some(re) = self.state.search.as_ref() {
                    commands.push(Command::SearchDownNext(re.as_str().to_owned()));
                } else {
                    return Err(ViewError::from("nothing to search"));
                }
            }
            "N" => {
                if let Some(re) = self.state.search.as_ref() {
                    commands.push(Command::SearchUp(re.as_str().to_owned()));
                } else {
                    return Err(ViewError::from("nothing to search"));
                }
            }
            "gg" => commands.push(Command::JumpLine(0)),
            "GG" => {
                align_bottom = true;
                commands.push(Command::JumpLine(-height));
            }
            "j" => commands.push(Command::MoveLine(1)),
            "J" => commands.push(Command::MoveLine(FAST_SCROLL_LINES)),
            "k" => commands.push(Command::MoveLine(-1)),
            "K" => commands.push(Command::MoveLine(-FAST_SCROLL_LINES)),
            x if x.to_lowercase().ends_with("gg") => {
                let line = x
                    .get(..x.len() - 2)
                    .unwrap()
                    .parse::<i64>()
                    .map_err(|_| ViewError::from("not a number"))?;
                commands.push(Command::JumpLine(line));
            }
            x if x.to_lowercase().ends_with("pp") => {
                let jump_pos_percent = x
                    .get(..x.len() - 2)
                    .unwrap()
                    .parse::<f64>()
                    .map_err(|_| ViewError::from("not a number"))?;
                commands.push(Command::JumpFileRatio(jump_pos_percent / 100.0));
            }
            x if x.starts_with("/") && x.ends_with("\n") => {
                let pattern = x.get(1..x.len() - 1).unwrap_or("");
                if pattern.is_empty() {
                    self.state.search = None;
                } else {
                    let re = Regex::new(pattern).map_err(|_| ViewError::from("invalid regex"))?;
                    self.state.search = Some(re);
                    commands.push(Command::SearchDown(pattern.to_string()));
                }
            }
            _ => command_done = self.state.command.ends_with("\n"),
        };

        if command_done {
            self.state.command.clear();
        }

        return Ok(commands);
    }

    fn refresh<B: backend::Backend>(f: &mut Frame<B>, front: &FrontendState, back: &BackendState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
            .split(f.size());

        let text = match front.search.as_ref() {
            None => Text::from(back.text.clone()),
            Some(re) => {
                let match_style = Style::default().bg(Color::DarkGray);
                let mut lines = Vec::new();
                for mut line in back.text.lines() {
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
        if back.follow {
            flags.push("Follow".to_owned())
        }
        if front.wrap {
            flags.push("Wrap".to_owned())
        }
        if let Some(re) = &front.search {
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
            if !back.status.is_empty() {
                format!("Backend error: {}", back.status)
            } else if let Some(e) = front.error.as_ref() {
                format!("Frontend error: {}", e)
            } else {
                format!("Command: {}", front.command)
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
}
