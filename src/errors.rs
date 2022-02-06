use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone)]
pub struct ViewError {
    msg: String,
}

impl From<String> for ViewError {
    fn from(msg: String) -> Self {
        Self { msg }
    }
}

impl From<&str> for ViewError {
    fn from(msg: &str) -> Self {
        Self {
            msg: String::from(msg),
        }
    }
}

impl Display for ViewError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}
