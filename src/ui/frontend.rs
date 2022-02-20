use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::{future::FutureExt, select, StreamExt};
use human_bytes::human_bytes;
use log::info;
use regex::Regex;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_async_std::Signals;
use std::{
    borrow::Cow,
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
    errors::Result,
    file_view::ViewError,
    ui::{
        backend::{BackendState, Command},
        errors::{ChannelError, FrontendError},
    },
    utils::text::convert_tabs,
};

const FAST_SCROLL_LINES: i64 = 5;

enum ColorMode {
    Default,
    Log,
    Similarity,
}

pub struct Frontend {
    terminal: Option<Terminal<backend::CrosstermBackend<Stdout>>>,
    command: String,
    errors: RefCell<Vec<String>>,
    search: Option<Regex>,
    wrap: bool,
    stop: bool,
    follow: bool,
    right_offset: usize,
    tab_width: usize,
    color_mode: ColorMode,
    last_sent_resize: Command,
    last_sent_command: RefCell<Command>,
    command_sender: RefCell<UnboundedSender<Command>>,
    cancel_sender: RefCell<UnboundedSender<()>>,
    state_receiver: Receiver<BackendState>,
}

impl Frontend {
    pub fn new(
        command_sender: UnboundedSender<Command>,
        cancel_sender: UnboundedSender<()>,
        state_receiver: Receiver<BackendState>,
    ) -> io::Result<Self> {
        let crossterm_backend = backend::CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(crossterm_backend)?;
        return Ok(Self {
            terminal: Some(terminal),
            command: String::new(),
            errors: RefCell::from(Vec::new()),
            last_sent_resize: Command::Resize(None, 0),
            last_sent_command: RefCell::from(Command::Resize(None, 0)),
            right_offset: 0,
            tab_width: 4,
            color_mode: ColorMode::Default,
            search: None,
            wrap: true,
            stop: false,
            follow: false,
            command_sender: RefCell::from(command_sender),
            cancel_sender: RefCell::from(cancel_sender),
            state_receiver,
        });
    }

    fn update_backend_size(&mut self, width: usize, height: usize) {
        let cmd = Command::Resize(if self.wrap { Some(width) } else { None }, height);
        if cmd != self.last_sent_resize {
            self.last_sent_resize = cmd;
            self.send_command(self.last_sent_resize.clone());
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut events_reader = EventStream::new();
        let mut signals_reader = Signals::new(TERM_SIGNALS)?;

        let term_size = self.terminal.as_ref().unwrap().size().unwrap();
        self.update_backend_size(term_size.width.into(), term_size.height.into());

        while !self.stop {
            self.update()?;

            select! {
                maybe_event = events_reader.next().fuse() => match maybe_event {
                    Some(Ok(Event::Key(key))) => self.handle_key(key),
                    Some(Ok(Event::Resize(_, height))) => self.send_command(Command::Resize(None, height as usize)),
                    Some(Ok(_)) => {},
                    Some(Err(e)) => return Err(e.into()),
                    None => return Err(FrontendError::EndOfEventStream.into()),
                },
                maybe_state = self.state_receiver.changed().fuse() => match maybe_state {
                    Ok(_) => (),
                    Err(_) => return Err(ChannelError::State.into())
                },
                maybe_signal = signals_reader.next().fuse() => match maybe_signal {
                    Some(signal) => {
                        info!("received signal {}", signal);
                        return Ok(());
                    },
                    None => return Err(FrontendError::EndOfSignalStream.into())
                },
            }
        }

        return Ok(());
    }

    fn update(&mut self) -> Result<()> {
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
                    self.send_cancel();
                }
            }
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } => self.command.push(c),
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::SHIFT,
            } => self.send_command(Command::MoveLine(FAST_SCROLL_LINES)),
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => self.send_command(Command::MoveLine(1)),
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::SHIFT,
            } => self.send_command(Command::MoveLine(-FAST_SCROLL_LINES)),
            KeyEvent {
                code: KeyCode::Up, ..
            } => self.send_command(Command::MoveLine(-1)),
            KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::SHIFT,
            } => self.right_offset += FAST_SCROLL_LINES as usize,
            KeyEvent {
                code: KeyCode::Right,
                ..
            } => self.right_offset += 1,
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::SHIFT,
            } => self.right_offset = self.right_offset.saturating_sub(FAST_SCROLL_LINES as usize),
            KeyEvent {
                code: KeyCode::Left,
                ..
            } => self.right_offset = self.right_offset.saturating_sub(1),
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => self.send_command(Command::MoveLine(height)),
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => self.send_command(Command::MoveLine(-height)),
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.command.clear();
                self.search = None;
                self.send_cancel();
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
                self.right_offset = 0;
            }
            "f" => {
                self.follow = !self.follow;
                self.send_command(Command::Follow(self.follow));
            }
            "n" => {
                if let Some(re) = self.search.as_ref() {
                    self.send_command(Command::SearchDownNext(re.as_str().to_owned()));
                } else {
                    self.push_error("nothing to search".to_owned());
                }
            }
            "N" => {
                if let Some(re) = self.search.as_ref() {
                    self.send_command(Command::SearchUp(re.as_str().to_owned()));
                } else {
                    self.push_error("nothing to search".to_owned());
                }
            }
            "gg" => self.send_command(Command::JumpLine(1)),
            "GG" => self.send_command(Command::JumpLine(-1)),
            "j" => self.send_command(Command::MoveLine(1)),
            "J" => self.send_command(Command::MoveLine(FAST_SCROLL_LINES)),
            "k" => self.send_command(Command::MoveLine(-1)),
            "K" => self.send_command(Command::MoveLine(-FAST_SCROLL_LINES)),
            "l" => self.right_offset += 1,
            "L" => self.right_offset += FAST_SCROLL_LINES as usize,
            "h" => self.right_offset = self.right_offset.saturating_sub(1),
            "H" => self.right_offset = self.right_offset.saturating_sub(FAST_SCROLL_LINES as usize),
            "clog" => self.color_mode = ColorMode::Log,
            "csim" => self.color_mode = ColorMode::Similarity,
            "cdef" => self.color_mode = ColorMode::Default,
            x if x.starts_with("m") && x.len() > 1 => {
                self.send_command(Command::SaveMark(String::from(&x[1..2])))
            }
            x if x.starts_with("'") && x.len() > 1 => {
                self.send_command(Command::LoadMark(String::from(&x[1..2])))
            }
            x if x.to_lowercase().ends_with("gg") => {
                if let Ok(line) = x.get(..x.len() - 2).unwrap().parse::<i64>() {
                    self.send_command(Command::JumpLine(line))
                } else {
                    self.push_error("not a number".to_owned());
                }
            }
            x if x.to_lowercase().ends_with("pp") => {
                if let Ok(jump_pos_percent) = x.get(..x.len() - 2).unwrap().parse::<f64>() {
                    self.send_command(Command::JumpFileRatio(jump_pos_percent / 100.0))
                } else {
                    self.push_error("not a number".to_owned());
                }
            }
            x if x.starts_with("/") && x.ends_with("\n") => {
                let pattern = x.get(1..x.len() - 1).unwrap_or("");
                if pattern.is_empty() {
                    self.search = None;
                } else if let Ok(re) = Regex::new(pattern).map_err(|_| ViewError::InvalidRegex) {
                    self.search = Some(re);
                    self.send_command(Command::SearchDown(pattern.to_string()));
                } else {
                    self.push_error("invalid regex".to_owned());
                }
            }
            x if x.ends_with("tw") => {
                if let Ok(width) = x.get(..x.len() - 2).unwrap().parse::<usize>() {
                    self.tab_width = width
                } else {
                    self.push_error("not a number".to_owned());
                }
            }
            _ => command_done = self.command.ends_with("\n"),
        };

        if command_done {
            self.command.clear();
        }
    }

    fn refresh<B: backend::Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
            .split(f.size());

        let text_width = chunks[1].width as usize;
        let text_height = chunks[1].height as usize;
        self.update_backend_size(text_width, text_height);

        let back = self.state_receiver.borrow();
        let backend_text = convert_tabs(
            back.text.iter().map(|x| Cow::from(x)).collect(),
            self.tab_width,
        );

        let text = {
            let mut lines = Vec::new();

            for line in backend_text.iter().map(|x| x.as_ref()) {
                lines.push(self.color_line(line));
            }

            if self.right_offset > 0 {
                lines = self.shift_lines(lines, self.right_offset);
            }

            if self.wrap {
                lines = self.wrap_lines(lines, text_width);
            }

            Text::from(lines)
        };

        let mut flags = Vec::new();
        if back.follow {
            flags.push("Follow".to_owned())
        }
        if self.wrap {
            flags.push("Wrap".to_owned())
        }
        if !back.marks.is_empty() {
            flags.push(format!("Marks: {}", back.marks.join("")));
        }
        if let Some(re) = &self.search {
            flags.push(format!("/{}", re.to_string()));
        }

        let header_title = format!(
            "{} - {}",
            back.real_file_path,
            human_bytes(back.file_size as f64)
        );
        let header = Text::from(
            [
                format!(
                    "Line {}, Offset {} ({:.1}%){}",
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
                ),
                self.build_status(&back),
            ]
            .join("\n"),
        );

        let paragraph = Paragraph::new(header)
            .style(Style::default())
            .block(Block::default().title(header_title).borders(Borders::ALL))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[0]);

        let paragraph = Paragraph::new(text)
            .style(Style::default())
            .block(Block::default())
            .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[1]);
    }

    fn build_status(&self, back: &BackendState) -> String {
        // Go over all backend errors and remove what's irrelevant
        // to the user
        let back_errors = back
            .errors
            .iter()
            .filter(|x| match x.downcast_ref::<ViewError>() {
                Some(ViewError::EOF) | Some(ViewError::BOF) => {
                    matches![*self.last_sent_command.borrow(), Command::MoveLine(_)]
                }
                _ => true,
            })
            .map(|x| format!("{}", x))
            .collect::<Vec<String>>();

        if !self.command.is_empty() {
            format!("Command: {}", self.command)
        } else if !back_errors.is_empty() {
            format!("Backend error: {}", back_errors.join(", "))
        } else if !self.errors.borrow().is_empty() {
            format!("Frontend error: {}", self.errors.borrow().join(", "))
        } else {
            "".to_string()
        }
    }

    fn color_line<'a>(&self, line: &'a str) -> Spans<'a> {
        if let Some(re) = self.search.as_ref() {
            self.color_line_regex(line, re)
        } else {
            match self.color_mode {
                ColorMode::Log => self.color_line_log(line),
                ColorMode::Similarity => self.color_line_similariry(line),
                _ => self.color_line_default(line),
            }
        }
    }

    fn color_line_regex<'a>(&self, mut line: &'a str, re: &Regex) -> Spans<'a> {
        let mut spans = Vec::new();

        while let Some(m) = re.find(line) {
            spans.push(Span::raw(&line[..m.start()]));
            spans.push(Span::styled(
                m.as_str(),
                Style::default().bg(Color::DarkGray),
            ));

            line = &line.get(m.end()..).unwrap_or("");
        }

        spans.push(Span::raw(line));
        return Spans::from(spans);
    }

    fn color_line_log<'a>(&self, line: &'a str) -> Spans<'a> {
        self.color_line_default(line)
    }

    fn color_line_similariry<'a>(&self, line: &'a str) -> Spans<'a> {
        self.color_line_default(line)
    }

    fn color_line_default<'a>(&self, line: &'a str) -> Spans<'a> {
        let mut spans = Vec::new();
        spans.push(Span::raw(line));
        Spans::from(spans)
    }

    fn shift_lines<'a>(&self, lines: Vec<Spans<'a>>, offset: usize) -> Vec<Spans<'a>> {
        let mut out_lines = Vec::new();
        for spans in lines {
            let mut out_spans = Vec::new();
            let mut offset_left = offset;

            for span in spans.0 {
                if offset_left == 0 {
                    out_spans.push(span);
                } else if span.content.len() <= offset_left {
                    offset_left -= span.content.len()
                } else {
                    out_spans.push(Span::styled(
                        (&span.content[offset_left..]).to_string(),
                        span.style,
                    ));
                    offset_left = 0;
                }
            }

            out_lines.push(Spans::from(out_spans));
        }
        return out_lines;
    }

    fn wrap_lines<'a>(&self, lines: Vec<Spans<'a>>, width: usize) -> Vec<Spans<'a>> {
        let mut out_lines = Vec::new();
        let mut out_spans = Vec::new();

        for spans in lines {
            let mut width_left = width;
            for span in spans.0 {
                let mut content = span.content.as_ref();
                while !content.is_empty() {
                    if width_left >= content.len() {
                        out_spans.push(Span::styled(content.to_string(), span.style));
                        width_left -= content.len();
                        content = "";
                    } else {
                        let (left, right) = content.split_at(width_left);
                        out_spans.push(Span::styled(left.to_string(), span.style));
                        content = right;

                        out_lines.push(Spans::from(out_spans));
                        out_spans = Vec::new();
                        width_left = width;
                    }
                }
            }
            out_lines.push(Spans::from(out_spans));
            out_spans = Vec::new();
        }

        return out_lines;
    }

    fn send_command(&self, command: Command) {
        if let Err(e) = self.command_sender.borrow_mut().send(command.clone()) {
            self.push_error(format!("command channel error: {}", e));
        }
        *self.last_sent_command.borrow_mut() = command;
    }

    fn send_cancel(&self) {
        if let Err(e) = self.cancel_sender.borrow_mut().send(()) {
            self.push_error(format!("cancel channel error: {}", e));
        }
    }

    fn push_error(&self, error: String) {
        self.errors.borrow_mut().push(error);
    }
}
