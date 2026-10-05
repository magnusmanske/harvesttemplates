use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Every handler error maps to one of these. Internal errors are logged with
/// a short id; only the id reaches the client.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not logged in")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    TooManyRequests(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    const fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<crate::harvest::JobError> for ApiError {
    fn from(e: crate::harvest::JobError) -> Self {
        match e {
            crate::harvest::JobError::Invalid(msg) => Self::BadRequest(msg),
            crate::harvest::JobError::Failed(e) => Self::Internal(e),
        }
    }
}

impl From<crate::harvest::ClaimError> for ApiError {
    fn from(e: crate::harvest::ClaimError) -> Self {
        Self::TooManyRequests(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = match &self {
            Self::Internal(e) => {
                let id = format!("{:06x}", rand::random::<u32>() & 0xff_ffff);
                tracing::error!(error_id = %id, "internal error: {e:#}");
                format!("internal error (id {id})")
            }
            other => other.to_string(),
        };
        (self.status(), Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_do_not_leak_details() {
        let resp = ApiError::Internal(anyhow::anyhow!("password=hunter2")).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn statuses() {
        assert_eq!(ApiError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::bad_request("x").status(), StatusCode::BAD_REQUEST);
    }
}
