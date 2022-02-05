mod file_buffer;
mod file_view;
mod ui;

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
    let file_view = match args.path {
        path if path.ends_with(".bz2") => BufferedFileView::new_bzip2(path),
        path => BufferedFileView::new_plaintext(path),
    };
    return match file_view {
        Ok(mut file_view) => ui::run(&mut file_view),
        Err(err) => Err(err),
    };
}
