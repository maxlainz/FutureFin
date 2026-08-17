//! Errores OAuth en el formato de la spec (RFC 6749 §5.2): JSON
//! `{"error": "...", "error_description": "..."}`.
//!
//! Deliberadamente NO es `ApiError`: el formato del body y los códigos (`invalid_grant`,
//! `invalid_client`, …) los fija la RFC, no el API propio. Toda respuesta lleva
//! `Cache-Control: no-store` (RFC 6749 §5.1 — las respuestas del token endpoint
//! transportan secretos).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug)]
pub struct OAuthError {
    pub status: StatusCode,
    pub code: &'static str,
    pub description: String,
}

impl OAuthError {
    pub fn invalid_request(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            description: d.into(),
        }
    }

    /// 401 SIEMPRE (no 400): es la señal exacta con la que claude.ai re-registra el
    /// cliente vía DCR — y gracias a ella un restore de backup sin tablas OAuth
    /// se auto-recupera sin intervención.
    pub fn invalid_client(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "invalid_client",
            description: d.into(),
        }
    }

    pub fn invalid_grant(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_grant",
            description: d.into(),
        }
    }

    /// RFC 8707: el `resource` pedido no es el nuestro.
    pub fn invalid_target(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_target",
            description: d.into(),
        }
    }

    pub fn unsupported_grant_type() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "unsupported_grant_type",
            description: "supported grant types: authorization_code, refresh_token".into(),
        }
    }

    /// RFC 7591 (DCR).
    pub fn invalid_client_metadata(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_client_metadata",
            description: d.into(),
        }
    }

    /// RFC 7591 (DCR).
    pub fn invalid_redirect_uri(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_redirect_uri",
            description: d.into(),
        }
    }

    pub fn server_error() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "server_error",
            description: "internal error".into(),
        }
    }

    /// Cupo de DCR agotado (anti-flood del registro abierto).
    pub fn temporarily_unavailable(d: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "temporarily_unavailable",
            description: d.into(),
        }
    }
}

/// El detalle de un error de DB no viaja al cliente (mismo criterio que `ApiError::Db`).
impl From<sqlx::Error> for OAuthError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(?err, "database error in oauth endpoint");
        Self::server_error()
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body<'a> {
            error: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            error_description: Option<&'a str>,
        }
        let body = Body {
            error: self.code,
            error_description: (!self.description.is_empty()).then_some(self.description.as_str()),
        };
        let mut resp = (self.status, Json(body)).into_response();
        resp.headers_mut().insert(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-store"),
        );
        if self.code == "invalid_client" {
            // RFC 6749 §5.2: el 401 de client-auth lleva su challenge.
            resp.headers_mut().insert(
                http::header::WWW_AUTHENTICATE,
                http::HeaderValue::from_static(r#"Basic realm="FutureFin""#),
            );
        }
        resp
    }
}
