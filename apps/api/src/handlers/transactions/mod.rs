//! Histórico de gasto mensual: transacciones datadas (`/v1/transactions`).
//!
//! Importa CSV bancarios (MyInvestor/N26) o efectivo a mano, los categoriza con las categorías
//! de Ingreso/Gasto, y sirve la comparativa mes real vs presupuesto vs promedio histórico.
//! CRUD per-user (scoping con `LedgerView` en las lecturas; `owner_user_id` en las escrituras).
//!
//! ## Contrato de cache condicionado al modo (`savings_source`)
//! Por defecto (`savings_source = budget`, modo A) las transacciones **no son inputs del motor de
//! proyección** (que arranca en el mes 0 con el ledger vivo, no con el histórico de gasto) → sus
//! mutaciones **no invalidan** la cache de proyección: invalidar solo tiraría una entrada caliente
//! sin cambiar ni un número.
//!
//! En cambio, en los modos que **usan transacciones** (`transactions_avg`, modo B; y
//! `budget_income_real_expense`, modo C) la proyección deriva el gasto (y en B también el income) del
//! promedio real 12m → las transacciones **SÍ son inputs del engine**, así que toda mutación que
//! cambie el conjunto de transacciones **debe invalidar** la cache. Ese gating vive en
//! `invalidate_projection_if_savings_uses_transactions` (abajo, sobre `SavingsSource::uses_transactions`)
//! y lo llaman las mutaciones de `crud.rs`/`import.rs`/`recurring.rs`/`reconcile.rs` tras commitear
//! (conciliar/desconciliar cambia QUÉ cuenta en el promedio → también es mutación del conjunto).
//! **Crear** una regla de categorización y los previews **nunca** invalidan. El **borrado de una
//! plantilla recurrente SÍ invalida (COND, corrección 4.0.0)**: sus instancias sobreviven
//! (`ON DELETE SET NULL`) pero pierden el enlace y pasan a contar como movimientos REALES — cambia
//! la **clasificación**, no el conjunto, y eso puede activar un mes que el promedio ignoraba
//! (mecanismo completo en la cabecera de `delete_recurring_rule_core`, `recurring.rs`). Regresión:
//! `transactions_projection_cache.rs`. Ojo: **aplicar** una regla retroactivamente (backfill) sí es
//! una mutación del conjunto y sí invalida COND — es otra ruta, con su propio test.
//!
//! ## Conciliación de transferencias (3.5.0)
//! Un movimiento **conciliado** (`transfer_counterpart_id IS NOT NULL`) es una pata de una
//! transferencia interna enlazada a su contrapartida: sigue visible en los listados pero queda
//! excluido de todos los agregados de flujo (`summary.rs`, `history/cashflow` months). El pase de
//! auto-conciliación (`reconcile.rs`) corre post-commit tras toda mutación del conjunto, en
//! best-effort, ANTES de la invalidación de cache (una sola invalidación cubre mutación + pase).

pub mod aggregate;
pub mod crud;
pub mod csv_presets;
pub mod duplicates;
pub mod idempotency;
pub mod import;
pub mod reconcile;
pub mod recurring;
pub mod rules;
pub mod schema;
pub mod summary;

use crate::error::ApiError;
use crate::handlers::installation::projection_savings_source;
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use chrono::NaiveDate;
use sqlx::{FromRow, PgConnection};
use std::sync::Arc;
use uuid::Uuid;

// Re-exports para el wiring de openapi.rs.
pub use aggregate::aggregate_transactions;
pub use crud::{
    create_batch, create_transaction, delete_import, delete_transaction, list_imports,
    list_months, list_transactions, patch_batch, patch_transaction,
};
pub use duplicates::list_duplicates;
pub use import::{import_confirm, import_preview};
pub use reconcile::{
    confirm_transfer_match, reconcile_now, reconcile_pair, suggest_transfer_matches,
    unreconcile_transaction,
};
pub use recurring::{delete_recurring_rule, list_recurring_rules, materialize_recurring};
pub use rules::{apply_rule, create_rule, delete_rule, list_rules, patch_rule};
pub use summary::{get_category_series, get_transactions_summary};

pub use schema::{
    BatchCreateBody, CategoryComparisonLine, CreateRuleBody, CreateTransactionBody,
    CreateTransactionRequest,
    ImportBatchResponse, ImportConfirmBody, ImportConfirmResponse, ImportDecision, ImportPreviewBody,
    ImportPreviewResponse, MaterializeResponse, MonthEntry, PatchRuleBody, PatchTransactionBody,
    PendingAssignment, PreviewRow, RecurrenceSpec, ReconcilePairBody, ReconcilePairResponse,
    ReconcileRunResponse,
    RecurringRuleResponse, RuleResponse, TransactionResponse, TransactionsSummaryResponse,
};

// ---------------------------------------------------------------------------
// Fila de transacción + builder de response (compartido por crud.rs e import.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
pub struct TxnRow {
    pub id: Uuid,
    pub import_id: Option<Uuid>,
    pub source: String,
    pub op_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub concept: String,
    pub amount: rust_decimal::Decimal,
    pub currency: String,
    pub kind: Option<String>,
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub linked_asset_id: Option<Uuid>,
    pub linked_liability_id: Option<Uuid>,
    pub notes: Option<String>,
    pub recurring_rule_id: Option<Uuid>,
    pub transfer_counterpart_id: Option<Uuid>,
    pub transfer_reconciled_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transfer_reconciled_source: Option<String>,
    pub transfer_counterpart_concept: Option<String>,
    pub transfer_counterpart_op_date: Option<NaiveDate>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub const TXN_SELECT: &str = r#"SELECT t.id, t.import_id, t.source, t.op_date, t.value_date,
              t.concept, t.amount, t.currency, t.kind, t.category_id,
              c.name AS category_name, t.linked_asset_id, t.linked_liability_id,
              t.notes, t.recurring_rule_id,
              t.transfer_counterpart_id, t.transfer_reconciled_at, t.transfer_reconciled_source,
              tc.concept AS transfer_counterpart_concept,
              tc.op_date AS transfer_counterpart_op_date,
              t.created_at, t.updated_at
       FROM transactions t
       LEFT JOIN categories c ON c.id = t.category_id
       LEFT JOIN transactions tc ON tc.id = t.transfer_counterpart_id"#;

pub fn row_to_response(r: TxnRow) -> TransactionResponse {
    // Fuente de verdad de «conciliada» = counterpart presente. El ON DELETE SET NULL de la FK no
    // limpia reconciled_at/source, así que esos campos solo se serializan con contrapartida viva.
    let reconciled = r.transfer_counterpart_id.is_some();
    TransactionResponse {
        id: r.id,
        import_id: r.import_id,
        source: r.source,
        op_date: r.op_date.format("%Y-%m-%d").to_string(),
        value_date: r.value_date.map(|d| d.format("%Y-%m-%d").to_string()),
        concept: r.concept,
        amount: r.amount,
        currency: r.currency,
        kind: r.kind,
        category_id: r.category_id,
        category_name: r.category_name,
        linked_asset_id: r.linked_asset_id,
        linked_liability_id: r.linked_liability_id,
        notes: r.notes,
        recurring_rule_id: r.recurring_rule_id,
        transfer_counterpart_id: r.transfer_counterpart_id,
        transfer_reconciled_at: r.transfer_reconciled_at.filter(|_| reconciled),
        transfer_reconciled_source: r.transfer_reconciled_source.filter(|_| reconciled),
        transfer_counterpart_concept: r.transfer_counterpart_concept.filter(|_| reconciled),
        transfer_counterpart_op_date: r
            .transfer_counterpart_op_date
            .filter(|_| reconciled)
            .map(|d| d.format("%Y-%m-%d").to_string()),
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// ---------------------------------------------------------------------------
// Validadores que tocan la BD (compartidos por crud/import/rules)
// ---------------------------------------------------------------------------

/// Valida el par `(kind, category_id)` **sin resolver nada**:
/// - `savings` → `category_id` debe ser `None` (`savings_no_category`).
/// - `expense`/`income` con categoría → la categoría debe existir en la instalación y su scope
///   debe coincidir con el kind (`category_scope_mismatch`); **sin categoría → OK**.
///
/// Ese último «OK» ya NO es un estado persistible desde 4.15.0 (el CHECK
/// `transactions_category_required_check` lo prohíbe): este validador sobrevive para los caminos
/// que solo miran una intención — el preview del import, las reglas efímeras del wizard y la
/// validación de una regla de categorización, que puede no asignar categoría. **Toda escritura
/// de movimientos pasa por [`resolve_category_for_kind`]**, que valida esto y además rellena la
/// categoría por defecto cuando falta.
pub async fn assert_transaction_category(
    pool: &sqlx::PgPool,
    iid: Uuid,
    kind: &str,
    category_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if kind == "savings" {
        if category_id.is_some() {
            return Err(ApiError::BadRequest(
                "savings_no_category: savings transactions must have no category".into(),
            ));
        }
        return Ok(());
    }
    let Some(cid) = category_id else {
        return Ok(());
    };
    let scope: Option<String> = sqlx::query_scalar(
        r#"SELECT scope FROM categories WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(cid)
    .bind(iid)
    .fetch_optional(pool)
    .await?;
    let Some(scope) = scope else {
        return Err(ApiError::BadRequest(
            "category_not_in_installation: category_id must reference a category in this installation"
                .into(),
        ));
    };
    if scope != kind {
        return Err(ApiError::BadRequest(format!(
            "category_scope_mismatch: kind '{kind}' requires a category of scope '{kind}' (got '{scope}')"
        )));
    }
    Ok(())
}

/// Categoría DEFINITIVA de un movimiento a partir de su kind y de la categoría pedida (4.15.0).
///
/// Punto único de las escrituras: alta manual, lote, PATCH individual, PATCH en lote, aplicación
/// retroactiva de una regla y restore de backup. Devuelve:
/// - `savings` → `None` (y 400 `savings_no_category` si venía una categoría);
/// - `income`/`expense` con categoría → esa misma, ya validada por scope;
/// - `income`/`expense` **sin categoría** → la categoría POR DEFECTO del scope
///   (`categories.is_fallback`), o 400 `fallback_category_missing` si la instalación no tiene.
///
/// Es lo que convierte «sin categoría» de estado persistible a estado **irrepresentable**: la base
/// lo prohíbe con un CHECK y esta función es la que hace que ninguna escritura legítima choque
/// contra él. `kind` viene ya normalizado (`normalize_kind`).
pub(crate) async fn resolve_category_for_kind(
    pool: &sqlx::PgPool,
    iid: Uuid,
    kind: &str,
    category_id: Option<Uuid>,
) -> Result<Option<Uuid>, ApiError> {
    assert_transaction_category(pool, iid, kind, category_id).await?;
    if kind == "savings" {
        return Ok(None);
    }
    match category_id {
        Some(id) => Ok(Some(id)),
        None => Ok(Some(
            crate::handlers::categories::fallback_category_id(pool, iid, kind).await?,
        )),
    }
}

/// El asset (si se da) debe existir en la instalación.
pub async fn assert_asset_in_installation(
    pool: &sqlx::PgPool,
    iid: Uuid,
    asset_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if let Some(id) = asset_id {
        let ok: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM assets WHERE id = $1 AND installation_id = $2)"#,
        )
        .bind(id)
        .bind(iid)
        .fetch_one(pool)
        .await?;
        if !ok {
            return Err(ApiError::BadRequest(
                "linked_asset_not_found: asset must belong to this installation".into(),
            ));
        }
    }
    Ok(())
}

/// El liability (si se da) debe existir en la instalación.
pub async fn assert_liability_in_installation(
    pool: &sqlx::PgPool,
    iid: Uuid,
    liability_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if let Some(id) = liability_id {
        let ok: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM liabilities WHERE id = $1 AND installation_id = $2)"#,
        )
        .bind(id)
        .bind(iid)
        .fetch_one(pool)
        .await?;
        if !ok {
            return Err(ApiError::BadRequest(
                "linked_liability_not_found: liability must belong to this installation".into(),
            ));
        }
    }
    Ok(())
}

/// Siguiente ordinal libre para `(owner, fingerprint)`: `MAX(ordinal)+1` (0 si no hay ninguno).
/// Robusto frente a huecos por borrados (evita colisiones con la constraint UNIQUE).
pub async fn next_fingerprint_ordinal(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    fingerprint: &str,
) -> Result<i32, ApiError> {
    let next: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE(MAX(fingerprint_ordinal), -1) + 1
           FROM transactions
           WHERE installation_id = $1 AND owner_user_id = $2 AND fingerprint = $3"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(fingerprint)
    .fetch_one(&mut *conn)
    .await?;
    Ok(next)
}

/// Invalida la cache de proyección tras una mutación de transacciones **solo si** la instalación está
/// en un modo que usa transacciones (`transactions_avg` o `budget_income_real_expense` →
/// `SavingsSource::uses_transactions`); entonces las transacciones son inputs del engine. En modo
/// `budget` no hace nada (contrato histórico: las transacciones no tocan la proyección). Llamar tras
/// commitear la mutación con éxito. La invalidación real corre en background (`tokio::spawn`).
///
/// **Best-effort**: se ejecuta después de `tx.commit()`, con la escritura ya persistida. Un error del
/// SELECT de `savings_source` NO debe convertir una mutación exitosa en un 5xx: el cliente reintentaría
/// y duplicaría la transacción (el `fingerprint_ordinal` no lo impide). Por eso nunca propaga: un fallo
/// solo se registra y la cache caliente queda como estaba (la sirve el TTL / el próximo warm-up).
pub(crate) async fn invalidate_projection_if_savings_uses_transactions(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
) {
    match projection_savings_source(&state.pool, iid).await {
        Ok(s) if s.uses_transactions() => {
            refresh_projection_after_mutation(state, iid, user_id).await
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = ?e, "post-commit projection invalidation skipped")
        }
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn transactions_router() -> Router {
    let import_limit = crate::routes::BACKUP_IMPORT_BODY_LIMIT_BYTES;
    Router::new()
        .route("/", get(list_transactions).post(create_transaction))
        .route("/batch", post(create_batch).patch(patch_batch))
        .route("/months", get(list_months))
        .route("/summary", get(get_transactions_summary))
        .route("/aggregate", get(aggregate_transactions))
        .route("/duplicates", get(list_duplicates))
        .route("/category-series", get(get_category_series))
        .route(
            "/import/preview",
            post(import_preview).layer(DefaultBodyLimit::max(import_limit)),
        )
        .route(
            "/import/confirm",
            post(import_confirm).layer(DefaultBodyLimit::max(import_limit)),
        )
        .route("/imports", get(list_imports))
        .route("/imports/{id}", delete(delete_import))
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/{id}", patch(patch_rule).delete(delete_rule))
        .route("/rules/{id}/apply", post(apply_rule))
        // Reglas recurrentes: los segmentos estáticos deben resolverse antes que `/{id}`.
        .route("/recurring", get(list_recurring_rules))
        .route("/recurring/materialize", post(materialize_recurring))
        .route("/recurring/{id}", delete(delete_recurring_rule))
        // Conciliación de transferencias: los segmentos estáticos antes que `/{id}`.
        .route("/reconcile", post(reconcile_now))
        // Sugerencias (GET, lectura pura) y confirmación de una de ellas por su `match_id`. El
        // GET va aparte del POST del pase a propósito: un `?dry_run` sobre el POST sería un GET
        // que muta, que es la clase de decisión que ya costó cara en este repositorio.
        .route("/transfer-matches", get(suggest_transfer_matches))
        .route("/transfer-matches/{match_id}", post(confirm_transfer_match))
        .route(
            "/{id}/reconcile",
            post(reconcile_pair).delete(unreconcile_transaction),
        )
        .route("/{id}", patch(patch_transaction).delete(delete_transaction))
}
