use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use human_bytes::human_bytes;
use regex::{bytes, Regex};
use std::{io, panic, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::file_view::FileView;

struct Ui {
    file_view: Box<dyn FileView>,
    command: String,
    status: String,
    search_pattern: Option<Regex>,
    wrap: bool,
    align_bottom: bool,
    follow: bool,
}

impl Ui {
    fn new(file_view: Box<dyn FileView>) -> Self {
        Self {
            file_view,
            command: String::new(),
            status: String::new(),
            search_pattern: None,
            wrap: true,
            align_bottom: false,
            follow: false,
        }
    }
}

pub fn run(file_view: Box<dyn FileView>) -> io::Result<()> {
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

    let mut ui = Ui::new(file_view);
    loop {
        terminal.draw(|f| refresh(f, &mut ui))?;

        if crossterm::event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                ui.status.clear();
                let line_before = ui.file_view.current_line();
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
                    } => ui
                        .file_view
                        .down(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => ui
                        .file_view
                        .up(lines)
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => ui
                        .file_view
                        .down(term_size.height.into())
                        .unwrap_or_else(|e| ui.status = format!("{:?}", e)),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => ui
                        .file_view
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
                match ui.command.as_str() {
                    "q" => break,
                    "w" => ui.wrap = !ui.wrap,
                    "f" => ui.follow = !ui.follow,
                    "n" => {
                        if let Some(re) = ui.search_pattern.as_ref() {
                            ui.file_view.down(1).ok();
                            ui.file_view
                                .down_to_line_matching(&bytes::Regex::new(re.as_str()).unwrap())
                                .unwrap_or_else(|e| ui.status = format!("{:?}", e));
                        }
                    }
                    "N" => {
                        if let Some(re) = ui.search_pattern.as_ref() {
                            ui.file_view.up(1).ok();
                            ui.file_view
                                .up_to_line_matching(&bytes::Regex::new(re.as_str()).unwrap())
                                .unwrap_or_else(|e| ui.status = format!("{:?}", e));
                        }
                    }
                    "gg" => ui.file_view.top(),
                    "GG" => {
                        ui.file_view.bottom();
                        ui.file_view.up(term_size.height.into()).ok();
                        align_bottom = true;
                    }
                    x if x.to_lowercase() == "j" => {
                        ui.file_view
                            .down(lines)
                            .unwrap_or_else(|e| ui.status = format!("{:?}", e));
                    }
                    x if x.to_lowercase() == "k" => {
                        ui.file_view
                            .up(lines)
                            .unwrap_or_else(|e| ui.status = format!("{:?}", e));
                    }
                    x if x.to_lowercase().ends_with("gg") => {
                        x.get(..x.len() - 2)
                            .unwrap()
                            .parse::<u64>()
                            .map(|line| ui.file_view.jump_to_line(line))
                            .ok();
                    }
                    x if x.to_lowercase().ends_with("pp") => {
                        x.get(..x.len() - 2)
                            .unwrap()
                            .parse::<f64>()
                            .map(|percent| {
                                ui.file_view.jump_to_byte(
                                    (ui.file_view.file_size() as f64 * percent / 100.0) as u64,
                                )
                            })
                            .ok();
                    }
                    x if x.starts_with("/") && x.ends_with("\n") => {
                        let pattern = x.get(1..x.len() - 1).unwrap_or("");
                        if pattern.is_empty() {
                            ui.search_pattern = None;
                            break;
                        }
                        ui.search_pattern = Regex::new(pattern).map(|x| Some(x)).unwrap_or(None);
                        if let Some(re) = ui.search_pattern.as_ref() {
                            ui.file_view
                                .down_to_line_matching(&bytes::Regex::new(re.as_str()).unwrap())
                                .unwrap_or_else(|e| ui.status = format!("{:?}", e));
                        }
                    }
                    _ => done = ui.command.ends_with("\n"),
                }

                if done {
                    ui.command.clear()
                }

                if align_bottom {
                    ui.align_bottom = true;
                } else if ui.file_view.current_line() != line_before {
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

fn refresh<B: Backend>(f: &mut Frame<B>, ui: &mut Ui) {
    if ui.follow {
        ui.file_view.bottom();
        ui.file_view.up(f.size().height.into()).ok();
        ui.align_bottom = true;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Percentage(100)].as_ref())
        .split(f.size());

    let text = loop {
        let height: u64 = chunks[1].height.into();
        let mut text = match ui.file_view.view(height) {
            Ok(x) => x,
            Err(e) => {
                ui.status = format!("{:?}", e);
                ""
            }
        }
        .to_owned();
        if ui.wrap {
            text = wrap_text(text, chunks[1].width.into());
            if ui.align_bottom
                && text.lines().count() > height as usize
                && ui.file_view.down(1).is_ok()
            {
                continue;
            }
        }
        break text;
    };
    let text = match ui.search_pattern.as_ref() {
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
    if ui.follow {
        flags.push("Follow")
    }
    if ui.wrap {
        flags.push("Wrap")
    }

    let header = Text::from(format!(
        "Line {}, Offset {} ({:.1}%){}\n{}: {}",
        ui.file_view
            .current_line()
            .map(|x| x.to_string())
            .unwrap_or("?".to_owned()),
        human_bytes(ui.file_view.offest() as f64),
        100.0 * ui.file_view.offest() as f64 / ui.file_view.file_size() as f64,
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
                    ui.file_view.file_path(),
                    human_bytes(ui.file_view.file_size() as f64)
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
