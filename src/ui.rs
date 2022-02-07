use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use human_bytes::human_bytes;
use regex::{bytes, Regex};
use std::{fmt::Display, io, panic, str::from_utf8, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

use crate::{errors::ViewError, file_view::FileView};

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

    fn set_error<D: Display>(&mut self, e: D) {
        self.status = format!("{}", e);
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
        restore_terminal().unwrap_or_else(|err| eprintln!("Error restoring terminal: {:?}", err));
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
                        if ui.command.is_empty() {
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
                        ui.command.push(c);
                        Ok(())
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => ui.file_view.down(lines),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => ui.file_view.up(lines),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => ui.file_view.down(term_size.height.into()),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => ui.file_view.up(term_size.height.into()),
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
                        ui.command.push('\n');
                        Ok(())
                    }
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        ui.command.pop();
                        Ok(())
                    }
                    _ => Ok(()),
                };

                let mut done = true;
                let res2 = match ui.command.as_str() {
                    "q" => break,
                    "w" => {
                        ui.wrap = !ui.wrap;
                        Ok(())
                    }
                    "f" => {
                        ui.follow = !ui.follow;
                        Ok(())
                    }
                    "n" => {
                        if let Some(re) = ui.search_pattern.as_ref() {
                            ui.file_view.down_to_line_matching(
                                &bytes::Regex::new(re.as_str()).unwrap(),
                                true,
                            )
                        } else {
                            Err(ViewError::from("nothing to search"))
                        }
                    }
                    "N" => {
                        if let Some(re) = ui.search_pattern.as_ref() {
                            ui.file_view
                                .up_to_line_matching(&bytes::Regex::new(re.as_str()).unwrap())
                        } else {
                            Err(ViewError::from("nothing to search"))
                        }
                    }
                    "gg" => {
                        ui.file_view.top();
                        Ok(())
                    }
                    "GG" => {
                        align_bottom = true;
                        ui.file_view.bottom();
                        ui.file_view.up(term_size.height.into())
                    }
                    x if x.to_lowercase() == "j" => ui.file_view.down(lines),
                    x if x.to_lowercase() == "k" => ui.file_view.up(lines),
                    x if x.to_lowercase().ends_with("gg") => x
                        .get(..x.len() - 2)
                        .unwrap()
                        .parse::<u64>()
                        .map_err(|_| ViewError::from("not a number"))
                        .and_then(|line| ui.file_view.jump_to_line(line)),
                    x if x.to_lowercase().ends_with("pp") => x
                        .get(..x.len() - 2)
                        .unwrap()
                        .parse::<f64>()
                        .map_err(|_| ViewError::from("not a number"))
                        .map(|percent| {
                            ui.file_view.jump_to_byte(
                                (ui.file_view.file_size() as f64 * percent / 100.0) as u64,
                            )
                        }),
                    x if x.starts_with("/") && x.ends_with("\n") => {
                        let pattern = x.get(1..x.len() - 1).unwrap_or("");
                        if pattern.is_empty() {
                            ui.search_pattern = None;
                            break;
                        }
                        match (Regex::new(pattern), bytes::Regex::new(pattern)) {
                            (Ok(unicode_re), Ok(bytes_re)) => {
                                ui.search_pattern = Some(unicode_re);
                                ui.file_view.down_to_line_matching(&bytes_re, false)
                            }
                            _ => Err(ViewError::from("invalid regex")),
                        }
                    }
                    _ => {
                        done = ui.command.ends_with("\n");
                        Ok(())
                    }
                };

                if reset {
                    ui.command.clear();
                    ui.search_pattern = None;
                }

                if done {
                    ui.command.clear()
                }

                if align_bottom {
                    ui.align_bottom = true;
                } else if ui.file_view.current_line() != line_before {
                    ui.align_bottom = false;
                }

                match res1.and_then(|_| res2) {
                    Err(e) => {
                        ui.set_error(e);
                        continue;
                    }
                    _ => (),
                }
            }
        }
    }

    // restore terminal
    restore_terminal()?;
    return Ok(());
}

fn wrap_text(text: String, width: usize) -> String {
    assert!(width > 0);
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
        let text = match ui.file_view.view(height) {
            Ok(x) => x,
            Err(e) => {
                ui.set_error(e);
                b""
            }
        };
        let mut text = from_utf8(text)
            .map(|x| x.to_owned())
            .unwrap_or_else(|e| format!("invalid utf-8: {:?}", e));

        if ui.wrap {
            text = wrap_text(text, chunks[1].width.into());
            if ui.align_bottom && text.lines().count() > height as usize {
                match ui.file_view.down(1) {
                    Ok(_) => continue,
                    Err(e) => ui.set_error(e),
                }
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
        flags.push("Follow".to_owned())
    }
    if ui.wrap {
        flags.push("Wrap".to_owned())
    }
    if let Some(re) = &ui.search_pattern {
        flags.push(format!("/{}", re.to_string()));
    }

    let header = Text::from(format!(
        "Line {}, Offset {} ({:.1}%){}\n{}",
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
            format!("Command: {}", ui.command)
        } else {
            format!("Status: {}", ui.status)
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
