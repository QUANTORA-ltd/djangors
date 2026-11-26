use thiserror::Error;

/// Error types for static file collection and serving operations.
#[derive(Error, Debug)]
pub enum StaticFilesError {
    /// An I/O error occurred during file operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// A file hashing error occurred.
    #[error("Hashing error: {0}")]
    Hashing(String),

    /// An S3-compatible storage operation failed.
    #[error("S3 storage error: {0}")]
    S3(String),

    /// Image decoding, resizing, or encoding failed.
    #[error("Image processing error: {0}")]
    Image(String),
}
