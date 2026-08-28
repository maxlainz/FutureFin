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
//! Las claves **por ítem** de un lote siguen rechazadas (`idempotency_key_batch_unsupported`): un
//! lote es todo-o-nada en una sola transacción, así que una clave por ítem tendría que decidir qué
//! hacer con «3 de 5 se reproducen», y aceptar el campo para tirarlo sería la peor de las opciones
//! — el llamante se creería protegido.
//!
//! ## El lote SÍ tiene clave: una sola, la suya (4.4.0)
//!
//! «3 de 5 se reproducen» no tiene semántica porque la pregunta está mal planteada: el lote no son
//! cinco unidades de trabajo con cinco claves, es **UNA** unidad de trabajo atómica. Y una unidad
//! de trabajo lleva una clave. `BatchCreateBody.idempotency_key` es esa clave: misma clave + mismo
//! cuerpo (los N ítems, en el mismo orden) ⇒ los N movimientos originales, sin crear nada; misma
//! clave + cuerpo distinto ⇒ 409 `idempotency_key_conflict`, gana el primero. El caso parcial no
//! existe, porque los N INSERT y las N reclamaciones viajan en la MISMA transacción.
//!
//! El detalle de almacenamiento: la tabla guarda `transaction_id` (uno), y un lote crea N. Se
//! escribe **una fila por ítem** con la clave derivada `{clave}#b{i}` y —esto es lo importante— el
//! hash del LOTE ENTERO en todas ellas. Así:
//!   * cualquier cambio en cualquier ítem, o en su orden, o en el número de ítems, mueve el hash de
//!     las N filas a la vez → el reintento choca en la primera y nunca «reproduce a medias»;
//!   * las N filas se escriben y se deshacen juntas con los N movimientos;
//!   * una colisión con una clave individual que el usuario haya llamado literalmente `X#b3` es
//!     posible y **ruidosa** (409), nunca un resultado silenciosamente incorrecto.
//! No hace falta ninguna tabla ni columna nueva.

use crate::error::ApiError;
use crate::handlers::transactions::crud::PreparedTxn;
use crate::money::money_out;
use sqlx::PgConnection;
use uuid::Uuid;

/// Longitud máxima de la clave, en caracteres. Espejo del `CHECK` de la migración.
const MAX_KEY_CHARS: usize = 200;

/// Longitud máxima de la clave de un LOTE. Más corta que la individual porque el lote almacena una
/// fila por ítem bajo la clave derivada `{clave}#b{i}`, y el `CHECK` de la columna corta en 200.
/// Con `MAX_BATCH = 1000` el sufijo ocupa 5 caracteres: 180 + 5 deja margen de sobra.
const MAX_BATCH_KEY_CHARS: usize = 180;

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

/// Emite un campo con su longitud delante: un texto que contenga el separador no puede hacerse
/// pasar por otro cuerpo.
fn field(out: &mut String, s: &str) {
    out.push_str(&format!("{}:{}\n", s.len(), s));
}

fn opt_uuid(out: &mut String, id: Option<Uuid>) {
    match id {
        Some(v) => field(out, &v.to_string()),
        None => field(out, ""),
    }
}

/// Vuelca UN movimiento ya validado al búfer. Lo comparten la huella individual y la del lote: si
/// divergieran, un lote de un solo ítem y su alta individual dejarían de hashear lo mismo por
/// razones que nadie recordaría.
fn append_body(buf: &mut String, p: &PreparedTxn, is_recurring: bool) {
    field(buf, &p.op_date.to_string());
    match p.value_date {
        Some(d) => field(buf, &d.to_string()),
        None => field(buf, ""),
    }
    field(buf, &p.concept);
    field(buf, &money_out(p.amount).to_string());
    field(buf, &p.kind);
    opt_uuid(buf, p.category_id);
    opt_uuid(buf, p.linked_asset_id);
    opt_uuid(buf, p.linked_liability_id);
    field(buf, p.notes.as_deref().unwrap_or(""));
    field(buf, if is_recurring { "recurring" } else { "single" });
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
    let mut buf = String::with_capacity(256);
    // Versión del formato: si algún día cambia lo que se hashea, las claves viejas no deben
    // «reproducirse» contra una huella calculada de otra manera.
    field(&mut buf, "v1");
    append_body(&mut buf, p, is_recurring);
    crate::auth::secret::sha256_hex(buf.as_bytes())
}

/// Huella del LOTE ENTERO: el marcador de formato, el NÚMERO de ítems y los ítems en orden.
///
/// El número entra en el hash a propósito. Sin él, un reintento con el mismo prefijo y menos ítems
/// hashearía distinto solo por el contenido — y si alguna vez coincidiera, «reproducir» devolvería
/// más movimientos de los que el llamante creía haber pedido. Con `n` dentro, cualquier cambio de
/// tamaño es un 409, no una sorpresa.
pub(super) fn batch_request_hash(prepared: &[PreparedTxn], recurring: &[bool]) -> String {
    let mut buf = String::with_capacity(256 * prepared.len().max(1));
    field(&mut buf, "batch-v1");
    field(&mut buf, &prepared.len().to_string());
    for (p, r) in prepared.iter().zip(recurring.iter()) {
        append_body(&mut buf, p, *r);
    }
    crate::auth::secret::sha256_hex(buf.as_bytes())
}

/// Clave derivada del ítem `i` de un lote. Ver el doc del módulo.
pub(super) fn batch_item_key(key: &str, i: usize) -> String {
    format!("{key}#b{i}")
}

/// Normaliza la clave de un LOTE (misma validación que la individual, con un tope más corto para
/// que quepa el sufijo derivado). Reutiliza el código `idempotency_key_invalid`: es el mismo
/// problema del cliente y merece la misma frase en la UI.
pub(super) fn normalize_batch_key(raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(key) = normalize_key(raw)? else {
        return Ok(None);
    };
    if key.chars().count() > MAX_BATCH_KEY_CHARS {
        return Err(ApiError::BadRequest(
            "idempotency_key_invalid: a batch idempotency_key must be 1 to 180 characters".into(),
        ));
    }
    Ok(Some(key))
}

/// Recupera, EN ORDEN, los `n` movimientos que un lote creó bajo `key`.
///
/// Exige que las `n` filas existan y que todas lleven el hash del lote. Una fila que falta no es un
/// «reproduce lo que puedas»: significa que el movimiento se borró después (la FK es
/// `ON DELETE CASCADE`), así que el lote original ya no existe entero y devolverlo sería mentir.
/// Eso es un 409, con el mismo código que cualquier otra reutilización de clave que no se puede
/// honrar.
pub(super) async fn lookup_batch_ids(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner: Uuid,
    key: &str,
    n: usize,
    request_hash: &str,
) -> Result<Vec<Uuid>, ApiError> {
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let derived = batch_item_key(key, i);
        let Some(existing) = lookup(pool, iid, owner, &derived).await? else {
            return Err(ApiError::ConflictWith(
                "idempotency_key_conflict: this idempotency_key created a batch whose movements no longer exist in full (one of them was deleted); use a new key".into(),
            ));
        };
        ids.push(replay_or_conflict(&existing, request_hash)?);
    }
    Ok(ids)
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
