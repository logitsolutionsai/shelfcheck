//! Errors and their exit codes.
//!
//! Exit codes are shared with recordchaos and promochaos:
//! 0 clean, 1 problems found, 2 invalid input or config,
//! 3 a required file is missing or unreadable, 4 internal error.

use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The input or the configuration is invalid (exit 2).
    #[error("{0}")]
    Input(String),
    /// A required file is missing or cannot be read (exit 3).
    #[error("{path}: {reason}")]
    Unavailable { path: String, reason: String },
    /// A bug in shelfcheck (exit 4).
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Input(_) => 2,
            Error::Unavailable { .. } => 3,
            Error::Internal(_) => 4,
        }
    }

    pub fn input(msg: impl Into<String>) -> Self {
        Error::Input(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(msg.into())
    }

    /// An error opening or reading a file. A missing or unreadable file is
    /// exit 3; anything else about the bytes is the caller's business.
    pub fn io(path: &Path, err: &std::io::Error) -> Self {
        Error::Unavailable {
            path: path.display().to_string(),
            reason: err.to_string(),
        }
    }

    /// An input file whose content cannot be parsed (exit 2).
    pub fn parse(path: &str, line: u64, msg: impl std::fmt::Display) -> Self {
        Error::Input(format!("{path}:{line}: {msg}"))
    }
}
