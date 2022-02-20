use log::info;
use regex::bytes;
use std::{
    collections::HashMap,
    error::Error,
    fs::canonicalize,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    select,
    sync::{mpsc::UnboundedReceiver, watch::Sender},
    time::{self, Duration},
};

use crate::{
    errors::Result,
    file_view::{FileView, ViewError, ViewState},
    ui::errors::{BackendError, ChannelError},
};

#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    MoveLine(i64),
    JumpLine(i64),
    JumpFileRatio(f64),
    SearchDown(String),
    SearchDownNext(String),
    SearchUp(String),
    Follow(bool),
    Resize(Option<usize>, usize),
    SaveMark(String),
    LoadMark(String),
}

pub struct BackendState {
    pub file_path: String,
    pub real_file_path: String,
    pub file_size: u64,
    pub errors: Vec<Rc<Box<dyn Error>>>,
    pub current_line: Option<i64>,
    pub offset: u64,
    pub text: Vec<String>,
    pub follow: bool,
    pub marks: Vec<String>,
}

impl BackendState {
    pub fn new() -> Self {
        return Self {
            file_path: String::new(),
            real_file_path: String::new(),
            text: Vec::new(),
            errors: Vec::new(),
            follow: false,
            file_size: 0,
            current_line: None,
            offset: 0,
            marks: Vec::new(),
        };
    }
}

struct CommandHandler {
    command_receiver: UnboundedReceiver<Command>,
    state_sender: Sender<BackendState>,
    file_path: String,
    file_view: FileView,
    view_width: Option<usize>,
    view_height: usize,
    cancelled: Rc<AtomicBool>,
    marks: HashMap<String, ViewState>,
    follow: bool,
    command_errors: Vec<Rc<Box<dyn Error>>>,
}

struct CancelHandler {
    cancel_receiver: UnboundedReceiver<()>,
    cancelled: Rc<AtomicBool>,
}

pub struct Backend {
    command_handler: CommandHandler,
    cancel_handler: CancelHandler,
}

impl Backend {
    pub async fn new(
        command_receiver: UnboundedReceiver<Command>,
        cancel_receiver: UnboundedReceiver<()>,
        state_sender: Sender<BackendState>,
        path: &str,
    ) -> Result<Self> {
        let cancelled = Rc::from(AtomicBool::from(false));
        let file_view = FileView::new(path).await?;
        return Ok(Self {
            command_handler: CommandHandler {
                command_receiver,
                state_sender,
                file_path: path.to_string(),
                file_view,
                view_width: None,
                view_height: 0,
                cancelled: cancelled.clone(),
                follow: false,
                command_errors: Vec::new(),
                marks: HashMap::new(),
            },
            cancel_handler: CancelHandler {
                cancel_receiver,
                cancelled: cancelled.clone(),
            },
        });
    }

    pub async fn run(&mut self) -> Result<()> {
        select! {
            res = self.command_handler.run() => res,
            res = self.cancel_handler.run() => res,
        }
    }
}

impl CancelHandler {
    async fn run(&mut self) -> Result<()> {
        loop {
            match self.cancel_receiver.recv().await {
                Some(_) => self.cancelled.store(true, Ordering::Release),
                None => return Err(ChannelError::Cancel.into()),
            }
        }
    }
}

impl CommandHandler {
    async fn run(&mut self) -> Result<()> {
        self.send_state().await?;
        let mut prev_file_size = 0;

        loop {
            if self.cancelled.load(Ordering::Acquire) {
                // flush all pending commands
                while let Ok(_) = self.command_receiver.try_recv() {}
                self.cancelled.store(false, Ordering::Release);
            }

            let sleep_time_ms = if self.follow { 100 } else { 10000 };

            select! {
                 msg = self.command_receiver.recv() => {
                    let command = match msg {
                        Some(command) => command,
                        None => return Err(ChannelError::Command.into()),
                    };

                    self.command_errors.clear();
                    if let Err(e) = self.handle_command(command).await {
                        self.command_errors.push(Rc::from(e));
                    }
                },
                _ = time::sleep(Duration::from_millis(sleep_time_ms)) => {
                    let file_size = self.file_view.file_size().await;
                    if file_size == prev_file_size {
                        continue;
                    }
                    prev_file_size = file_size;
                },
            }

            self.maybe_reload_file().await?;

            if self.follow {
                while self.file_view.down(1_000_000).await.is_ok() {}
            }

            self.send_state().await?;
        }
    }

    async fn handle_command(&mut self, command: Command) -> Result<()> {
        info!("command: {:?}", command);
        let res = match command {
            Command::Follow(follow) => {
                self.follow = follow;
                self.file_view.bottom().await
            }
            Command::SearchDown(pattern) => {
                self.file_view
                    .down_to_line_matching(
                        &bytes::Regex::new(&pattern).map_err(|_| ViewError::InvalidRegex)?,
                        false,
                        &self.cancelled,
                    )
                    .await
            }
            Command::SearchDownNext(pattern) => {
                self.file_view
                    .down_to_line_matching(
                        &bytes::Regex::new(&pattern).map_err(|_| ViewError::InvalidRegex)?,
                        true,
                        &self.cancelled,
                    )
                    .await
            }
            Command::SearchUp(pattern) => {
                self.file_view
                    .up_to_line_matching(
                        &bytes::Regex::new(&pattern).map_err(|_| ViewError::InvalidRegex)?,
                        &self.cancelled,
                    )
                    .await
            }
            Command::MoveLine(lines) => {
                if lines > 0 {
                    self.file_view.down(lines as u64).await
                } else if lines < 0 {
                    self.file_view.up((-lines) as u64).await
                } else {
                    Ok(())
                }
            }
            Command::JumpLine(line) => self.file_view.jump_to_line(line).await,
            Command::JumpFileRatio(ratio) => {
                let pos = self.file_view.file_size().await as f64 * ratio;
                self.file_view.jump_to_byte(pos as u64).await
            }
            Command::Resize(w, h) => {
                self.view_width = w;
                self.view_height = h;
                Ok(())
            }
            Command::SaveMark(name) => {
                self.marks.insert(name, self.file_view.save_state());
                Ok(())
            }
            Command::LoadMark(name) => {
                if let Some(state) = self.marks.get(&name) {
                    self.file_view.load_state(state)
                } else {
                    Err(BackendError::UnknownMark(name).into())
                }
            }
        };

        return res;
    }

    async fn generate_state(&mut self) -> BackendState {
        let mut state = BackendState::new();

        state.file_path = self.file_path.clone();
        state.real_file_path = self.file_view.real_file_path().to_owned();

        let offset_before = self.file_view.offset();
        state.text = match self.file_view.view(self.view_height, self.view_width).await {
            Ok(x) => x,
            Err(e) => {
                state.errors.push(Rc::from(e));
                Vec::new()
            }
        };

        state.file_size = self.file_view.file_size().await;
        state.current_line = self.file_view.current_line();
        state.offset = self.file_view.offset();
        state.follow = self.follow;
        state.errors = self.command_errors.clone();
        state.marks = self.marks.keys().map(|x| x.clone()).collect();

        if offset_before > state.offset {
            // building the view shifted the view upwards,
            // we hit the EOF
            state.errors.push(Rc::new(Box::from(ViewError::EOF)));
        }

        return state;
    }

    async fn send_state(&mut self) -> Result<()> {
        let state = self.generate_state().await;
        self.state_sender
            .send(state)
            .map_err(|_| ChannelError::State)?;
        Ok(())
    }

    async fn maybe_reload_file(&mut self) -> Result<()> {
        let real_file_path = canonicalize(&self.file_path)?.to_string_lossy().to_string();
        if real_file_path != self.file_view.real_file_path() {
            info!("reloading file");
            self.file_view = FileView::new(&self.file_path).await?;
        }
        return Ok(());
    }
}
