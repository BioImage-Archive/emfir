use thiserror::Error;
use std::io;

#[derive(Debug, Error)]
pub enum MrcError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid MRC format: {0}")]
    Format(String),
}