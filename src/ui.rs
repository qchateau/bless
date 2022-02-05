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

struct Ui {
    command: String,
    status: String,
    wrap: bool,
    align_bottom: bool,
    follow: bool,
}

impl Ui {
    fn new() -> Self {
        Self {
            command: String::new(),
            status: String::new(),
            wrap: true,
            align_bottom: false,
            follow: false,
        }
    }
}

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

    let mut ui = Ui::new();
    loop {
        terminal.draw(|f| refresh(f, &mut ui, file_view))?;

        if crossterm::event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                ui.status.clear();
                let line_before = file_view.current_line();
                let mut align_bottom = false;
                let term_size = terminal.size().unwrap();
                let lines = match key.modifiers {
                    KeyModifiers::SHIFT => 5,
                    _ => 1,
                };

                match key {
                    KeyEvent {
                        modifiers: KeyModifiers::CONTROL,
                        code: KeyCode::Char('c'),
                    } => {
                        if ui.command.is_empty() {
                            break;
                        } else {
                            ui.command.clear()
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } => ui.command.push(c),
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => file_view
                        .down(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => file_view
                        .up(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => file_view
                        .down(term_size.height.into())
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => file_view
                        .up(term_size.height.into())
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => ui.command.clear(),
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => ui.command.push('\n'),
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        ui.command.pop();
                    }
                    _ => (),
                }

                let mut done = true;
                if ui.command.to_lowercase() == "j" {
                    file_view
                        .down(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e))
                } else if ui.command.to_lowercase() == "k" {
                    file_view
                        .up(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e))
                } else if ui.command == "w" {
                    ui.wrap = !ui.wrap
                } else if ui.command == "f" {
                    ui.follow = !ui.follow;
                } else if ui.command == "GG" {
                    file_view.bottom();
                    file_view.up(term_size.height.into()).ok();
                    align_bottom = true;
                } else if ui.command == "gg" {
                    file_view.top();
                } else if ui.command.to_lowercase().ends_with("gg") {
                    ui.command
                        .get(..ui.command.len() - 2)
                        .unwrap()
                        .parse::<u64>()
                        .map(|line| file_view.jump_to_line(line))
                        .ok();
                } else if ui.command.to_lowercase().ends_with("pp") {
                    ui.command
                        .get(..ui.command.len() - 2)
                        .unwrap()
                        .parse::<f64>()
                        .map(|percent| {
                            file_view.jump_to_byte(
                                (file_view.file_size() as f64 * percent / 100.0) as u64,
                            )
                        })
                        .ok();
                } else if ui.command == "q" {
                    break;
                } else {
                    done = ui.command.ends_with("\n")
                }

                if done {
                    ui.command.clear()
                }

                if align_bottom {
                    ui.align_bottom = true;
                } else if file_view.current_line() != line_before {
                    ui.align_bottom = false;
                }
            }
        }
    }

    // restore terminal
    restore_terminal()?;
    return Ok(());
}

fn wrap_text(text: String, width: usize) -> String {
    let mut lines = Vec::new();
    for mut line in text.lines() {
        while line.len() > width {
            lines.push(line.get(..width).unwrap());
            line = &line[width..];
        }
        lines.push(line);
    }
    return lines.join("\n");
}

fn refresh<B: Backend>(f: &mut Frame<B>, ui: &mut Ui, file_view: &mut dyn FileView) {
    if ui.follow {
        file_view.bottom();
        file_view.up(f.size().height.into()).ok();
        ui.align_bottom = true;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
        .split(f.size());

    let text = loop {
        let height: u64 = chunks[1].height.into();
        let mut text = match file_view.view(height) {
            Ok(x) => x,
            Err(e) => {
                ui.status = format!("{:?}", e);
                String::new()
            }
        };
        if ui.wrap {
            text = wrap_text(text, chunks[1].width.into());
            if ui.align_bottom
                && text.lines().count() > height as usize
                && file_view.down(1).is_ok()
            {
                continue;
            }
        }
        break text;
    };

    let mut flags = Vec::new();
    if ui.follow {
        flags.push("Follow")
    }
    if ui.wrap {
        flags.push("Wrap")
    }

    let header = Text::from(format!(
        "Line {}, Offset {} ({:.1}%){}\n{}: {}",
        file_view
            .current_line()
            .map(|x| x.to_string())
            .unwrap_or("?".to_owned()),
        human_bytes(file_view.offest() as f64),
        100.0 * file_view.offest() as f64 / file_view.file_size() as f64,
        if flags.is_empty() {
            "".to_owned()
        } else {
            format!(", {}", flags.join(", "))
        },
        if ui.status.is_empty() {
            "Command"
        } else {
            "Status"
        },
        if ui.status.is_empty() {
            &ui.command
        } else {
            &ui.status
        },
    ));

    let paragraph = Paragraph::new(header)
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

    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .block(Block::default())
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[1]);
}
