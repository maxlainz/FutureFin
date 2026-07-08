//! Histórico de gasto mensual: transacciones datadas (`/v1/transactions`).
//!
//! Importa CSV bancarios (MyInvestor/N26) o efectivo a mano, los categoriza con las categorías
//! de Ingreso/Gasto, y sirve la comparativa mes real vs presupuesto vs promedio histórico.
//! CRUD per-user (scoping con `LedgerView` en las lecturas; `owner_user_id` en las escrituras).
//!
//! ## Contrato no-cache (igual que history.rs)
//! Las transacciones **no son inputs del motor de proyección** (que arranca en el mes 0 con el
//! ledger vivo, no con el histórico de gasto). Por eso **ningún handler de este módulo llama a
//! `refresh_projection_after_mutation`**: invalidar la cache aquí solo tiraría una entrada
//! caliente sin cambiar ni un número de la proyección. Un test de regresión
//! (`transactions_projection_cache.rs`) fija esta ausencia.

pub mod crud;
pub mod csv_presets;
pub mod import;
pub mod recurring;
pub mod rules;
pub mod schema;
pub mod summary;

use crate::error::ApiError;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use chrono::NaiveDate;
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

// Re-exports para el wiring de openapi.rs.
pub use crud::{
    create_batch, create_transaction, delete_import, delete_transaction, list_imports,
    list_months, list_transactions, patch_transaction,
};
pub use import::{import_confirm, import_preview};
pub use recurring::{delete_recurring_rule, list_recurring_rules, materialize_recurring};
pub use rules::{create_rule, delete_rule, list_rules, patch_rule};
pub use summary::get_transactions_summary;

pub use schema::{
    BatchCreateBody, CategoryComparisonLine, CreateRuleBody, CreateTransactionBody,
    ImportBatchResponse, ImportConfirmBody, ImportConfirmResponse, ImportDecision, ImportPreviewBody,
    ImportPreviewResponse, MaterializeResponse, MonthEntry, PatchRuleBody, PatchTransactionBody,
    PreviewRow, RecurrenceSpec, RecurringRuleResponse, RuleResponse, TransactionResponse,
    TransactionsSummaryResponse,
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
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub const TXN_SELECT: &str = r#"SELECT t.id, t.import_id, t.source, t.op_date, t.value_date,
              t.concept, t.amount, t.currency, t.kind, t.category_id,
              c.name AS category_name, t.linked_asset_id, t.linked_liability_id,
              t.notes, t.recurring_rule_id, t.created_at, t.updated_at
       FROM transactions t
       LEFT JOIN categories c ON c.id = t.category_id"#;

pub fn row_to_response(r: TxnRow) -> TransactionResponse {
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
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// ---------------------------------------------------------------------------
// Validadores que tocan la BD (compartidos por crud/import/rules)
// ---------------------------------------------------------------------------

/// Valida el par `(kind, category_id)`:
/// - `savings` → `category_id` debe ser `None` (`savings_no_category`).
/// - `expense`/`income` con categoría → la categoría debe existir en la instalación y su scope
///   debe coincidir con el kind (`category_scope_mismatch`); sin categoría → OK ("Sin categoría").
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
            "category_id must reference a category in this installation".into(),
        ));
    };
    if scope != kind {
        return Err(ApiError::BadRequest(format!(
            "category_scope_mismatch: kind '{kind}' requires a category of scope '{kind}' (got '{scope}')"
        )));
    }
    Ok(())
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

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn transactions_router() -> Router {
    let import_limit = crate::routes::BACKUP_IMPORT_BODY_LIMIT_BYTES;
    Router::new()
        .route("/", get(list_transactions).post(create_transaction))
        .route("/batch", post(create_batch))
        .route("/months", get(list_months))
        .route("/summary", get(get_transactions_summary))
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
        // Reglas recurrentes: los segmentos estáticos deben resolverse antes que `/{id}`.
        .route("/recurring", get(list_recurring_rules))
        .route("/recurring/materialize", post(materialize_recurring))
        .route("/recurring/{id}", delete(delete_recurring_rule))
        .route("/{id}", patch(patch_transaction).delete(delete_transaction))
}
