//! Servidor MCP de FutureFin: tools de lectura Y escritura sobre las mismas core fns que
//! sirven los handlers HTTP (`*_core` / `projection_series_cached`). Cero SQL propio y
//! cero validación paralela: cada tool de lectura serializa el MISMO struct serde que el
//! endpoint (Decimal-as-string intacto) y cada tool de escritura llama a la misma core de
//! mutación (que lleva DENTRO la invalidación de cache), así handler y tool no pueden
//! divergir. Las escrituras devuelven respuestas compactas `{id, resumen}`, no el response
//! HTTP entero.
//!
//! Gate de escritura: TODA tool de escritura pasa primero por `require_mcp_write` (rol
//! vivo con `role_can_write` + kill-switch `installation.mcp_write_enabled` leído por
//! request). Las lecturas no lo consultan.
//!
//! Errores: los de dominio/validación devuelven `CallToolResult{is_error:true}` con el
//! mismo JSON `{error, message}` del wire HTTP (el LLM puede leerlo y corregir el input);
//! `Db`/`Unavailable` se sanitizan a `ErrorData` interno (detalle solo a tracing), espejo
//! del contrato de `error.rs`.

use crate::error::{ApiError, ErrorBody};
use crate::handlers::allocation_rules::{list_allocation_rules_core, patch_allocation_rule_core};
use crate::handlers::assets::{asset_delete_effects, delete_asset_core};
use crate::handlers::liabilities::{delete_liability_core, liability_delete_effects};
use crate::handlers::assets::{create_asset_core, list_assets_core, patch_asset_core};
use crate::handlers::budget::{
    budget_snapshot_core, create_budget_entry_core, delete_budget_entry_core,
    patch_budget_entry_core,
};
use crate::handlers::categories::{create_category_core, list_categories_core};
use crate::handlers::history::{
    capture_snapshots_core, delete_snapshot_core, history_cashflow_core, history_series_core,
    list_snapshots_core,
};
use crate::handlers::installation::{installation_access_core, settings_user_core};
use crate::handlers::liabilities::{create_liability_core, list_liabilities_core};
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::planning::{
    create_planning_flow_core, delete_planning_flow_core, list_planning_flows_core,
    patch_planning_flow_core,
};
use crate::handlers::projection::{
    projection_series_cached, simulate_projection_core, SimulationSpec,
};
use crate::handlers::summary::summary_core;
use crate::handlers::transactions::crud::{
    create_transaction_core, delete_import_core, delete_transaction_core, get_transaction_core,
    list_imports_core, list_months_core, list_transactions_core, patch_transaction_core,
};
use crate::handlers::transactions::recurring::{
    delete_recurring_rule_core, list_recurring_rules_core, materialize_recurring_core,
};
use crate::handlers::transactions::rules::{
    create_categorization_rule_core, list_categorization_rules_core,
};
use crate::handlers::transactions::summary::{
    category_monthly_series_core, transactions_summary_core,
};
use crate::mcp::auth::{require_mcp_write, McpIdentity};
use crate::state::{AppState, Density};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub struct FutureFinMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

impl FutureFinMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

/// Extrae la identidad que dejó el middleware Bearer en las extensions del request HTTP
/// (rmcp propaga las `http::request::Parts` hasta el contexto de la tool).
fn identity(ctx: &RequestContext<RoleServer>) -> Result<McpIdentity, ErrorData> {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<McpIdentity>())
        .cloned()
        .ok_or_else(|| ErrorData::internal_error("missing request identity", None))
}

/// Misma semántica que `?view=` en HTTP: `"mine"` → Mine, cualquier otra cosa → Household.
fn resolve_view(view: &Option<String>) -> LedgerView {
    LedgerViewQuery { view: view.clone() }.resolve()
}

/// `Ok(T)` → JSON del struct del endpoint tal cual. `Err` → ver [`to_tool_outcome`].
fn to_tool_result<T: serde::Serialize>(
    res: Result<T, ApiError>,
) -> Result<CallToolResult, ErrorData> {
    match res {
        Ok(v) => {
            let json = serde_json::to_string(&v)
                .map_err(|e| ErrorData::internal_error(format!("serialization: {e}"), None))?;
            Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
        }
        Err(e) => to_tool_outcome(e),
    }
}

/// Mapea `ApiError` al contrato MCP. Dominio/validación → tool error legible (mismo JSON
/// `{error, message}` que HTTP). Infraestructura → error de protocolo sanitizado.
fn to_tool_outcome(e: ApiError) -> Result<CallToolResult, ErrorData> {
    match &e {
        ApiError::Db(err) => {
            tracing::error!(?err, "mcp tool database error");
            Err(ErrorData::internal_error("internal error", None))
        }
        ApiError::Unavailable => Err(ErrorData::internal_error("dependency unavailable", None)),
        _ => {
            let body = ErrorBody {
                error: e.code(),
                message: e.sanitised_message(),
            };
            let json = serde_json::to_string(&body)
                .unwrap_or_else(|_| r#"{"error":"internal","message":"internal error"}"#.into());
            Ok(CallToolResult::error(vec![ContentBlock::text(json)]))
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ViewParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo ("household").
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProjectionParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Horizonte en meses (12–840; fuera de rango se clampa). Omitido = horizonte derivado de
    /// la instalación — y única variante servida desde cache: un `months` explícito recomputa
    /// la proyección entera (~centenares de ms), así que pásalo solo si de verdad necesitas
    /// otro horizonte.
    #[serde(default)]
    #[schemars(range(min = 12, max = 840))]
    pub months: Option<u32>,
    /// Incluir la serie por activo (una serie de valores por cada activo). Default false
    /// para mantener la respuesta compacta.
    #[serde(default)]
    pub include_asset_series: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HistoryParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Limita la serie a los últimos N meses (1–1200). Omitido = desde el snapshot más
    /// antiguo (un backfill de años puede ser mucho payload: acota si solo necesitas lo
    /// reciente).
    #[serde(default)]
    #[schemars(range(min = 1, max = 1200))]
    pub window_months: Option<i64>,
    /// Incluir la serie por activo (un array por activo × puntos). Default false.
    #[serde(default)]
    pub include_asset_series: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransactionsSummaryParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Año del mes seleccionado. Se pasa junto con `month`; omitidos = último mes completo.
    #[serde(default)]
    pub year: Option<i32>,
    /// Mes 1..12 del mes seleccionado.
    #[serde(default)]
    pub month: Option<u32>,
    /// Ventana del promedio ponderado: "3" | "6" | "12" | "ytd" | "all". Default "6".
    #[serde(default)]
    pub avg_window: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTransactionsParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Filtra por mes "YYYY-MM" (op_date dentro del mes).
    #[serde(default)]
    pub month: Option<String>,
    /// Filtra por tipo: "expense" | "income" | "savings".
    #[serde(default)]
    pub kind: Option<String>,
    /// Filtra por id de categoría (UUID).
    #[serde(default)]
    pub category_id: Option<String>,
    /// Filtra por lote de import (UUID de `list_transaction_imports`).
    #[serde(default)]
    pub import_id: Option<String>,
    /// Máximo de movimientos devueltos (1–500). Default 100. La respuesta indica
    /// `total_count` y `truncated`.
    #[serde(default)]
    #[schemars(range(min = 1, max = 500))]
    pub limit: Option<u32>,
    /// Desplazamiento de paginación (movimientos a saltar, orden fecha DESC). Default 0.
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CategoriesParams {
    /// Filtra por scope: "asset" | "liability" | "income" | "expense". Omitido = todas.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CategorySeriesParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// "expense" | "income".
    pub kind: String,
    /// Limita la serie a una categoría (UUID de list_categories). Omitido = todas las del
    /// kind con datos.
    #[serde(default)]
    pub category_id: Option<String>,
    /// Ventana en meses (1–60, default 12). El último mes es el actual (parcial).
    #[serde(default)]
    #[schemars(range(min = 1, max = 60))]
    pub window_months: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CashflowParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Meses de ventana (1–120, default 24).
    #[serde(default)]
    #[schemars(range(min = 1, max = 120))]
    pub window_months: Option<i64>,
    /// Incluir la curva fina por activo (payload de chart). Default false: el agregado
    /// mensual es lo útil para analizar.
    #[serde(default)]
    pub include_curve: Option<bool>,
    /// "weekly" (default) | "daily". "daily" exige window_months <= 6. Solo aplica con
    /// include_curve.
    #[serde(default)]
    pub resolution: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SnapshotsParams {
    /// Filtra por año del snapshot (1900–3000). Omitido = todos.
    #[serde(default)]
    #[schemars(range(min = 1900, max = 3000))]
    pub year: Option<i32>,
    /// Filtra por tipo: "asset" | "liability". Omitido = ambos.
    #[serde(default)]
    pub kind: Option<String>,
    /// Incluir el detalle por ítem de cada snapshot. Default false (solo cabecera y total).
    #[serde(default)]
    pub include_items: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OneOffExpenseParam {
    /// Importe del gasto puntual (> 0), string decimal.
    pub amount: String,
    /// Mes de la proyección (1..=horizonte). Exactamente uno de month_index o date.
    #[serde(default)]
    pub month_index: Option<u32>,
    /// Fecha "YYYY-MM-DD" (se mapea al mes como un planning flow real).
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssetReturnOverrideParam {
    /// UUID del activo (de list_assets).
    pub asset_id: String,
    /// Rentabilidad anual esperada en % (> -100; los negativos componen pérdidas), string decimal.
    pub expected_annual_return_percent: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimulateParams {
    /// "mine" = solo los datos del usuario del token; omitido = hogar completo.
    #[serde(default)]
    pub view: Option<String>,
    /// Horizonte en meses (12–840). Omitido = horizonte de la instalación.
    #[serde(default)]
    #[schemars(range(min = 12, max = 840))]
    pub months: Option<u32>,
    /// Incluir las series decimadas baseline/escenario. Default false (solo KPIs y deltas).
    #[serde(default)]
    pub include_series: Option<bool>,
    /// Gasto puntual («¿y si me compro X?»): drena caja el mes indicado con la cascada real.
    #[serde(default)]
    pub one_off_expense: Option<OneOffExpenseParam>,
    /// Gasto mensual extra REAL (>= 0, string decimal): mueve también el target FIRE y las
    /// bases de caps — «vivir gastando más».
    #[serde(default)]
    pub extra_monthly_expense: Option<String>,
    /// Ajuste de caja mensual NEUTRO (>= 0, se resta): menos ahorro sin mover el target FIRE
    /// ni los caps. Usa este para «ahorro X € menos al mes sin cambiar mi objetivo».
    #[serde(default)]
    pub extra_monthly_cash_adjustment: Option<String>,
    /// Ahorro mensual extra (>= 0): más caja asignable vía la cascada, sin mover el target.
    #[serde(default)]
    pub extra_monthly_savings: Option<String>,
    /// SWR en % (0–4, string decimal): «¿y si el SWR fuera 3?».
    #[serde(default)]
    pub swr_pct: Option<String>,
    /// Inflación anual asumida en % (0–50, string decimal).
    #[serde(default)]
    pub annual_inflation_percent: Option<String>,
    /// Gasto ANUAL de jubilación (> 0, string decimal): sustituye la base del target FIRE y el
    /// gasto post-jubilación.
    #[serde(default)]
    pub retirement_annual_expense: Option<String>,
    /// Overrides de rentabilidad por activo.
    #[serde(default)]
    pub asset_return_overrides: Option<Vec<AssetReturnOverrideParam>>,
}

/// Parsea un string decimal de un parámetro de tool con error tipado.
fn parse_decimal_param(name: &str, raw: &str) -> Result<rust_decimal::Decimal, ApiError> {
    raw.trim()
        .parse::<rust_decimal::Decimal>()
        .map_err(|_| ApiError::BadRequest(format!("{name} must be a decimal string")))
}

fn parse_uuid_param(name: &str, raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw.trim()).map_err(|_| ApiError::BadRequest(format!("{name} must be a UUID")))
}

fn parse_opt_uuid_param(name: &str, raw: &Option<String>) -> Result<Option<Uuid>, ApiError> {
    raw.as_ref().map(|r| parse_uuid_param(name, r)).transpose()
}

fn parse_date_param(name: &str, raw: &str) -> Result<chrono::NaiveDate, ApiError> {
    raw.trim()
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{name} must be YYYY-MM-DD")))
}

// ---------------------------------------------------------------------------
// Params de las tools de escritura (issue #3). Importes SIEMPRE strings decimales; UUIDs y
// fechas como strings (se parsean con error tipado). Las validaciones de dominio viven en las
// core fns compartidas con los handlers HTTP.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateTransactionParams {
    /// Fecha de la operación "YYYY-MM-DD".
    pub op_date: String,
    pub concept: String,
    /// Importe FIRMADO como string decimal: gasto negativo ("-23.50"), ingreso positivo,
    /// aportación de ahorro negativa. Nunca 0.
    pub amount: String,
    /// "expense" | "income" | "savings" (savings SIN categoría).
    pub kind: String,
    /// Categoría (UUID de list_categories; el scope debe casar con el kind).
    #[serde(default)]
    pub category_id: Option<String>,
    /// Activo vinculado (destino de una aportación savings).
    #[serde(default)]
    pub linked_asset_id: Option<String>,
    /// Pasivo vinculado (cuota de un préstamo).
    #[serde(default)]
    pub linked_liability_id: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// true = crea además la plantilla recurrente mensual (y backfillea los meses cerrados
    /// desde op_date). Fechas demasiado antiguas se rechazan.
    #[serde(default)]
    pub recurring: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateTransactionParams {
    /// UUID del movimiento (propio) a editar.
    pub id: String,
    #[serde(default)]
    pub op_date: Option<String>,
    #[serde(default)]
    pub concept: Option<String>,
    /// Importe firmado como string decimal (nunca 0).
    #[serde(default)]
    pub amount: Option<String>,
    /// "expense" | "income" | "savings".
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    /// true = quitar la categoría.
    #[serde(default)]
    pub clear_category: Option<bool>,
    #[serde(default)]
    pub linked_asset_id: Option<String>,
    #[serde(default)]
    pub clear_linked_asset: Option<bool>,
    #[serde(default)]
    pub linked_liability_id: Option<String>,
    #[serde(default)]
    pub clear_linked_liability: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub clear_notes: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CaptureSnapshotParams {
    /// Qué capturar: ["asset"], ["liability"] o ambos. Omitido = ambos.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreatePlanningFlowParams {
    pub title: String,
    /// Categoría income|expense (UUID de list_categories).
    pub category_id: String,
    /// Importe > 0 como string decimal (el signo lo da el scope de la categoría).
    pub expected_amount: String,
    /// "YYYY-MM-DD" opcional. Sin fecha, el flujo se reparte en los próximos 90 días.
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Mostrar como marcador en el chart (requiere due_date).
    #[serde(default)]
    pub show_in_chart: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdatePlanningFlowParams {
    /// UUID del flujo (de list_planning_flows).
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    /// Importe > 0 como string decimal.
    #[serde(default)]
    pub expected_amount: Option<String>,
    /// "YYYY-MM-DD". Incompatible con clear_due_date.
    #[serde(default)]
    pub due_date: Option<String>,
    /// true = borrar la fecha (el flujo pasa a repartirse en 90 días y sale del chart).
    #[serde(default)]
    pub clear_due_date: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub show_in_chart: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCategoryParams {
    /// "asset" | "liability" | "income" | "expense".
    pub scope: String,
    pub name: String,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCategorizationRuleParams {
    /// Patrón a buscar en el concepto normalizado (p.ej. "MERCADONA").
    pub pattern: String,
    /// "substring" (default) | "prefix" | "exact".
    #[serde(default)]
    pub match_kind: Option<String>,
    /// Banco de origen ("myinvestor" | "n26"…); omitido = agnóstica (cualquier banco).
    #[serde(default)]
    pub source: Option<String>,
    /// "expense" | "income" | "savings" (savings sin categoría).
    pub assign_kind: String,
    /// Categoría a asignar (UUID; scope acorde al assign_kind).
    #[serde(default)]
    pub assign_category_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateAssetValueParams {
    /// UUID del activo (de list_assets).
    pub asset_id: String,
    /// Valor actual >= 0 como string decimal («mi fondo vale ahora 52300»).
    #[serde(default)]
    pub current_value: Option<String>,
    /// Rentabilidad anual esperada en % (> -100; negativos componen pérdidas), string decimal.
    #[serde(default)]
    pub expected_annual_return_percent: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateAssetParams {
    pub name: String,
    /// Categoría con scope asset (UUID de list_categories).
    pub category_id: String,
    /// Valor actual >= 0, string decimal.
    pub current_value: String,
    /// Líquido = drenable para gastos (default true).
    #[serde(default)]
    pub is_liquid: Option<bool>,
    /// Rentabilidad anual esperada en % (> -100), string decimal.
    #[serde(default)]
    pub expected_annual_return_percent: Option<String>,
    /// Precio de compra >= 0 (base de coste), string decimal.
    #[serde(default)]
    pub purchase_price: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateLiabilityParams {
    pub label: String,
    /// Categoría con scope liability (UUID de list_categories).
    pub category_id: String,
    /// Principal >= 0 como string decimal. Obligatorio salvo derive_principal_from_plan.
    #[serde(default)]
    pub principal: Option<String>,
    /// true = derivar el principal del plan de pago (exige payment_amount + payment_frequency
    /// + payment_end_date; amortización francesa hacia atrás).
    #[serde(default)]
    pub derive_principal_from_plan: Option<bool>,
    /// TAE en % >= 0, string decimal.
    #[serde(default)]
    pub apr_percent: Option<String>,
    /// Cuota como string decimal (> 0 si se pasa).
    #[serde(default)]
    pub payment_amount: Option<String>,
    /// "monthly" | "weekly" (obligatoria si hay payment_amount).
    #[serde(default)]
    pub payment_frequency: Option<String>,
    /// "YYYY-MM-DD" — fin del plan de pago.
    #[serde(default)]
    pub payment_end_date: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateBudgetEntryParams {
    /// Categoría income|expense (UUID de list_categories).
    pub category_id: String,
    /// Importe mensual > 0 como string decimal.
    pub amount: String,
    /// El ingreso persiste tras la jubilación (pensión, alquileres…). Default false.
    #[serde(default)]
    pub persists_after_retirement: Option<bool>,
    /// El gasto termina al jubilarse. Incompatible con expense_end_date. Default false.
    #[serde(default)]
    pub ends_at_retirement: Option<bool>,
    /// "YYYY-MM-DD" — el gasto termina en esa fecha. Incompatible con ends_at_retirement.
    #[serde(default)]
    pub expense_end_date: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateBudgetEntryParams {
    /// UUID de la entrada (de get_budget).
    pub id: String,
    #[serde(default)]
    pub category_id: Option<String>,
    /// Importe mensual > 0 como string decimal.
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub persists_after_retirement: Option<bool>,
    #[serde(default)]
    pub ends_at_retirement: Option<bool>,
    /// "YYYY-MM-DD". Incompatible con clear_expense_end_date.
    #[serde(default)]
    pub expense_end_date: Option<String>,
    /// true = borrar la fecha fin del gasto.
    #[serde(default)]
    pub clear_expense_end_date: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateAllocationRuleParams {
    /// UUID de la regla (de list_allocation_rules).
    pub rule_id: String,
    /// Importe de la regla como string decimal: euros/mes para kind=fixed, % para
    /// kind=percent. (El kind y el orden no se editan desde chat.)
    #[serde(default)]
    pub amount: Option<String>,
    /// Cap: {"kind": "amount"|"months_expense"|"income_multiple", "value": "..."}. clear_cap
    /// lo elimina.
    #[serde(default)]
    pub cap_kind: Option<String>,
    #[serde(default)]
    pub cap_value: Option<String>,
    /// true = quitar el cap.
    #[serde(default)]
    pub clear_cap: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteRecurringRuleParams {
    /// UUID de la plantilla (de list_recurring_rules).
    pub id: String,
    /// Sin confirm=true la tool NO borra: devuelve un preview de la plantilla.
    #[serde(default)]
    pub confirm: Option<bool>,
}

/// Params comunes de los deletes con preview/confirm del tramo 3.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteByIdParams {
    /// UUID del recurso a borrar.
    pub id: String,
    /// Sin confirm=true la tool NO borra: devuelve un preview con los efectos.
    #[serde(default)]
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaxBracketParam {
    /// Umbral superior del tramo como string decimal; null/omitido SOLO en el último tramo.
    #[serde(default)]
    pub up_to: Option<String>,
    /// Porcentaje del tramo (0–99), string decimal.
    pub pct: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateFireSettingsParams {
    /// SWR en % (0–4), string decimal.
    #[serde(default)]
    pub swr_pct: Option<String>,
    /// Inflación anual asumida en % (0–50), string decimal.
    #[serde(default)]
    pub annual_inflation_assumption_percent: Option<String>,
    /// "budget" (A) | "transactions_avg" (B) | "budget_income_real_expense" (C).
    #[serde(default)]
    pub savings_source: Option<String>,
    /// "manual" | "annual_expense" | "current_income".
    #[serde(default)]
    pub fire_number_mode: Option<String>,
    /// Objetivo manual > 0, string decimal (requerido con mode=manual).
    #[serde(default)]
    pub fire_number_manual_amount: Option<String>,
    #[serde(default)]
    pub taxes_enabled: Option<bool>,
    /// Tramos fiscales COMPLETOS (sustituyen a los actuales; umbrales crecientes, solo el
    /// último sin up_to).
    #[serde(default)]
    pub tax_brackets: Option<Vec<TaxBracketParam>>,
    /// Sin confirm=true NO se persiste nada: devuelve el before/after validado (preview).
    #[serde(default)]
    pub confirm: Option<bool>,
}

const LIST_TRANSACTIONS_DEFAULT_LIMIT: usize = 100;
const LIST_TRANSACTIONS_MAX_LIMIT: usize = 500;

#[tool_router]
impl FutureFinMcp {
    #[tool(
        name = "get_summary",
        description = "Resumen financiero del hogar: patrimonio neto, totales de activos/pasivos, salud financiera (ingresos/gastos mensuales, tasa de ahorro, runway de líquidos) y desgloses por categoría. Importes como strings decimales.",
        annotations(title = "Resumen financiero", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_summary(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(summary_core(&self.state.pool, id.installation_id, id.user_id, view).await)
    }

    #[tool(
        name = "get_projection",
        description = "Proyección de patrimonio y jubilación (FIRE): serie futura de patrimonio neto (~82 puntos, mes 0-12 mensual y anual después), objetivo FIRE por mes, mes estimado de jubilación (jubilacion_month_index), hitos de patrimonio y supuestos usados. Los valores de las series son números en euros nominales. Omite `months` salvo necesidad: sin él la respuesta sale de cache; con él se recomputa entera.",
        annotations(title = "Proyección FIRE", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_projection(
        &self,
        Parameters(p): Parameters<ProjectionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        let res = projection_series_cached(
            &self.state,
            id.user_id,
            id.installation_id,
            view,
            p.months,
            // Densidad fija hybrid: ~82 puntos ≈5 KB. La serie mensual completa (~841
            // puntos) no aporta nada a un LLM y multiplica el contexto.
            Density::Hybrid,
        )
        .await
        .map(|mut r| {
            if !p.include_asset_series.unwrap_or(false) {
                r.asset_series = Vec::new();
            }
            r
        });
        to_tool_result(res)
    }

    #[tool(
        name = "get_budget",
        description = "Presupuesto mensual: entradas de ingreso/gasto persistidas, cuotas derivadas de los pasivos activos y totales normalizados a equivalente mensual.",
        annotations(title = "Presupuesto", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_budget(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            budget_snapshot_core(&self.state.pool, id.installation_id, id.user_id, view).await,
        )
    }

    #[tool(
        name = "get_transactions_summary",
        description = "Comparativa del mes: gasto/ingreso real por categoría vs presupuesto vs promedio ponderado de meses anteriores. Sin year/month usa el último mes completo.",
        annotations(title = "Comparativa mensual", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_transactions_summary(
        &self,
        Parameters(p): Parameters<TransactionsSummaryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            transactions_summary_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                view,
                p.year,
                p.month,
                p.avg_window,
                None,
            )
            .await,
        )
    }

    #[tool(
        name = "list_transactions",
        description = "Movimientos (gastos, ingresos, ahorro) con filtros por mes, tipo, categoría y lote de import, orden fecha descendente. Paginado en SQL: devuelve total_count y truncated; usa limit (max 500) y offset para pedir más páginas.",
        annotations(title = "Movimientos", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_transactions(
        &self,
        Parameters(p): Parameters<ListTransactionsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);

        let limit = p.limit.map(|l| l as usize).unwrap_or(LIST_TRANSACTIONS_DEFAULT_LIMIT);
        if limit == 0 || limit > LIST_TRANSACTIONS_MAX_LIMIT {
            return to_tool_outcome(ApiError::BadRequest(format!(
                "limit must be between 1 and {LIST_TRANSACTIONS_MAX_LIMIT}"
            )));
        }
        let offset = p.offset.unwrap_or(0) as i64;
        let parse_uuid = |raw: &Option<String>, field: &str| -> Result<Option<Uuid>, ApiError> {
            match raw {
                Some(raw) => Uuid::parse_str(raw.trim())
                    .map(Some)
                    .map_err(|_| ApiError::BadRequest(format!("{field} must be a UUID"))),
                None => Ok(None),
            }
        };
        let category_id = match parse_uuid(&p.category_id, "category_id") {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let import_id = match parse_uuid(&p.import_id, "import_id") {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };

        let res = list_transactions_core(
            &self.state.pool,
            id.installation_id,
            id.user_id,
            view,
            p.month.as_deref(),
            p.kind.as_deref(),
            category_id,
            import_id,
            Some(limit as i64),
            offset,
        )
        .await
        .map(|(page, total_count)| {
            let truncated = offset + (page.len() as i64) < total_count;
            serde_json::json!({
                "total_count": total_count,
                "offset": offset,
                "truncated": truncated,
                "transactions": page,
            })
        });
        to_tool_result(res)
    }

    #[tool(
        name = "get_history",
        description = "Serie histórica de patrimonio neto reconstruida desde los snapshots del usuario (interpolación servidor). month_index 0 = mes actual, negativos = pasado. Los valores de las series son números para el chart; los markers son los snapshots reales. Acota con window_months si solo necesitas lo reciente; asset_series es opt-in.",
        annotations(title = "Histórico de patrimonio", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_history(
        &self,
        Parameters(p): Parameters<HistoryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            history_series_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                view,
                p.window_months,
                p.include_asset_series.unwrap_or(false),
            )
            .await,
        )
    }

    #[tool(
        name = "list_assets",
        description = "Activos del hogar (o del usuario con view=mine): valor actual, liquidez, rentabilidad anual esperada y aportación mensual objetivo resuelta por las reglas de asignación.",
        annotations(title = "Activos", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_assets(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_assets_core(&self.state.pool, id.installation_id, id.user_id, view).await,
        )
    }

    #[tool(
        name = "list_liabilities",
        description = "Pasivos activos (deudas/préstamos): principal, TAE, cuota y frecuencia de pago, fecha fin del plan. Los pasivos con plan de pago ya vencido se filtran.",
        annotations(title = "Pasivos", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_liabilities(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_liabilities_core(&self.state.pool, id.installation_id, id.user_id, view).await,
        )
    }

    #[tool(
        name = "list_planning_flows",
        description = "Próximos: entradas y salidas puntuales previstas (con fecha opcional), p.ej. pagas extra, IRPF, un viaje. No son recurrentes ni parte del presupuesto mensual.",
        annotations(title = "Próximos", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_planning_flows(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_planning_flows_core(&self.state.pool, id.installation_id, id.user_id, view)
                .await,
        )
    }

    #[tool(
        name = "get_settings",
        description = "Ajustes de la instalación: divisa base, zona horaria, inflación anual asumida y configuración FIRE (modo del objetivo, SWR, tramos fiscales, fuente del ahorro), el rol del usuario del token y su identidad (user: id, username, birth_date — la DOB que fija el horizonte de proyección).",
        annotations(title = "Ajustes", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_settings(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let access = installation_access_core(&self.state.pool, id.user_id)
            .await
            .and_then(|opt| opt.ok_or(ApiError::Forbidden));
        let res = match access {
            Ok(access) => settings_user_core(&self.state.pool, id.user_id)
                .await
                .map(|user| {
                    serde_json::json!({
                        "installation": access.installation,
                        "role": access.role,
                        "user": user,
                    })
                }),
            Err(e) => Err(e),
        };
        to_tool_result(res)
    }

    #[tool(
        name = "simulate_projection",
        description = "What-if de proyección/FIRE sin persistir NADA: simula baseline y escenario con overrides (gasto puntual, gasto mensual extra real vs ajuste de caja neutro, ahorro extra, SWR, inflación, gasto anual de jubilación, rentabilidad por activo — negativa válida hasta -100 exclusivo) y devuelve KPIs + deltas (mes de jubilación, patrimonio final, target FIRE, runway). Importes como strings decimales. Series opt-in con include_series. Coste ~2 simulaciones (cientos de ms); no toca la cache.",
        annotations(title = "Simular escenario", read_only_hint = true, open_world_hint = false)
    )]
    async fn simulate_projection(
        &self,
        Parameters(p): Parameters<SimulateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);

        // Parseo de strings → tipos (las cotas de dominio se validan en el core).
        let build_spec = || -> Result<SimulationSpec, ApiError> {
            let mut spec = SimulationSpec {
                months: p.months,
                include_series: p.include_series.unwrap_or(false),
                ..Default::default()
            };
            if let Some(one_off) = &p.one_off_expense {
                spec.one_off_amount =
                    Some(parse_decimal_param("one_off_expense.amount", &one_off.amount)?);
                spec.one_off_month_index = one_off.month_index;
                spec.one_off_date = match &one_off.date {
                    Some(raw) => Some(raw.trim().parse().map_err(|_| {
                        ApiError::BadRequest(
                            "one_off_expense.date must be YYYY-MM-DD".into(),
                        )
                    })?),
                    None => None,
                };
            }
            let parse_opt = |name: &str, raw: &Option<String>| -> Result<_, ApiError> {
                raw.as_ref()
                    .map(|r| parse_decimal_param(name, r))
                    .transpose()
            };
            spec.extra_monthly_expense =
                parse_opt("extra_monthly_expense", &p.extra_monthly_expense)?;
            spec.extra_monthly_cash_adjustment = parse_opt(
                "extra_monthly_cash_adjustment",
                &p.extra_monthly_cash_adjustment,
            )?;
            spec.extra_monthly_savings =
                parse_opt("extra_monthly_savings", &p.extra_monthly_savings)?;
            spec.swr_pct = parse_opt("swr_pct", &p.swr_pct)?;
            spec.annual_inflation_percent =
                parse_opt("annual_inflation_percent", &p.annual_inflation_percent)?;
            spec.retirement_annual_expense =
                parse_opt("retirement_annual_expense", &p.retirement_annual_expense)?;
            if let Some(overrides) = &p.asset_return_overrides {
                for o in overrides {
                    let asset_id = Uuid::parse_str(o.asset_id.trim()).map_err(|_| {
                        ApiError::BadRequest("asset_return_overrides.asset_id must be a UUID".into())
                    })?;
                    let pct = parse_decimal_param(
                        "asset_return_overrides.expected_annual_return_percent",
                        &o.expected_annual_return_percent,
                    )?;
                    spec.asset_return_overrides.push((asset_id, pct));
                }
            }
            Ok(spec)
        };
        let spec = match build_spec() {
            Ok(s) => s,
            Err(e) => return to_tool_outcome(e),
        };

        to_tool_result(
            simulate_projection_core(&self.state.pool, id.installation_id, id.user_id, view, spec)
                .await,
        )
    }

    #[tool(
        name = "list_allocation_rules",
        description = "Cascada de asignación del ahorro mensual: reglas ordenadas por prioridad (kind fixed|percent|remainder, importe, cap opcional, enabled) y el activo destino de cada una. Explica por qué el ahorro va donde va; list_assets muestra el resultado resuelto por activo.",
        annotations(title = "Reglas de asignación", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_allocation_rules(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_allocation_rules_core(&self.state.pool, id.installation_id, id.user_id, view)
                .await,
        )
    }

    #[tool(
        name = "list_categories",
        description = "Catálogo de categorías de la instalación: id, scope (asset|liability|income|expense), nombre y orden. Úsalo para resolver nombre→id antes de filtrar o crear movimientos.",
        annotations(title = "Categorías", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_categories(
        &self,
        Parameters(p): Parameters<CategoriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        to_tool_result(
            list_categories_core(&self.state.pool, id.installation_id, p.scope.as_deref()).await,
        )
    }

    #[tool(
        name = "get_category_monthly_series",
        description = "Evolución mensual del gasto o ingreso por categoría: un punto por mes (cero-relleno, magnitudes >= 0 como strings decimales) para cada categoría con datos en la ventana. Responde «¿cómo evoluciona mi gasto en X?». El último mes es el actual (parcial).",
        annotations(title = "Serie mensual por categoría", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_category_monthly_series(
        &self,
        Parameters(p): Parameters<CategorySeriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        let category_id = match &p.category_id {
            Some(raw) => match Uuid::parse_str(raw.trim()) {
                Ok(u) => Some(u),
                Err(_) => {
                    return to_tool_outcome(ApiError::BadRequest(
                        "category_id must be a UUID".into(),
                    ))
                }
            },
            None => None,
        };
        to_tool_result(
            category_monthly_series_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                view,
                &p.kind,
                category_id,
                p.window_months,
            )
            .await,
        )
    }

    #[tool(
        name = "get_history_cashflow",
        description = "Flujo de caja mensual real por tipo (expense/income/savings + net, strings decimales, meses firmados hacia atrás desde el actual): la mejor fuente para «¿cuánto entra y sale de verdad cada mes?». La curva fina por activo es opt-in (include_curve).",
        annotations(title = "Cash-flow histórico", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_history_cashflow(
        &self,
        Parameters(p): Parameters<CashflowParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            history_cashflow_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                view,
                p.window_months,
                p.resolution.as_deref(),
                p.include_curve.unwrap_or(false),
            )
            .await,
        )
    }

    #[tool(
        name = "list_recurring_rules",
        description = "Plantillas de movimientos recurrentes del usuario del token (nómina, gimnasio…): concepto, importe, kind, categoría y el cursor last_materialized_month (si es anterior al último mes cerrado hay materialización pendiente). Siempre own-user, sin view.",
        annotations(title = "Recurrentes", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recurring_rules(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        to_tool_result(
            list_recurring_rules_core(&self.state.pool, id.installation_id, id.user_id).await,
        )
    }

    #[tool(
        name = "list_categorization_rules",
        description = "Reglas de categorización aprendidas del usuario del token: patrón (substring|prefix|exact), banco de origen opcional y asignación (kind + categoría). Explican cómo se categorizó un concepto y evitan crear duplicados. Solo afectan a imports futuros. Siempre own-user, sin view.",
        annotations(title = "Reglas de categorización", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_categorization_rules(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        to_tool_result(
            list_categorization_rules_core(&self.state.pool, id.installation_id, id.user_id)
                .await,
        )
    }

    #[tool(
        name = "list_transaction_months",
        description = "Meses con movimientos (YYYY-MM, orden DESC) con su nº de transacciones e is_complete (false solo para el mes civil en curso). Orienta las consultas: evita pedir mes a mes a ciegas.",
        annotations(title = "Meses con datos", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_transaction_months(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_months_core(&self.state.pool, id.installation_id, id.user_id, view).await,
        )
    }

    #[tool(
        name = "list_snapshots",
        description = "Snapshots del histórico de patrimonio del usuario del token (cabecera: fecha, kind asset|liability, source capture|backfill, total). El detalle por ítem es opt-in con include_items. Siempre own-user, sin view.",
        annotations(title = "Snapshots", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_snapshots(
        &self,
        Parameters(p): Parameters<SnapshotsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let res = list_snapshots_core(
            &self.state.pool,
            id.installation_id,
            id.user_id,
            p.year,
            p.kind.as_deref(),
        )
        .await
        .map(|mut snaps| {
            if !p.include_items.unwrap_or(false) {
                for s in &mut snaps {
                    s.items.clear();
                }
            }
            snaps
        });
        to_tool_result(res)
    }

    // -----------------------------------------------------------------------
    // Tools de ESCRITURA (issue #3). Toda tool de escritura pasa primero por
    // `require_mcp_write` (rol vivo + toggle `mcp_write_enabled`), llama a la MISMA core fn
    // que su handler HTTP (la invalidación de cache vive dentro de la core) y devuelve una
    // respuesta compacta, no el response HTTP entero.
    // -----------------------------------------------------------------------

    #[tool(
        name = "create_transaction",
        description = "Registra un movimiento manual («apunta 23,50 € de cena de ayer»): fecha, concepto, importe FIRMADO como string decimal (gasto negativo, ingreso positivo, aportación de ahorro negativa), kind (expense|income|savings; savings SIN categoría), categoría opcional (el scope debe casar con el kind) y links opcionales a activo/pasivo. Con recurring=true crea además la plantilla recurrente mensual y rellena los meses cerrados intermedios. OJO: reenviar el mismo movimiento crea OTRO movimiento (los duplicados manuales son legítimos) — no repitas la llamada si ya respondió ok.",
        annotations(title = "Crear movimiento", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_transaction(
        &self,
        Parameters(p): Parameters<CreateTransactionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::transactions::schema::CreateTransactionBody, ApiError> {
            Ok(crate::handlers::transactions::schema::CreateTransactionBody {
                op_date: parse_date_param("op_date", &p.op_date)?,
                value_date: None,
                concept: p.concept.clone(),
                amount: parse_decimal_param("amount", &p.amount)?,
                kind: p.kind.clone(),
                category_id: parse_opt_uuid_param("category_id", &p.category_id)?,
                linked_asset_id: parse_opt_uuid_param("linked_asset_id", &p.linked_asset_id)?,
                linked_liability_id: parse_opt_uuid_param(
                    "linked_liability_id",
                    &p.linked_liability_id,
                )?,
                notes: p.notes.clone(),
                recurrence: if p.recurring.unwrap_or(false) {
                    Some(crate::handlers::transactions::schema::RecurrenceSpec {})
                } else {
                    None
                },
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let t = create_transaction_core(&self.state, id.installation_id, id.user_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": t.id,
                "resumen": format!("{} · {} · {} ({})", t.op_date, t.concept, t.amount, t.kind.as_deref().unwrap_or("-")),
                "category_name": t.category_name,
                "recurring_rule_id": t.recurring_rule_id,
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_transaction",
        description = "Corrige o recategoriza un movimiento PROPIO («eso era comida, no ocio»): cualquier campo es opcional; los flags clear_* ponen a null. Movimientos de otro usuario → not_found. En importadas la huella de dedup queda anclada al CSV original.",
        annotations(title = "Editar movimiento", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_transaction(
        &self,
        Parameters(p): Parameters<UpdateTransactionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<(Uuid, crate::handlers::transactions::schema::PatchTransactionBody), ApiError> {
            let txn_id = parse_uuid_param("id", &p.id)?;
            let body = crate::handlers::transactions::schema::PatchTransactionBody {
                op_date: p.op_date.as_deref().map(|d| parse_date_param("op_date", d)).transpose()?,
                value_date: None,
                clear_value_date: None,
                concept: p.concept.clone(),
                amount: p
                    .amount
                    .as_deref()
                    .map(|a| parse_decimal_param("amount", a))
                    .transpose()?,
                kind: p.kind.clone(),
                category_id: parse_opt_uuid_param("category_id", &p.category_id)?,
                clear_category: p.clear_category,
                linked_asset_id: parse_opt_uuid_param("linked_asset_id", &p.linked_asset_id)?,
                clear_linked_asset: p.clear_linked_asset,
                linked_liability_id: parse_opt_uuid_param(
                    "linked_liability_id",
                    &p.linked_liability_id,
                )?,
                clear_linked_liability: p.clear_linked_liability,
                notes: p.notes.clone(),
                clear_notes: p.clear_notes,
            };
            Ok((txn_id, body))
        };
        let (txn_id, body) = match run() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let t = patch_transaction_core(&self.state, id.installation_id, id.user_id, txn_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": t.id,
                "resumen": format!("{} · {} · {} ({})", t.op_date, t.concept, t.amount, t.kind.as_deref().unwrap_or("-")),
                "category_name": t.category_name,
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "capture_snapshot",
        description = "«Guarda una foto de mi patrimonio hoy»: captura un snapshot del histórico con los activos y/o pasivos VIVOS del usuario del token. Upsert por día civil — recapturar el mismo día SOBRESCRIBE la foto de ese día con el ledger actual. No afecta a la proyección (los snapshots no son inputs del engine).",
        annotations(title = "Capturar snapshot", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn capture_snapshot(
        &self,
        Parameters(p): Parameters<CaptureSnapshotParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let out = capture_snapshots_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                crate::handlers::history::CaptureBody { kinds: p.kinds.clone() },
            )
            .await?;
            Ok(serde_json::json!({
                "snapshot_date": out.snapshots.first().map(|s| s.snapshot_date_ymd.clone()),
                "snapshots": out
                    .snapshots
                    .iter()
                    .map(|s| serde_json::json!({"id": s.id, "kind": s.kind, "total": s.total.to_string(), "items": s.items.len()}))
                    .collect::<Vec<_>>(),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "materialize_recurring",
        description = "«Ponme al día los recurrentes»: materializa las instancias pendientes de las plantillas recurrentes del usuario del token hasta el último mes cerrado. Idempotente por cursor (repetirla no duplica) y nunca crea fechas futuras.",
        annotations(title = "Materializar recurrentes", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn materialize_recurring(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            materialize_recurring_core(&self.state, id.installation_id, id.user_id).await
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_planning_flow",
        description = "Añade una entrada a «Próximos» («apunta que en octubre pago 800 € de IRPF»): título, categoría income/expense, importe > 0 (string decimal) y fecha opcional. Alimenta directamente la proyección — usa simulate_projection si quieres enseñar el impacto antes.",
        annotations(title = "Crear próximo", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_planning_flow(
        &self,
        Parameters(p): Parameters<CreatePlanningFlowParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::planning::CreatePlanningFlowBody, ApiError> {
            Ok(crate::handlers::planning::CreatePlanningFlowBody {
                category_id: parse_uuid_param("category_id", &p.category_id)?,
                title: p.title.clone(),
                expected_amount: parse_decimal_param("expected_amount", &p.expected_amount)?,
                due_date: p
                    .due_date
                    .as_deref()
                    .map(|d| parse_date_param("due_date", d))
                    .transpose()?,
                notes: p.notes.clone(),
                sort_index: None,
                show_in_chart: p.show_in_chart,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let f = create_planning_flow_core(&self.state, id.installation_id, id.user_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": f.id,
                "resumen": format!("{} · {} ({:?}){}", f.title, f.expected_amount, f.direction,
                    f.due_date.map(|d| format!(" · {d}")).unwrap_or_default()),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_planning_flow",
        description = "Edita una entrada de «Próximos»: cualquier campo es opcional; clear_due_date=true borra la fecha (y desmarca show_in_chart). Alimenta la proyección.",
        annotations(title = "Editar próximo", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_planning_flow(
        &self,
        Parameters(p): Parameters<UpdatePlanningFlowParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<(Uuid, crate::handlers::planning::PatchPlanningFlowBody), ApiError> {
            let flow_id = parse_uuid_param("id", &p.id)?;
            if p.due_date.is_some() && p.clear_due_date == Some(true) {
                return Err(ApiError::BadRequest(
                    "provide either due_date or clear_due_date, not both".into(),
                ));
            }
            // Tri-state del PATCH HTTP: omitido = sin cambio; null = borrar; string = fijar.
            let due_date = if p.clear_due_date == Some(true) {
                Some(serde_json::Value::Null)
            } else if let Some(d) = &p.due_date {
                parse_date_param("due_date", d)?;
                Some(serde_json::Value::String(d.trim().to_string()))
            } else {
                None
            };
            Ok((
                flow_id,
                crate::handlers::planning::PatchPlanningFlowBody {
                    category_id: parse_opt_uuid_param("category_id", &p.category_id)?,
                    title: p.title.clone(),
                    expected_amount: p
                        .expected_amount
                        .as_deref()
                        .map(|a| parse_decimal_param("expected_amount", a))
                        .transpose()?,
                    due_date,
                    notes: p.notes.clone(),
                    sort_index: None,
                    show_in_chart: p.show_in_chart,
                },
            ))
        };
        let (flow_id, body) = match run() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let f = patch_planning_flow_core(&self.state, id.installation_id, id.user_id, flow_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": f.id,
                "resumen": format!("{} · {} ({:?}){}", f.title, f.expected_amount, f.direction,
                    f.due_date.map(|d| format!(" · {d}")).unwrap_or_default()),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_category",
        description = "Crea una categoría («crea la categoría Mascotas»): scope (asset|liability|income|expense) + nombre. Duplicado en el mismo scope → resource conflict. Desbloquea la categorización cuando falta la categoría.",
        annotations(title = "Crear categoría", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_category(
        &self,
        Parameters(p): Parameters<CreateCategoryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::categories::CreateCategoryBody, ApiError> {
            Ok(crate::handlers::categories::CreateCategoryBody {
                scope: crate::handlers::categories::CategoryScope::parse(&p.scope)?,
                name: p.name.clone(),
                sort_index: p.sort_index,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let c = create_category_core(&self.state.pool, id.installation_id, body).await?;
            Ok(serde_json::json!({"id": c.id, "scope": c.scope, "name": c.name}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_categorization_rule",
        description = "Crea una regla de categorización («a partir de ahora, todo lo de MERCADONA es supermercado»): pattern + match_kind (substring default | prefix | exact), source opcional (null = cualquier banco), assign_kind y categoría opcional (savings sin categoría). Solo afecta a imports FUTUROS — nunca recategoriza movimientos existentes. Duplicado (source, pattern) → resource conflict.",
        annotations(title = "Crear regla de categorización", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_categorization_rule(
        &self,
        Parameters(p): Parameters<CreateCategorizationRuleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::transactions::schema::CreateRuleBody, ApiError> {
            Ok(crate::handlers::transactions::schema::CreateRuleBody {
                match_kind: p.match_kind.clone(),
                pattern: p.pattern.clone(),
                source: p.source.clone(),
                assign_kind: Some(p.assign_kind.clone()),
                assign_category_id: parse_opt_uuid_param(
                    "assign_category_id",
                    &p.assign_category_id,
                )?,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let r = create_categorization_rule_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                body,
            )
            .await?;
            Ok(serde_json::json!({
                "id": r.id,
                "resumen": format!("{} «{}» → {} {}", r.match_kind, r.pattern,
                    r.assign_kind.as_deref().unwrap_or("-"),
                    r.assign_category_name.as_deref().unwrap_or("(sin categoría)")),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_asset_value",
        description = "Actualiza la valoración de un activo («mi fondo vale ahora 52.300 €»): current_value y/o expected_annual_return_percent (> -100; negativos componen pérdidas). Subset deliberado del PATCH completo. Sin owner-check: cualquier member edita cualquier activo del hogar (contrato del ledger). Devuelve valor anterior y nuevo. Mueve la proyección entera.",
        annotations(title = "Actualizar valor de activo", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_asset_value(
        &self,
        Parameters(p): Parameters<UpdateAssetValueParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<(Uuid, crate::handlers::assets::PatchAssetBody), ApiError> {
            if p.current_value.is_none() && p.expected_annual_return_percent.is_none() {
                return Err(ApiError::BadRequest(
                    "provide current_value and/or expected_annual_return_percent".into(),
                ));
            }
            Ok((
                parse_uuid_param("asset_id", &p.asset_id)?,
                crate::handlers::assets::PatchAssetBody {
                    category_id: None,
                    name: None,
                    current_value: p
                        .current_value
                        .as_deref()
                        .map(|v| parse_decimal_param("current_value", v))
                        .transpose()?,
                    purchase_price: None,
                    is_liquid: None,
                    expected_annual_return_percent: p
                        .expected_annual_return_percent
                        .as_deref()
                        .map(|v| parse_decimal_param("expected_annual_return_percent", v))
                        .transpose()?,
                    notes: None,
                    sort_index: None,
                },
            ))
        };
        let (asset_id, body) = match run() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            // Valor anterior: del listado core (sin SQL propio en el módulo MCP).
            let before = list_assets_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|a| a.id == asset_id)
            .map(|a| a.current_value);
            let a = patch_asset_core(&self.state, id.installation_id, id.user_id, asset_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": a.id,
                "name": a.name,
                "valor_anterior": before.map(|v| v.to_string()),
                "valor_nuevo": a.current_value.to_string(),
                "expected_annual_return_percent": a.expected_annual_return_percent.map(|v| v.to_string()),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_asset",
        description = "Da de alta un activo («he abierto un depósito de 10.000 € al 3 %»): nombre, categoría scope asset, valor actual, liquidez (default true), rentabilidad esperada opcional (> -100). Mueve la proyección entera.",
        annotations(title = "Crear activo", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_asset(
        &self,
        Parameters(p): Parameters<CreateAssetParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::assets::CreateAssetBody, ApiError> {
            Ok(crate::handlers::assets::CreateAssetBody {
                category_id: parse_uuid_param("category_id", &p.category_id)?,
                name: p.name.clone(),
                current_value: parse_decimal_param("current_value", &p.current_value)?,
                purchase_price: p
                    .purchase_price
                    .as_deref()
                    .map(|v| parse_decimal_param("purchase_price", v))
                    .transpose()?,
                is_liquid: p.is_liquid,
                expected_annual_return_percent: p
                    .expected_annual_return_percent
                    .as_deref()
                    .map(|v| parse_decimal_param("expected_annual_return_percent", v))
                    .transpose()?,
                notes: p.notes.clone(),
                sort_index: None,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let a = create_asset_core(&self.state, id.installation_id, id.user_id, body).await?;
            Ok(serde_json::json!({
                "id": a.id,
                "resumen": format!("{} · {} ({})", a.name, a.current_value,
                    if a.is_liquid { "líquido" } else { "ilíquido" }),
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_liability",
        description = "Da de alta un pasivo (deuda/préstamo): label, categoría scope liability, y principal explícito O derive_principal_from_plan=true con el plan completo (cuota + frecuencia monthly|weekly + fecha fin — el principal se deriva por amortización francesa). Mueve la proyección entera.",
        annotations(title = "Crear pasivo", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_liability(
        &self,
        Parameters(p): Parameters<CreateLiabilityParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::liabilities::CreateLiabilityBody, ApiError> {
            Ok(crate::handlers::liabilities::CreateLiabilityBody {
                category_id: parse_uuid_param("category_id", &p.category_id)?,
                label: p.label.clone(),
                type_tag: None,
                derive_principal_from_plan: p.derive_principal_from_plan,
                principal: p
                    .principal
                    .as_deref()
                    .map(|v| parse_decimal_param("principal", v))
                    .transpose()?,
                apr_percent: p
                    .apr_percent
                    .as_deref()
                    .map(|v| parse_decimal_param("apr_percent", v))
                    .transpose()?,
                payment_amount: p
                    .payment_amount
                    .as_deref()
                    .map(|v| parse_decimal_param("payment_amount", v))
                    .transpose()?,
                payment_frequency: p
                    .payment_frequency
                    .as_deref()
                    .map(crate::handlers::liabilities::PaymentFrequency::parse)
                    .transpose()?,
                payment_end_date: p
                    .payment_end_date
                    .as_deref()
                    .map(|d| parse_date_param("payment_end_date", d))
                    .transpose()?,
                notes: p.notes.clone(),
                sort_index: None,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let l = create_liability_core(&self.state, id.installation_id, id.user_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": l.id,
                "resumen": format!("{} · principal {}", l.label, l.principal),
                "principal_derived_from_plan": l.principal_derived_from_plan,
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "create_budget_entry",
        description = "Añade una partida al presupuesto mensual: categoría income|expense + importe > 0. En modo A el presupuesto es la fuente del ahorro proyectado: esto mueve la proyección entera — considera enseñar antes el impacto con simulate_projection. ends_at_retirement y expense_end_date son excluyentes.",
        annotations(title = "Crear partida de presupuesto", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn create_budget_entry(
        &self,
        Parameters(p): Parameters<CreateBudgetEntryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<crate::handlers::budget::CreateBudgetEntryBody, ApiError> {
            Ok(crate::handlers::budget::CreateBudgetEntryBody {
                category_id: parse_uuid_param("category_id", &p.category_id)?,
                amount: parse_decimal_param("amount", &p.amount)?,
                notes: p.notes.clone(),
                sort_index: None,
                persists_after_retirement: p.persists_after_retirement.unwrap_or(false),
                ends_at_retirement: p.ends_at_retirement.unwrap_or(false),
                expense_end_date: p
                    .expense_end_date
                    .as_deref()
                    .map(|d| parse_date_param("expense_end_date", d))
                    .transpose()?,
            })
        };
        let body = match run() {
            Ok(b) => b,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let b = create_budget_entry_core(&self.state, id.installation_id, id.user_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": b.id,
                "category_id": b.category_id,
                "scope": b.scope,
                "amount_monthly": b.amount.to_string(),
                "persists_after_retirement": b.persists_after_retirement,
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_budget_entry",
        description = "Edita una partida del presupuesto («sube el presupuesto de ocio a 250 €»): cualquier campo es opcional; clear_expense_end_date borra la fecha fin. Mueve la proyección entera en modo A.",
        annotations(title = "Editar partida de presupuesto", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_budget_entry(
        &self,
        Parameters(p): Parameters<UpdateBudgetEntryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<(Uuid, crate::handlers::budget::PatchBudgetEntryBody), ApiError> {
            if p.expense_end_date.is_some() && p.clear_expense_end_date == Some(true) {
                return Err(ApiError::BadRequest(
                    "provide either expense_end_date or clear_expense_end_date, not both".into(),
                ));
            }
            Ok((
                parse_uuid_param("id", &p.id)?,
                crate::handlers::budget::PatchBudgetEntryBody {
                    category_id: parse_opt_uuid_param("category_id", &p.category_id)?,
                    amount: p
                        .amount
                        .as_deref()
                        .map(|v| parse_decimal_param("amount", v))
                        .transpose()?,
                    notes: p.notes.clone(),
                    sort_index: None,
                    persists_after_retirement: p.persists_after_retirement,
                    ends_at_retirement: p.ends_at_retirement,
                    expense_end_date: p
                        .expense_end_date
                        .as_deref()
                        .map(|d| parse_date_param("expense_end_date", d))
                        .transpose()?,
                    clear_expense_end_date: p.clear_expense_end_date,
                },
            ))
        };
        let (entry_id, body) = match run() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let b = patch_budget_entry_core(&self.state, id.installation_id, id.user_id, entry_id, body)
                .await?;
            Ok(serde_json::json!({
                "id": b.id,
                "category_id": b.category_id,
                "scope": b.scope,
                "amount_monthly": b.amount.to_string(),
                "persists_after_retirement": b.persists_after_retirement,
            }))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_allocation_rule",
        description = "Edita una regla de la cascada de asignación («aporta 200 € más al mes al fondo indexado»): amount (euros para fixed, % para percent), cap (kind+value o clear_cap) y enabled. Deliberadamente SIN create/delete/reorder desde chat: los invariantes del sumidero los enforcea el servidor con errores tipados (remainder_required, uncapped_remainder_exists). Mueve la proyección entera.",
        annotations(title = "Editar regla de asignación", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_allocation_rule(
        &self,
        Parameters(p): Parameters<UpdateAllocationRuleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let run = || -> Result<(Uuid, crate::handlers::allocation_rules::PatchAllocationRuleBody), ApiError> {
            if p.amount.is_none()
                && p.cap_kind.is_none()
                && p.clear_cap.is_none()
                && p.enabled.is_none()
            {
                return Err(ApiError::BadRequest(
                    "provide at least one of amount, cap_kind/cap_value, clear_cap, enabled".into(),
                ));
            }
            if p.clear_cap == Some(true) && p.cap_kind.is_some() {
                return Err(ApiError::BadRequest(
                    "provide either a cap or clear_cap, not both".into(),
                ));
            }
            let cap = if p.clear_cap == Some(true) {
                Some(serde_json::Value::Null)
            } else if let Some(kind) = &p.cap_kind {
                Some(serde_json::json!({"kind": kind, "value": p.cap_value}))
            } else {
                None
            };
            Ok((
                parse_uuid_param("rule_id", &p.rule_id)?,
                crate::handlers::allocation_rules::PatchAllocationRuleBody {
                    target_asset_id: None,
                    kind: None,
                    amount: p
                        .amount
                        .as_ref()
                        .map(|a| serde_json::Value::String(a.clone())),
                    cap,
                    enabled: p.enabled,
                    notes: None,
                },
            ))
        };
        let (rule_id, body) = match run() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let before = list_allocation_rules_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|r| r.id == rule_id);
            let r = patch_allocation_rule_core(&self.state, id.installation_id, id.user_id, rule_id, body)
                .await?;
            Ok(serde_json::json!({"id": r.id, "antes": before, "despues": r}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_recurring_rule",
        description = "Retira una plantilla recurrente («deja de apuntarme el gimnasio»). Solo borra la PLANTILLA: las instancias ya materializadas sobreviven. Sin confirm=true no borra nada — devuelve un preview con la plantilla y su cursor last_materialized_month.",
        annotations(title = "Borrar recurrente", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_recurring_rule(
        &self,
        Parameters(p): Parameters<DeleteRecurringRuleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let rule_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            // Preview vía la core de listado (cero SQL propio); 404 si no es suya.
            let rule = list_recurring_rules_core(&self.state.pool, id.installation_id, id.user_id)
                .await?
                .into_iter()
                .find(|r| r.id == rule_id)
                .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_recurring_rule",
                    "effects": {
                        "rule": rule,
                        "nota": "solo se borra la plantilla; las instancias materializadas se conservan",
                    },
                }));
            }
            delete_recurring_rule_core(&self.state.pool, id.installation_id, id.user_id, rule_id)
                .await?;
            Ok(serde_json::json!({"id": rule_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_transaction",
        description = "Borra un movimiento PROPIO (hard delete; movimientos de otro usuario → not_found). Sin confirm=true no borra: devuelve el movimiento completo como preview.",
        annotations(title = "Borrar movimiento", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_transaction(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let txn_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let txn =
                get_transaction_core(&self.state.pool, id.installation_id, id.user_id, txn_id)
                    .await?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_transaction",
                    "effects": {"transaction": txn},
                }));
            }
            delete_transaction_core(&self.state, id.installation_id, id.user_id, txn_id).await?;
            Ok(serde_json::json!({"id": txn_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "update_fire_settings",
        description = "Cambia la configuración FIRE de la instalación — SOLO el owner: SWR, inflación asumida, fuente del ahorro (modo A budget | B transactions_avg | C budget_income_real_expense), modo del objetivo, importe manual, impuestos y tramos. Merge campo a campo sobre el estado actual: los campos omitidos NUNCA se resetean. Sin confirm=true no persiste nada — devuelve el before/after validado. Es el mayor radio de todas las tools: mueve la proyección entera; considera enseñar antes el impacto con simulate_projection.",
        annotations(title = "Configurar FIRE", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn update_fire_settings(
        &self,
        Parameters(p): Parameters<UpdateFireSettingsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let build = || -> Result<crate::handlers::installation::FireSettingsPatch, ApiError> {
            let mut patch = crate::handlers::installation::FireSettingsPatch::default();
            patch.swr_pct = p
                .swr_pct
                .as_deref()
                .map(|v| parse_decimal_param("swr_pct", v))
                .transpose()?;
            patch.annual_inflation_assumption_percent = p
                .annual_inflation_assumption_percent
                .as_deref()
                .map(|v| parse_decimal_param("annual_inflation_assumption_percent", v))
                .transpose()?;
            patch.fire_number_manual_amount = p
                .fire_number_manual_amount
                .as_deref()
                .map(|v| parse_decimal_param("fire_number_manual_amount", v))
                .transpose()?;
            patch.taxes_enabled = p.taxes_enabled;
            // Enums: reusar el Deserialize custom (misma lista de variantes y aliases que HTTP).
            patch.savings_source = p
                .savings_source
                .as_ref()
                .map(|s| {
                    serde_json::from_value(serde_json::Value::String(s.trim().to_string()))
                        .map_err(|e| ApiError::BadRequest(format!("savings_source: {e}")))
                })
                .transpose()?;
            patch.fire_number_mode = p
                .fire_number_mode
                .as_ref()
                .map(|s| {
                    serde_json::from_value(serde_json::Value::String(s.trim().to_string()))
                        .map_err(|e| ApiError::BadRequest(format!("fire_number_mode: {e}")))
                })
                .transpose()?;
            patch.tax_brackets = p
                .tax_brackets
                .as_ref()
                .map(|brackets| {
                    brackets
                        .iter()
                        .map(|b| {
                            Ok(crate::handlers::installation::TaxBracket {
                                up_to: b
                                    .up_to
                                    .as_deref()
                                    .map(|v| parse_decimal_param("tax_brackets.up_to", v))
                                    .transpose()?,
                                pct: parse_decimal_param("tax_brackets.pct", &b.pct)?,
                            })
                        })
                        .collect::<Result<Vec<_>, ApiError>>()
                })
                .transpose()?;
            Ok(patch)
        };
        let patch = match build() {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            // El PATCH de instalación es owner-only también por HTTP.
            if id.role != crate::handlers::membership::MembershipRole::Owner {
                return Err(ApiError::Forbidden);
            }
            let apply = p.confirm.unwrap_or(false);
            let outcome = crate::handlers::installation::patch_fire_settings_core(
                &self.state,
                id.installation_id,
                id.user_id,
                patch,
                apply,
            )
            .await?;
            if apply {
                Ok(serde_json::json!({"applied": true, "outcome": outcome}))
            } else {
                Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "update_fire_settings",
                    "effects": outcome,
                }))
            }
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_planning_flow",
        description = "Borra una entrada de «Próximos». Sin confirm=true devuelve el flujo como preview. Mueve la proyección entera.",
        annotations(title = "Borrar próximo", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_planning_flow(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let flow_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let flow = list_planning_flows_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|f| f.id == flow_id)
            .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_planning_flow",
                    "effects": {"flow": flow},
                }));
            }
            delete_planning_flow_core(&self.state, id.installation_id, id.user_id, flow_id)
                .await?;
            Ok(serde_json::json!({"id": flow_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_budget_entry",
        description = "Borra una partida del presupuesto. Sin confirm=true devuelve la partida como preview. En modo A mueve la proyección entera.",
        annotations(title = "Borrar partida de presupuesto", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_budget_entry(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let entry_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let entry = budget_snapshot_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .entries
            .into_iter()
            .find(|e| e.id == entry_id)
            .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_budget_entry",
                    "effects": {"entry": entry},
                }));
            }
            delete_budget_entry_core(&self.state, id.installation_id, id.user_id, entry_id)
                .await?;
            Ok(serde_json::json!({"id": entry_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_asset",
        description = "Borra un activo del hogar. Sin confirm=true devuelve un preview con los efectos colaterales: los movimientos y lotes de import vinculados quedan desvinculados (SET NULL), no se borran. Mueve la proyección entera.",
        annotations(title = "Borrar activo", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_asset(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let asset_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let asset = list_assets_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|a| a.id == asset_id)
            .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                let effects =
                    asset_delete_effects(&self.state.pool, id.installation_id, asset_id).await?;
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_asset",
                    "effects": {
                        "asset": {"id": asset.id, "name": asset.name, "current_value": asset.current_value.to_string()},
                        "unlinked": effects,
                    },
                }));
            }
            delete_asset_core(&self.state, id.installation_id, id.user_id, asset_id).await?;
            Ok(serde_json::json!({"id": asset_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_liability",
        description = "Borra un pasivo del hogar. Sin confirm=true devuelve un preview con los efectos: los movimientos vinculados quedan desvinculados (SET NULL). Mueve la proyección entera.",
        annotations(title = "Borrar pasivo", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_liability(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let liab_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let liab = list_liabilities_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|l| l.id == liab_id)
            .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                let unlinked =
                    liability_delete_effects(&self.state.pool, id.installation_id, liab_id)
                        .await?;
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_liability",
                    "effects": {
                        "liability": {"id": liab.id, "label": liab.label, "principal": liab.principal.to_string()},
                        "transactions_unlinked": unlinked,
                    },
                }));
            }
            delete_liability_core(&self.state, id.installation_id, id.user_id, liab_id).await?;
            Ok(serde_json::json!({"id": liab_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_snapshot",
        description = "Borra un snapshot PROPIO del histórico (sus items caen en cascada). Sin confirm=true devuelve la cabecera + nº de items como preview. No afecta a la proyección.",
        annotations(title = "Borrar snapshot", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_snapshot(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let snap_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let snap =
                list_snapshots_core(&self.state.pool, id.installation_id, id.user_id, None, None)
                    .await?
                    .into_iter()
                    .find(|s| s.id == snap_id)
                    .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_snapshot",
                    "effects": {
                        "snapshot": {
                            "id": snap.id,
                            "kind": snap.kind,
                            "snapshot_date": snap.snapshot_date_ymd,
                            "total": snap.total.to_string(),
                            "items_deleted": snap.items.len(),
                        },
                    },
                }));
            }
            delete_snapshot_core(&self.state.pool, id.installation_id, id.user_id, snap_id)
                .await?;
            Ok(serde_json::json!({"id": snap_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "delete_import",
        description = "Borra un lote de import Y TODAS sus transacciones en cascada. Sin confirm=true devuelve un preview con el lote (fuente, fichero, txn_count). Mismo contrato que el ?confirm=true del endpoint HTTP.",
        annotations(title = "Borrar lote de import", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn delete_import(
        &self,
        Parameters(p): Parameters<DeleteByIdParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let import_id = match parse_uuid_param("id", &p.id) {
            Ok(v) => v,
            Err(e) => return to_tool_outcome(e),
        };
        let res = async {
            require_mcp_write(&self.state.pool, &id).await?;
            let batch = list_imports_core(
                &self.state.pool,
                id.installation_id,
                id.user_id,
                LedgerView::Household,
            )
            .await?
            .into_iter()
            .find(|b| b.id == import_id)
            .ok_or(ApiError::NotFound)?;
            if !p.confirm.unwrap_or(false) {
                return Ok(serde_json::json!({
                    "preview": true,
                    "confirm_required": true,
                    "action": "delete_import",
                    "effects": {"transactions_deleted": batch.txn_count, "import": batch},
                }));
            }
            delete_import_core(&self.state, id.installation_id, id.user_id, import_id).await?;
            Ok(serde_json::json!({"id": import_id, "deleted": true}))
        }
        .await;
        to_tool_result(res)
    }

    #[tool(
        name = "list_transaction_imports",
        description = "Lotes de import CSV (fuente bancaria, fichero original, cuenta vinculada, nº de movimientos, orden created_at DESC). Usa el id como filtro import_id en list_transactions para auditar un lote.",
        annotations(title = "Lotes de import", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_transaction_imports(
        &self,
        Parameters(p): Parameters<ViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = identity(&ctx)?;
        let view = resolve_view(&p.view);
        to_tool_result(
            list_imports_core(&self.state.pool, id.installation_id, id.user_id, view).await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FutureFinMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("futurefin", self.state.version).with_title("FutureFin"),
            )
            .with_instructions(
                "Finanzas del hogar FutureFin: lectura, simulación (simulate_projection, sin \
                 persistir) y escritura. Los importes monetarios son strings decimales en la \
                 divisa base de la instalación (EUR salvo que get_settings diga otra cosa); las \
                 series de charts (projection/history) usan números. `view=\"mine\"` filtra a \
                 los datos del usuario del token; por defecto se devuelve el agregado del hogar. \
                 Empieza por get_summary para el estado actual y get_settings para el contexto. \
                 Las tools de escritura respetan el rol del token (los viewers no escriben) y el \
                 ajuste `mcp_write_enabled` de la instalación (con la escritura desactivada \
                 devuelven `mcp_write_disabled` — explícaselo al usuario, no reintentes); las \
                 destructivas piden `confirm: true` y sin él devuelven un preview.",
            )
    }
}
