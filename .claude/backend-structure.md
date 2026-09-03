# Backend Structure (`apps/api/src/`)

> **Dueño de**: el mapa de módulos de `apps/api/src` y la receta de handler nuevo. **NO es dueño de**: rutas y contratos (api-routes.md), decisiones (architecture-contract).

Espejo de [`frontend-structure.md`](frontend-structure.md) para el backend: qué vive en cada módulo de `apps/api/src/` y, más abajo, la receta paso a paso para añadir un handler nuevo. Para el contrato de cada ruta (payloads, códigos de error, auth) ver [`api-routes.md`](api-routes.md); para el porqué de una decisión, `futurefin-architecture-contract`.

## Módulos de `apps/api/src/`

Entry point: `main.rs` (bin); los módulos compartidos del crate se declaran en `lib.rs`.

- `routes/mod.rs` — full route map; all routes under `/v1/` except `/health`, `/openapi.json`, `/mcp` y el protocolo OAuth. `DefaultBodyLimit` caps requests at 1 MiB globally, 16 MiB on `/backup/user-import*` — **pero `DefaultBodyLimit` actúa vía extractores y `/mcp` es un `route_service`**, así que su tope se fija aparte y explícitamente en `mcp::MCP_MAX_REQUEST_BODY_BYTES` (1 MiB; sin esa línea regía el default de rmcp, 4 MiB). Aquí viven también las **dos** capas CORS: la del API con `allow_credentials(true)` y la de `/mcp` sin credenciales — el `merge` de `mcp` va **después** del `.layer(...)` a propósito, porque `Router::layer` solo envuelve lo ya registrado.
- `state.rs` — `AppState` (pool, cookie_secure, session_ttl_days, version) y **los DOS caches de
  proyección**: `projection_cache` (`ProjectionCacheKey { installation_id, view, owner_user_id,
  density }`) y, desde 5.0.0/WP6b, `bands_cache` (`BandsCacheKey { installation_id, user_id, paths,
  seed }`, las bandas de Monte Carlo). Comparten TTL (`PROJECTION_CACHE_TTL`, 60 min sliding) y
  —lo que de verdad importa— **las dos invalidaciones**: `invalidate_projection_by_installation` y
  `invalidate_projection_by_user` borran los dos mapas. Van separados porque la clave de las bandas
  lleva dos ejes que la serie no tiene (`paths`, `seed`) y su contenido cuesta un orden de magnitud
  más; mezclarlos habría hecho que un cambio de semilla tirara la serie determinista por el suelo.
  La clave de bandas **no** lleva `view`: solo existe `mine` (§Projection bands de
  [`api-routes.md`](api-routes.md)).
- `error.rs` — `ApiError` → `(StatusCode, JSON {error, code, message})` via `IntoResponse`, donde `code` es el **código estable** que sale del prefijo `snake_code:` del mensaje (desde 3.10.0; sin prefijo válido cae a la clase HTTP). Ese mismo `ErrorBody` es el que viaja en los errores de las tools MCP. `impl From<sqlx::Error>` detects SQLSTATE 23505 → `Conflict` (409), 23503 → `BadRequest`; handlers can just `?` any `sqlx::Error` without manual mapping.
- `auth/` — password hashing (Argon2id)
- `handlers/session.rs` — `require_session_user` reads cookie `ff_session` → validates against `sessions` table
- `handlers/api_tokens.rs` — tokens de API por usuario (Bearer `ffp_…`, solo se persiste el SHA-256; CRUD `/v1/api-tokens` por cookie) + `require_api_token`, la credencial del servidor MCP
- `mcp/` — servidor MCP embebido (`/mcp`, Streamable HTTP, rmcp 3.1): **68 tools** (28 de lectura+simulación, 40 de escritura) que llaman a las mismas core fns `*_core` que los handlers HTTP (cero deriva, Decimal-as-string intacto; la invalidación de cache vive dentro de las cores de mutación). Auth = middleware Bearer (`mcp/auth.rs`) con identidad y rol vivos por request; toda escritura pasa por `require_mcp_write`, que son **tres puertas en orden** — rol vivo → scope de la credencial (`api_tokens.scope`, `read_only` corta) → toggle `installation.mcp_write_enabled` (Ajustes → Integraciones) — y **cada llamada al gate abre una fila en `mcp_write_audit`** que la tool cierra con `settled(...)`. Las 17 con preview piden `confirm: true` (sin él devuelven un preview) y **8 de ellas exigen además el `confirm_token` de un solo uso que solo emite ese preview**; 18 escrituras publican además el bloque `impact`. Desde la Fase 6 el servidor declara también la capacidad **`prompts`** (3 flujos, `prompts/list` + `prompts/get`, sin tocar la BD). `FUTUREFIN_MCP_ENABLED=0` **no lo desmonta**: la ruta se monta igual y responde 404 JSON `mcp_disabled`. Los contadores no se cuentan a mano — los congela `every_write_tool_in_the_source_calls_require_mcp_write` (test de integración en `apps/api/tests/mcp_write.rs`, sin BD)
- `handlers/changes.rs` — `GET /v1/changes`: qué se ha tocado desde una fecha, leyendo los `updated_at` que ya se mantienen en varias tablas. **No cubre borrados** (no hay tombstones) y la respuesta lo declara: no es una auditoría, es «qué ha cambiado de lo que sigue existiendo»
- `handlers/installation.rs` — singleton installation, FIRE settings, `require_installation_member`
- `handlers/membership.rs` — roles: `owner`, `member`, `viewer`; `role_can_write` used by handlers
- `handlers/person_view.rs` — `LedgerView` enum (`Household` / `Mine`) **plus helpers** `scope_where(table_alias)`, `next_arg_index()`, `bind_scope_as`, `bind_scope_scalar`, `as_str()`. Use them instead of duplicating `match view { Household | Mine }` blocks — they enforce consistent placeholder ordering across both branches. `as_str()` es la etiqueta pública (`"household"` | `"mine"`) que las respuestas **ecoan**: existe desde 4.4.0 porque el eco vivía copiado en cuatro handlers como `if view == Mine { "mine" } else { "household" }`, y ese brazo `else` convertía cualquier variante nueva en `"household"` sin avisar. `resolve(as_str(v)) == v` está pinneado en `as_str_round_trips_through_resolve`. **Desde 5.0.0 el default de `resolve()` es `Mine`** (R2): ausente o vacío = el scope del solicitante, `household` explícito. Aquí también vive `require_row_owner` (D21). Y el ensamblado de la proyección (`build_installation_projection_input`, `handlers/projection.rs`) toma desde 5.0.0 **la fecha de nacimiento del usuario cuyo perfil simula** (parámetro `birth_date`, justo detrás de `retirement_profile`): es lo que convierte `target_retirement_age` en un mes del bucle, y en el agregado del hogar cada miembro pasa la SUYA — sin ese parámetro, una simulación por miembro heredaría la edad del solicitante.
- `handlers/projection.rs` — el ensamblado del motor y todas sus lecturas. Dos cosas de 5.0.0
  WP5-2b que hay que saber antes de tocarlo:
  - **`compute_strategy_solves(input, forced_retirement_month, strategy)`** es la ÚNICA función que
    decide qué solve de `crates/engine/src/solve.rs` corresponde a cada estrategia. Tiene dos
    llamantes con inputs distintos —`run_member_projection` (por miembro, el resultado se guarda en
    la entrada de cache, M4) y `simulate_projection_core` (baseline y escenario, para los deltas)—
    y si cada uno decidiera por su cuenta, `retire_at_age` podría publicar un ahorro necesario en la
    serie y `null` en el what-if. Corre SIEMPRE dentro de `heavy::run_projection_sim`: son hasta 26
    proyecciones enteras. `disposable_monthly_of` es su gemelo para el margen, por la misma razón.
  - **`PlanFireTarget::new` se construye UNA vez por respuesta.** La forma de conveniencia
    `fire_target_at_month_index_with_plan` rehace la tabla del puente (`O(P)` gross-ups) en cada
    llamada, y la serie del objetivo la consulta una vez por punto: medido, **1.943 ms** de MISS con
    pensión con fecha, contra 13 ms tras hoistarla. Cualquier lectura nueva que evalúe el objetivo
    punto a punto tiene que reusar el evaluador, no la función libre.
  - **`BuiltProjection::asset_volatility_percent` es un vector PARALELO a `input.assets`** (5.0.0
    WP6b) y se rellena en el MISMO `map` que los construye. El motor `Decimal` lo ignora; es
    entrada exclusiva de Monte Carlo, que **falla** si la longitud no cuadra. La alineación es una
    propiedad de construcción a propósito: una σ descolocada produce bandas estrechas y creíbles —
    el peor fallo posible en esa superficie — y ningún assert de tipo la cazaría. Regresión de
    comportamiento: `projection_bands.rs::the_volatility_vector_follows_the_asset_order`.
- `handlers/projection_bands.rs` — **`GET /v1/projection/bands`** (5.0.0/WP6b): la superficie HTTP
  de `futurefin_engine_stochastic::project_percentile_bands`, su cache propio y las conversiones
  de frontera. Tres funciones que viven aquí a propósito y las usa también `projection.rs` (para el
  eje `monte_carlo` de `simulate_projection`): `volatilities_f64` —la ÚNICA que produce `f64` para
  el crate estocástico, de modo que las bandas y el what-if conviertan igual—, `probability_out`
  —la única por la que sale un número de ese crate, y sale como PROBABILIDAD, nunca como euros— y
  `success_verdict` —el semáforo de D28, con la comparación en puntos porcentuales enteros para que
  «exactamente el umbral» salga verde—. El handler se monta dentro de `projection_router()`.
- `handlers/summary.rs` — `summary_core` toma **`&AppState`, no `&PgPool`** desde 5.0.0 WP5-2b: el
  bloque `plan` (D27) se lee de la cache de proyección y, si no hay entrada, se calcula por
  `projection_series_cached`. Es deliberado que el Resumen dependa del estado: la alternativa era
  una segunda fórmula para las mismas seis cifras, y dos superficies que contestan distinto a la
  misma pregunta es el fallo que esta casa no publica. `plan_from_series` **copia campos y no hace
  una sola cuenta**. Desde WP6b `attach_success` hace lo mismo con el KPI «Éxito del plan», leyendo
  del cache de BANDAS por `projection_bands_cached` (caminos y semilla por defecto): el tile del
  Resumen y el fan chart de Jubilación citan **la misma ejecución** de Monte Carlo. Si el sorteo
  falla, el Resumen no se cae — tres `null` con `success_absent_reason`, y el resto del plan sigue
  viajando.
- `handlers/history.rs` — per-user net-worth **snapshots** under `/v1/history` (capture / backfill CRUD / interpolated series + `GET /v1/history/cashflow` tier-2). Manual snapshots of the user's asset + liability items; the engine (`history.rs`) reconstructs the past series between them. Snapshots are NOT projection inputs → their mutations do **not** invalidate the projection cache. **Cotas de publicación (4.4.0, Fase 5)**: `GET /v1/history/series` sin `window_months` devuelve los **últimos 120 meses** (`DEFAULT_HISTORY_WINDOW_MONTHS`), ya no todo el histórico — `1200` sigue siendo «todo», y la respuesta declara `window_months` / `window_truncated` / `first_snapshot_date_ymd`; los numéricos de chart se publican a **2 decimales** (`CHART_DP`) y `month_fraction` a **4** (`MONTH_FRACTION_DP`), redondeo de publicación como `money_out` — la interpolación sigue exacta. En `/v1/history/cashflow` la **curva fina** se acota a **36 meses** (`MAX_FINE_CURVE_WINDOW_MONTHS`) y pasarse **no es un 400**: llegan los `months[]` completos y `fine_absent_reason` dice por qué falta `fine` (`not_requested` | `window_too_large_for_curve` | `no_asset_linked_transactions` | `no_snapshots_to_anchor`).
- `handlers/transactions/` — per-user **histórico de gasto mensual** under `/v1/transactions` (import CSV MyInvestor/N26, movimientos manuales, reglas de categorización, comparativa mes vs budget vs promedio ponderado, y **movimientos recurrentes**). Modules: `crud.rs`, `import.rs` (preview→confirm stateless, presets en `csv_presets.rs`), `reconcile.rs` (conciliación de transferencias, 3.5.0: pase automático determinista de importes opuestos a ≤5 días + par/desconciliación manual — un movimiento **conciliado** sigue visible pero queda fuera de TODOS los agregados de flujo), `rules.rs`, `aggregate.rs` (`GET /v1/transactions/aggregate`: suma/conteo agrupados por mes, categoría o kind **dentro de SQL** — el predicado de conciliadas va en la core, no en el modelo que lee las filas), `duplicates.rs` (`GET /v1/transactions/duplicates`: agrupa por la huella canónica que ya usa el dedup del import), `summary.rs` (incluye el helper `transactions_avg` que consumen los modos B y C, contando solo «meses reales»: los meses solo-recurrentes y las transferencias conciliadas se excluyen; desde el issue #5 la comparativa de la pestaña Movimientos usa **el mismo predicado de mes real**), `recurring.rs` (plantillas recurrentes + **convergencia**: desde 3.9.0 las instancias existen exactamente en los meses con datos reales, sin cursor), `schema.rs`. Las transacciones son inputs del engine **solo en los modos que usan transacciones** (`fire_settings.savings_source ∈ {transactions_avg (B), budget_income_real_expense (C)}`, gate `SavingsSource::uses_transactions()`; desde 3.9.0 las **ventanas del promedio son configurables por lado** — ingreso y gasto, meses + semántica): en esos casos las mutaciones invalidan la cache de proyección vía `invalidate_projection_if_savings_uses_transactions` (best-effort post-commit); con `savings_source = budget` (default, modo A) **ningún handler invalida** (contrato histórico intacto). `rules.rs` y los previews nunca invalidan; **el borrado de una regla recurrente SÍ invalida (COND, corrección 4.0.0)** — no cambia el conjunto pero sí su **clasificación**: el `ON DELETE SET NULL` convierte las instancias huérfanas en movimientos reales y puede activar un mes que el promedio ignoraba (regresión: `transactions_projection_cache.rs`).
- `db.rs` — pool setup (`max=10, min=1, idle_timeout=10min, max_lifetime=30min`) + `sqlx::migrate!` runner. No more auto-repair loop; if a checksum mismatches in dev, fix manually via `DELETE FROM _sqlx_migrations WHERE version = X` and rerun.
- **`tests/`** — integration tests against a real Postgres (schema-isolated per test). See [`.claude/tests.md`](.claude/tests.md).

## Cómo añadir un handler

Step-by-step pattern used throughout the codebase.

## 1. Create handler file `apps/api/src/handlers/foo.rs`

```rust
use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{LedgerView, LedgerViewQuery};
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, post, patch, delete};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// Response type
#[derive(Debug, Serialize, ToSchema)]
pub struct FooResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

// Request body
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFooBody {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
}

#[utoipa::path(
    post,
    path = "/v1/foo",
    tag = "foo",
    request_body = CreateFooBody,
    responses(
        (status = 201, description = "Created", body = FooResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No session"),
        (status = 403, description = "Insufficient role"),
    )
)]
pub async fn create_foo(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateFooBody>,
) -> Result<(axum::http::StatusCode, Json<FooResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    
    // DB insert...
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO foo (installation_id, owner_user_id, amount) VALUES ($1, $2, $3) RETURNING id"#
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(body.amount)
    .fetch_one(&state.pool)
    .await?;
    
    Ok((axum::http::StatusCode::CREATED, Json(FooResponse { id, amount: body.amount })))
}

pub fn foo_router() -> Router {
    Router::new()
        .route("/", get(list_foos).post(create_foo))
        .route("/{id}", patch(patch_foo).delete(delete_foo))
}
```

## 2. Register in `handlers/mod.rs`
```rust
pub mod foo;
```

## 3. Wire route in `routes/mod.rs`
```rust
use crate::handlers::foo::foo_router;
// in app_router():
.nest("/foo", foo_router())
```

Tres trampas del cableado, las tres cobradas ya en este repo (D18, D21):

- **Si la ruta va tras un flag, móntala igual y que el HANDLER diga que no.** La forma del router no
  puede depender del entorno: una ruta ausente cae al fallback final, que en la imagen publicada es
  un `ServeDir`, y `ServeDir` **no llama a su fallback para métodos distintos de GET/HEAD** → un
  `POST` se lleva un **405 con cuerpo vacío** y un `GET` se lleva el **shell de la SPA en
  `text/html`**. Patrón correcto: `/v1/auth/sso` (`sso_disabled`) y `/mcp` (`mcp_disabled`), que
  responden 404/401 JSON con código estable. Y tu test tiene que montar el fallback real
  (`TestConfig::web_static_root`) o estará probando un router que no se publica.
- **`DefaultBodyLimit` solo alcanza a lo que pasa por un extractor.** Si tu ruta es un
  `route_service` (o lee el body a mano), fija su tope explícitamente y añádele una fila a
  `apps/api/tests/body_limits.rs` — si no, el tope real es el que traiga la librería (I11).
- **`route_layer`, no `layer`, para cualquier capa que no deba tocar el fallback.** `Router::layer`
  envuelve también el fallback del router, y un `merge` posterior se lo lleva al router destino: una
  capa de auth puesta con `layer` acaba interceptando **toda ruta desconocida** de la aplicación.

## 4. Register types in `openapi.rs`
Add `FooResponse`, `CreateFooBody` to the `components(schemas(...))` list, and the handler fn to
`paths(...)`.

**Autenticación en la spec (4.0.0)**: `openapi.rs` declara `security(("ff_session" = []))` **global**,
así que un handler con sesión no necesita decir nada. Un endpoint **público** debe llevar
`security(())` en su `#[utoipa::path]` — hoy lo llevan exactamente **siete** (`grep -rn 'security(())' apps/api/src | wc -l`): `health_check`,
`ready_check`, `register`, `login`, `sso_login` (desde 4.3.0: no lleva credencial de FutureFin
porque la credencial la pone el proxy de confianza en una cabecera) y, desde 4.3.1, los dos de
«Entrar con Home Assistant» (`/v1/auth/ha/start` + `/v1/auth/ha/callback`, `handlers/ha_sso.rs`). Sin esa declaración global la spec presentaba 81 operaciones con
sesión obligatoria como públicas y cualquier cliente generado nacía sin credencial.

**Dos trampas que `tests/openapi_contract.rs` ya vigila** — si tu handler las dispara, el test falla
y no hace falta que las recuerdes, pero conviene saber por qué existe:
- **Colisión de nombre de componente**: utoipa nombra el schema por el **último segmento del tipo**,
  así que dos structs `ImportPreviewResponse` en módulos distintos se machacan y ambos endpoints
  acaban apuntando al mismo `$ref`. Desambigua con `#[schema(as = OtroNombre)]`.
- **Path con plantilla sin `params(...)`**: `/v1/foo/{id}` sin declarar `id` produce un documento
  formalmente inválido.

## 5. Add migration if needed
```bash
touch apps/api/migrations/$(date +%Y%m%d%H%M%S)_add_foo.sql
```

## Key patterns
- **Split extractor+auth vs `*_core`**: keep session/membership/role checks in the handler fn and move everything else (validation, SQL, response building, cache invalidation post-commit) into a `pub(crate) async fn foo_core(state, iid, user_id, …)`. That core is what an MCP tool reuses — a handler written monolithically forces the extraction later (see `patch_liability_core`, extracted when `update_liability` shipped). Architecture rationale: `futurefin-architecture-contract` D14.
- All monetary `Decimal` fields need `#[serde(with = "rust_decimal::serde::str")]` for JSON serialization as string
- Optional decimals: `#[serde(with = "rust_decimal::serde::str_option")]`
- `?view=mine` support: add `Query(q): Query<LedgerViewQuery>`, then `let view = q.resolve()`, then build SQL via `view.scope_where("alias")` + bind via `view.bind_scope_as(query_as(&sql), iid, user.id.0)`. **Never** hand-write a `match view { Household => sql_a, Mine => sql_b }` block — the two branches drift (we already had a bug where the `Mine` branch had `$3` and `$2` swapped).
- Never return `sqlx::Error` directly — `impl From<sqlx::Error> for ApiError` maps 23505 → 409 and 23503 → 400 automatically. Just `?` it.
- For long horizons / large datasets in CPU-bound work, wrap in `tokio::task::spawn_blocking` so the Tokio runtime stays responsive.
- Add an integration test in `apps/api/tests/{topic}.rs` using `common::TestApp::spawn()` (see [`.claude/tests.md`](tests.md)). One test per surprising behavior is plenty.
- **Client side: la URL nueva pasa por `apiUrl`.** Si el frontend llama a tu endpoint con un
  `fetch(` directo, envuélvela en `apiUrl("/v1/foo")` (`apps/web/src/lib/basePath.ts`) — es lo que
  hace que la app funcione bajo un subpath (Ingress de Home Assistant). Los wrappers de
  `api/client.ts` ya lo aplican solos. Ver [`frontend-structure.md`](frontend-structure.md)
  §Subpath tras proxy.
- **Si tu handler emite o borra la cookie de sesión**, usa los helpers de `handlers/auth.rs`
  —`session_cookie_path(&state, &headers)`, `session_cookie(&state, sid, path)`,
  `session_cookie_removal(path)`— en vez de construir el `Cookie` a mano: el `Path` va acotado al
  prefijo público de la request, y un borrado con otro `Path` **no** casa con la cookie viva. Ver
  [`auth-and-membership.md`](auth-and-membership.md) §Cookie.

## 6. Add a regression test
Drop a file in `apps/api/tests/foo.rs` that exercises your handler end-to-end:
```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn create_foo_succeeds() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let resp = app.post_json_with_cookie(
        "/v1/foo",
        serde_json::json!({ "amount": "1234" }),
        &owner.cookie,
    ).await;
    assert_eq!(resp.status, http::StatusCode::CREATED);
}
```
Run with `TEST_DATABASE_URL=postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test cargo test --workspace`.

## 7. Evaluate the MCP surface (mandatory)

The MCP catalog (`apps/api/src/mcp/server.rs`) is a derived surface of this API: a new or
changed endpoint must end in exactly one of **tool added/updated**, **deliberate omission
recorded**, or **n/a** — never silence. The decision rubric, the omission register and the
add-a-tool recipe live in
[`futurefin-mcp-parity`](skills/futurefin-mcp-parity/SKILL.md); the gate is enforced by
`futurefin-change-control` §1 (class "API contract"). If the answer is "tool", the `*_core`
split from Key patterns above is the prerequisite.
