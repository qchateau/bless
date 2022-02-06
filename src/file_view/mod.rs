mod buffered_file_view;

use crate::errors::ViewError;
pub use buffered_file_view::BufferedFileView;
use regex::bytes::Regex;
use std::fmt::Debug;

pub trait FileView: Debug {
    fn file_path(&self) -> &str;
    fn file_size(&self) -> u64;
    fn current_line(&self) -> Option<i64>;
    fn offest(&self) -> u64;
    fn view(&mut self, nlines: u64) -> Result<&[u8], ViewError>;
    fn up(&mut self, lines: u64) -> Result<(), ViewError>;
    fn up_to_line_matching(&mut self, regex: &Regex) -> Result<(), ViewError>;
    fn down(&mut self, lines: u64) -> Result<(), ViewError>;
    fn down_to_line_matching(&mut self, regex: &Regex, skip_current: bool)
        -> Result<(), ViewError>;
    fn jump_to_line(&mut self, line: u64) -> Result<(), ViewError>;
    fn jump_to_byte(&mut self, bytes: u64);
    fn top(&mut self);
    fn bottom(&mut self);
}
