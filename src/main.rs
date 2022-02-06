mod errors;
mod file_buffer;
mod file_view;
mod ui;
mod utils;

use crate::file_view::BufferedFileView;
use clap::Parser;
use std::io;

#[derive(Parser)]
struct Args {
    /// Path to the file to read
    path: String,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    return match BufferedFileView::new(args.path) {
        Ok(file_view) => ui::run(Box::new(file_view)),
        Err(err) => Err(err),
    };
}
