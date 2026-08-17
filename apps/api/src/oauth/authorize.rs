//! Validación del authorization request y emisión del code.
//!
//! La validación vive UNA vez aquí y la consumen los dos endpoints de la SPA
//! (`GET /v1/oauth/authorize-details` y `POST /v1/oauth/authorize`) — nunca dupliques
//! estas reglas en un handler. La distinción crítica (OAuth 2.1 §7.12.2):
//!
//! - Error FATAL (client_id desconocido, redirect_uri sin match exacto): NO se puede
//!   redirigir — hacerlo sería un open redirect. La SPA pinta el error y ahí muere.
//! - Error REDIRIGIBLE (response_type/PKCE/resource malos con cliente y redirect
//!   válidos): va al redirect_uri registrado como `?error=…&state=…`.

use super::error::OAuthError;
use super::url::{append_query, mcp_resource_url};
use crate::auth::secret::{generate_opaque_secret, sha256_hex};
use crate::error::ApiError;
use serde::Deserialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

/// TTL del authorization code (claude lo canjea en segundos; 2 min cubre redirects lentos).
const CODE_TTL: &str = "2 minutes";

/// Parámetros del authorization request tal como llegan en la query de
/// `/oauth/authorize` (la SPA los reenvía tal cual).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AuthorizeParams {
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub response_type: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug)]
pub struct ValidatedAuthorizeRequest {
    /// PK interna (`oauth_clients.id`).
    pub client_row_id: Uuid,
    pub client_name: String,
    pub client_uri: Option<String>,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: Option<String>,
    pub scope: Option<String>,
    pub resource: String,
}

#[derive(Debug)]
pub enum AuthorizeParamError {
    /// No redirigible: la SPA pinta `code` y no ofrece continuar.
    Fatal { code: &'static str },
    /// Redirigible al redirect_uri registrado como `?error=<code>`.
    Redirectable {
        code: &'static str,
        description: String,
        /// El redirect_uri YA validado contra el registro (por eso es seguro redirigir).
        redirect_uri: String,
        state: Option<String>,
    },
}

pub async fn validate_authorize_params(
    pool: &PgPool,
    base_url: &str,
    p: &AuthorizeParams,
) -> Result<ValidatedAuthorizeRequest, AuthorizeParamError> {
    // 1-2. Cliente y redirect: sin ambos válidos TODO es fatal.
    let Some(client_id) = p.client_id.as_deref().filter(|s| !s.is_empty()) else {
        return Err(AuthorizeParamError::Fatal { code: "missing_client_id" });
    };
    let row: Option<(Uuid, String, Option<String>, Vec<String>)> = sqlx::query_as(
        r#"SELECT id, client_name, client_uri, redirect_uris
           FROM oauth_clients WHERE client_id = $1"#,
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!(?e, "db error validating authorize params");
        AuthorizeParamError::Fatal { code: "server_error" }
    })?;
    let Some((client_row_id, client_name, client_uri, redirect_uris)) = row else {
        return Err(AuthorizeParamError::Fatal { code: "unknown_client" });
    };
    let Some(redirect_uri) = p.redirect_uri.as_deref().filter(|s| !s.is_empty()) else {
        return Err(AuthorizeParamError::Fatal { code: "missing_redirect_uri" });
    };
    // Match EXACTO de string completa contra lo registrado — ni prefijo, ni solo host.
    if !redirect_uris.iter().any(|u| u == redirect_uri) {
        return Err(AuthorizeParamError::Fatal { code: "redirect_uri_mismatch" });
    }

    let redirectable = |code: &'static str, description: String| AuthorizeParamError::Redirectable {
        code,
        description,
        redirect_uri: redirect_uri.to_string(),
        state: p.state.clone(),
    };

    // 3. response_type.
    if p.response_type.as_deref() != Some("code") {
        return Err(redirectable(
            "unsupported_response_type",
            "only response_type=code is supported".into(),
        ));
    }
    // 4-5. PKCE S256 obligatorio (OAuth 2.1; `plain` se rechaza).
    match p.code_challenge_method.as_deref() {
        Some("S256") => {}
        Some(_) | None => {
            return Err(redirectable(
                "invalid_request",
                "code_challenge_method must be S256".into(),
            ))
        }
    }
    let Some(challenge) = p.code_challenge.as_deref().filter(|c| {
        (43..=128).contains(&c.len())
            && c.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~'))
    }) else {
        return Err(redirectable(
            "invalid_request",
            "code_challenge is required (43-128 chars)".into(),
        ));
    };
    // 6. resource (RFC 8707): si viene, debe ser NUESTRO recurso MCP.
    let ours = mcp_resource_url(base_url);
    let resource = match p.resource.as_deref().filter(|s| !s.is_empty()) {
        None => ours,
        Some(r) if r.trim_end_matches('/') == ours => ours,
        Some(_) => {
            return Err(redirectable(
                "invalid_target",
                "resource does not identify this MCP server".into(),
            ))
        }
    };

    Ok(ValidatedAuthorizeRequest {
        client_row_id,
        client_name,
        client_uri,
        redirect_uri: redirect_uri.to_string(),
        code_challenge: challenge.to_string(),
        state: p.state.clone(),
        scope: p.scope.clone().filter(|s| !s.is_empty()),
        resource,
    })
}

/// Approve: crea (o reutiliza) el grant activo del par (cliente, usuario) y emite el
/// code, en UNA transacción. Devuelve el code en claro (solo se persiste su SHA-256).
pub async fn issue_authorization_code(
    pool: &PgPool,
    req: &ValidatedAuthorizeRequest,
    user_id: Uuid,
) -> Result<String, ApiError> {
    let mut tx = pool.begin().await?;

    // El índice UNIQUE parcial (client_id, user_id) WHERE revoked_at IS NULL permite el
    // upsert: re-consentir refresca el grant vivo en vez de duplicar la fila del panel.
    let (grant_id,): (Uuid,) = sqlx::query_as(
        r#"INSERT INTO oauth_grants (client_id, user_id, scope, resource)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (client_id, user_id) WHERE revoked_at IS NULL
           DO UPDATE SET scope = EXCLUDED.scope, resource = EXCLUDED.resource
           RETURNING id"#,
    )
    .bind(req.client_row_id)
    .bind(user_id)
    .bind(&req.scope)
    .bind(&req.resource)
    .fetch_one(&mut *tx)
    .await?;

    let code = generate_opaque_secret("");
    sqlx::query(
        r#"INSERT INTO oauth_authorization_codes
               (code_hash, grant_id, redirect_uri, code_challenge, code_challenge_method,
                resource, scope, expires_at)
           VALUES ($1, $2, $3, $4, 'S256', $5, $6, now() + $7::interval)"#,
    )
    .bind(sha256_hex(code.as_bytes()))
    .bind(grant_id)
    .bind(&req.redirect_uri)
    .bind(&req.code_challenge)
    .bind(&req.resource)
    .bind(&req.scope)
    .bind(CODE_TTL)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(code)
}

/// Redirect de éxito: `code` + `state` (eco literal) + `iss` (RFC 9207).
pub fn success_redirect(
    redirect_uri: &str,
    code: &str,
    state: Option<&str>,
    issuer: &str,
) -> Result<String, OAuthError> {
    let mut params: Vec<(&str, &str)> = vec![("code", code), ("iss", issuer)];
    if let Some(s) = state {
        params.push(("state", s));
    }
    append_query(redirect_uri, &params)
}

/// Redirect de error (deny o error redirigible): `error` (+`error_description`) + `state` + `iss`.
pub fn error_redirect(
    redirect_uri: &str,
    error: &str,
    description: Option<&str>,
    state: Option<&str>,
    issuer: &str,
) -> Result<String, OAuthError> {
    let mut params: Vec<(&str, &str)> = vec![("error", error), ("iss", issuer)];
    if let Some(d) = description {
        params.push(("error_description", d));
    }
    if let Some(s) = state {
        params.push(("state", s));
    }
    append_query(redirect_uri, &params)
}
