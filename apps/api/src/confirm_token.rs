//! Confirmación en **dos fases** de las escrituras MCP irreversibles (Fase 3, issue #84).
//!
//! ## El agujero
//!
//! Desde el issue #3 las tools destructivas piden `confirm: true` y sin él devuelven un preview.
//! Eso se lee como una salvaguarda de dos fases, pero `confirm` es **un booleano del propio
//! esquema de la tool**: nada impide que el modelo lo escriba en la PRIMERA llamada. Un
//! `delete_import` con `confirm: true` de entrada borra el lote y sus 400 movimientos sin que
//! nadie haya visto nunca el número 400. El preview era *prompting*, no un control.
//!
//! ## El contrato
//!
//! - El **preview** (`confirm: false`) es lo único que emite un token (`ffpv_…`), ligado a la
//!   tool, a los argumentos normalizados y a la **huella de los efectos que acaba de enseñar**.
//! - La **confirmación** lo exige: sin token, `confirm_token_required`; con uno que no case,
//!   `confirm_token_invalid`.
//! - Antes de escribir, el servidor **recalcula los efectos** y compara la huella. Si el mundo se
//!   movió entre las dos llamadas —el lote creció, el pasivo ganó movimientos vinculados—, el
//!   token ya no vale: `confirm_token_stale`. Es la ventana que el `confirm` booleano no podía ni
//!   ver, porque no había nada con lo que comparar.
//! - **Un solo uso** (`consumed_at` marcado dentro del mismo UPDATE que lo valida: dos
//!   confirmaciones simultáneas no pueden ganar las dos) y **TTL de 10 minutos**.
//!
//! El precedente exacto es `oauth_authorization_codes` (`oauth/token.rs`): un solo uso vía
//! `consumed_at`, TTL corto, secreto hash-only. La única diferencia deliberada es el plazo — allí
//! son 2 minutos porque quien responde es una máquina; aquí hay una persona leyendo un preview en
//! un chat, y 2 minutos convertirían la salvaguarda en un fallo intermitente. Caducado, no se
//! renueva: se vuelve a previsualizar, que además reenseña los números.
//!
//! ## Dónde se exige (y dónde NO)
//!
//! El token cuesta un round-trip extra por operación, así que **no** se exige en las 14 tools con
//! preview: solo donde confirmar sin haber mirado destruye algo que la conversación no puede
//! reconstruir. Ver `mcp/server.rs::TOKEN_GATED_TOOLS` para la lista y el criterio.
//!
//! ## Por qué vive aquí y no en `mcp/`
//!
//! Es maquinaria de protocolo con estado propio, como `oauth/`: `apps/api/src/mcp/` no contiene
//! SQL salvo el `SELECT` del kill-switch en `auth.rs` (D14 — una tool con SQL propio es un bloqueo
//! de revisión automático), y este módulo no es una core de dominio que puedan compartir los
//! handlers HTTP: **no hay** camino HTTP con dos fases. Sí es `pub` para que la suite de
//! integración pueda probar el ciclo completo emitir→consumir sin pasar por una tool.

use crate::auth::secret::{generate_opaque_secret, sha256_hex};
use crate::error::ApiError;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Prefijo del secreto. Sigue la familia de credenciales opacas del repo (`ffp_`, `ffo_`,
/// `ffr_`): un vistazo al valor dice qué es y de dónde salió.
pub const PREFIX: &str = "ffpv_";

/// Minutos que vive un token. Ver el doc del módulo: hay una persona en el bucle.
pub const TTL_MINUTES: i64 = 10;

/// Token recién emitido. El secreto viaja UNA vez, dentro del preview.
#[derive(Debug)]
pub struct IssuedToken {
    pub secret: String,
    pub expires_at: DateTime<Utc>,
}

/// Huella estable de un valor JSON: **claves de objeto ordenadas a todos los niveles**.
///
/// No se hashea `serde_json::to_string` a secas a propósito. El orden de las claves de un
/// `serde_json::Value` depende de si el crate está compilado con `preserve_order` (mapa de
/// inserción) o sin él (`BTreeMap`), y de en qué orden las escribió el `json!` que lo construyó.
/// Un hash que dependa de eso convertiría un cambio de dependencia —o de estilo— en un
/// `confirm_token_stale` intermitente sobre efectos idénticos: el peor fallo posible para una
/// salvaguarda, porque enseña a desconfiar de ella.
pub fn digest(value: &serde_json::Value) -> String {
    let mut buf = String::with_capacity(512);
    canonical(value, &mut buf);
    sha256_hex(buf.as_bytes())
}

fn canonical(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // Los números del wire MCP que importan aquí son enteros (contadores) o llegan ya como
        // strings decimales; `to_string` de un `serde_json::Number` es determinista.
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        // Con longitud delante: dos strings distintos no pueden concatenarse hasta parecer los
        // mismos ("ab"+"c" vs "a"+"bc").
        serde_json::Value::String(s) => {
            out.push_str(&format!("s{}:{}", s.len(), s));
        }
        serde_json::Value::Array(items) => {
            out.push_str(&format!("a{}[", items.len()));
            for item in items {
                canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str(&format!("o{}{{", keys.len()));
            for k in keys {
                out.push_str(&format!("k{}:{}=", k.len(), k));
                canonical(&map[k], out);
                out.push(',');
            }
            out.push('}');
        }
    }
}

/// Emite el token del preview y poda los caducados.
///
/// **Best-effort en la poda, estricto en la emisión**: si el INSERT falla, el preview falla — un
/// preview que promete un token que no existe dejaría al llamante en un bucle de
/// `confirm_token_invalid` sin explicación.
pub async fn issue(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    user_id: Uuid,
    tool: &str,
    args_hash: &str,
    effects_hash: &str,
) -> Result<IssuedToken, ApiError> {
    gc_expired(pool).await;
    let secret = generate_opaque_secret(PREFIX);
    let expires_at = Utc::now() + Duration::minutes(TTL_MINUTES);
    sqlx::query(
        r#"INSERT INTO mcp_confirm_tokens
               (token_hash, installation_id, user_id, tool, args_hash, effects_hash, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(sha256_hex(secret.as_bytes()))
    .bind(installation_id)
    .bind(user_id)
    .bind(tool)
    .bind(args_hash)
    .bind(effects_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(IssuedToken { secret, expires_at })
}

/// Consume el token de la confirmación. Devuelve `Ok(())` **solo** si el token existe, es de este
/// usuario y esta instalación, es de esta tool, no ha caducado, no se había usado, y los
/// argumentos y los efectos son los mismos que se previsualizaron.
///
/// El UPDATE marca `consumed_at` **antes** de comparar las huellas, y es deliberado: un token que
/// llega con los efectos cambiados ya ha cumplido su función (avisar), y dejarlo vivo permitiría
/// reintentar hasta que una carrera lo dejara pasar. Se consume y el llamante vuelve a
/// previsualizar — que es exactamente lo que debe hacer, porque los números que vio ya no valen.
pub async fn consume(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    user_id: Uuid,
    tool: &str,
    token: Option<&str>,
    args_hash: &str,
    effects_hash: &str,
) -> Result<(), ApiError> {
    // Literales COMPLETOS, nunca compuestos con `format!`: `error_codes_parity` extrae los códigos
    // del fuente y solo ve literales; uno interpolado degradaría en silencio al mensaje genérico
    // de la SPA.
    let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err(ApiError::BadRequest(
            "confirm_token_required: this operation cannot be confirmed blind — call it first \
             without confirm to get a preview, show the user its `effects`, and pass back the \
             `confirm_token` it returns together with confirm=true"
                .into(),
        ));
    };

    let row: Option<(String, String, String)> = sqlx::query_as(
        r#"UPDATE mcp_confirm_tokens
           SET consumed_at = now()
           WHERE token_hash = $1
             AND installation_id = $2
             AND user_id = $3
             AND consumed_at IS NULL
             AND expires_at > now()
           RETURNING tool, args_hash, effects_hash"#,
    )
    .bind(sha256_hex(token.as_bytes()))
    .bind(installation_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let Some((row_tool, row_args, row_effects)) = row else {
        return Err(ApiError::BadRequest(
            "confirm_token_invalid: the confirm_token is unknown, already used or expired (they \
             last 10 minutes and work exactly once) — run the preview again and confirm with the \
             new token"
                .into(),
        ));
    };
    if row_tool != tool || row_args != args_hash {
        return Err(ApiError::BadRequest(
            "confirm_token_invalid: this confirm_token was issued for a different operation — a \
             token only confirms the exact tool and target it previewed"
                .into(),
        ));
    }
    if row_effects != effects_hash {
        return Err(ApiError::BadRequest(
            "confirm_token_stale: the effects changed since the preview, so the confirmation no \
             longer describes what would happen — run the preview again, show the user the new \
             numbers and confirm with the new token"
                .into(),
        ));
    }
    Ok(())
}

/// Poda perezosa de los caducados y consumidos. Corre en el camino de escritura (D5: nunca en un
/// GET) y su fallo no tumba la operación.
async fn gc_expired(pool: &sqlx::PgPool) {
    if let Err(e) = sqlx::query(r#"DELETE FROM mcp_confirm_tokens WHERE expires_at < now()"#)
        .execute(pool)
        .await
    {
        tracing::warn!(error = ?e, "no se pudieron podar los confirm tokens caducados");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// El orden de las claves NO puede mover la huella: si lo moviera, un cambio de estilo en un
    /// `json!` produciría `confirm_token_stale` sobre efectos idénticos.
    #[test]
    fn la_huella_no_depende_del_orden_de_las_claves() {
        let a = json!({"entity": {"id": "x", "label": "Hipoteca"}, "side_effects": {"n": 3}});
        let b = json!({"side_effects": {"n": 3}, "entity": {"label": "Hipoteca", "id": "x"}});
        assert_eq!(digest(&a), digest(&b));
    }

    /// …pero cualquier cambio de CONTENIDO sí la mueve, incluido un contador que sube.
    #[test]
    fn la_huella_cambia_con_los_efectos() {
        let before = json!({"side_effects": {"transactions_deleted": 3}});
        let after = json!({"side_effects": {"transactions_deleted": 4}});
        assert_ne!(digest(&before), digest(&after));
    }

    /// Concatenar no puede fabricar colisiones: las longitudes van delante de cada string y clave.
    #[test]
    fn las_longitudes_impiden_colisiones_por_concatenacion() {
        assert_ne!(
            digest(&json!({"a": "bc", "d": ""})),
            digest(&json!({"a": "b", "cd": ""}))
        );
    }
}
