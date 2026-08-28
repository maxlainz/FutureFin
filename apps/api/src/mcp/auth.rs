//! Autenticación Bearer del endpoint `/mcp`.
//!
//! Middleware axum que corta ANTES del servicio MCP. Acepta las DOS credenciales del
//! servidor MCP, despachadas por prefijo del Bearer: tokens de API (`ffp_`,
//! `require_api_token`) y access tokens OAuth (`ffo_`, `require_oauth_access_token`).
//! Tras cualquiera de las dos, la membership se re-resuelve VIVA
//! (`require_installation_member`) — nada se congela en la credencial (D14). La
//! identidad viaja en las extensions del request: rmcp propaga las
//! `http::request::Parts` hasta el `RequestContext` de cada tool.
//!
//! El 401 anuncia la Protected Resource Metadata (RFC 9728 §5.1) para que los clientes
//! OAuth descubran el authorization server. SOLO el 401: un 403 (usuario pending,
//! membership revocada) con `WWW-Authenticate` mandaría a claude a re-autenticarse en
//! bucle — token nuevo, mismo 403, otra vuelta.

use crate::error::{ApiError, ErrorBody};
use crate::handlers::api_tokens::{require_api_token, TokenScope};
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::MembershipRole;
use crate::oauth::access::require_oauth_access_token;
use crate::oauth::url::{public_base_url, resource_metadata_url};
use crate::state::AppState;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use std::sync::Arc;
use uuid::Uuid;

/// Qué credencial autenticó el request. Lo consume la traza de `require_mcp_write` (y,
/// en su momento, la tabla de auditoría); las tools solo miran
/// `user_id`/`installation_id`/`role`.
#[derive(Debug, Clone)]
pub enum McpCredential {
    ApiToken { token_id: Uuid },
    OAuth { grant_id: Uuid, token_id: Uuid },
}

/// Identidad resuelta del token: SIEMPRE el estado vivo (rol e installation se leen en
/// cada request, nunca se congelan en el token).
#[derive(Debug, Clone)]
pub struct McpIdentity {
    pub user_id: Uuid,
    pub installation_id: Uuid,
    pub role: MembershipRole,
    pub credential: McpCredential,
    /// Techo de la CREDENCIAL, independiente del rol de la persona. Se lee vivo en el mismo
    /// SELECT que autentica (D14: no hay nada congelado en el secreto). Los access tokens OAuth
    /// no tienen scope propio todavía y entran como `read_write` — ver la nota de
    /// `oauth/metadata.rs` sobre por qué `scopes_supported` sigue ausente.
    pub scope: TokenScope,
}

pub async fn mcp_bearer_auth(state: Arc<AppState>, mut req: Request, next: Next) -> Response {
    // Se clonan los headers antes de los awaits: el body de axum no es `Sync`, así que
    // un `&Request` retenido a través de un await haría el future no-`Send`.
    let authorization = req.headers().get(http::header::AUTHORIZATION).cloned();
    let headers = req.headers().clone();
    match authenticate(&state, authorization).await {
        Ok(identity) => {
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        Err(e) => {
            let status = e.status();
            let mut resp = e.into_response();
            if status == StatusCode::UNAUTHORIZED {
                let value = match public_base_url(&state, &headers) {
                    Ok(base) => format!(
                        r#"Bearer realm="FutureFin", resource_metadata="{}""#,
                        resource_metadata_url(&base)
                    ),
                    Err(_) => "Bearer".to_string(),
                };
                if let Ok(v) = http::HeaderValue::from_str(&value) {
                    resp.headers_mut().insert(http::header::WWW_AUTHENTICATE, v);
                }
            }
            resp
        }
    }
}

async fn authenticate(
    state: &AppState,
    authorization: Option<http::HeaderValue>,
) -> Result<McpIdentity, ApiError> {
    let bearer = authorization
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.strip_prefix("Bearer "))
        .map(str::trim);

    let (user_id, credential, scope) = match bearer {
        Some(t) if t.starts_with(crate::oauth::ACCESS_TOKEN_PREFIX) => {
            let id = require_oauth_access_token(&state.pool, authorization.as_ref()).await?;
            (
                id.user_id,
                McpCredential::OAuth {
                    grant_id: id.grant_id,
                    token_id: id.token_id,
                },
                // OAuth no negocia scope (ver `oauth/metadata.rs`): la conexión concedida por
                // consentimiento vale lo que valga el rol vivo, como antes del scope.
                TokenScope::ReadWrite,
            )
        }
        // Todo lo demás (incl. prefijos desconocidos) pasa por api_tokens, que ya
        // devuelve el 401 indistinto.
        _ => {
            let token = require_api_token(&state.pool, authorization.as_ref()).await?;
            (
                token.user_id,
                McpCredential::ApiToken {
                    token_id: token.token_id,
                },
                token.scope,
            )
        }
    };

    let (installation_id, role) = require_installation_member(&state.pool, user_id).await?;
    Ok(McpIdentity {
        user_id,
        installation_id,
        role,
        credential,
        scope,
    })
}

/// Días que se conservan las filas de `mcp_write_audit`.
///
/// Un año: la ventana en la que una persona repasa sus finanzas («¿qué pasó en enero?») y sigue
/// pudiendo cruzar el log con un `.ffbackup` para reconstruir lo borrado. Es una constante y no
/// una variable de entorno a propósito — como `MAX_ACTIVE_TOKENS_PER_USER` o
/// `MAX_REGISTERED_CLIENTS`: un eje de configuración más cuesta documentación en tres sitios y
/// nadie ha pedido moverlo todavía.
const AUDIT_RETENTION_DAYS: i32 = 365;

/// Fila de auditoría ABIERTA: se ha registrado la intención, falta el desenlace.
///
/// Existe porque un log que dice «hecho» antes de que la operación termine es peor que no tener
/// log. `require_mcp_write` inserta la fila con `outcome = 'attempted'` —la afirmación más fuerte
/// que es cierta en ese instante— y quien llame a la core fn la cierra con
/// [`McpWriteAudit::settle`], que es la ÚNICA vía por la que la fila puede decir `ok`.
///
/// Si el handle se descarta sin cerrar (o el proceso muere a mitad), la fila se queda en
/// `attempted` con `settled_at IS NULL`, que es exactamente la verdad: se intentó, no se sabe
/// cómo acabó. `#[must_use]` está para que ese caso sea un aviso del compilador y no un
/// descubrimiento seis meses después leyendo la tabla.
///
/// Es `pub` (y no `pub(crate)`) por la misma razón que las core fns de los handlers: los tests de
/// integración viven en otro crate y el ciclo completo insertar→cerrar tiene que poder probarse
/// sin depender de que `mcp/server.rs` ya esté cableado.
#[must_use = "cierra la fila de auditoría con `settle(...)`; si se descarta, el registro se queda \
              en `attempted` y nunca dirá si la escritura llegó a ocurrir"]
#[derive(Debug)]
pub struct McpWriteAudit {
    /// `None` si el INSERT falló: la auditoría es best-effort y jamás tumba la operación.
    row_id: Option<Uuid>,
    tool: String,
}

impl McpWriteAudit {
    /// Cierra la fila con el desenlace REAL de la operación.
    ///
    /// `targets` son los ids de las filas que la operación **realmente** mutó (la creada, la
    /// borrada, las N de un lote). Se guardan porque son identificadores opacos: dicen QUÉ se tocó
    /// sin guardar nada de lo que contenía — ver la cabecera de la migración sobre por qué los
    /// argumentos no se persisten ni en claro ni en digest.
    ///
    /// **Convención para las tools con preview/confirm**: un preview (`confirm: false`) no muta
    /// nada, así que se cierra con `&[]`. `outcome = 'ok'` con `target_ids` vacío significa
    /// exactamente eso —«la llamada fue bien y no tocó ninguna fila»— y es lo que separa en el log
    /// un borrado consumado de un simple sondeo de qué se borraría.
    ///
    /// El `WHERE … AND settled_at IS NULL` hace la fila **write-once**: cerrada una vez, no hay
    /// forma de reescribirla desde aquí.
    pub async fn settle<T>(
        self,
        pool: &sqlx::PgPool,
        result: &Result<T, ApiError>,
        targets: &[Uuid],
    ) {
        let Some(row_id) = self.row_id else {
            return;
        };
        // Solo el CÓDIGO estable del error, nunca el mensaje: los `BadRequest` sí pueden llevar
        // datos que ha escrito la persona.
        let (outcome, error_code) = match result {
            Ok(_) => ("ok", None),
            Err(e) => ("failed", Some(ErrorBody::from_api_error(e).code)),
        };
        let res = sqlx::query(
            r#"UPDATE mcp_write_audit
               SET outcome = $2, error_code = $3, target_ids = $4, settled_at = now()
               WHERE id = $1 AND settled_at IS NULL"#,
        )
        .bind(row_id)
        .bind(outcome)
        .bind(error_code)
        .bind(targets)
        .execute(pool)
        .await;
        if let Err(e) = res {
            tracing::warn!(
                tool = %self.tool,
                audit_row = %row_id,
                error = ?e,
                "no se pudo cerrar la fila de auditoría MCP; queda como `attempted`"
            );
        }
    }
}

/// Gate de TODA tool de escritura MCP: rol con permiso de escritura + scope de la credencial +
/// kill-switch vivo `installation.mcp_write_enabled`. Se ejecuta por request (misma filosofía que
/// el resto de la identidad: revocar, estrechar o apagar = corte en la siguiente llamada, sin
/// reinicios).
///
/// Orden de las puertas, de la más fundamental a la más circunstancial:
///   1. **Rol vivo** — «esta persona no escribe» → `Forbidden` (código `forbidden`; la variante
///      no lleva mensaje y `sanitised_message` lo fija).
///   2. **Scope de la credencial** — «esta persona escribe, pero este token no» →
///      `mcp_token_read_only`. Solo resta: un token `read_write` de un `viewer` sigue sin escribir.
///   3. **Toggle de la instalación** — «hoy nadie escribe por MCP» → `mcp_write_disabled`.
/// Las dos últimas van por `BadRequest` porque es la única variante que propaga el mensaje al
/// wire, y el LLM necesita leer el MOTIVO para explicárselo al usuario en vez de reintentar a
/// ciegas. Las dos primeras no tocan la base: solo la tercera consulta.
///
/// `tool` es el nombre de la tool que llama. Es OBLIGATORIO y no decorativo: este gate es el
/// único punto por el que pasan las 31 escrituras, así que es donde vive la traza Y la tabla de
/// auditoría. Sin él, un borrado masivo por MCP no deja absolutamente ningún rastro de qué tool
/// lo hizo: `delete_transaction` es hard delete y el único registro que existía
/// (`api_tokens.last_used_at`) tiene throttle de 60 s.
///
/// **Orden de escritura del registro** (por qué no miente):
///   1. `tracing::info!` primero — no necesita la base, así que sobrevive a que la base sea justo
///      lo que falla. Es el nivel por defecto de la imagen publicada (`futurefin_api=info`).
///   2. Se resuelve el gate.
///   3. UNA fila: `denied` + código si el gate rechazó (nace ya cerrada: el gate ES toda la
///      operación), `attempted` si dejó pasar. Aquí nunca se escribe `ok`: en este instante la
///      operación no ha corrido.
///   4. La operación corre.
///   5. [`McpWriteAudit::settle`] cierra la fila con `ok`/`failed`.
/// Un fallo escribiendo la auditoría se traga con un `warn!` y NO tumba la operación del usuario:
/// el precio es una escritura sin rastro en la tabla (mitigada por el paso 1, que no depende de
/// la base); el precio contrario —convertir una escritura ya válida en un 5xx porque el log no
/// pudo escribirse— es peor.
pub async fn require_mcp_write(
    pool: &sqlx::PgPool,
    id: &McpIdentity,
    tool: &str,
) -> Result<McpWriteAudit, ApiError> {
    let credential = match id.credential {
        McpCredential::ApiToken { token_id } => ("api_token", token_id),
        McpCredential::OAuth { token_id, .. } => ("oauth", token_id),
    };
    tracing::info!(
        tool,
        user_id = %id.user_id,
        installation_id = %id.installation_id,
        role = id.role.as_str(),
        scope = id.scope.as_str(),
        credential_kind = credential.0,
        credential_id = %credential.1,
        "mcp write attempt"
    );

    let verdict = evaluate_write_gate(pool, id).await;
    match &verdict {
        Ok(()) => {}
        // Un error de base al leer el toggle no es una denegación de política: no se sabe nada,
        // así que no se afirma nada. La traza del paso 1 ya quedó en el log.
        Err(ApiError::Db(_)) => return Err(verdict.unwrap_err()),
        Err(e) => {
            let code = ErrorBody::from_api_error(e).code;
            record_audit(pool, id, tool, credential, "denied", Some(&code)).await;
            return Err(verdict.unwrap_err());
        }
    }

    let row_id = record_audit(pool, id, tool, credential, "attempted", None).await;
    Ok(McpWriteAudit {
        row_id,
        tool: tool.to_string(),
    })
}

/// Las tres puertas, sin efectos secundarios. Separada para que `require_mcp_write` pueda
/// auditar el veredicto antes de propagarlo.
async fn evaluate_write_gate(pool: &sqlx::PgPool, id: &McpIdentity) -> Result<(), ApiError> {
    if !crate::handlers::membership::role_can_write(id.role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    if !id.scope.can_write() {
        return Err(ApiError::BadRequest(
            "mcp_token_read_only: esta credencial es de solo lectura; crea un token con permiso \
             de escritura en Ajustes → Integraciones si necesitas modificar datos"
                .into(),
        ));
    }
    let enabled: bool =
        sqlx::query_scalar(r#"SELECT mcp_write_enabled FROM installation WHERE id = $1"#)
            .bind(id.installation_id)
            .fetch_one(pool)
            .await?;
    if !enabled {
        return Err(ApiError::BadRequest(
            "mcp_write_disabled: la escritura vía MCP está desactivada en esta instalación \
             (Ajustes → Integraciones, solo el propietario puede activarla)"
                .into(),
        ));
    }
    Ok(())
}

/// Inserta la fila de auditoría y poda las caducadas. **Best-effort**: cualquier error sale por
/// `warn!` y devuelve `None`; nunca se propaga.
///
/// La poda vive aquí —en el camino de ESCRITURA que hace crecer la tabla, jamás en un GET (D5)—
/// siguiendo el precedente de `gc_orphan_clients` en `POST /oauth/register`. Es autorregulada:
/// una instalación parada no poda porque tampoco crece. Va DESPUÉS del INSERT a propósito: si la
/// poda falla (bloqueo, timeout), la fila de auditoría ya está escrita.
async fn record_audit(
    pool: &sqlx::PgPool,
    id: &McpIdentity,
    tool: &str,
    credential: (&str, Uuid),
    outcome: &str,
    error_code: Option<&str>,
) -> Option<Uuid> {
    let row: Result<Uuid, _> = sqlx::query_scalar(
        r#"INSERT INTO mcp_write_audit
               (installation_id, user_id, credential_kind, credential_id, role, tool,
                outcome, error_code, settled_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                   CASE WHEN $7 = 'attempted' THEN NULL ELSE now() END)
           RETURNING id"#,
    )
    .bind(id.installation_id)
    .bind(id.user_id)
    .bind(credential.0)
    .bind(credential.1)
    .bind(id.role.as_str())
    .bind(tool)
    .bind(outcome)
    .bind(error_code)
    .fetch_one(pool)
    .await;

    let row_id = match row {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(
                tool,
                outcome,
                error = ?e,
                "no se pudo registrar la auditoría de escritura MCP; la operación continúa"
            );
            None
        }
    };

    if let Err(e) = sqlx::query(
        r#"DELETE FROM mcp_write_audit WHERE at < now() - make_interval(days => $1)"#,
    )
    .bind(AUDIT_RETENTION_DAYS)
    .execute(pool)
    .await
    {
        tracing::warn!(error = ?e, "no se pudo podar mcp_write_audit; se reintenta en la siguiente escritura");
    }

    row_id
}
