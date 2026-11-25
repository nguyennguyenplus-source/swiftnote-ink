use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwiftNoteError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse/serialize error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Media processing error: {0}")]
    Media(String),

    #[error("Invalid path or UTF-8 string")]
    InvalidInput,
}

pub type Result<T> = std::result::Result<T, SwiftNoteError>;
