use regex::bytes::Regex;
use std::str::from_utf8;
use tokio::sync::{mpsc::UnboundedReceiver, watch::Sender};

use crate::{errors::ViewError, file_view::FileView};

#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    MoveLine(i64),
    JumpLine(i64),
    JumpFileRatio(f64),
    SearchDown(String),
    SearchDownNext(String),
    SearchUp(String),
    Follow(bool),
    Resize(u64),
}

#[derive(Clone)]
pub struct BackendState {
    pub file_path: String,
    pub file_size: u64,
    pub error: Option<String>,
    pub current_line: Option<i64>,
    pub offset: u64,
    pub text: String,
    pub follow: bool,
}

impl BackendState {
    pub fn new() -> Self {
        return Self {
            file_path: String::new(),
            text: String::new(),
            error: None,
            follow: false,
            file_size: 0,
            current_line: None,
            offset: 0,
        };
    }
}

pub struct Backend {
    command_receiver: UnboundedReceiver<Command>,
    state_sender: Sender<BackendState>,
    file_view: FileView,
    view_size: u64,
}

impl Backend {
    pub fn new(
        command_receiver: UnboundedReceiver<Command>,
        state_sender: Sender<BackendState>,
        file_view: FileView,
    ) -> Self {
        return Self {
            command_receiver,
            state_sender,
            file_view,
            view_size: 0,
        };
    }
    pub async fn run(&mut self) -> Result<(), ViewError> {
        let mut state = BackendState::new();
        self.update_state(&mut state).await;
        self.state_sender
            .send(state)
            .map_err(|_| ViewError::from("state channel error"))?;

        loop {
            let command = match self.command_receiver.recv().await {
                Some(command) => command,
                None => return Err(ViewError::from("command channel error")),
            };
            let mut state = BackendState::new();
            if let Err(e) = self.handle_command(command, &mut state).await {
                state.error = Some(format!("{}", e));
            }
            self.update_state(&mut state).await;
            self.state_sender
                .send(state)
                .map_err(|_| ViewError::from("state channel error"))?;
        }
    }
    async fn handle_command(
        &mut self,
        command: Command,
        state: &mut BackendState,
    ) -> Result<(), ViewError> {
        eprintln!("command: {:?}", command);
        let res = match command {
            Command::Follow(follow) => {
                state.follow = follow;
                Ok(())
            }
            Command::SearchDown(pattern) => {
                self.file_view
                    .down_to_line_matching(
                        &Regex::new(&pattern).map_err(|_| ViewError::from("invalid regex"))?,
                        false,
                    )
                    .await
            }
            Command::SearchDownNext(pattern) => {
                self.file_view
                    .down_to_line_matching(
                        &Regex::new(&pattern).map_err(|_| ViewError::from("invalid regex"))?,
                        true,
                    )
                    .await
            }
            Command::SearchUp(pattern) => {
                self.file_view
                    .up_to_line_matching(
                        &Regex::new(&pattern).map_err(|_| ViewError::from("invalid regex"))?,
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
            Command::Resize(size) => {
                self.view_size = size;
                Ok(())
            }
        };

        return res;
    }

    async fn update_state(&mut self, state: &mut BackendState) {
        if state.follow {
            self.file_view.bottom().await;
            // FIXME
            // self.file_view.up(state.term_size.height.into()).await.ok();
        }

        state.file_path = self.file_view.file_path().to_owned();
        state.text = loop {
            let text = match self.file_view.view(self.view_size).await {
                Ok(x) => x,
                Err(e) => {
                    state.error = Some(if let Some(error) = state.error.as_ref() {
                        error.clone() + &format!(", {}", e)
                    } else {
                        format!("{}", e)
                    });
                    b""
                }
            };
            break from_utf8(text)
                .map(|x| x.to_owned())
                .unwrap_or_else(|e| format!("invalid utf-8: {:?}", e));
        };

        state.file_size = self.file_view.file_size().await;
        state.current_line = self.file_view.current_line();
        state.offset = self.file_view.offset();
    }
}
