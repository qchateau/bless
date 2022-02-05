use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use human_bytes::human_bytes;
use std::{io, panic, time::Duration};
use tui::backend::CrosstermBackend;
use tui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::file_view::FileView;

pub fn run(file_view: &mut dyn FileView) -> io::Result<()> {
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    let restore_terminal = || -> io::Result<()> {
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

    let mut command = String::new();
    loop {
        terminal.draw(|f| refresh(f, &command, file_view))?;

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
                    } => command.clear(),
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } => command.push(c),
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => file_view.down(lines),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => file_view.up(lines),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => file_view.down(term_size.height.into()),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => file_view.up(term_size.height.into()),
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => command.clear(),
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => command.push('\n'),
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        command.pop();
                    }
                    _ => (),
                }

                let mut done = true;
                if command.to_lowercase() == "j" {
                    file_view.down(lines)
                } else if command.to_lowercase() == "k" {
                    file_view.up(lines)
                } else if command == "GG" {
                    file_view.bottom();
                } else if command == "gg" {
                    file_view.top();
                } else if command.to_lowercase().ends_with("gg") {
                    command
                        .get(..command.len() - 2)
                        .unwrap()
                        .parse::<u64>()
                        .map(|line| file_view.jump_to_line(line))
                        .ok();
                } else if command.to_lowercase().ends_with("pp") {
                    command
                        .get(..command.len() - 2)
                        .unwrap()
                        .parse::<f64>()
                        .map(|percent| {
                            file_view.jump_to_byte(
                                (file_view.file_size() as f64 * percent / 100.0) as u64,
                            )
                        })
                        .ok();
                } else if command == "q" {
                    break;
                } else {
                    done = command.ends_with("\n")
                }

                if done {
                    command.clear()
                }
            }
        }
    }

    // restore terminal
    restore_terminal()?;
    return Ok(());
}

fn refresh<B: Backend>(f: &mut Frame<B>, command: &str, file_view: &mut dyn FileView) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
        .split(f.size());

    let text = Text::from(format!(
        "Line {}, Offset {} ({:.1}%)\nCommand: {}",
        file_view
            .current_line()
            .map(|x| x.to_string())
            .unwrap_or("?".to_owned()),
        human_bytes(file_view.offest() as f64),
        100.0 * file_view.offest() as f64 / file_view.file_size() as f64,
        command
    ));
    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(
            Block::default()
                .title(format!(
                    "{} - {}",
                    file_view.file_path(),
                    human_bytes(file_view.file_size() as f64)
                ))
                .borders(Borders::ALL),
        )
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[0]);

    let text = loop {
        let height: u64 = chunks[1].height.into();
        let text = file_view.view(height);
        let missing = height.saturating_sub(text.lines().count() as u64);
        if file_view.current_line() == Some(0) || missing <= 0 {
            break text;
        }
        file_view.up(missing);
    };

    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(Block::default())
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[1]);
}
