use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use human_bytes::human_bytes;
use std::{
    cmp::min,
    error::Error,
    fmt,
    fs::File,
    io::{self, Seek, SeekFrom},
    os::unix::fs::FileExt,
    panic,
    time::Duration,
};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

const BUFFER_SIZE: usize = 0xffff;

#[derive(Parser)]
struct Args {
    /// Path to the file to read
    path: String,
}

struct App {
    file_path: String,
    file: File,
    buffer_offset: usize,
    buffer: Vec<u8>,
    view_offset: usize,
    current_line: Option<usize>,
    command: String,
    status: String,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("file", &self.file)
            .field("buffer_offset", &self.buffer_offset)
            .field("view_offset", &self.view_offset)
            .field("command", &self.command)
            .field("status", &self.status)
            .finish()
    }
}

impl App {
    fn new(path: String) -> Result<App, Box<dyn Error>> {
        let file = File::open(&path)?;
        return Ok(App {
            file_path: path,
            file,
            buffer_offset: 0,
            buffer: Vec::new(),
            view_offset: 0,
            current_line: Some(0),
            command: String::new(),
            status: String::new(),
        });
    }
    fn load_prev(&mut self) -> usize {
        let shift = min(self.buffer_offset, BUFFER_SIZE);

        let mut prev_buffer = vec![0u8; shift];

        let read_bytes = loop {
            let read_bytes = self
                .file
                .read_at(
                    prev_buffer.as_mut_slice(),
                    (self.buffer_offset - shift).try_into().unwrap(),
                )
                .unwrap();
            if read_bytes > 0 {
                break read_bytes;
            }
            let file_size = file_size(&mut self.file);
            self.buffer_offset = file_size.try_into().unwrap();
            if file_size == 0 {
                break 0;
            }
        };
        prev_buffer.resize(read_bytes, 0);
        self.status = format!("read {} bytes at {} (prev)", self.buffer_offset, read_bytes);

        prev_buffer.append(&mut self.buffer);
        self.buffer = prev_buffer;
        self.buffer_offset -= read_bytes;
        self.view_offset += read_bytes;
        return read_bytes;
    }
    fn load_next(&mut self) -> usize {
        let mut next_buffer = vec![0u8; BUFFER_SIZE];
        let read_bytes = self
            .file
            .read_at(
                next_buffer.as_mut_slice(),
                (self.buffer_offset + self.buffer.len()).try_into().unwrap(),
            )
            .unwrap();
        next_buffer.resize(read_bytes, 0);
        self.status = format!("read {} bytes at {} (next)", read_bytes, self.buffer_offset);

        let read_size = next_buffer.len();
        self.buffer.append(&mut next_buffer);
        return read_size;
    }
    fn offest(&self) -> usize {
        return self.buffer_offset + self.view_offset;
    }
    fn view(&mut self, nlines: usize) -> String {
        let mut is_end = false;
        loop {
            let view = self.buffer.get(self.view_offset..).unwrap_or(b"");
            let view = std::str::from_utf8(view);
            if view.is_err() {
                return format!("utf-8 error: {}", view.unwrap_err());
            }
            let view = view.unwrap();
            if is_end || view.lines().count() >= nlines {
                return view.to_owned();
            }
            is_end = self.load_next() == 0;
        }
    }
    fn up(&mut self, mut lines: usize) {
        while lines > 0 {
            let above = self
                .buffer
                .get(..self.view_offset.saturating_sub(1))
                .unwrap();
            match above.iter().rposition(|&x| x == b'\n') {
                Some(pos) => {
                    self.status = "scrolled up without loading".to_owned();
                    self.view_offset = pos + 1;
                    self.current_line = self.current_line.map(|x| x - 1);
                    lines -= 1;
                }
                None => {
                    if self.buffer_offset == 0 {
                        self.view_offset = 0;
                        self.current_line = Some(0);
                        return;
                    } else {
                        self.load_prev();
                    }
                }
            }
        }
    }
    fn down(&mut self, mut lines: usize) {
        while lines > 0 {
            match self
                .buffer
                .get(self.view_offset..)
                .unwrap_or(b"")
                .iter()
                .position(|&x| x == b'\n')
            {
                Some(pos) => {
                    self.status = "scrolled down without loading".to_owned();
                    self.view_offset += pos + 1;
                    self.current_line = self.current_line.map(|x| x + 1);
                    lines -= 1;
                }
                None => {
                    if self.load_next() == 0 {
                        return;
                    }
                }
            }
        }
    }
    fn do_move(&mut self, lines: i64) {
        if lines > 0 {
            self.down(lines.try_into().unwrap())
        } else {
            self.up((-lines).try_into().unwrap())
        }
    }
    fn jump(&mut self, line: usize) {
        if self.current_line.is_none() {
            self.top()
        }

        self.do_move(line as i64 - self.current_line.unwrap() as i64)
    }
    fn jump_bytes(&mut self, bytes: usize) {
        self.buffer = Vec::new();
        self.buffer_offset = bytes;
        self.view_offset = 0;
        self.current_line = None;
        self.up(1);
    }
    fn top(&mut self) {
        self.jump_bytes(0);
    }
    fn bottom(&mut self) {
        self.buffer = Vec::new();
        self.buffer_offset = self
            .file
            .seek(std::io::SeekFrom::End(0))
            .unwrap_or(0)
            .try_into()
            .unwrap();
        self.view_offset = 0;
        self.current_line = None;
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // read file
    let args = Args::parse();

    // setup terminal
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    let restore_terminal = || -> Result<(), Box<dyn Error>> {
        // restore terminal
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        return Ok(());
    };

    let default_panic = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_terminal().unwrap_or_else(|err| println!("Error restoring terminal: {:?}", err));
        default_panic(panic_info);
    }));

    // create app and run it
    let app = App::new(args.path)?;
    let res = run_app(&mut terminal, app);

    // restore terminal
    restore_terminal()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn file_size(file: &mut File) -> u64 {
    let pos = file.stream_position().unwrap_or(0);
    let end = file.seek(SeekFrom::End(0)).unwrap_or(0);
    file.seek(SeekFrom::Start(pos)).ok();
    return end;
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if crossterm::event::poll(Duration::from_secs(1))? {
            if let Event::Key(key) = event::read()? {
                let term_size = terminal.size().unwrap();
                let lines = match key.modifiers {
                    KeyModifiers::SHIFT => 5,
                    _ => 1,
                };

                match key {
                    KeyEvent {
                        modifiers: KeyModifiers::CONTROL,
                        code: KeyCode::Char('c'),
                    } => app.command.clear(),
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } => app.command.push(c),
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => app.down(lines),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => app.up(lines),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => app.down(term_size.height.into()),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => app.up(term_size.height.into()),
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => app.command.clear(),
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => app.command.push('\n'),
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        app.command.pop();
                    }
                    _ => (),
                }

                let file_size = file_size(&mut app.file);
                let mut done = true;
                if app.command.to_lowercase() == "j" {
                    app.down(lines)
                } else if app.command.to_lowercase() == "k" {
                    app.up(lines)
                } else if app.command == "GG" {
                    app.bottom();
                } else if app.command == "gg" {
                    app.top();
                } else if app.command.to_lowercase().ends_with("gg") {
                    app.command
                        .get(..app.command.len() - 2)
                        .unwrap()
                        .parse::<usize>()
                        .map(|line| app.jump(line))
                        .ok();
                } else if app.command.to_lowercase().ends_with("pp") {
                    app.command
                        .get(..app.command.len() - 2)
                        .unwrap()
                        .parse::<f64>()
                        .map(|percent| {
                            app.jump_bytes((file_size as f64 * percent / 100.0) as usize)
                        })
                        .ok();
                } else if app.command == "q" {
                    return Ok(());
                } else {
                    done = app.command.ends_with("\n")
                }

                if done {
                    app.command.clear()
                }
            }
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
        .split(f.size());

    let text = Text::from(format!(
        "Line {}, Offset {} ({:.1}%)\nCommand: {}",
        app.current_line
            .map(|x| x.to_string())
            .unwrap_or("?".to_owned()),
        human_bytes(app.offest() as f64),
        100.0 * app.offest() as f64 / file_size(&mut app.file) as f64,
        app.command
    ));
    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(
            Block::default()
                .title(format!(
                    "{} - {}",
                    app.file_path.as_str(),
                    human_bytes(file_size(&mut app.file) as f64)
                ))
                .borders(Borders::ALL),
        )
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[0]);

    let text = loop {
        let height: usize = chunks[1].height.into();
        let text = app.view(height);
        let missing = height.saturating_sub(text.lines().count().into());
        if app.current_line == Some(0) || missing <= 0 {
            break text;
        }
        app.up(missing);
    };

    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(Block::default())
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[1]);
}
