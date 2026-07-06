# API Route Map

All routes in `apps/api/src/routes/mod.rs`. Routes under `/v1/` require valid session cookie `ff_session` unless noted.

## Top-level (no auth required)
| Method | Path | Handler |
|--------|------|---------|
| GET | `/health` | `health::health_check` |
| GET | `/openapi.json` | `openapi::openapi_json` |

## /v1 routes

### Health
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/health` | No auth |
| GET | `/v1/ready` | DB ping |

### Auth (`/v1/auth/`)
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/auth/register` | No prior session needed. First user auto-becomes installation owner. |
| POST | `/v1/auth/login` | Sets `ff_session` cookie |
| POST | `/v1/auth/logout` | Clears cookie + DB session |
| GET | `/v1/auth/me` | Current user info |
| PATCH | `/v1/auth/me` | Update `birth_date` |

### Installation
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/installation/session-context` | Returns `{installation_initialized, access}` — used for routing the UI gate |
| GET | `/v1/installation` | Own membership + installation snapshot |
| PATCH | `/v1/installation` | Owner only. Updates tz, inflation, show_age_mode, fire_settings (no existe `target_age` — eliminado en v1.0.6) |
| POST | `/v1/installation/setup` | Creates singleton installation (409 if exists) |

### Pending users (`/v1/installation/pending-users/`)
Owner-only management of users awaiting approval.

### Categories (`/v1/categories/`)
Scopes: `asset`, `liability`, `income`, `expense`. Per-installation.

### Assets (`/v1/assets/`)
Accepts `?view=mine` to filter by `owner_user_id`. The asset record no longer carries contribution fields — those live in `/v1/allocation-rules/`.

Each `AssetResponse` row carries `owner_user_id: Option<Uuid>` (`null`/absent = shared row). It is **display data only** (used by the frontend snapshot-prompt trigger to know which assets are "mine" in household view), never a security boundary — scoping still happens via `?view=mine`. Serialized as a uuid string, omitted when `None` (`skip_serializing_if`).

### Allocation rules (`/v1/allocation-rules/`)
Cascade rules that route the monthly surplus (`income − expense − debt_service`) into assets, in priority order. Accepts `?view=mine` to scope by `owner_user_id`.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/allocation-rules` | List ordered by `priority ASC`. |
| POST | `/v1/allocation-rules` | Body: `{target_asset_id, kind, amount?, cap_kind?, cap_value?, notes?, enabled?}`. Auto-assigns the next `priority` in the scope. |
| PATCH | `/v1/allocation-rules/{id}` | Updates fields. `amount` accepts a string, number, or `null` (only for `remainder`). `cap` accepts `{kind, value}` or `null`. Does **not** change `priority`. Rejects with 400 + `remainder_required` if it would orphan the scope. |
| DELETE | `/v1/allocation-rules/{id}` | Returns 400 `remainder_required` if deleting the last `remainder` rule in scope. |
| POST | `/v1/allocation-rules/reorder` | Body: `{ids: [uuid,...]}`. Must list exactly the rules in the current scope; reassigns `priority` 1..N in the given order in one transaction. |

Rule kinds:
- `fixed` — €/mes; `amount` required, ≥ 0.
- `percent` — `amount` ∈ [0, 100], applied to the **surplus remaining at this step** (cascade pure).
- `remainder` — `amount` ignored (must be NULL). At least one per scope is enforced server-side.

Cap kinds (all optional; `cap_kind`/`cap_value` are paired):
- `amount` — absolute € target value for the destination asset.
- `months_expense` — N × (monthly expense + debt service); evaluated per-month against current state.
- `income_multiple` — N × monthly income.

### Liabilities (`/v1/liabilities/`)
Accepts `?view=mine`. `principal_derived_from_plan` flag indicates auto-derived principal from planning flows.

**Expiration filter**: rows with `payment_end_date < today` are hidden from `GET /v1/liabilities` (and from totals/breakdowns in `/summary`, derived lines in `/budget`, debt service in `/projection`). The rows are not deleted — the filter is `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. Use `installation.calendar_tz` to compute `today`.

### Summary (`/v1/summary/`)
Aggregated net worth, financial health metrics, category breakdowns. Accepts `?view=mine`. `total_liabilities` and breakdowns exclude expired rows (see Liabilities note above).

### Budget (`/v1/budget/`)
Income/expense entries + derived lines from liabilities. Accepts `?view=mine`. Derived lines only show liabilities with `payment_end_date > today`.

### Planning (`/v1/planning/`)
Upcoming cash flows (one-off inflows/outflows) with due dates.

### Projection (`/v1/projection/`)
Net-worth series via `futurefin-engine`. Accepts `?view=mine` and `?months=N` (12–840).

Response (`ProjectionSeriesResponse`) includes:
- `points[]` — `{month_index, net_worth, contributed_capital}` for months 0..=N. **`net_worth` y `contributed_capital` se serializan como `f64`** (no Decimal-as-string) por rendimiento: ~30 KB menos en JSON y evita ~5.000 `parseDisplayDecimal` cliente. Precisión <1 € en horizontes de 70 años.
- `months`, `horizon_years`, `horizon_basis` — effective horizon (`lifespan_90` | `fallback_no_demographics` | `months_override`)
- `starting_net_worth`, `monthly_delta_assumption` — snapshot values at month 0 (Decimal-as-string para totales)
- `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date` — UI axis helpers
- `milestones[]` — next 3 net-worth milestones (1/2.5/5×10ⁿ thresholds), each with `target`, `reached_month_index`, `reached_date_ymd`
- `compound_outpaces_true_savings_month_index` — first month where compound return > base monthly savings (optional)
- `fire_target_series: f64[]`, `asset_series[].values: f64[]` — arrays grandes paralelos a `points` (también `f64`).

> La misma excepción f64 cubre los arrays por punto de `GET /v1/history/series` (`points[].net_worth/assets_total/liabilities_total`, `asset_series[].values`, `markers[].total`) — misma justificación chart-only. Hay UNA sola definición de `serialize_decimal_as_f64` (`pub(crate)`, en `handlers/projection.rs`), usada solo por projection e history.

**Cache server-side**: `AppState` mantiene un cache in-memory por `(installation_id, view, owner_user_id)` con sliding TTL de 60 min. Hits sub-ms; el GET sin cache hace el cómputo full (~500 ms). Invalidación automática: cualquier mutación en assets/liabilities/budget/planning/allocation/installation/user.birth_date llama `state.invalidate_projection_by_installation(iid)`. Logout llama `state.invalidate_projection_by_user(user_id)` (solo `view=mine`). Warm-up: `tokio::spawn` tras `POST /v1/auth/login` recomputa `view=household` para que el primer GET sea hit. Sin warm-up tras mutación (evita race condition de warm-ups concurrentes).

**Compresión**: todos los endpoints pasan por `tower_http::compression::CompressionLayer::new().gzip(true)`. `/v1/projection/series` baja de ~260 KB a ~30 KB con `Content-Encoding: gzip`.

**Densidad (`?density=hybrid`)**: con `?density=hybrid` el response decima los arrays grandes (`points`, `fire_target_series`, `asset_series[].values`) a un patrón mixto — mes 0..12 mensual + mes 24, 36, ..., months. Total ~82 puntos en lugar de ~841. JSON ~5 KB. El compute interno del engine es idéntico (840 meses); solo cambia la serialización. Cada densidad tiene su propia entry en el cache (`ProjectionCacheKey.density`). Milestones, FIRE crossover y compound marker se calculan sobre el array full (no decimado) para no perder precisión. El campo `density: "monthly" | "hybrid"` viaja en el response para que el cliente sepa qué tiene.

**Two-phase loading en el cliente**: `App.tsx` dispara `?density=hybrid` y `?density=monthly` en paralelo. El hybrid suele llegar primero (JSON más pequeño) → se renderiza el chart con menos puntos. Cuando llega el monthly, se reemplaza dentro de `startTransition()` (sin bloquear inputs). Si ambos son cache hit, ambos llegan en <10 ms → el hybrid no añade latencia perceptible.

### Backup
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/backup/user-export` | Returns `.ffbackup` binary for the **current user only**. Body: `{password, ui_preferences?}`. Encrypted with the user's account password (Argon2id KDF → AES-256-GCM). Any role. |
| POST | `/v1/backup/user-import/preview` | Body: `{file_b64, password}`. Returns counts of what would be imported. Write role required. |
| POST | `/v1/backup/user-import` | Body: `{file_b64, password, confirm_replace: true}`. **Destructive**: replaces all `owner_user_id = current_user` rows in `assets/liabilities/budget_entries/planning_flows` in a transaction. Write role required. |

The `.ffbackup` format is a versioned, encrypted binary container — see [`backup_user/crypto.rs`](../apps/api/src/handlers/backup_user/crypto.rs) for the frame layout and [`backup_user/schema.rs`](../apps/api/src/handlers/backup_user/schema.rs) for the payload schema + migration layer (`schema_version`).

### History snapshots (`/v1/history/snapshots/`)
Snapshots manuales, **per-user**, del patrimonio en un día civil (`installation.calendar_tz`), de los que el servidor reconstruye la serie histórica de net worth. Dos `kind` independientes: `asset` | `liability` (singular, como el CHECK de DB). Siempre **own-data** (`owner_user_id = usuario`); estos endpoints **no** aceptan `?view=mine` (no aplican los helpers `LedgerView`). CRUD Decimal-as-string; `total` = Σ items calculado en Rust (nunca almacenado). Auth: cualquier miembro puede leer; mutaciones requieren `role_can_write` (owner/member) o `403`.

| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/history/snapshots/capture` | Body `{kinds?: ["asset","liability"]}` (omitido → ambos; `[]` → 400 `kinds_empty`; valor desconocido → 400 `invalid_kind`). Por kind, en una transacción: fecha = hoy civil; **upsert** de cabecera (`ON CONFLICT ... DO UPDATE SET source='capture', updated_at=now()`) → la captura del mismo día sobrescribe silenciosamente; borra items y los recopia del **ledger propio** (assets: `id→source_item_id`, `name→label`, `current_value→value`, sin términos; liabilities no expiradas: además copia `apr_percent`/`payment_amount`/`payment_frequency`). Filas compartidas (`owner_user_id IS NULL`) excluidas por construcción; 0 filas propias → snapshot válido con 0 items. Respuesta **200** `{snapshots:[SnapshotResponse]}`. **No** invalida la cache de proyección (los snapshots no son inputs del engine). |
| GET | `/v1/history/snapshots?year=YYYY&kind=` | Siempre own-user. `year` opcional (1900..=3000) filtrado como **rango de fechas** (`>= YYYY-01-01 AND < (YYYY+1)-01-01`); `kind` opcional. Orden `snapshot_date DESC, kind ASC`; items incluidos, orden `label ASC`. Solo lectura (nunca muta). → **200** `[SnapshotResponse]` (array plano, como los demás GET de listado). |
| POST | `/v1/history/snapshots` | Backfill. Body `{kind, snapshot_date, items:[{item_id?, label, value, apr_percent?, payment_amount?, payment_frequency?}]}`. Códigos 400 estables: `snapshot_date_in_future`, `snapshot_date_too_old` (<1900-01-01), `too_many_items` (>500), `duplicate_item_id`, `terms_only_for_liabilities`; bounds de `value`/términos copiados de assets/liabilities. `item_id` ausente → UUID de servidor (devuelto). Fecha (usuario,kind,fecha) ocupada → **409**. `source='backfill'`. → **201** `SnapshotResponse`. |
| PUT | `/v1/history/snapshots/{id}` | Body `{snapshot_date?, items?}` — `items` omitido → conserva los items (solo actualiza cabecera/fecha); `items` presente (incluso `[]`) → reemplazo completo. `kind` inmutable. Guardia `id + installation + owner` → **404** si no es tuyo (no revela existencia). Mover a fecha ocupada → **409**. `source` intacto, `updated_at=now()`. → **200** `SnapshotResponse`. |
| DELETE | `/v1/history/snapshots/{id}` | **204**; misma guardia 404; items en cascada. |

`SnapshotResponse`: `{id, kind, snapshot_date_ymd, source, total (Decimal-string), items:[{item_id (=source_item_id), label, value, apr_percent?, payment_amount?, payment_frequency?}] orden label ASC, created_at, updated_at}`.

### History series (`GET /v1/history/series`)
Serie histórica de net worth **interpolada server-side** desde los snapshots (el cliente no interpola). Solo lectura, cualquier miembro (viewer incluido). Acepta `?view=mine` vía helpers `LedgerView`: household = TODOS los snapshots de la instalación (todos tienen `owner_user_id NOT NULL`), agregados per-user en Rust; mine = solo los del usuario. Sin `?density`, sin cache y sin `spawn_blocking` — deliberado: el cómputo es sub-ms (decenas de snapshots × decenas de meses).

Response (`HistorySeriesResponse`) — los numéricos por punto en **f64** (ver nota en §Projection):
- `anchor_date_ymd` (hoy civil de la instalación), `anchor_month_first_ymd` (fecha del punto `month_index = 0`), `view` (`household` | `mine`)
- `points[]` — `{month_index: i32 ≤ 0 (contiguos k_min..=0, incluye el mes 0), net_worth, assets_total, liabilities_total}`; `net_worth = A − L`
- `asset_series[]` — `{asset_id (= source_item_id), asset_name, values: f64[] paralelo a points}`. Agrupado por `source_item_id` **entre usuarios** (valores sumados); nombre = el asset vivo si el id coincide, si no el label del snapshot más reciente que lo contiene; orden `asset_name ASC, asset_id ASC`. Solo los assets tienen serie por item (paridad con projection).
- `markers[]` — uno por snapshot en scope: `{date_ymd, month_index, month_fraction = month_index + (día−1)/días_del_mes, kind, owner_user_id, total (Σ items)}`
- 0 snapshots en scope → **200** con los tres arrays vacíos.

Algoritmo: ancla = primero-de-mes del hoy civil; timelines por `(owner_user_id, kind)` (fechas ascendentes + vectores de observación paralelos por `source_item_id`); a cada timeline se le añade la observación virtual «hoy» con las filas vivas del owner (assets y liabilities no expiradas del scope, ambas con conjunto extra `owner_user_id IS NOT NULL` — las filas compartidas nunca participan), salvo que el último snapshot real sea de hoy. La interpolación vive en `crates/engine/src/history.rs` (`evaluate_timeline`): assets lineal en días civiles, liabilities amortización francesa corregida por residuo (exacta en ambos extremos; cuota `weekly → ×52/12`). Usuarios sin snapshots de un kind no tienen timeline → no aportan (household = suma de los usuarios que snapshotean). Como todo GET: nunca muta.

## Auth pattern in handlers

Every protected handler calls:
```rust
let user = require_session_user(&jar, &state.pool).await?;
let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
// For write ops:
if !role_can_write(role.as_str()) { return Err(ApiError::Forbidden); }
```

## View-scoping pattern

For any endpoint that accepts `?view=mine`, **do not** write two `match view { Household => sqlx::query_as("…installation_id = $1…"), Mine => sqlx::query_as("…installation_id = $1 AND owner_user_id = $2…") }` branches. Use the helpers in `handlers/person_view.rs`:

```rust
let view = q.resolve(); // Query<LedgerViewQuery>
let scope = view.scope_where("a"); // table alias optional; "" = no prefix
let today_ph = view.next_arg_index(); // 2 (Household) or 3 (Mine)
let sql = format!(
    "SELECT ... FROM assets a WHERE {scope} AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph}) ORDER BY ...",
);
let rows: Vec<MyRow> = view
    .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
    .bind(today)
    .fetch_all(pool)
    .await?;
```

For `sqlx::query_scalar`, use `bind_scope_scalar` instead. The helpers guarantee placeholder order ($1=iid, optional $2=owner_user_id) so the two branches can never drift.

## Error mapping

`impl From<sqlx::Error> for ApiError` (in `error.rs`) auto-detects:
- `23505` (unique_violation) → `ApiError::Conflict` (409)
- `23503` (foreign_key_violation) → `ApiError::BadRequest("referenced record missing")`

Handlers should just `?` any `sqlx::Error`; never write per-call `.map_err(...)` to translate codes.
