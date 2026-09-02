//! Import de CSV bancario: `POST /v1/transactions/import/{preview,confirm}`.
//!
//! Stateless: el preview no escribe nada y devuelve un `file_sha256`; el confirm reenvía el
//! mismo `file_b64` + `file_sha256` (anti file-swap) más un `decisions[]` paralelo por índice.
//! Dedup por huella (source · op_date · importe canónico · concepto normalizado) + ordinal.
//! El confirm invalida la cache de proyección solo en los modos que usan transacciones
//! (`transactions_avg` y `budget_income_real_expense`, vía
//! `invalidate_projection_if_savings_uses_transactions`); el preview nunca escribe ni invalida. Ver el
//! contrato en `transactions/mod.rs`.

use crate::error::ApiError;
use crate::handlers::installation::installation_base_currency;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::csv_presets::{
    decode_bytes, is_savings_hint, resolve_preset, transfer_flags, ParsedRow,
};
use crate::handlers::transactions::rules::{load_rules, match_rule, learn_rule, LoadedRule};
use crate::handlers::transactions::schema::{
    compute_fingerprint, derive_rule_pattern, normalize_kind, ImportConfirmBody,
    ImportConfirmResponse, ImportPreviewBody, ImportPreviewResponse, PendingAssignment,
    PreviewRow,
};
use crate::handlers::transactions::reconcile::auto_reconcile_after_mutation;
use crate::handlers::transactions::{
    assert_asset_in_installation, assert_liability_in_installation, assert_transaction_category,
    invalidate_projection_if_savings_uses_transactions, next_fingerprint_ordinal,
};
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn decode_file_b64(file_b64: &str) -> Result<Vec<u8>, ApiError> {
    B64.decode(file_b64.trim().as_bytes())
        .map_err(|_| ApiError::BadRequest("file_b64_invalid: file_b64 is not valid base64".into()))
}

fn default_kind_by_sign(amount: Decimal) -> String {
    if amount.is_sign_negative() {
        "expense".into()
    } else {
        "income".into()
    }
}

/// Las dos categorías POR DEFECTO de la instalación (4.15.0), cargadas UNA vez por request.
///
/// `Option` y no `Uuid` a propósito: el preview es una lectura y no debe reventar por un estado
/// que él no puede arreglar. La migración `20260902120000` garantiza que existen; si aun así
/// faltara una, la fila se sugiere sin categoría y quien se lleva el 400 con nombre propio
/// (`category_required`, y `fallback_category_missing` en las demás vías) es el confirm — que es
/// donde el usuario puede hacer algo al respecto.
#[derive(Debug, Default, Clone, Copy)]
struct Fallbacks {
    income: Option<Uuid>,
    expense: Option<Uuid>,
}

impl Fallbacks {
    fn for_kind(&self, kind: &str) -> Option<Uuid> {
        match kind {
            "income" => self.income,
            "expense" => self.expense,
            _ => None,
        }
    }

    /// `true` si `category_id` es justo la categoría por defecto de `kind`. Es el gate del
    /// aprendizaje de reglas: ver `learn_rules` en el confirm.
    fn is_fallback_of(&self, kind: &str, category_id: Option<Uuid>) -> bool {
        category_id.is_some() && category_id == self.for_kind(kind)
    }
}

async fn load_fallbacks(pool: &sqlx::PgPool, iid: Uuid) -> Result<Fallbacks, ApiError> {
    let rows: Vec<(String, Uuid)> = sqlx::query_as(
        r#"SELECT scope, id FROM categories WHERE installation_id = $1 AND is_fallback"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;
    let mut out = Fallbacks::default();
    for (scope, id) in rows {
        match scope.as_str() {
            "income" => out.income = Some(id),
            "expense" => out.expense = Some(id),
            _ => {}
        }
    }
    Ok(out)
}

/// Sugerencia de `(kind, category_id, procedencia)` para una fila del preview.
///
/// La procedencia (`"rule"` | `"fallback"`) viaja al wizard porque las dos categorías se pintan
/// igual y significan cosas distintas: una la eligió el usuario alguna vez, la otra es el cajón.
/// El wizard la usa para no propagar la por defecto por automatch y para no ofrecerla como regla.
fn suggest_kind_category(
    matched: Option<&LoadedRule>,
    savings_hint: bool,
    amount: Decimal,
    fallbacks: &Fallbacks,
) -> (String, Option<Uuid>, Option<&'static str>) {
    let (kind, from_rule) = match matched {
        Some(r) => (
            r.assign_kind
                .clone()
                .unwrap_or_else(|| default_kind_by_sign(amount)),
            r.assign_category_id,
        ),
        None if savings_hint => ("savings".to_string(), None),
        None => (default_kind_by_sign(amount), None),
    };
    if kind == "savings" {
        // La inversión no lleva categoría por diseño: ni regla ni cajón.
        return (kind, None, None);
    }
    match from_rule {
        Some(c) => (kind, Some(c), Some("rule")),
        None => {
            let fb = fallbacks.for_kind(&kind);
            let source = fb.is_some().then_some("fallback");
            (kind, fb, source)
        }
    }
}

/// Máximo de `pending_assignments` por preview: cota de sanidad muy por encima de cualquier
/// sesión real del wizard (una asignación por concepto distinto ya clasificado).
const PENDING_ASSIGNMENTS_MAX: usize = 200;

/// Convierte los `pending_assignments` de la sesión del wizard en reglas EFÍMERAS con la
/// misma forma que crearía `learn_rule` en el confirm (`substring` + patrón derivado + source
/// del preset), para que `match_rule` las evalúe con la precedencia completa junto a las
/// persistidas. `updated_at = now()` les da la frescura que tendría la regla recién aprendida
/// (gana los empates de precedencia, igual que ganaría tras persistirse).
///
/// Mismo gate que el aprendizaje real: sin categoría y sin `kind=savings` no hay regla; **y
/// tampoco la hay cuando la categoría es la POR DEFECTO del scope** (4.15.0). El porqué es el
/// mismo en las dos puertas: desde que todo ingreso/gasto lleva categoría, «Otros gastos» es lo
/// que el servidor pone cuando NO sabe, no una decisión del usuario. Aprenderla —o propagarla por
/// automatch dentro de la misma sesión del wizard— convertiría cada import en cientos de reglas
/// «X → Otros gastos» que después ganan la precedencia y tapan a las reglas de verdad.
/// Y el mismo guard que el confirm: un patrón derivado vacío (concepto vacío) no genera regla —
/// un substring vacío matchearía TODOS los conceptos.
async fn ephemeral_rules_from_pending(
    pool: &sqlx::PgPool,
    iid: Uuid,
    preset_id: &str,
    pending: &[PendingAssignment],
    fallbacks: &Fallbacks,
) -> Result<Vec<LoadedRule>, ApiError> {
    if pending.len() > PENDING_ASSIGNMENTS_MAX {
        return Err(ApiError::BadRequest(format!(
            "pending_assignments_too_many: at most {PENDING_ASSIGNMENTS_MAX} entries"
        )));
    }
    let now = chrono::Utc::now();
    let mut validated_categories: HashSet<(String, Option<Uuid>)> = HashSet::new();
    let mut out = Vec::new();
    for pa in pending {
        let kind = normalize_kind(&pa.kind)?;
        if pa.category_id.is_none() && kind != "savings" {
            continue;
        }
        if fallbacks.is_fallback_of(&kind, pa.category_id) {
            continue;
        }
        // Valida kind↔scope de la categoría una sola vez por combinación (son repetitivas).
        if validated_categories.insert((kind.clone(), pa.category_id)) {
            assert_transaction_category(pool, iid, &kind, pa.category_id).await?;
        }
        let pattern = derive_rule_pattern(&pa.concept);
        if pattern.is_empty() {
            continue;
        }
        out.push(LoadedRule {
            id: Uuid::nil(),
            match_kind: "substring".into(),
            pattern,
            source: Some(preset_id.to_string()),
            assign_kind: Some(kind),
            assign_category_id: pa.category_id,
            updated_at: now,
            ephemeral: true,
        });
    }
    Ok(out)
}

/// Conteo de filas ya en BD por huella (para el estado `already_imported`).
async fn existing_counts(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner: Uuid,
    fingerprints: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    if fingerprints.is_empty() {
        return Ok(HashMap::new());
    }
    let mut uniq: Vec<String> = fingerprints.to_vec();
    uniq.sort();
    uniq.dedup();
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT fingerprint, COUNT(*)::bigint
           FROM transactions
           WHERE installation_id = $1 AND owner_user_id = $2 AND fingerprint = ANY($3)
           GROUP BY fingerprint"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(&uniq)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn load_category_names(
    pool: &sqlx::PgPool,
    iid: Uuid,
) -> Result<HashMap<Uuid, String>, ApiError> {
    let rows: Vec<(Uuid, String)> =
        sqlx::query_as(r#"SELECT id, name FROM categories WHERE installation_id = $1"#)
            .bind(iid)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/import/preview
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions/import/preview",
    tag = "transactions",
    request_body = ImportPreviewBody,
    responses(
        (status = 200, description = "Preview del import (sin escrituras)", body = ImportPreviewResponse),
        (status = 400, description = "CSV irreconocible / base64 inválido / parseo"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn import_preview(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ImportPreviewBody>,
) -> Result<Json<ImportPreviewResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let bytes = decode_file_b64(&body.file_b64)?;
    let file_sha256 = sha256_hex(&bytes);
    let text = decode_bytes(&bytes);
    let preset = resolve_preset(&body.source, &text)?;
    let rows: Vec<ParsedRow> = preset.parse(&text)?;

    assert_asset_in_installation(&state.pool, iid, body.account_asset_id).await?;

    let fallbacks = load_fallbacks(&state.pool, iid).await?;
    let mut rules = load_rules(&state.pool, iid, user.id.0).await?;
    rules.extend(
        ephemeral_rules_from_pending(
            &state.pool,
            iid,
            preset.id(),
            &body.pending_assignments,
            &fallbacks,
        )
        .await?,
    );
    let cat_names = load_category_names(&state.pool, iid).await?;
    let base_currency = installation_base_currency(&state.pool, iid).await?;
    let transfer = transfer_flags(&rows);

    let fingerprints: Vec<String> = rows
        .iter()
        .map(|r| compute_fingerprint(preset.id(), r.op_date, r.amount, &r.concept))
        .collect();
    let existing = existing_counts(&state.pool, iid, user.id.0, &fingerprints).await?;

    let mut seen: HashMap<String, i64> = HashMap::new();
    let mut out_rows = Vec::with_capacity(rows.len());
    let (mut new_count, mut already_count, mut transfer_count, mut precat_count, mut cur_warn_count) =
        (0u32, 0u32, 0u32, 0u32, 0u32);

    for (i, r) in rows.iter().enumerate() {
        let fp = &fingerprints[i];
        let already = existing.get(fp).copied().unwrap_or(0);
        let seen_n = seen.entry(fp.clone()).or_insert(0);
        let status = if *seen_n < already {
            already_count += 1;
            "already_imported"
        } else {
            new_count += 1;
            "new"
        };
        *seen_n += 1;

        let matched = match_rule(&rules, preset.id(), &r.concept);
        let savings_hint = is_savings_hint(&r.concept);
        let (kind, category_id, category_source) =
            suggest_kind_category(matched, savings_hint, r.amount, &fallbacks);
        if matched.is_some() {
            precat_count += 1;
        }
        let category_name = category_id.and_then(|id| cat_names.get(&id).cloned());
        let currency_warning = r.currency != base_currency;
        if currency_warning {
            cur_warn_count += 1;
        }
        if transfer[i] {
            transfer_count += 1;
        }

        out_rows.push(PreviewRow {
            index: i as u32,
            op_date: r.op_date.format("%Y-%m-%d").to_string(),
            value_date: r.value_date.map(|d| d.format("%Y-%m-%d").to_string()),
            concept: r.concept.clone(),
            amount: r.amount,
            currency: r.currency.clone(),
            status: status.into(),
            suggested_kind: kind,
            suggested_category_id: category_id,
            suggested_category_name: category_name,
            suggested_category_source: category_source,
            suggested_transfer: transfer[i],
            currency_warning,
            // Una regla efímera no está persistida: su id sintético no debe publicarse.
            matched_rule_id: matched.filter(|m| !m.ephemeral).map(|m| m.id),
        });
    }

    Ok(Json(ImportPreviewResponse {
        source: preset.id().to_string(),
        file_sha256,
        row_count: rows.len() as u32,
        new_count,
        already_imported_count: already_count,
        suggested_transfer_count: transfer_count,
        precategorized_count: precat_count,
        currency_warning_count: cur_warn_count,
        rows: out_rows,
    }))
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/import/confirm
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions/import/confirm",
    tag = "transactions",
    request_body = ImportConfirmBody,
    responses(
        (status = 200, description = "Import aplicado", body = ImportConfirmResponse),
        (status = 400, description = "sha/nº filas no coinciden con el preview, o validación de fila (incluido `category_required`: una decisión income/expense sin `category_id`)"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
        (status = 409, description = "Huella duplicada (doble-confirm concurrente)"),
    )
)]
pub async fn import_confirm(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ImportConfirmBody>,
) -> Result<Json<ImportConfirmResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    let bytes = decode_file_b64(&body.file_b64)?;
    let file_sha256 = sha256_hex(&bytes);
    if !file_sha256.eq_ignore_ascii_case(body.file_sha256.trim()) {
        return Err(ApiError::BadRequest(
            "preview_confirm_mismatch: file_sha256 does not match the previewed file".into(),
        ));
    }
    let text = decode_bytes(&bytes);
    let preset = resolve_preset(&body.source, &text)?;
    let rows: Vec<ParsedRow> = preset.parse(&text)?;

    if body.decisions.len() != rows.len() {
        return Err(ApiError::BadRequest(format!(
            "preview_confirm_mismatch: decisions ({}) must be parallel to the {} parsed rows",
            body.decisions.len(),
            rows.len()
        )));
    }

    // Validación temprana del `account_asset_id` y `original_filename`.
    assert_asset_in_installation(&state.pool, iid, body.account_asset_id).await?;
    let base_currency = installation_base_currency(&state.pool, iid).await?;
    let fallbacks = load_fallbacks(&state.pool, iid).await?;
    let original_filename = match &body.original_filename {
        Some(f) => {
            let t = f.trim();
            if t.chars().count() > 300 {
                return Err(ApiError::BadRequest(
                    "original_filename_too_long: original_filename must be at most 300 characters".into(),
                ));
            }
            (!t.is_empty()).then(|| t.to_string())
        }
        None => None,
    };

    let fingerprints: Vec<String> = rows
        .iter()
        .map(|r| compute_fingerprint(preset.id(), r.op_date, r.amount, &r.concept))
        .collect();
    let existing = existing_counts(&state.pool, iid, user.id.0, &fingerprints).await?;

    let mut tx = state.pool.begin().await?;

    // Cabecera del lote (se borra al final si no se importa nada → import_id = null).
    let import_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO transaction_imports
               (installation_id, owner_user_id, source, account_asset_id, original_filename)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(preset.id())
    .bind(body.account_asset_id)
    .bind(original_filename.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    let mut seen: HashMap<String, i64> = HashMap::new();
    let mut next_ord: HashMap<String, i32> = HashMap::new();
    let mut learned_patterns: HashSet<(Option<Uuid>, String)> = HashSet::new();
    let (mut imported, mut skipped, mut discarded) = (0u32, 0u32, 0u32);

    for (i, r) in rows.iter().enumerate() {
        let d = &body.decisions[i];
        let fp = &fingerprints[i];

        // Estado (avanza el contador de ocurrencias para TODAS las filas, se importe o no).
        let already = existing.get(fp).copied().unwrap_or(0);
        let seen_n = seen.entry(fp.clone()).or_insert(0);
        let is_already = *seen_n < already;
        *seen_n += 1;

        if d.discard {
            discarded += 1;
            continue;
        }
        if is_already && !d.force {
            skipped += 1;
            continue;
        }

        // Validaciones de la fila a importar.
        if r.currency != base_currency {
            return Err(ApiError::BadRequest(format!(
                "currency_mismatch: row {i} has currency '{}' but this installation keeps its books in {base_currency}",
                r.currency
            )));
        }
        if r.amount.is_zero() {
            return Err(ApiError::BadRequest(format!(
                "amount_zero: row {i} has a zero amount"
            )));
        }
        let kind = normalize_kind(&d.kind)?;
        // ESTRICTO a propósito, y es la única vía de escritura que no rellena la categoría por
        // defecto en silencio: en el wizard la categoría de cada fila SE VE, y el preview ya la
        // trae puesta (`suggested_category_source: "fallback"`). Un confirm sin categoría es una
        // decisión que se perdió por el camino —una fila que el cliente construyó a mano, un
        // preview de antes de 4.15.0—, no una elección; aceptarla y silenciarla con el cajón
        // enterraría el error en la atribución de un mes entero.
        if kind != "savings" && d.category_id.is_none() {
            return Err(ApiError::BadRequest(format!(
                "category_required: row {i} is '{kind}' and carries no category_id; every income and expense needs one (the preview already suggests the default category of its scope)"
            )));
        }
        assert_transaction_category(&state.pool, iid, &kind, d.category_id).await?;
        assert_asset_in_installation(&state.pool, iid, d.linked_asset_id).await?;
        assert_liability_in_installation(&state.pool, iid, d.linked_liability_id).await?;

        let ord = match next_ord.get(fp) {
            Some(&o) => o,
            None => next_fingerprint_ordinal(&mut tx, iid, user.id.0, fp).await?,
        };

        sqlx::query(
            r#"INSERT INTO transactions
                   (installation_id, owner_user_id, import_id, source, op_date, value_date,
                    concept, amount, currency, kind, category_id, fingerprint, fingerprint_ordinal,
                    linked_asset_id, linked_liability_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'EUR', $9, $10, $11, $12, $13, $14)"#,
        )
        .bind(iid)
        .bind(user.id.0)
        .bind(import_id)
        .bind(preset.id())
        .bind(r.op_date)
        .bind(r.value_date)
        .bind(&r.concept)
        .bind(r.amount)
        .bind(&kind)
        .bind(d.category_id)
        .bind(fp)
        .bind(ord)
        .bind(d.linked_asset_id)
        .bind(d.linked_liability_id)
        .execute(&mut *tx)
        .await?;

        next_ord.insert(fp.clone(), ord + 1);
        imported += 1;

        // Aprendizaje: solo cuando hay una decisión de categorización con contenido. Un
        // patrón derivado vacío no se aprende: como substring matchearía TODOS los conceptos
        // futuros del banco. Hoy es inalcanzable desde aquí (`clean_concept` convierte el
        // concepto vacío en «(sin concepto)»), pero la creación manual lo rechaza
        // (`rule_pattern_empty`) y esta puerta no debe ser la única sin el guard — el mismo
        // que aplica `ephemeral_rules_from_pending` a los conceptos que manda el cliente.
        // El gate del aprendizaje lleva desde 4.15.0 una condición más: **nunca se aprende la
        // categoría POR DEFECTO**. Es la que el servidor pone cuando ninguna regla casó, así que
        // aprenderla escribiría una regla «este concepto → Otros gastos» por cada concepto nuevo
        // del extracto —cientos tras el primer import— y esas reglas ganarían después la
        // precedencia sobre las que el usuario sí quiso. Mismo criterio que
        // `ephemeral_rules_from_pending`.
        if body.learn_rules
            && (d.category_id.is_some() || kind == "savings")
            && !fallbacks.is_fallback_of(&kind, d.category_id)
        {
            let pattern = derive_rule_pattern(&r.concept);
            if !pattern.is_empty() {
                learn_rule(&mut tx, iid, user.id.0, preset.id(), &pattern, &kind, d.category_id)
                    .await?;
                learned_patterns.insert((d.category_id, pattern));
            }
        }
    }

    let final_import_id = if imported == 0 {
        // Lote vacío: se borra para no dejar cabeceras sin transacciones.
        sqlx::query(r#"DELETE FROM transaction_imports WHERE id = $1"#)
            .bind(import_id)
            .execute(&mut *tx)
            .await?;
        None
    } else {
        Some(import_id)
    };

    tx.commit().await?;

    // Pase de auto-conciliación sobre TODO el dataset del owner (no solo este lote): la
    // contrapartida de una pata recién importada puede venir de un import anterior. Best-effort
    // (0 si falla) y ANTES de la única invalidación de cache.
    let reconciled_pairs = if imported > 0 {
        auto_reconcile_after_mutation(&state, iid, user.id.0).await
    } else {
        0
    };
    // Un import ACTIVA meses (les mete su primer movimiento real) → convergencia antes de la
    // única invalidación: los recurrentes de esos meses nacen aquí.
    if imported > 0 {
        crate::handlers::transactions::recurring::converge_recurring_after_mutation(&state, iid)
            .await;
    }
    invalidate_projection_if_savings_uses_transactions(&state, iid, user.id.0).await;
    Ok(Json(ImportConfirmResponse {
        import_id: final_import_id,
        imported,
        skipped_already_imported: skipped,
        discarded,
        rules_learned: learned_patterns.len() as u32,
        reconciled_pairs,
    }))
}