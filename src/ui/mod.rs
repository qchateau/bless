mod backend;
mod errors;
mod frontend;

use crate::{
    errors::Result,
    ui::errors::BackendError,
    ui::{
        backend::{Backend, BackendState},
        frontend::Frontend,
    },
};
use tokio::{
    select,
    sync::{mpsc, watch},
};

pub struct Ui {
    backend: Backend,
    frontend: Frontend,
}

impl Ui {
    pub async fn new(path: &str) -> Result<Self> {
        let (state_sender, state_receiver) = watch::channel(BackendState::new());
        let (command_sender, command_receiver) = mpsc::unbounded_channel();
        let (cancel_sender, cancel_receiver) = mpsc::unbounded_channel();
        let backend = Backend::new(command_receiver, cancel_receiver, state_sender, path).await?;
        let frontend = Frontend::new(command_sender, cancel_sender, state_receiver)?;
        return Ok(Self { backend, frontend });
    }
    pub async fn run(&mut self) -> Result<()> {
        return select! {
            res = self.frontend.run() => res,
            res = self.backend.run() => res.and(Err(BackendError::Stopped.into())),
        };
    }
}
