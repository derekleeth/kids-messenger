use serde::Serialize;

/// An error a command returns to the page, as a message it can show.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The hub couldn't be reached.
    #[error("Can't reach the family hub right now. It'll send when it's back.")]
    Offline(String),
    /// The hub said no, with a reason for the person.
    #[error("{0}")]
    Hub(String),
    /// This device was removed from the family.
    #[error("This device isn't part of the family anymore. Ask a parent to add it again.")]
    Revoked,
    #[error("{0}")]
    Invalid(String),
    #[error("Not set up yet.")]
    NotEnrolled,
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("database setup: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("file: {0}")]
    Io(#[from] std::io::Error),
    #[error("data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self::Other(e.to_string())
    }
}
