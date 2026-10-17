//! `/v1/plans` — «Planes» de 6.0.0 (F2-1: tablas + CRUD).
//!
//! Un plan es una lista de cambios sobre la realidad (ingreso/gasto que sube o baja, un activo
//! que entra o se vende, un préstamo que se firma), agrupados en bloques, con sus supuestos
//! (inflación, indexación, horizonte, ajuste de mercado). Como mucho uno por persona está activo.
//!
//! **F2-1 no mueve ninguna cifra**: la proyección todavía no lee estas tablas (F2-4). Diseño:
//! `.claude/rfc/planes-6.0.md`; plan de la fase: `.claude/rfc/planes-6.0-f2.md`.
//!
//! - [`crud`]: planes y bloques, lectura anidada, activación.
//! - [`changes`]: cambios.
//! - [`validate`]: supuestos, parámetros por tipo y forma de un cambio.
//! - [`engine_map`]: la regla única de meses (fecha → mes del bucle); el resto llega en F2-4.
//!
//! Sin tools MCP hasta F2-6 (hueco abierto en la skill `futurefin-mcp-parity` §3.2). Las cores
//! `*_core` ya están separadas para que esas tools las reutilicen.

pub mod changes;
pub mod crud;
pub mod engine_map;
pub mod validate;

use axum::routing::{get, patch, post};
use axum::Router;

pub fn plans_router() -> Router {
    Router::new()
        .route("/", get(crud::list_plans).post(crud::create_plan))
        .route("/deactivate", post(crud::deactivate_plans))
        .route(
            "/{id}",
            get(crud::get_plan)
                .patch(crud::patch_plan)
                .delete(crud::delete_plan),
        )
        .route("/{id}/activate", post(crud::activate_plan))
        .route("/{id}/blocks", post(crud::create_plan_block))
        .route(
            "/{id}/blocks/{block_id}",
            patch(crud::patch_plan_block).delete(crud::delete_plan_block),
        )
        .route("/{id}/changes", post(changes::create_plan_change))
        .route(
            "/{id}/changes/{change_id}",
            patch(changes::patch_plan_change).delete(changes::delete_plan_change),
        )
}
