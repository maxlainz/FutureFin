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
    Unprocessable,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    /// Petición bien formada pero rechazada por una regla de negocio (422). El mensaje sigue el
    /// estilo `snake_code: descripción` del resto de validaciones del módulo.
    #[error("{0}")]
    Unprocessable(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("resource conflict")]
    Conflict,
    #[error("database error")]
    Db(sqlx::Error),
    #[error("service unavailable")]
    Unavailable,
    #[error("not found")]
    NotFound,
}

/// Mapea automáticamente los SQLSTATE más comunes a respuestas HTTP coherentes, evitando que
/// cada handler tenga que distinguir entre violación de unique, FK rota y otros errores. Códigos
/// menos comunes caen al `Db(_)` genérico → 500.
impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = err {
            match db.code().as_deref() {
                Some("23505") => return ApiError::Conflict,
                Some("23503") => {
                    return ApiError::BadRequest("referenced record missing".into());
                }
                _ => {}
            }
        }
        ApiError::Db(err)
    }
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Conflict => StatusCode::CONFLICT,
            ApiError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn sanitised_message(&self) -> String {
        match self {
            ApiError::BadRequest(s) => s.clone(),
            ApiError::Unprocessable(s) => s.clone(),
            ApiError::Unauthorized => "authentication required".into(),
            ApiError::Forbidden => "forbidden".into(),
            ApiError::NotFound => "not found".into(),
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
            ApiError::Unprocessable(_) => ErrorCode::Unprocessable,
            ApiError::Unauthorized => ErrorCode::Unauthorized,
            ApiError::Forbidden => ErrorCode::Forbidden,
            ApiError::NotFound => ErrorCode::NotFound,
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
