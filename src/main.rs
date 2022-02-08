mod errors;
mod file_buffer;
mod file_view;
mod term;
mod ui;
mod utils;

use crate::{file_view::BufferedFileView, term::ConfigureTerm, ui::Ui};
use clap::Parser;
use std::{
    io, panic,
    sync::{Arc, Mutex},
};

#[derive(Parser)]
struct Args {
    /// Path to the file to read
    path: String,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let view = BufferedFileView::new(args.path)?;
    let mut ui = Ui::new(Box::new(view));
    let term = Arc::new(Mutex::new(Some(ConfigureTerm::new()?)));
    let term_copy = term.clone();

    let default_panic = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        term_copy.lock().unwrap().take().unwrap();
        default_panic(panic_info);
    }));

    let res = ui.run();
    term.lock().unwrap().as_mut().unwrap().cleanup();

    return res;
}
