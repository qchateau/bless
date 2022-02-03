use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, fs, io, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

#[derive(Parser)]
struct Args {
    /// Path to the file to read
    path: String,
}

struct App {
    data: String,
    offset: usize,
    command: String,
}

impl App {
    fn new(data: String) -> App {
        App {
            data,
            offset: 0,
            command: String::new(),
        }
    }
    fn view(&self) -> &str {
        return self.data.get(self.offset..).unwrap_or("");
    }
    fn up(&mut self) {
        if self.offset == 0 {
            return;
        }
        let before = self.data.get(..self.offset - 1).unwrap_or("");
        self.offset = match before.rfind('\n') {
            Some(pos) => pos + 1,
            None => 0,
        }
    }
    fn down(&mut self) {
        self.offset += match self.view().find('\n') {
            Some(pos) => pos + 1,
            None => 0,
        }
    }
    fn top(&mut self) {
        self.offset = 0;
    }
    fn bottom(&mut self) {
        self.offset = self.data.rfind('\n').unwrap_or(0);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // read file
    let args = Args::parse();

    let data = fs::read_to_string(args.path).expect("Something went wrong reading the file");

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let app = App::new(data);
    let res = run_app(&mut terminal, app);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if crossterm::event::poll(Duration::from_secs(1))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Down => app.down(),
                    KeyCode::Up => app.up(),
                    KeyCode::Char(c) => app.command.push(c),
                    _ => (),
                }
            }
            let mut done = true;
            match app.command.as_str() {
                "GG" => app.bottom(),
                "gg" => app.top(),
                _ => done = false,
            }
            if done {
                app.command.clear()
            }
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Percentage(10), Constraint::Percentage(90)].as_ref())
        .split(f.size());

    let text = tui::text::Text::from("Command: ".to_owned() + app.command.as_str());
    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(Block::default().title("Header").borders(Borders::ALL))
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[0]);

    let text = tui::text::Text::from(app.view());
    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(Block::default())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, chunks[1]);
}
