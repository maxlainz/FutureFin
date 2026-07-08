//! Import de CSV bancario: `POST /v1/transactions/import/{preview,confirm}`.
//!
//! Stateless: el preview no escribe nada y devuelve un `file_sha256`; el confirm reenvía el
//! mismo `file_b64` + `file_sha256` (anti file-swap) más un `decisions[]` paralelo por índice.
//! Dedup por huella (source · op_date · importe canónico · concepto normalizado) + ordinal.
//! El confirm invalida la cache de proyección solo en modo `transactions_avg`
//! (`invalidate_projection_if_transactions_avg`); el preview nunca escribe ni invalida. Ver el
//! contrato en `transactions/mod.rs`.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::csv_presets::{
    decode_bytes, is_savings_hint, resolve_preset, transfer_flags, ParsedRow,
};
use crate::handlers::transactions::rules::{load_rules, match_rule, learn_rule, LoadedRule};
use crate::handlers::transactions::schema::{
    compute_fingerprint, derive_rule_pattern, normalize_kind, ImportConfirmBody,
    ImportConfirmResponse, ImportPreviewBody, ImportPreviewResponse, PreviewRow,
};
use crate::handlers::transactions::{
    assert_asset_in_installation, assert_liability_in_installation, assert_transaction_category,
    invalidate_projection_if_transactions_avg, next_fingerprint_ordinal,
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
        .map_err(|_| ApiError::BadRequest("file_b64 is not valid base64".into()))
}

fn default_kind_by_sign(amount: Decimal) -> String {
    if amount.is_sign_negative() {
        "expense".into()
    } else {
        "income".into()
    }
}

/// Sugerencia de `(kind, category_id)` para una fila del preview.
fn suggest_kind_category(
    matched: Option<&LoadedRule>,
    savings_hint: bool,
    amount: Decimal,
) -> (String, Option<Uuid>) {
    if let Some(r) = matched {
        let kind = r
            .assign_kind
            .clone()
            .unwrap_or_else(|| default_kind_by_sign(amount));
        return (kind, r.assign_category_id);
    }
    if savings_hint {
        return ("savings".into(), None);
    }
    (default_kind_by_sign(amount), None)
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

    let rules = load_rules(&state.pool, iid, user.id.0).await?;
    let cat_names = load_category_names(&state.pool, iid).await?;
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
        let (kind, category_id) = suggest_kind_category(matched, savings_hint, r.amount);
        if matched.is_some() {
            precat_count += 1;
        }
        let category_name = category_id.and_then(|id| cat_names.get(&id).cloned());
        let currency_warning = r.currency != "EUR";
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
            suggested_transfer: transfer[i],
            currency_warning,
            matched_rule_id: matched.map(|m| m.id),
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
        (status = 400, description = "sha/nº filas no coinciden con el preview, o validación de fila"),
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
    let original_filename = match &body.original_filename {
        Some(f) => {
            let t = f.trim();
            if t.chars().count() > 300 {
                return Err(ApiError::BadRequest(
                    "original_filename must be at most 300 characters".into(),
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
        if r.currency != "EUR" {
            return Err(ApiError::BadRequest(format!(
                "currency_not_eur: row {i} has currency '{}' (only EUR is supported)",
                r.currency
            )));
        }
        if r.amount.is_zero() {
            return Err(ApiError::BadRequest(format!(
                "amount_zero: row {i} has a zero amount"
            )));
        }
        let kind = normalize_kind(&d.kind)?;
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

        // Aprendizaje: solo cuando hay una decisión de categorización con contenido.
        if body.learn_rules && (d.category_id.is_some() || kind == "savings") {
            let pattern = derive_rule_pattern(&r.concept);
            learn_rule(&mut tx, iid, user.id.0, preset.id(), &pattern, &kind, d.category_id).await?;
            learned_patterns.insert((d.category_id, pattern));
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

    invalidate_projection_if_transactions_avg(&state, iid, user.id.0).await;
    Ok(Json(ImportConfirmResponse {
        import_id: final_import_id,
        imported,
        skipped_already_imported: skipped,
        discarded,
        rules_learned: learned_patterns.len() as u32,
    }))
}
