#![forbid(unsafe_code)]
//! Language identification, a port of py3langid with its model in compact form (see
//! `model.rs`). Modified from rust-py3langid 0.4.1; see UPSTREAM.md.

mod identifier;
mod model;
#[cfg(feature = "pack")]
mod original;
mod preprocess;

pub use identifier::{Identifier, LanguageScore, Options};
pub use model::Model;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidModel(String),
    InvalidConfiguration(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidModel(message) => write!(formatter, "invalid model: {message}"),
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid configuration: {message}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
