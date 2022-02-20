use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

#[derive(Debug, Clone)]
pub enum ChannelError {
    Command,
    Cancel,
    State,
}

impl Display for ChannelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command => f.write_str("command channel error"),
            Self::Cancel => f.write_str("cancel channel error"),
            Self::State => f.write_str("state channel error"),
        }
    }
}

impl Error for ChannelError {}

#[derive(Debug, Clone)]
pub enum BackendError {
    Stopped,
    UnknownMark(String),
}

impl Display for BackendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => f.write_str("backend stopped"),
            Self::UnknownMark(x) => write!(f, "unknown mark: {}", x),
        }
    }
}

impl Error for BackendError {}

#[derive(Debug, Clone)]
pub enum FrontendError {
    EndOfEventStream,
    EndOfSignalStream,
}

impl Display for FrontendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EndOfEventStream => f.write_str("end of event stream"),
            Self::EndOfSignalStream => f.write_str("end of signal stream"),
        }
    }
}

impl Error for FrontendError {}
