//! Error types for the GameSync core engine.

/// All fallible operations in the core return this `Result`.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("invalid glob pattern: {0}")]
    Glob(#[from] globset::Error),

    /// A destructive operation was refused because the game's process is still
    /// running. Acting now risks reading or writing a half-written save.
    #[error("game '{0}' appears to be running; refusing to {1}")]
    GameRunning(String, &'static str),

    #[error("not found: {0}")]
    NotFound(String),

    /// The data store is encrypted and no key has been provided. Unlock first.
    #[error("the data store is encrypted; unlock it first")]
    Locked,

    /// A stored object failed its checksum, or a restored file did not match
    /// its recorded hash. Never proceed past this.
    #[error("integrity error: {0}")]
    Integrity(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
