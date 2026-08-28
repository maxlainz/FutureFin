//! Claves de idempotencia del alta manual de movimientos (`POST /v1/transactions`).
//!
//! ## El problema
//!
//! `next_fingerprint_ordinal` resuelve el ordinal con `MAX(ordinal) + 1`, así que **un reintento
//! tras un timeout crea una segunda fila y responde 201**. Quien llama no puede distinguir «no
//! llegó» de «llegó y se perdió la respuesta», y ese es justo el estado en el que un cliente
//! desatendido reintenta. En los modos que usan transacciones (`transactions_avg`,
//! `budget_income_real_expense`) los movimientos son **inputs del motor**: un gasto duplicado
//! infla el promedio ponderado 12m, baja el ahorro mensual y retrasa la fecha de jubilación
//! proyectada — sin ningún síntoma visible.
//!
//! ## El contrato
//!
//! - **Opt-in.** Sin `idempotency_key` no se toca esta tabla y el comportamiento es exactamente el
//!   de siempre: reenviar el mismo movimiento crea OTRO movimiento. Los duplicados legítimos
//!   existen (dos cafés de 1,80 € el mismo día) y para eso está el `fingerprint_ordinal`; cambiar
//!   el default rompería un contrato documentado en la propia descripción de la tool MCP.
//! - **Misma clave + mismo cuerpo ⇒ la fila original**, sin crear nada y sin error. La respuesta es
//!   idéntica a la de la primera vez (mismo `id`, mismo 201): esa igualdad ES la idempotencia.
//! - **Misma clave + cuerpo distinto ⇒ 409 `idempotency_key_conflict`. Gana el primero.** Las otras
//!   dos salidas son peores: devolver la fila original diría «tu segundo movimiento, el distinto,
//!   se creó» —mentira que se materializa como un gasto que falta—, y crear una segunda fila
//!   anularía la clave justo cuando más señal da. Un cliente que reintenta con el cuerpo cambiado
//!   tiene un bug, y el servidor no debe premiarlo: le devuelve el `id` de la fila que ocupa la
//!   clave para que pueda mirarla y decidir.
//! - **Ámbito `(installation, owner)`.** La clave la elige el cliente: dos miembros pueden elegir
//!   la misma. Con ámbito de instalación, la clave de Bob «reproduciría» el movimiento de Alice y
//!   le devolvería una fila ajena — una fuga entre miembros, no una colisión benigna. Es además el
//!   mismo ámbito que el del `fingerprint` y el de todo lo per-user del módulo.
//! - **Caducidad 24 h con poda perezosa dentro del propio POST** (precedente:
//!   `oauth/register.rs::gc_orphan_clients`; nunca en un GET — D5, reads never mutate). Una clave
//!   protege contra el reintento de una petición **en vuelo**: la ventana útil son segundos, y 24 h
//!   deja tres órdenes de magnitud de margen. Caducar retira la PROTECCIÓN, nunca un movimiento.
//!
//! ## Lo que NO cubre
//!
//! Solo el alta individual. `POST /v1/transactions/batch` **rechaza** la clave en vez de ignorarla
//! (`idempotency_key_batch_unsupported`): un lote es todo-o-nada en una sola transacción, así que
//! una clave por ítem tendría que decidir qué hacer con «3 de 5 se reproducen», y aceptar el campo
//! para tirarlo sería la peor de las opciones — el llamante se creería protegido.

use crate::error::ApiError;
use crate::handlers::transactions::crud::PreparedTxn;
use crate::money::money_out;
use sqlx::PgConnection;
use uuid::Uuid;

/// Longitud máxima de la clave, en caracteres. Espejo del `CHECK` de la migración.
const MAX_KEY_CHARS: usize = 200;

/// Retención de las claves. Ver el doc del módulo: la ventana útil son segundos; esto es margen.
const RETENTION: &str = "24 hours";

/// Fila viva de la tabla: a qué movimiento pertenece la clave y con qué cuerpo se creó.
pub(super) struct ClaimedKey {
    pub(super) transaction_id: Uuid,
    pub(super) request_hash: String,
}

/// Normaliza y valida la clave del cuerpo. `None` (o el campo ausente) desactiva la idempotencia.
///
/// Un espacio en blanco no es una clave: si alguien manda `" "` está intentando activarla y
/// merece un error, no un silencio que la desactiva.
pub(super) fn normalize_key(raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > MAX_KEY_CHARS {
        return Err(ApiError::BadRequest(
            "idempotency_key_invalid: idempotency_key must be 1 to 200 characters".into(),
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "idempotency_key_invalid: idempotency_key must be 1 to 200 characters".into(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

/// Huella del cuerpo **ya validado**, no del JSON crudo.
///
/// Se hashean los valores NORMALIZADOS (`PreparedTxn`), así que dos peticiones que describen el
/// mismo movimiento con distinta forma —`"10"` y `"10.00"`, concepto con espacios de sobra— son
/// el mismo reintento y se reproducen en vez de chocar. `money_out` fija la escala a 4 decimales:
/// sin él, `Decimal::to_string()` conserva la escala de entrada y `"10"` ≠ `"10.0000"` para el
/// hash aunque sean el mismo importe.
///
/// Los campos de texto van con su longitud delante: un concepto que contenga el separador no
/// puede hacerse pasar por otro cuerpo.
pub(super) fn request_hash(p: &PreparedTxn, is_recurring: bool) -> String {
    fn field(out: &mut String, s: &str) {
        out.push_str(&format!("{}:{}\n", s.len(), s));
    }
    fn opt_uuid(out: &mut String, id: Option<Uuid>) {
        match id {
            Some(v) => field(out, &v.to_string()),
            None => field(out, ""),
        }
    }

    let mut buf = String::with_capacity(256);
    // Versión del formato: si algún día cambia lo que se hashea, las claves viejas no deben
    // «reproducirse» contra una huella calculada de otra manera.
    field(&mut buf, "v1");
    field(&mut buf, &p.op_date.to_string());
    match p.value_date {
        Some(d) => field(&mut buf, &d.to_string()),
        None => field(&mut buf, ""),
    }
    field(&mut buf, &p.concept);
    field(&mut buf, &money_out(p.amount).to_string());
    field(&mut buf, &p.kind);
    opt_uuid(&mut buf, p.category_id);
    opt_uuid(&mut buf, p.linked_asset_id);
    opt_uuid(&mut buf, p.linked_liability_id);
    field(&mut buf, p.notes.as_deref().unwrap_or(""));
    field(&mut buf, if is_recurring { "recurring" } else { "single" });
    crate::auth::secret::sha256_hex(buf.as_bytes())
}

/// Poda perezosa de las claves caducadas. **Best-effort**: corre en el camino de escritura y su
/// fallo no debe convertir un alta legítima en un 5xx (el cliente reintentaría… y duplicaría, que
/// es justo lo que este módulo existe para impedir).
pub(super) async fn gc_expired(pool: &sqlx::PgPool) {
    let sql = format!(
        "DELETE FROM transaction_idempotency_keys WHERE created_at < now() - interval '{RETENTION}'"
    );
    if let Err(e) = sqlx::query(&sql).execute(pool).await {
        tracing::warn!(error = ?e, "idempotency key GC skipped");
    }
}

/// Busca la clave viva de este `(installation, owner)`.
pub(super) async fn lookup(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner: Uuid,
    key: &str,
) -> Result<Option<ClaimedKey>, ApiError> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        r#"SELECT transaction_id, request_hash
           FROM transaction_idempotency_keys
           WHERE installation_id = $1 AND owner_user_id = $2 AND idempotency_key = $3"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(transaction_id, request_hash)| ClaimedKey {
        transaction_id,
        request_hash,
    }))
}

/// Reclama la clave para `transaction_id`, **en la misma transacción que el INSERT del movimiento**.
///
/// Devuelve `false` si otra petición con la misma clave ganó la carrera. `ON CONFLICT DO NOTHING`
/// en vez de mirar el SQLSTATE 23505 a mano: el mapeo SQLSTATE→HTTP vive solo en `error.rs` (I10)
/// y aquí no queremos un error, queremos una rama de control.
pub(super) async fn claim(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    key: &str,
    request_hash: &str,
    transaction_id: Uuid,
) -> Result<bool, ApiError> {
    let claimed: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO transaction_idempotency_keys
               (installation_id, owner_user_id, idempotency_key, request_hash, transaction_id)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (installation_id, owner_user_id, idempotency_key) DO NOTHING
           RETURNING transaction_id"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(key)
    .bind(request_hash)
    .bind(transaction_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(claimed.is_some())
}

/// Decide qué hacer con una clave ya ocupada: reproducir (mismo cuerpo) o 409 (cuerpo distinto).
///
/// La comparación del hash es una igualdad de cadenas normal, no `constant_time_eq`: aquí no hay
/// secreto que proteger — la clave la eligió el propio llamante y el hash es de su propio cuerpo.
pub(super) fn replay_or_conflict(existing: &ClaimedKey, request_hash: &str) -> Result<Uuid, ApiError> {
    if existing.request_hash == request_hash {
        return Ok(existing.transaction_id);
    }
    let id = existing.transaction_id;
    Err(ApiError::ConflictWith(format!(
        "idempotency_key_conflict: this idempotency_key already created transaction {id} with a different body; the first write wins — use a new key for a different movement"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_clave_en_blanco_es_un_error_no_un_silencio() {
        assert!(normalize_key(Some("   ".into())).is_err());
        assert!(normalize_key(Some("\u{0}abc".into())).is_err());
        assert!(normalize_key(Some("x".repeat(201))).is_err());
        assert_eq!(normalize_key(None).unwrap(), None);
        assert_eq!(
            normalize_key(Some("  k-1  ".into())).unwrap().as_deref(),
            Some("k-1")
        );
    }
}
