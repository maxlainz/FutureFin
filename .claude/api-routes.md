# API Route Map

All routes in `apps/api/src/routes/mod.rs`. Routes under `/v1/` require valid session cookie `ff_session` unless noted.

## Top-level
| Method | Path | Handler | Auth |
|--------|------|---------|------|
| GET | `/health` | `health::health_check` | none |
| GET | `/openapi.json` | `openapi::openapi_json` | none |
| POST/GET/DELETE | `/mcp` | `mcp::mcp_router` (rmcp `StreamableHttpService`) | Bearer `ffp_…` (api_tokens) **o** `ffo_…` (OAuth) — ver sección MCP abajo |
| GET | `/.well-known/oauth-protected-resource` | `oauth::metadata::protected_resource` | none |
| GET | `/.well-known/oauth-protected-resource/mcp` | `oauth::metadata::protected_resource` (mismo handler; sufijo de path RFC 9728 §3.1) | none |
| GET | `/.well-known/oauth-authorization-server` | `oauth::metadata::authorization_server` | none |
| GET | `/.well-known/oauth-authorization-server/mcp` | `oauth::metadata::authorization_server` (mismo handler; sufijo RFC 8414 §3.1) | none |
| POST | `/oauth/register` | `oauth::register::register_client` (DCR, RFC 7591) | **ninguna — registro público** |
| POST | `/oauth/token` | `oauth::token::token` (`authorization_code`+PKCE / `refresh_token`) | client auth (`none` \| `client_secret_basic` \| `client_secret_post`) |
| POST | `/oauth/revoke` | `oauth::token::revoke` (RFC 7009) | client auth (idem) |

> `GET /oauth/authorize` **no tiene ruta backend**: la sirve el fallback SPA de `main.rs`. Ver la
> sección OAuth abajo — registrarla es un error que rompe la pantalla de consentimiento.

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
| PATCH | `/v1/installation` | Owner only. Updates tz, inflation, show_age_mode, fire_settings (incluye `savings_source: "budget" \| "transactions_avg" \| "budget_income_real_expense"` — fuente del ahorro de la simulación; no existe `target_age` — eliminado en v1.0.6) |
| POST | `/v1/installation/setup` | Creates singleton installation (409 if exists) |

### Pending users (`/v1/installation/pending-users/`)
Owner-only management of users awaiting approval.

### API tokens (`/v1/api-tokens`) — handler `api_tokens.rs`
Credencial Bearer del servidor MCP (`/mcp`). Gestión autenticada por cookie de sesión; cualquier
miembro (viewer incluido) gestiona SOLO los suyos — el token hereda identidad y rol vivo, no puede
hacer nada que su dueño no pueda ya.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/api-tokens` | Lista propios (`token_prefix`, nunca el secreto ni el hash), orden `created_at DESC`. Incluye revocados (auditoría). |
| POST | `/v1/api-tokens` | Body `{label (1..64), expires_in_days? (1..=3650)}` → 201 con `token` (secreto `ffp_` + 43 chars base64url) **una única vez**. Máx. 10 activos por usuario → 400 `token_limit_reached`. |
| DELETE | `/v1/api-tokens/{id}` | Soft-revoke (`revoked_at = now()`). Id ajeno o ya revocado → 404. |

- **Solo se persiste el SHA-256 hex** del secreto (`token_hash` UNIQUE); lookup O(1), sin
  comparación de secretos en Rust.
- `require_api_token(pool, authorization)` (mismo archivo) valida `Bearer ffp_…` → 401 para todo
  fallo (ausente/malformado/revocado/expirado, sin distinguir). Actualiza `last_used_at` con
  throttle de 60 s (telemetría de auth, análoga a `sessions` — no viola reads-never-mutate).

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

**Base de los caps en `/v1/assets` (v2.2.0)**: el `contribution_target_amount` que devuelven `GET/POST/PATCH /v1/assets` resuelve `months_expense` / `income_multiple` con los escalares **efectivos** del engine — o sea, en modo B/C con datos el gasto/income salen del promedio real 12m, no del presupuesto (antes se resolvían siempre con presupuesto y el objetivo no casaba ni con la aportación del mes 1 mostrada en la misma respuesta ni con la proyección). Un único `assets_projection_context` (`handlers/projection.rs`) devuelve `{nominals, income_monthly, expense_with_debt}` de **un solo** `build_installation_projection_input` por request; sustituye a `first_month_asset_contribution_nominals_map` + `monthly_income_expense_debt_for_view` (eliminados).

### Liabilities (`/v1/liabilities/`)
Accepts `?view=mine`. `principal_derived_from_plan` flag indicates auto-derived principal from planning flows.

**Expiration filter**: rows with `payment_end_date < today` are hidden from `GET /v1/liabilities` (and from totals/breakdowns in `/summary`, derived lines in `/budget`, debt service in `/projection`). The rows are not deleted — the filter is `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. Use `installation.calendar_tz` to compute `today`.

### Summary (`/v1/summary/`)
Aggregated net worth, financial health metrics, category breakdowns. Accepts `?view=mine`. `total_liabilities` and breakdowns exclude expired rows (see Liabilities note above).

**`financial_health` sigue el toggle `fire_settings.savings_source`** (3 modos; gate `SavingsSource::uses_transactions()` = B o C). Con datos:
- **Modo B (`transactions_avg`)**: `income_monthly_equivalent`, `expense_regular_monthly_equivalent`, `net_monthly_equivalent` (= income_eff − expense_eff − Σ cuotas nominales activas) y `savings_rate` salen del promedio real 12m con resta híbrida de cuotas (mismo helper `effective_avg_income_expense` que la proyección), no del presupuesto.
- **Modo C (`budget_income_real_expense`)**: igual que B pero `income_monthly_equivalent` **conserva el income del presupuesto** (NO se sobreescribe); `expense_regular_monthly_equivalent = expense_eff` y `net_monthly_equivalent = income (presupuesto) − expense_eff − debt_service`. El `match` sobre `savings_source` es exhaustivo (`Budget` es rama inalcanzable no-op, guardada por `uses_transactions()`).
- **Base de gasto derivada/total (v2.2.0)**: en B/C con datos, `expense_derived_monthly_equivalent` = **servicio de deuda** de los pasivos activos (`payment_end_date IS NULL OR >= today`, cuota normalizada a mensual) y `expense_total_monthly_equivalent` = `expense_eff + debt_service`. Antes se dejaban los valores del presupuesto, lo que rompía en B/C las dos identidades que en modo A siempre valen y que ahora se cumplen en los **tres** modos:
  - `expense_total_monthly_equivalent = expense_regular_monthly_equivalent + expense_derived_monthly_equivalent`
  - `net_monthly_equivalent = income_monthly_equivalent − expense_total_monthly_equivalent`
- **Fallback**: `months_with_data == 0` en B/C → el bloque `financial_health` completo es **idéntico** al de modo A (runway incluido).

Campos de `financial_health` relacionados con el modo y el runway:
- `savings_source` (`"budget" | "transactions_avg" | "budget_income_real_expense"`) — modo **efectivo** tras el fallback (B o C con `months_with_data == 0` → devuelve `"budget"`).
- `savings_source_months_with_data` (`u32`) — meses **reales** del promedio (ver §Transactions); `0` en modo A y en el fallback.
- `runway_months` (Decimal-string, opcional) — meses que los activos **líquidos** cubren `expense_total_monthly_equivalent`, **componiendo** la rentabilidad esperada de esos líquidos (media ponderada por valor de los multiplicadores mensuales) y la inflación del gasto (`installation.annual_inflation_assumption_percent`, clampada a ≥ 0). Lo calcula `futurefin_engine::liquid_runway_months` (ver [`engine.md`](engine.md) §Runway). **No** es `liquid_assets_total / expense_total`, salvo que rentabilidad e inflación sean 0 (y el umbral SWR no se cumpla), caso en el que se reduce exactamente a esa división. Como sigue `expense_total`, en B/C se calcula sobre la base de gasto real. Se **omite** del JSON (`skip_serializing_if`) cuando es `None`: sin base de gasto (`expense_total == 0`) o runway indefinido. El valor `1200` (`MAX_RUNWAY_MONTHS`) **no** es un centinela de infinito sino un **suelo**: significa «al menos 100 años» (el bucle agotó el tope sin cumplir el umbral SWR) y la UI lo pinta «+100 años».
- `runway_is_indefinite` (`bool`) — desde **v2.3.0** lo decide el **umbral SWR**, no sobrevivir el cap: `true` ⟺ la retirada anual bruta no supera el SWR sobre el saldo líquido, es decir `gross_up(expense_total × 12) × 100 ≤ liquid_assets_total × swr_pct`, con `swr_pct`/`tax_brackets`/`taxes_enabled` de `installation.fire_settings` (pestaña Jubilación) y el **mismo** `gross_up_net_annual_fire` del target FIRE. Entonces `runway_months` no viaja. Con `swr_pct ≤ 0` nunca es `true`. Con `expense_total == 0` es `false` (no hay base de gasto, no es que esté cubierto). El disparador es deliberadamente independiente de rentabilidad e inflación (que gobiernan solo el caso finito). La UI muestra «Infinito (dentro del SWR 3,5 %)» en el primer caso y oculta la tarjeta en el segundo. **API no breaking**: tipo y nullabilidad de ambos campos son los de v2.2.0.

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
- `savings_source` + `savings_source_months_with_data` (v2.2.0) — fuente del ahorro **efectiva** (tras el fallback) que produjo `monthly_delta_assumption`, y los meses reales del promedio; mismo naming y semántica que en `/v1/summary`. Aditivos: los sirve `BuiltProjection` sin queries extra, para que el chart etiquete la base del Δ mensual sin pedir `/v1/summary`.

> La misma excepción f64 cubre los arrays por punto de `GET /v1/history/series` (`points[].net_worth/assets_total/liabilities_total`, `asset_series[].values`, `markers[].total`) — misma justificación chart-only. Hay UNA sola definición de `serialize_decimal_as_f64` (`pub(crate)`, en `handlers/projection.rs`), usada solo por projection e history.

**Cache server-side**: `AppState` mantiene un cache in-memory por `(installation_id, view, owner_user_id)` con sliding TTL de 60 min. Hits sub-ms; el GET sin cache hace el cómputo full (~500 ms). Invalidación automática: cualquier mutación en assets/liabilities/budget/planning/allocation/installation/user.birth_date llama `state.invalidate_projection_by_installation(iid)`; las mutaciones de **transactions** invalidan **solo en los modos que usan transacciones** (`fire_settings.savings_source ∈ {transactions_avg, budget_income_real_expense}`, i.e. `SavingsSource::uses_transactions()`; ver sección Transactions). Logout llama `state.invalidate_projection_by_user(user_id)` (solo `view=mine`). Warm-up: `tokio::spawn` tras `POST /v1/auth/login` recomputa `view=household` para que el primer GET sea hit. Sin warm-up tras mutación (evita race condition de warm-ups concurrentes).

**Compresión**: todos los endpoints pasan por `tower_http::compression::CompressionLayer::new().gzip(true)`. `/v1/projection/series` baja de ~260 KB a ~30 KB con `Content-Encoding: gzip`.

**Densidad (`?density=hybrid`)**: con `?density=hybrid` el response decima los arrays grandes (`points`, `fire_target_series`, `asset_series[].values`) a un patrón mixto — mes 0..12 mensual + mes 24, 36, ..., months. Total ~82 puntos en lugar de ~841. JSON ~5 KB. El compute interno del engine es idéntico (840 meses); solo cambia la serialización. Cada densidad tiene su propia entry en el cache (`ProjectionCacheKey.density`). Milestones, FIRE crossover y compound marker se calculan sobre el array full (no decimado) para no perder precisión. El campo `density: "monthly" | "hybrid"` viaja en el response para que el cliente sepa qué tiene.

**Two-phase loading en el cliente**: `App.tsx` dispara `?density=hybrid` y `?density=monthly` en paralelo. El hybrid suele llegar primero (JSON más pequeño) → se renderiza el chart con menos puntos. Cuando llega el monthly, se reemplaza dentro de `startTransition()` (sin bloquear inputs). Si ambos son cache hit, ambos llegan en <10 ms → el hybrid no añade latencia perceptible.

### Backup
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/backup/user-export` | Returns `.ffbackup` binary for the **current user only**. Body: `{password, ui_preferences?}`. Encrypted with the user's account password (Argon2id KDF → AES-256-GCM). Any role. |
| POST | `/v1/backup/user-import/preview` | Body: `{file_b64, password}`. Returns counts of what would be imported. Write role required. |
| POST | `/v1/backup/user-import` | Body: `{file_b64, password, confirm_replace: true}`. **Destructive**: replaces **all** `owner_user_id = current_user` user-scoped rows (`assets/liabilities/budget_entries/planning_flows/allocation_rules/history_snapshots/transactions/transaction_imports/categorization_rules/recurring_transaction_rules`) in a single transaction, then invalidates the projection cache. Write role required. Table order + re-link details: [`data-model.md`](data-model.md) §Per-user `.ffbackup`. |

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
| GET | `/v1/history/snapshots/prefill?kind=&date=` | Pre-rellena el panel de backfill. **Siempre own-user** (sin `?view`); solo lectura (viewer incluido, nunca muta). `kind` requerido ∈ {asset, liability} (ausente/inválido → 400 `invalid_kind`); `date` requerida `YYYY-MM-DD` (ausente → 400; futura → `snapshot_date_in_future`; <1900-01-01 → `snapshot_date_too_old`). Reconstruye el MISMO timeline own-user de `/history/series` (snapshots del kind + obs virtual «hoy» de las filas vivas no expiradas salvo que el último snapshot real sea hoy) y evalúa cada item en `date`. Sin timeline (0 snapshots del kind) → universo = filas vivas, `basis="live"`, valor = current_value/principal. Con timeline: antes del primer snapshot → valor del primer snapshot (`basis="first_snapshot"`) para los items presentes allí, resto `not_owned`; dentro de `[first,last]` interpola vía el motor (`amortized_segment_value`; activos lineal en días, pasivos amortización francesa) → `basis="interpolated"`; item sin obs ≤ `date` (posterior) o cuya última obs precede `date` y ausente después (borrado/vendido) → `value 0, existed false, basis="not_owned"`. Términos (pasivos) desde la obs de inicio de segmento, si no la obs con términos más cercana, si no la fila viva. Orden: `existed=true` primero (`label ASC`), luego `not_owned` (`label ASC`). Universo vacío → **200** items `[]`. → **200** `PrefillResponse`. |

`SnapshotResponse`: `{id, kind, snapshot_date_ymd, source, total (Decimal-string), items:[{item_id (=source_item_id), label, value, apr_percent?, payment_amount?, payment_frequency?}] orden label ASC, created_at, updated_at}`.

`PrefillResponse`: `{date_ymd, kind, items:[{item_id (=source_item_id), label, value, existed, basis, apr_percent?, payment_amount?, payment_frequency?}]}`. `value` es Decimal-string **redondeado a 2 decimales** (sugerencia display-grade que el usuario edita); los términos se ecoan sin redondear (son observaciones copiadas, no computadas); `existed` es bool; `basis` es string ∈ {`interpolated`, `first_snapshot`, `live`, `not_owned`}.

### History series (`GET /v1/history/series`)
Serie histórica de net worth **interpolada server-side** desde los snapshots (el cliente no interpola). Solo lectura, cualquier miembro (viewer incluido). Acepta `?view=mine` vía helpers `LedgerView`: household = TODOS los snapshots de la instalación (todos tienen `owner_user_id NOT NULL`), agregados per-user en Rust; mine = solo los del usuario. Sin `?density`, sin cache y sin `spawn_blocking` — deliberado: el cómputo es sub-ms (decenas de snapshots × decenas de meses).

Response (`HistorySeriesResponse`) — los numéricos por punto en **f64** (ver nota en §Projection):
- `anchor_date_ymd` (hoy civil de la instalación), `anchor_month_first_ymd` (fecha del punto `month_index = 0`), `view` (`household` | `mine`)
- `points[]` — `{month_index: i32 ≤ 0 (contiguos k_min..=0, incluye el mes 0), net_worth, assets_total, liabilities_total}`; `net_worth = A − L`
- `asset_series[]` — `{asset_id (= source_item_id), asset_name, values: f64[] paralelo a points}`. Agrupado por `source_item_id` **entre usuarios** (valores sumados); nombre = el asset vivo si el id coincide, si no el label del snapshot más reciente que lo contiene; orden `asset_name ASC, asset_id ASC`. Solo los assets tienen serie por item (paridad con projection).
- `markers[]` — uno por snapshot en scope: `{date_ymd, month_index, month_fraction = month_index + (día−1)/días_del_mes, kind, owner_user_id, total (Σ items)}`
- 0 snapshots en scope → **200** con los tres arrays vacíos.

Algoritmo: ancla = primero-de-mes del hoy civil; timelines por `(owner_user_id, kind)` (fechas ascendentes + vectores de observación paralelos por `source_item_id`); a cada timeline se le añade la observación virtual «hoy» con las filas vivas del owner (assets y liabilities no expiradas del scope, ambas con conjunto extra `owner_user_id IS NOT NULL` — las filas compartidas nunca participan), salvo que el último snapshot real sea de hoy. La interpolación vive en `crates/engine/src/history.rs` (`evaluate_timeline`): assets lineal en días civiles, liabilities amortización francesa corregida por residuo (exacta en ambos extremos; cuota `weekly → ×52/12`). Usuarios sin snapshots de un kind no tienen timeline → no aportan (household = suma de los usuarios que snapshotean). Como todo GET: nunca muta.

### History cash-flow (`GET /v1/history/cashflow`) — v1.6.0
Cash-flow histórico de las transacciones (tier-2 sobre los snapshots). Solo lectura, cualquier miembro. Acepta `?view=mine` vía `LedgerView`. **Nunca invalida la cache de proyección** (las transacciones no son inputs del engine). Sin cache; `spawn_blocking` solo en `resolution=daily`. Dos capas independientes en el mismo response:
1. **`months[]`** — agregado mensual **firmado** por kind, **Decimal-string** (son KPIs, escala 2dp). Solo un `GROUP BY (mes, kind)` sobre la ventana, independiente de los snapshots: `expense`/`savings` conservan su signo real (≤0), `income` ≥0, `net = expense + income + savings`. Contiguo `-window_months..=0` (incluye el mes 0 en curso), ascendente.
2. **`fine`** (opcional) — la **curva fina** de patrimonio (`weekly`/`daily`) donde los deltas de cash-flow **moldean** los assets vinculados sin contradecir los snapshots (curva anclada, `crates/engine`). Presente **solo** si hay transacciones vinculadas a algún asset **y** snapshots que anclar (si no, campo ausente). Patas de cash-flow: pata cuenta (batch con `account_asset_id` → `delta = +amount`) y pata destino de ahorro (`kind='savings'` con `linked_asset_id` → `delta = −amount`); una savings importada aparece en ambas (partida doble). `fine` = `{resolution, grid:[{date_ymd, month_index, month_fraction}], asset_series:[{asset_id, asset_name, values: f64[]}], net_worth: f64[]}`, todo paralelo a `grid`; la rejilla fina termina **exacta en hoy** (empalma con el vivo). `month_fraction` es el mismo helper que los `markers[]` de `/history/series` (fuente única → la escala mes→px no puede divergir).

Params: `view` (`mine` | omitido → household); `window_months` (i64, default 24, clamp 1..=120); `resolution` (`weekly` default | `daily`). **Gating de daily**: `resolution=daily` exige `window_months <= 6` → si no, **400** `daily_window_too_large`. Response `CashflowResponse {anchor_date_ymd, anchor_month_first_ymd, view, months, fine?}`; numéricos de `fine` en **f64** (misma excepción chart-only que projection/history-series), `months[]` en Decimal-string.

### Transactions (`/v1/transactions/`) — v1.6.0
Histórico de gasto mensual **per-user**: import de CSV bancario (MyInvestor/N26) o efectivo a mano, categorización con reglas aprendidas, y comparativa mes real vs presupuesto vs promedio. Decimal-as-string (importes firmados: negativo = cargo). **Invalidación de la cache de proyección condicionada al modo** (`fire_settings.savings_source`): en modo A (`budget`, default) las transacciones **no son inputs del engine** → ninguna mutación invalida; en los modos que usan transacciones (B `transactions_avg` y C `budget_income_real_expense` → `SavingsSource::uses_transactions()`) el ahorro de la simulación deriva del promedio real 12m → las mutaciones que cambian el conjunto (create/batch/patch/delete, delete import, import confirm, `recurring/materialize`) invalidan la cache vía `invalidate_projection_if_savings_uses_transactions` (best-effort post-commit, jamás convierte una mutación exitosa en 5xx). `rules.rs`, previews y el borrado de una regla recurrente nunca invalidan. Regresión (A/B/C + flip): `transactions_projection_cache.rs`.

**Promedio 12m que alimenta el engine** (`transactions_12m_avg`, distinto del summary de Movimientos): ventana `[first-of-month(today) − 12m, first-of-month(today))`. El denominador `months_with_data` y las sumas por kind/liability cuentan solo **meses reales** — meses del tramo con ≥1 transacción `recurring_rule_id IS NULL`. Un mes vacío o «pseudovacío» (solo instancias recurrentes materializadas, p. ej. tras un backfill) se excluye **por completo** (ni numerador ni denominador); un mes real cuenta entero, incluidas sus recurrentes. El `GET /v1/transactions/summary` de la pestaña Movimientos **NO** aplica este filtro (cuenta cualquier mes con datos) — divergencia deliberada. Lecturas: cualquier miembro (`?view=mine` vía `LedgerView` en los GET de listado/comparativa/imports; las **reglas** son siempre own-user, sin `?view`); escrituras siempre `owner_user_id = usuario` y exigen `role_can_write` o **403**. Import limit 16 MiB (`BACKUP_IMPORT_BODY_LIMIT_BYTES`, reutilizado). Códigos 400 estables entre comillas.

| Method | Path | Rol | Notas |
|--------|------|-----|-------|
| GET | `/v1/transactions?view=&month=&kind=&category_id=&import_id=` | lectura | Listado, orden `op_date DESC`. `month` = `YYYY-MM` (inválido → 400). → **200** `[TransactionResponse]`. |
| POST | `/v1/transactions` | write | Alta manual (efectivo, `import_id NULL`, `source='manual'`). Body `{op_date, value_date?, concept, amount, kind, category_id?, linked_asset_id?, linked_liability_id?, notes?, recurrence?}`. **`recurrence: {day_of_month?}`** (opcional): crea además una regla recurrente-plantilla y deja esta transacción enlazada como instancia de origen (`recurring_rule_id`); `day_of_month` omitido → el día de `op_date`, fuera de 1..31 → 400 `recurrence_day_out_of_range`. **Un alta con `op_date` pasada backfillea todas las instancias intermedias hasta hoy en el MISMO commit** (ya no depende de una llamada posterior a `/materialize`); `op_date` a más de 10 años atrás → **422** `recurrence_too_old`. 400: `invalid_kind`, `amount_zero`, `savings_no_category`, `category_scope_mismatch`, `linked_asset_not_found`, `linked_liability_not_found`. Huella duplicada → **409**. → **201** `TransactionResponse` (incluye `recurring_rule_id?`). |
| POST | `/v1/transactions/batch` | write | Alta manual multifila (1..=1000). Body `{transactions:[CreateTransactionBody]}`. Cada item acepta `recurrence` (misma semántica que el alta simple, backfill de meses intermedios incluido; item con `op_date` a >10 años → **422** `recurrence_too_old`). Ordinal de huella se avanza dentro del batch. → **201** `[TransactionResponse]`. |
| GET | `/v1/transactions/months?view=` | lectura | Meses con datos (`GROUP BY YYYY-MM`), orden DESC; `is_complete=false` para el mes en curso. → **200** `[MonthEntry]`. |
| GET | `/v1/transactions/summary?view=&year=&month=&avg_window=&avg_months=` | lectura | Comparativa del mes (default: último mes **completo**). **Ventana del promedio** con `avg_window` ∈ {`3`,`6`,`12`,`ytd`,`all`} (default `6`; trim + case-insensitive; inválido → 400 `avg_window must be one of 3, 6, 12, ytd, all`); `avg_months` (1..24) es **alias legado** y `avg_window` gana si vienen ambos. Promedio **ponderado**: denominador = `months_with_data` (meses del tramo `[window_start, selected)` con ≥1 transacción del scope, **no** el nº de meses del tramo) → un historial corto ya no diluye la media a 0. YTD = meses del año del mes seleccionado estrictamente anteriores (enero → tramo vacío); ALL = desde el mes del primer movimiento. Magnitudes ≥0 para comparar con budget (gasto = `−Σ`, ingreso = `+Σ`, ahorro = `−Σ`). **Sin `derived_debt_line`** (línea "Cuotas de pasivos" eliminada): `totals.expense_budget` = Σ budget de las categorías de gasto; las cuotas reales viven ya en su categoría de gasto ordinaria (sin doble conteo). Response añade `avg_window: string`, `window_months: u32`, `months_with_data: u32` y ya **no** trae `avg_months` ni `derived_debt_line`. 400: `year`/`month` fuera de rango o desapareados, `avg_window`/`avg_months` inválidos. → **200** `TransactionsSummaryResponse`. |
| POST | `/v1/transactions/import/preview` | write | **Stateless**, sin escrituras. Body `{source (auto\|myinvestor\|n26), file_b64, account_asset_id?}`. Autodetección por cabecera; dedup por huella (estado `new`/`already_imported`), heurísticas de transferencia y savings, matching de reglas. Devuelve `file_sha256` (a reenviar en confirm). 400: `csv_preset_unrecognized`, `csv_date_invalid`, `csv_amount_invalid`, base64 inválido. → **200** `ImportPreviewResponse`. |
| POST | `/v1/transactions/import/confirm` | write | Aplica el import. Body `{source, file_b64, file_sha256, decisions:[ImportDecision] (paralelo por índice a las filas), learn_rules=true, account_asset_id?, original_filename?}`. `file_sha256`/nº de filas deben coincidir con el preview → si no, 400 `preview_confirm_mismatch`. `decision.discard`/`force` por fila; solo EUR (`currency_not_eur`). `learn_rules` hace upsert de una regla por decisión categorizada. Lote vacío → cabecera borrada, `import_id: null`. Doble-confirm concurrente → **409**. → **200** `ImportConfirmResponse {import_id?, imported, skipped_already_imported, discarded, rules_learned}`. |
| GET | `/v1/transactions/imports?view=` | lectura | Lotes de import (orden `created_at DESC`), con `txn_count` y nombre de cuenta origen. → **200** `[ImportBatchResponse]`. |
| DELETE | `/v1/transactions/imports/{id}?confirm=true` | write | Deshace un import (transacciones en cascada). `confirm` debe ser `true` → si no, 400 `confirm_required`. Guardia id+installation+owner → **404** si no es tuyo. → **204**. |
| PATCH | `/v1/transactions/{id}` | write | Edita una transacción (guardia owner → **404**). `op_date`/`amount`/`concept` son **editables en manuales e importadas** (ya no hay `immutable_field`). La diferencia está en la huella de dedup: en **manuales** se recomputa al cambiarlos (tomando un ordinal libre, liberando el anterior); en **importadas** queda **anclada** a la del CSV original y nunca se recomputa → un re-import del mismo archivo sigue detectando el duplicado pese a la edición. Campos `clear_*` para borrar opcionales. Huella duplicada tras recomputar (solo manuales) → **409**. → **200** `TransactionResponse`. |
| DELETE | `/v1/transactions/{id}` | write | Borra (guardia owner → **404**). → **204**. |
| GET | `/v1/transactions/rules` | lectura | Reglas de categorización del usuario (orden `updated_at DESC`). → **200** `[RuleResponse]`. |
| POST | `/v1/transactions/rules` | write | Crea regla. Body `{match_kind? (substring\|prefix\|exact), pattern, source?, assign_kind (requerido), assign_category_id?}`. `(source, pattern)` duplicado → **409**. → **201** `RuleResponse`. |
| PATCH | `/v1/transactions/rules/{id}` | write | Edita (guardia owner → **404**). `clear_source`/`clear_assign_kind`/`clear_assign_category`. Colisión `(source, pattern)` → **409**. → **200** `RuleResponse`. |
| DELETE | `/v1/transactions/rules/{id}` | write | Borra (guardia owner → **404**). → **204**. |
| GET | `/v1/transactions/recurring` | lectura | Reglas recurrentes del usuario (**plantillas**), orden `created_at DESC`. **Siempre own-user** (sin `?view`), como las reglas de categorización. Cada regla trae `category_name`. → **200** `[RecurringRuleResponse]`. |
| POST | `/v1/transactions/recurring/materialize` | write | Genera las copias mensuales pendientes de TODAS las reglas del usuario, desde el cursor `last_materialized_month` (exclusivo) hasta el mes en curso, una por mes civil vencido (`source='manual'`, `recurring_rule_id` de la regla, `import_id NULL`). **Idempotente**: el cursor es la única fuente de idempotencia — re-materializar no duplica ni recrea instancias borradas (el cursor ya pasó ese mes). **Jamás crea `op_date` futuro**: el mes en curso solo se materializa cuando su día del mes ya ha llegado; `day_of_month` se clampa al fin de mes en meses cortos (feb/abr). Huella `manual` + ordinal siguiente → **nunca 409**. **No** invalida la cache de proyección. Body vacío. → **200** `{rules_processed, materialized}` (`MaterializeResponse`). |
| DELETE | `/v1/transactions/recurring/{id}` | write | Borra la plantilla (guardia id+installation+owner → **404**). Las instancias ya materializadas **se conservan** (`transactions.recurring_rule_id` es `ON DELETE SET NULL` → quedan como movimientos manuales sueltos). → **204**. |

**Recurrencia — notas**: no hay `PATCH` de plantilla (para cambiarla, bórrala y recréala). Las copias mensuales se crean por dos vías, ambas transaccionales: (a) el **backfill del alta** con `recurrence` (`POST /v1/transactions` o `/batch`), que rellena en el mismo commit todos los meses intermedios entre la `op_date` y hoy (cota 10 años → 422 `recurrence_too_old`); y (b) `POST /recurring/materialize`, para el avance de mes posterior. Ningún GET muta (los listados nunca generan instancias). El alta con `recurrence` además crea la regla-plantilla y deja enlazada la transacción de origen.

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

## MCP (`/mcp`, Streamable HTTP)

Servidor MCP embebido de **solo lectura** (v3.0.0), módulo `apps/api/src/mcp/` con el SDK oficial
`rmcp` 3.1 (spec 2026-07-28 sessionless + `LocalSessionManager` para clientes legacy con
`Mcp-Session-Id`). Mismo binario y puerto que el API; se monta en el router raíz junto a `/health`
(gana siempre al fallback SPA). Kill-switch: `FUTUREFIN_MCP_ENABLED=0` → el router ni se monta (404).

- **Auth (dos esquemas Bearer)**: middleware `mcp/auth.rs::mcp_bearer_auth` corta ANTES del
  protocolo y despacha **por prefijo del Bearer** → `ffo_` = access token OAuth
  (`oauth::access::require_oauth_access_token`), cualquier otra cosa (incl. prefijos desconocidos)
  = token de API (`handlers::api_tokens::require_api_token`, el 401 indistinto). Tras cualquiera de
  las dos, `require_installation_member` re-resuelve membership y rol **vivos** →
  `McpIdentity {user_id, installation_id, role, credential}` en las extensions del request, con
  `credential: McpCredential::{ApiToken{token_id} | OAuth{grant_id, token_id}}`; rmcp propaga las
  `http::request::Parts` hasta el `RequestContext` de cada tool. Fallo → 401/403 JSON
  `{error, message}`; **solo el 401** añade `WWW-Authenticate` (ver la nota del challenge en la
  sección OAuth).
- **Tools v1 (10, todas read-only)**: `get_summary`, `get_projection` (density **hybrid fija**,
  `asset_series` opt-in con `include_asset_series`, comparte la cache de proyección del handler),
  `get_budget`, `get_transactions_summary`, `list_transactions` (truncado a `limit` 1..500 def 100,
  responde `{total_count, truncated, transactions}`), `get_history`, `list_assets`,
  `list_liabilities`, `list_planning_flows`, `get_settings`. Todas menos `get_settings` aceptan
  `view: "household"|"mine"` (misma semántica que `?view=`).
- **Cero deriva handler↔tool**: cada tool llama a la MISMA core fn que el endpoint HTTP
  (`summary_core`, `projection_series_cached`, `budget_snapshot_core`, `transactions_summary_core`,
  `list_transactions_core`, `history_series_core`, `list_assets_core`, `list_liabilities_core`,
  `list_planning_flows_core`, `installation_access_core`) y serializa el mismo struct serde →
  Decimal-as-string intacto. Paridad congelada en `apps/api/tests/mcp_http.rs`.
- **Errores**: dominio/validación → `CallToolResult{is_error:true}` con el JSON `{error, message}`
  de `ErrorBody`; `Db`/`Unavailable` → `ErrorData::internal_error` sanitizado (detalle a tracing).
- **NO está en OpenAPI a propósito**: no es un recurso REST — es JSON-RPC cuyo contrato define la
  spec MCP y que se autodescribe vía `tools/list`.
- **Límite conocido de 3.0.0 — resuelto en 3.1.0**: el conector de claude.ai exige OAuth 2.1, que
  entonces estaba fuera de scope. Desde 3.1.0 el authorization server va embebido (sección
  siguiente) y `/mcp` acepta **dos** esquemas Bearer: `ffp_…` (token de API, pegado a mano) y
  `ffo_…` (access token OAuth, emitido por el flujo de consentimiento). Claude Code/Desktop sigue
  funcionando igual: `claude mcp add --transport http futurefin https://host/mcp --header
  "Authorization: Bearer ffp_…"`.

## OAuth 2.1 (v3.1.0)

Authorization server **embebido** en el mismo binario y puerto, módulo `apps/api/src/oauth/`
(protocolo) + `apps/api/src/handlers/oauth_consent.rs` (pantalla de consentimiento y panel). Existe
para una sola cosa: que el conector de claude.ai web pueda hablar con `/mcp`, que exige OAuth 2.1 y
no acepta un Bearer pegado a mano. FutureFin es a la vez **authorization server y resource server**
— no hay IdP externo, ni claves de firma, ni JWT. Regresión completa: `apps/api/tests/oauth_flow.rs`.

### Rutas de protocolo (nivel raíz, **fuera de OpenAPI**)

| Method | Path | Notas |
|--------|------|-------|
| GET | `/.well-known/oauth-protected-resource[/mcp]` | RFC 9728. `{resource: "{base}/mcp", authorization_servers: [base], bearer_methods_supported: ["header"]}`. Sin SELECT y sin mutación: solo refleja la URL pública. |
| GET | `/.well-known/oauth-authorization-server[/mcp]` | RFC 8414. `issuer`, `authorization_endpoint` (`{base}/oauth/authorize`), `token_endpoint`, `registration_endpoint`, `revocation_endpoint`, `code_challenge_methods_supported: ["S256"]` (único), `grant_types_supported: [authorization_code, refresh_token]`, `authorization_response_iss_parameter_supported: true`. **Sin `scopes_supported`** a propósito: MCP v1 es read-only entero, no hay scopes con función. |
| POST | `/oauth/register` | DCR (RFC 7591), **público y sin autenticación**. Body `{redirect_uris (1..5, requerido), client_name?, client_uri?, token_endpoint_auth_method?, grant_types?, response_types?}` → **201** `{client_id ("ffc_…"), client_id_issued_at, client_secret? ("ffcs_…"), client_secret_expires_at? (0 = no caduca), …}`. `token_endpoint_auth_method` omitido ⇒ `client_secret_basic` (default RFC 7591 §2) y se emite secreto; `none` ⇒ cliente público sin secreto (el caso de claude.ai). Errores `invalid_client_metadata` / `invalid_redirect_uri`. |
| POST | `/oauth/token` | `grant_type=authorization_code` (PKCE **S256 obligatorio**) o `grant_type=refresh_token` (rotación). Form-urlencoded. → `{access_token ("ffo_…"), token_type: "Bearer", expires_in: 3600, refresh_token ("ffr_…"), scope?}` + `Cache-Control: no-store`. |
| POST | `/oauth/revoke` | RFC 7009. Un `ffr_…` revoca el **grant entero** (§2.1: "desconectar" en claude corta todo); un `ffo_…` revoca solo su fila. Token desconocido → **200** igualmente (§2.2). |

**`GET /oauth/authorize` NO se registra en el backend — prohibido.** La sirve el fallback SPA
(`ServeDir(...).fallback(ServeFile(index.html))` de `main.rs`), porque la pantalla de consentimiento
es React. Si registraras cualquier método en ese path, axum devolvería **405** en los demás y un
method-mismatch **no cae al fallback**: mataría la pantalla en producción. Fijado por el test
`get_oauth_authorize_is_not_handled_by_the_api` y por el comentario de cabecera de `oauth/mod.rs`.

### Endpoints de la SPA (`/v1/oauth/*`, **sí en OpenAPI**) — handler `oauth_consent.rs`

| Method | Path | Auth | Notas |
|--------|------|------|-------|
| GET | `/v1/oauth/authorize-details` | **pública** (cookie opcional) | Valida los parámetros del authorization request y devuelve qué pintar. **Sin sesión a propósito**: solo devuelve metadata que el propio cliente registró (`client_name` — texto NO verificado —, `client_uri`, `redirect_host` — el único dato verificado —, `resource`), nada del usuario; a cambio, un `redirect_uri` que no cuadra se ve **antes** de teclear la contraseña. Con cookie válida añade `already_connected` / `connected_at`. `status` ∈ `consent` \| `invalid_request` (fatal: pintar el error, **jamás** redirigir) \| `redirect_error` (navegar a `redirect_to`). |
| POST | `/v1/oauth/authorize` | cookie + `require_installation_member` | Body `{approve: bool, …params del authorize (flatten)}` → **200** `{redirect_to}`, la URL a la que la SPA navega. Approve → `code` + `state` (eco literal) + `iss` (RFC 9207); deny → `error=access_denied` al redirect registrado (no dejar al cliente colgado). Error fatal → **400** `authorize_error: <code>`. 401 sin sesión, 403 si pending. |
| GET | `/v1/oauth/connections` | cookie + membership | Conexiones activas **del caller** (`oauth_grants` no revocados), orden `created_at DESC`: `{id, client_name, client_uri?, redirect_host?, created_at, last_used_at?}`. |
| DELETE | `/v1/oauth/connections/{id}` | cookie + membership | Soft-revoke (`revoked_at = now()`, `revoked_reason = 'user_panel'`) → **204**; corte inmediato. Solo grants propios: un id ajeno devuelve el mismo **404** que uno inexistente (no revela existencia). |

- **CSRF del POST, por partida doble**: la cookie es `SameSite=Lax` (un POST cross-site no la lleva)
  y el body es JSON, que no es un "simple request" → exige preflight, que la lista blanca CORS
  bloquea. **No cambies el body a form-urlencoded** (perderías la segunda mitad).
- **La validación del authorize vive UNA vez**, en `oauth::authorize::validate_authorize_params`, y
  la consumen los dos endpoints. Nunca dupliques esas reglas en un handler. La distinción crítica
  (OAuth 2.1 §7.12.2) es `AuthorizeParamError::Fatal` (client_id desconocido o `redirect_uri` sin
  match exacto → **no se puede redirigir**, sería un open redirect) vs `Redirectable`
  (`response_type`/PKCE/`resource` malos con cliente y redirect ya validados → error al
  `redirect_uri` registrado). El match del `redirect_uri` es de **string completa**, ni prefijo ni
  solo host.

### Contrato de tokens

| Credencial | Prefijo | Persistencia | TTL |
|---|---|---|---|
| `client_id` | `ffc_` | claro (no es secreto) | — |
| client secret | `ffcs_` | **solo** SHA-256 hex (`oauth_clients.client_secret_hash`) | no caduca (`client_secret_expires_at: 0`) |
| authorization code | *(sin prefijo)* | **solo** SHA-256 hex (PK `oauth_authorization_codes.code_hash`) | **2 min**, un solo uso |
| access token | `ffo_` | **solo** SHA-256 hex (`token_hash` UNIQUE) | **1 h** (`expires_in: 3600`) |
| refresh token | `ffr_` | **solo** SHA-256 hex (`token_hash` UNIQUE) | **90 días sin uso** (sliding: cada rotación emite uno nuevo con 90 días) |

- **Todos opacos y hash-only** — mismo contrato que `api_tokens` (`auth/secret.rs`:
  `generate_opaque_secret` = prefijo + 43 chars base64url de 32 bytes `OsRng`, `sha256_hex`,
  `generate_opaque_id` para `client_id`). Lookup O(1) por hash exacto, cero comparación de secretos
  en Rust. Nada se congela en el token: rol e installation se re-resuelven vivos en cada request.
- **Las expiries las calcula Postgres**, nunca Rust (`now() + $n::interval`).
- **El grant es la unidad de todo** (`oauth_grants`: una fila por app+usuario). Es lo que ve y
  revoca el panel, y lo que los `access.rs`/`token.rs` exigen vivo por JOIN → revocar una fila corta
  todos los tokens de esa app sin tocarlos, igual que borrar una sesión.
- **Rotación + reuse-detection**: cada canje de refresh consume el actual (`consumed_at`), emite uno
  nuevo y los encadena (`replaced_by`, auditoría de la rotación). Presentar un code o un refresh **ya
  consumido** es la señal de robo → se revoca el **grant entero** (OAuth 2.1 §4.3.1/§7.5), con
  `revoked_reason ∈ {code_reuse, refresh_token_reuse}`. Los dos grant types corren en una
  transacción con `FOR UPDATE` sobre la fila de la credencial.
- **Anti-flood del registro abierto**: `POST /oauth/register` hace GC perezoso (borra clientes de
  >24 h **sin ningún grant** — jamás uno consentido) y corta con `503 temporarily_unavailable` si
  quedan ≥1000 clientes. El GC vive en el POST y no en un GET (D5, reads never mutate).

### Formato de error — **no es `ApiError`**

Las rutas de protocolo devuelven `OAuthError` (`oauth/error.rs`): JSON
`{"error": "...", "error_description": "..."}` de RFC 6749 §5.2, no el `{error, message}` del API
propio, porque el body y los códigos (`invalid_request`, `invalid_client`, `invalid_grant`,
`invalid_target`, `unsupported_grant_type`, `invalid_client_metadata`, `invalid_redirect_uri`,
`server_error`, `temporarily_unavailable`) los fija la RFC. Toda respuesta lleva
`Cache-Control: no-store`. `invalid_client` es **siempre 401** (nunca 400): es la señal exacta con la
que claude.ai re-registra el cliente vía DCR — gracias a ella un restore de backup sin tablas OAuth
se auto-recupera sin intervención; ese 401 añade `WWW-Authenticate: Basic realm="FutureFin"`. Los
`/v1/oauth/*` sí hablan `ApiError` normal (son API propio). `oauth::access::require_oauth_access_token`
devuelve `ApiError` a propósito: alimenta al middleware de `/mcp`.

### Por qué el protocolo está fuera de OpenAPI

Igual que `/mcp`: su contrato lo fijan las RFC (8414/9728/7591/7009 + la spec de autorización MCP) y
los clientes lo descubren por los documentos `.well-known`, no por nuestro esquema. Duplicarlo en
`utoipa` solo crearía deriva. Los **cuatro** endpoints de la SPA sí están anotados
(`__path_authorize_details`, `__path_authorize_decision`, `__path_list_connections`,
`__path_revoke_connection`, tag `oauth`, en `openapi.rs`) porque son API propio.

### Kill-switch — con una excepción

`FUTUREFIN_MCP_ENABLED=0` desmonta el router de protocolo completo (`oauth_protocol_router()` no se
construye: las 7 rutas raíz caen al fallback) **y** los dos endpoints del flujo
(`/v1/oauth/authorize-details`, `POST /v1/oauth/authorize`). **`GET/DELETE /v1/oauth/connections[/{id}]`
se montan SIEMPRE** — precedente de `/v1/api-tokens`: con MCP apagado sigues pudiendo *ver y revocar*
credenciales que ya existen. La bifurcación está en `oauth_consent_router(mcp_enabled)` y en
`routes/mod.rs`; OAuth hoy no sirve a nada más que a MCP, de ahí que compartan el interruptor.

### El challenge del 401 de `/mcp` — solo el 401

Cuando `/mcp` rechaza por credencial (**401**), el middleware añade
`WWW-Authenticate: Bearer realm="FutureFin", resource_metadata="{base}/.well-known/oauth-protected-resource/mcp"`
(RFC 9728 §5.1) para que un cliente OAuth descubra el authorization server. **Un 403 nunca lo
lleva**: un usuario pending o con membership revocada recibiría el challenge, se re-autenticaría,
obtendría un token nuevo y volvería a comer el mismo 403 — bucle infinito. Si la URL pública no se
puede derivar, el header degrada a `Bearer` a secas.

### URL pública (issuer)

`oauth/url.rs::public_base_url` — `FUTUREFIN_PUBLIC_URL` si está fijada; si no, se **deriva del
request**: `X-Forwarded-Proto`/`X-Forwarded-Host` (primer valor de cada uno) o el header `Host`, con
un charset estricto (`host[:puerto]`, IPv6 entre corchetes; sin `/`, `@`, espacios ni controles;
≤255 chars) → si no cuadra, **400 `invalid_request`**. Sin `Host` tampoco hay issuer. Así producción
sigue sin requerir ninguna env var (promesa 3.0.0). Ver [`env-and-config.md`](env-and-config.md).
Los redirects se construyen **siempre** con `oauth::url::append_query` (escaping de `url::Url`) —
concatenar a mano es donde nacen los open redirect.

### Anti-clickjacking global

`main.rs` aplica `SetResponseHeaderLayer::overriding(X_FRAME_OPTIONS, "DENY")` **sobre el router
final** (API + fallback SPA), no sobre el sub-router `api`: la pantalla de consentimiento la sirve el
fallback, y era justo la que había que proteger. Nada de FutureFin se embebe legítimamente en iframes.
