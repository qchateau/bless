use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::{future::FutureExt, select, StreamExt};
use human_bytes::human_bytes;
use log::{debug, info};
use regex::Regex;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_async_std::Signals;
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
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
use unicode_width::UnicodeWidthStr;

use crate::{
    errors::Result,
    file_view::ViewError,
    ui::{
        backend::{BackendState, Command},
        errors::{ChannelError, FrontendError},
    },
    utils::{language::word_entropy, text::convert_tabs},
};

const FAST_SCROLL_LINES: i64 = 5;
const WORD_SEPARATOR: &str = "<>()[]{},;:='\",";
const HELP: &str = r#"
  MOVING

j, J, PageDown | Move down
k, K, PageUp   | Move up
l, L           | Move right
h, H           | Move left
<nr>gg         | Jump to line <nr>
<nr>pp         | Jump to <nr>th percent of the file
m<letter>      | Place marker <letter>
'<leter>       | Jump to marker <letter>


  SEARCHING

/pattern       | Jump to the first line matching "pattern"
n              | Jump to next match
N              | Jump to previous match


  DISPLAY / BEHAVIOR

w              | Toggle line wrap
f              | Follow updates
<nr>tw         | Set tab width to <nr>
cdef           | Default color mode
clog           | Color log mode
cent           | Color word entropy mode


  OTHER

Ctrl-C         | Cancel search, clear command, exit
Esc            | Cancel search, clear command
q              | Exit
?              | Show/hide this help
"#;

#[derive(PartialEq, Debug)]
enum ColorMode {
    Default,
    Log,
    Entropy,
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
    show_help: bool,
    last_sent_resize: Command,
    last_sent_command: RefCell<Command>,
    command_sender: RefCell<UnboundedSender<Command>>,
    cancel_sender: RefCell<UnboundedSender<()>>,
    state_receiver: Receiver<BackendState>,
    log_colors: Vec<(Regex, Style)>,
    entropy_colors: Vec<Style>,
    entropy_last_words: RefCell<Vec<(String, Style)>>,
}

impl Frontend {
    pub fn new(
        command_sender: UnboundedSender<Command>,
        cancel_sender: UnboundedSender<()>,
        state_receiver: Receiver<BackendState>,
    ) -> io::Result<Self> {
        let crossterm_backend = backend::CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(crossterm_backend)?;
        let log_colors = Frontend::make_log_colors();
        let entropy_colors = Frontend::make_entropy_colors();
        return Ok(Self {
            terminal: Some(terminal),
            command: String::new(),
            errors: RefCell::from(Vec::new()),
            last_sent_resize: Command::Resize(None, 0),
            last_sent_command: RefCell::from(Command::Resize(None, 0)),
            right_offset: 0,
            tab_width: 4,
            color_mode: ColorMode::Default,
            show_help: false,
            search: None,
            wrap: true,
            stop: false,
            follow: false,
            command_sender: RefCell::from(command_sender),
            cancel_sender: RefCell::from(cancel_sender),
            state_receiver,
            log_colors,
            entropy_colors,
            entropy_last_words: RefCell::from(Vec::new()),
        });
    }

    fn make_log_colors() -> Vec<(Regex, Style)> {
        return vec![
            (
                Regex::new("(?i)trace").unwrap(),
                Style::default().fg(Color::Cyan),
            ),
            (
                Regex::new("(?i)debug").unwrap(),
                Style::default().fg(Color::Green),
            ),
            (
                Regex::new("(?i)info").unwrap(),
                Style::default().fg(Color::Gray),
            ),
            (
                Regex::new("(?i)warn").unwrap(),
                Style::default().fg(Color::Yellow),
            ),
            (
                Regex::new("(?i)error").unwrap(),
                Style::default().fg(Color::Red),
            ),
            (
                Regex::new("(?i)fatal|critical").unwrap(),
                Style::default().fg(Color::LightRed),
            ),
        ];
    }

    fn make_entropy_colors() -> Vec<Style> {
        return vec![
            Style::default().fg(Color::LightRed),
            Style::default().fg(Color::LightYellow),
            Style::default().fg(Color::LightGreen),
            Style::default().fg(Color::LightCyan),
            Style::default().fg(Color::LightBlue),
            Style::default().fg(Color::LightMagenta),
            Style::default().fg(Color::Red),
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Green),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::Blue),
            Style::default().fg(Color::Magenta),
        ];
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
                if self.show_help {
                    self.show_help = false;
                } else if !self.command.is_empty() || self.search.is_some() {
                    self.command.clear();
                    self.search = None;
                    self.send_cancel();
                } else {
                    self.stop = true;
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
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.command.clear();
                    self.search = None;
                    self.send_cancel();
                }
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
            "?" => self.show_help = !self.show_help,
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
            "cent" => self.color_mode = ColorMode::Entropy,
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

        let text = if self.show_help {
            Text::from(HELP)
        } else {
            let lines: Vec<&str> = backend_text.iter().map(|x| x.as_ref()).collect();
            let mut lines = self.color_lines(lines);

            // for line in backend_text.iter().map(|x| x.as_ref()) {
            //     lines.push(self.color_line(line));
            // }

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
        } else if self.color_mode != ColorMode::Default {
            flags.push(format!("{:?}", self.color_mode))
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

    fn color_lines<'a>(&self, lines: Vec<&'a str>) -> Vec<Spans<'a>> {
        if let Some(re) = self.search.as_ref() {
            return lines
                .iter()
                .map(|lines| self.color_line_regex(lines, re))
                .collect();
        } else {
            match self.color_mode {
                ColorMode::Entropy => self.color_lines_entropy(lines),
                ColorMode::Log => lines
                    .iter()
                    .map(|lines| self.color_line_log(lines))
                    .collect(),
                _ => lines
                    .iter()
                    .map(|line| self.color_line_default(line))
                    .collect(),
            }
        }
    }

    fn color_line_regex<'a>(&self, mut line: &'a str, re: &Regex) -> Spans<'a> {
        let mut spans = Vec::new();

        while let Some(m) = re.find(line) {
            spans.push(Span::raw(&line[..m.start()]));
            spans.push(Span::styled(
                m.as_str(),
                Style::default().bg(Color::Yellow).fg(Color::Black),
            ));

            line = &line.get(m.end()..).unwrap_or("");
        }

        spans.push(Span::raw(line));
        return Spans::from(spans);
    }

    fn color_line_log<'a>(&self, line: &'a str) -> Spans<'a> {
        let mut spans = Vec::new();
        for (regex, style) in self.log_colors.iter() {
            if regex.is_match(line) {
                spans.push(Span::styled(line, style.clone()));
                return Spans::from(spans);
            }
        }
        spans.push(Span::raw(line));
        return Spans::from(spans);
    }

    fn color_lines_entropy<'a>(&self, lines: Vec<&'a str>) -> Vec<Spans<'a>> {
        // collect interesting words
        let word_regex = Regex::new(".*\\w").unwrap();
        let mut words_count: HashMap<&str, u64> = HashMap::new();
        for word in lines
            .iter()
            .map(|line| line.split_whitespace())
            .flatten()
            .map(|word| word.split(|x| WORD_SEPARATOR.contains(x)))
            .flatten()
            .filter(|word| word.len() >= 4)
            .map(|word| word_regex.find(word).map(|m| m.as_str()).unwrap_or(""))
        {
            *words_count.entry(word).or_default() += 1;
        }
        debug!("found {} interesting words", words_count.len());

        let mut words: Vec<String> = words_count
            .iter()
            .map(|(word, _)| word.to_string())
            .collect();
        words.sort_by_cached_key(|x| {
            (1000000.0 * word_entropy(x)) as u64 * words_count.get(x.as_str()).unwrap_or(&0)
        });
        words = words
            .into_iter()
            .rev()
            .take(self.entropy_colors.len())
            .collect();
        debug!("most interesting words: {:?}", words);

        let reused_words_styles: Vec<(String, Style)> = self
            .entropy_last_words
            .borrow()
            .iter()
            .filter(|(w, _)| words.contains(w))
            .cloned()
            .collect();
        let reused_styles: Vec<&Style> = reused_words_styles.iter().map(|(_, s)| s).collect();
        let reused_words: Vec<&String> = reused_words_styles.iter().map(|(w, _)| w).collect();

        let new_styles: Vec<Style> = self
            .entropy_colors
            .iter()
            .filter(|x| !reused_styles.contains(x))
            .cloned()
            .collect();
        let new_words: Vec<String> = words
            .iter()
            .filter(|x| !reused_words.contains(x))
            .cloned()
            .collect();

        let words: Vec<(String, Style)> = reused_words_styles
            .into_iter()
            .chain(new_words.into_iter().zip(new_styles.into_iter()))
            .collect();

        let mut res = Vec::new();
        for line in lines {
            res.push(self.color_words(line, &words));
        }
        *self.entropy_last_words.borrow_mut() = words;

        return res;
    }

    fn color_words<'a>(&self, line: &'a str, words: &Vec<(String, Style)>) -> Spans<'a> {
        let mut spans = Vec::new();
        let mut start = 0;
        let mut last_end = 0;
        loop {
            let slice = &line[start..];
            if slice.is_empty() {
                spans.push(Span::raw(&line[last_end..start]));
                break;
            };

            for (word, style) in words.iter() {
                if !slice.starts_with(word) {
                    continue;
                }

                if start != last_end {
                    spans.push(Span::raw(&line[last_end..start]));
                }
                spans.push(Span::styled(word.clone(), style.clone()));
                start += word.len();
                last_end = start;
                start -= 1;
                break;
            }

            start += 1;
            while !line.is_char_boundary(start) {
                start += 1;
            }
        }
        Spans::from(spans)
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
                } else if span.content.chars().count() <= offset_left {
                    offset_left -= span.content.chars().count()
                } else {
                    let content: String = span.content.chars().skip(offset_left).collect();
                    out_spans.push(Span::styled(content, span.style));
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
                    let content_width = UnicodeWidthStr::width(content);
                    if width_left >= content_width {
                        out_spans.push(Span::styled(content.to_string(), span.style));
                        width_left -= content_width;
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
