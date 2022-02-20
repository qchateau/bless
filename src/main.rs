mod errors;
mod file_buffer;
mod file_view;
mod term;
mod ui;
mod utils;

use crate::{errors::Result, term::ConfigureTerm, ui::Ui};
use clap::Parser;
use std::{
    panic,
    sync::{Arc, Mutex},
};

#[derive(Parser)]
struct Args {
    /// Path to the file to read
    path: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let term = Arc::new(Mutex::new(Some(ConfigureTerm::new()?)));
    let term_copy = term.clone();

    let default_panic = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        term_copy.lock().unwrap().take().unwrap();
        default_panic(panic_info);
    }));

    let mut ui = Ui::new(&args.path).await?;
    let res = ui.run().await;
    term.lock().unwrap().as_mut().unwrap().cleanup();
    return res;
}
