use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

#[derive(Debug, Clone)]
pub enum ViewError {
    BOF,
    EOF,
    NoMatchFound,
    Cancelled,
    InvalidRegex,
}

impl Display for ViewError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BOF => f.write_str("beginning of file"),
            Self::EOF => f.write_str("end of file"),
            Self::NoMatchFound => f.write_str("no match found"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::InvalidRegex => f.write_str("invalid regex"),
        }
    }
}

impl Error for ViewError {}
