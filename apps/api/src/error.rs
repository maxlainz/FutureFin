use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use tracing::error;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub error: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("username already exists")]
    Conflict,
    #[error("database error")]
    Db(#[from] sqlx::Error),
    #[error("service unavailable")]
    Unavailable,
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Conflict => StatusCode::CONFLICT,
            ApiError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn sanitised_message(&self) -> String {
        match self {
            ApiError::BadRequest(s) => s.clone(),
            ApiError::Unauthorized => "authentication required".into(),
            ApiError::Conflict => "resource conflict".into(),
            ApiError::Unavailable => "dependency unavailable".into(),
            ApiError::Db(err) => {
                error!(?err, "database error");
                "internal error".into()
            }
        }
    }

    fn code(&self) -> ErrorCode {
        match self {
            ApiError::BadRequest(_) => ErrorCode::BadRequest,
            ApiError::Unauthorized => ErrorCode::Unauthorized,
            ApiError::Conflict => ErrorCode::Conflict,
            ApiError::Unavailable => ErrorCode::Unavailable,
            ApiError::Db(_) => ErrorCode::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorBody {
            error: self.code(),
            message: self.sanitised_message(),
        };
        (status, Json(body)).into_response()
    }
}
