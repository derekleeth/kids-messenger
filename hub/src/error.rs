use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kit_protocol::ErrorBody;

/// What went wrong, with a message the app can show as it is.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("This device isn't part of the family (anymore). Ask a parent to add it again.")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Too many wrong codes. Wait a few minutes and try again.")]
    TooManyAttempts,
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("database setup: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("file: {0}")]
    Io(#[from] std::io::Error),
    #[error("data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let error = if status == StatusCode::INTERNAL_SERVER_ERROR {
            // The details go to the hub's log, not to the device.
            log::error!("{self}");
            "Something went wrong on the family hub. Try again in a moment.".to_string()
        } else {
            self.to_string()
        };
        (status, Json(ErrorBody { error })).into_response()
    }
}
