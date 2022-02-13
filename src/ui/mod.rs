mod backend;
mod frontend;

use std::io;
use tokio::{
    select,
    sync::{mpsc, watch},
};

use crate::{
    file_view::FileView,
    ui::{
        backend::{Backend, BackendState},
        frontend::Frontend,
    },
};

pub struct Ui {
    backend: Backend,
    frontend: Frontend,
}

impl Ui {
    pub fn new(file_view: FileView) -> io::Result<Self> {
        let (state_sender, state_receiver) = watch::channel(BackendState::new());
        let (command_sender, command_receiver) = mpsc::unbounded_channel();
        let (cancel_sender, cancel_receiver) = mpsc::unbounded_channel();
        let backend = Backend::new(command_receiver, cancel_receiver, state_sender, file_view);
        let frontend = Frontend::new(command_sender, cancel_sender, state_receiver)?;
        return Ok(Self { backend, frontend });
    }
    pub async fn run(&mut self) -> io::Result<()> {
        return select! {
            res = self.frontend.run() => match res {
                Err(err) => Err(io::Error::new(io::ErrorKind::Other, format!("frontend stopped: {}", err))),
                Ok(_) => Ok(())
            },
            _ = self.backend.run() => Err(io::Error::new(io::ErrorKind::Other, "backend stopped"))
        };
    }
}
