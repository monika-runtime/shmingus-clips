use serde::{Serialize, Serializer};

/// Error type returned by every Tauri command. Serializes to a plain string so
/// the frontend can render it directly.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("capture: {0}")]
    Capture(String),
    #[error("portal: {0}")]
    Portal(String),
    #[error("media: {0}")]
    Media(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl AppError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
