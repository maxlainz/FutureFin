//! Servidor MCP de FutureFin: 10 tools de SOLO LECTURA sobre las mismas core fns que
//! sirven los handlers HTTP (`*_core` / `projection_series_cached`). Cero SQL propio y
//! cero tipos de respuesta paralelos: cada tool serializa el MISMO struct serde que el
//! endpoint (Decimal-as-string intacto), así handler y tool no pueden divergir.
//!
//! Errores: los de dominio/validación devuelven `CallToolResult{is_error:true}` con el
//! mismo JSON `{error, message}` del wire HTTP (el LLM puede leerlo y corregir el input);
//! `Db`/`Unavailable` se sanitizan a `ErrorData` interno (detalle solo a tracing), espejo
//! del contrato de `error.rs`.

use crate::error::{ApiError, ErrorBody};
use crate::handlers::assets::list_assets_core;
use crate::handlers::budget::budget_snapshot_core;
use crate::handlers::history::history_series_core;
use crate::handlers::installation::{installation_access_core, settings_user_core};
use crate::handlers::liabilities::list_liabilities_core;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::planning::list_planning_flows_core;
use crate::handlers::projection::projection_series_cached;
use crate::handlers::summary::summary_core;
use crate::handlers::transactions::crud::list_transactions_core;
use crate::handlers::transactions::summary::transactions_summary_core;
use crate::mcp::auth::McpIdentity;
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
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FutureFinMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("futurefin", self.state.version).with_title("FutureFin"),
            )
            .with_instructions(
                "Datos financieros del hogar FutureFin (solo lectura). Los importes monetarios \
                 son strings decimales en la divisa base de la instalación (EUR salvo que \
                 get_settings diga otra cosa); las series de charts (projection/history) usan \
                 números. `view=\"mine\"` filtra a los datos del usuario del token; por defecto \
                 se devuelve el agregado del hogar. Empieza por get_summary para el estado \
                 actual y get_settings para el contexto de configuración.",
            )
    }
}
