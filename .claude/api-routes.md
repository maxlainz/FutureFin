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
| PATCH | `/v1/installation` | Owner only. Updates tz, inflation, show_age_mode, fire_settings (incluye `savings_source: "budget" \| "transactions_avg" \| "budget_income_real_expense"` — fuente del ahorro de la simulación; no existe `target_age` — eliminado en v1.0.6) y `mcp_write_enabled` (bool, kill-switch vivo de las tools de escritura MCP — issue #3) |
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

**`GET /v1/allocation-rules/resolution` (3.8.0)** — la cascada **resuelta** para el mes en curso.
Endpoint nuevo y no un envelope sobre `GET /v1/allocation-rules`: convertir aquel array en objeto
habría roto el contrato. Construye su propio `ProjectionInput` con horizonte 1 (mismo coste que
`GET /v1/assets`, una tanda de SELECTs) y **no pasa por la cache de proyección**, coherente con
`assets_projection_context`.

Devuelve `base_cash` —lo que la cascada reparte de verdad— **desglosado** en `recurring_net`
(`income − expense − debt_service`, estable) y `planning_component` (el tramo de los planning flows
sin fecha del mes en curso, que se agota en 90 días), con el flag `base_includes_transient`. Ese
desglose es el punto: un flag de «sobreasignación» a secas habría dicho «sí» y habría sido igual de
engañoso — la cascada **no** puede repartir de más (`take` se acota por intención, cap y caja, y el
bucle corta al agotarse), lo que pasaba es que la base incluye un término transitorio.

Por regla: `amount_intent` vs `amount_resolved` (si difieren sin `skipped_reason`, la regla fue
**recortada** por el cap — no saltada), `cap_ceiling`/`cap_room` y `skipped_reason` ∈ {`no_cash`,
`not_reached`, `cap_full`, `zero_amount`, `invalid_target`}. `no_cash` y `not_reached` **no se
colapsan**: «no te sobra dinero» y «las reglas de arriba se lo comieron» tienen remedios distintos.
Las reglas posteriores al corte por caja se emiten con `not_reached` en vez de desaparecer del
informe. Cierra con `per_asset` y `leftover_to_surplus_cash`, y la identidad
`Σ per_asset + leftover = base_cash` está pinneada en `allocation_resolution.rs`.

**Los tres campos de aportación de `/v1/assets` son cosas distintas** — la confusión entre ellos es
el defecto de contrato que abrió el issue #4:

- `contribution_nominal_monthly`: aporte del **primer mes** resuelto por la cascada. No es un
  importe mensual estable pese al nombre — la cascada reparte `net_cash_month`, que incluye el tramo
  de los planning flows sin fecha del mes en curso (`importe/90` por día natural), así que el valor
  **decrece cada día** y **salta el día 1 de cada mes**. Un número «mensual» que cambia a diario es
  una trampa para cualquiera que haga aritmética con él, humano o modelo.
- `contribution_recurring_monthly` (**3.8.0**): la MISMA cascada evaluada sobre el neto
  **recurrente** (`income − expense − debt_service`, sin el tramo de planning). Estable y
  reproducible: es el número que una persona quiere decir cuando dice «mi aportación mensual», y el
  único con el que tiene sentido hacer cuentas. Se calcula con una segunda pasada del engine sobre
  el mismo input con `planning_monthly_cash_adjustment[0] = 0` — reutilizar la cascada en vez de
  aproximarla garantiza caps y precedencia idénticos, sin ningún SELECT extra.
- `contribution_target_amount`: **no es una aportación**, es el tope en euros del activo.

Los dos primeros se sirven redondeados a **4 decimales** (política monetaria de la casa; antes
salían los 28 dígitos de la división).

**Base de los caps en `/v1/assets` (v2.2.0)**: el `contribution_target_amount` que devuelven `GET/POST/PATCH /v1/assets` resuelve `months_expense` / `income_multiple` con los escalares **efectivos** del engine — o sea, en modo B/C con datos el gasto/income salen del promedio real 12m, no del presupuesto (antes se resolvían siempre con presupuesto y el objetivo no casaba ni con la aportación del mes 1 mostrada en la misma respuesta ni con la proyección). Un único `assets_projection_context` (`handlers/projection.rs`) devuelve `{nominals, income_monthly, expense_with_debt}` de **un solo** `build_installation_projection_input` por request; sustituye a `first_month_asset_contribution_nominals_map` + `monthly_income_expense_debt_for_view` (eliminados).

### Liabilities (`/v1/liabilities/`)
Accepts `?view=mine`. `principal_derived_from_plan` flag indicates auto-derived principal from planning flows.

**`expense_category_id` (3.4.0, API breaking interno en el create)**: categoría de GASTO donde vive la cuota — el presupuesto y la comparativa de Movimientos atribuyen ahí el equivalente mensual del plan. **Obligatoria en `POST /v1/liabilities`** (campo requerido, validado scope `expense` + instalación; también en la tool MCP `create_liability`); en PATCH es **set-only** (asignar/cambiar, nunca vaciar). Los pasivos anteriores a 3.4.0 conservan `NULL` («sin asignar»: se comportan como antes, sin atribución) hasta que el usuario la asigne; el import de `.ffbackup` viejos también deja `NULL`. FK `ON DELETE SET NULL` (no bloquea borrar la categoría; el `remap_to` de `/v1/categories` sí la arrastra cuando la categoría remapeada es de gasto).

**Expiration filter**: rows with `payment_end_date < today` are hidden from `GET /v1/liabilities` (and from totals/breakdowns in `/summary`, derived lines in `/budget`, debt service in `/projection`). The rows are not deleted — the filter is `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. Use `installation.calendar_tz` to compute `today`.

### Summary (`/v1/summary/`)
Aggregated net worth, financial health metrics, category breakdowns. Accepts `?view=mine`. `total_liabilities` and breakdowns exclude expired rows (see Liabilities note above).

**`financial_health` sigue el toggle `fire_settings.savings_source`** (3 modos; gate `SavingsSource::uses_transactions()` = B o C). Con datos:
- **Modo B (`transactions_avg`)**: `income_monthly_equivalent`, `expense_regular_monthly_equivalent`, `net_monthly_equivalent` (= `income_avg − expense_avg`) y `savings_rate` salen del promedio real 12m **crudo** (reforma 3.4.0: las cuotas de pasivo ya viven dentro de los movimientos — sin resta híbrida ni debt service re-sumado), no del presupuesto.
- **Modo C (`budget_income_real_expense`)**: igual que B pero `income_monthly_equivalent` **conserva el income del presupuesto** (NO se sobreescribe); `expense_regular_monthly_equivalent = expense_avg` y `net_monthly_equivalent = income (presupuesto) − expense_avg`. El `match` sobre `savings_source` es exhaustivo (`Budget` es rama inalcanzable no-op, guardada por `uses_transactions()`).
- **Base de gasto derivada/total**: `expense_derived_monthly_equivalent` = **0 en los tres modos** desde la 3.7.0 — ya no existe una componente derivada que reportar. En modo A la cuota vive dentro de `expense_regular_monthly_equivalent` (fusión en el presupuesto); en B/C, dentro del promedio real de gasto (reforma 3.4.0: los pasivos solo restan su principal en `net_worth` y en la proyección) y `expense_total_monthly_equivalent` = `expense_avg`. El campo se mantiene por compatibilidad de contrato, igual que `monthly_net_excluding_derived_debt` y `savings_rate_excluding_derived_debt`, que desde la 3.7.0 son **idénticos** a `net_monthly_equivalent` y `savings_rate`. Las dos identidades valen en los **tres** modos (la primera, degenerada):
  - `expense_total_monthly_equivalent = expense_regular_monthly_equivalent + expense_derived_monthly_equivalent`
  - `net_monthly_equivalent = income_monthly_equivalent − expense_total_monthly_equivalent`
- **Fallback**: `months_with_data == 0` en B/C → el bloque `financial_health` completo es **idéntico** al de modo A (runway incluido).

Campos de `financial_health` relacionados con el modo y el runway:
- `savings_source` (`"budget" | "transactions_avg" | "budget_income_real_expense"`) — modo **efectivo** tras el fallback (B o C con `months_with_data == 0` → devuelve `"budget"`).
- `savings_source_months_with_data` (`u32`) — meses **reales** del promedio (ver §Transactions); `0` en modo A y en el fallback.

**KPI «ahorro real vs esperado» (aditivo, 3.2.0)** — tres campos de `financial_health` **independientes del toggle `savings_source`** (se sirven con el mismo valor en los tres modos; para servir el «real» también en modo A, `summary_core` llama siempre a `transactions_12m_avg` — 1 query extra sin transacciones, 3 con):
- `savings_expected_monthly_equivalent` (Decimal-string) — el **neto del presupuesto** (`net_monthly_equivalent` del snapshot de budget, cuotas de pasivo incluidas), capturado **antes** del override B/C → en B/C puede diferir del `net_monthly_equivalent` servido.
- `savings_actual_monthly_avg_12m` (Decimal-string, opcional, `skip_serializing_if`) — promedio **bruto** `income_avg − expense_avg` de `transactions_12m_avg` (12 meses civiles completos, meses solo-recurrentes excluidos). Las cuotas pagadas cuentan como gasto — simétrico al esperado, que incluye las cuotas del presupuesto. Desde la reforma 3.4.0, en modo B este «real» coincide con `net_monthly_equivalent` (misma base cruda). Se **omite** del JSON cuando no hay meses con datos (ausente ≠ «ahorras 0 €»).
- `savings_actual_months_with_data` (`u32`) — meses reales que sustentan el promedio anterior; a diferencia de `savings_source_months_with_data`, trae su valor real en los tres modos.
API no breaking (aditivo); `/v1/summary` no tiene cache → sin contrato de invalidación; esto **no** convierte las transacciones en input del engine (D12a intacto — en modo A la proyección sigue ignorándolas).
- `runway_months` (Decimal-string, opcional) — meses que los activos **líquidos** cubren `expense_total_monthly_equivalent`, **componiendo** la rentabilidad esperada de esos líquidos (media ponderada por valor de los multiplicadores mensuales) y la inflación del gasto (`installation.annual_inflation_assumption_percent`, clampada a ≥ 0). Lo calcula `futurefin_engine::liquid_runway_months` (ver [`engine.md`](engine.md) §Runway). **No** es `liquid_assets_total / expense_total`, salvo que rentabilidad e inflación sean 0 (y el umbral SWR no se cumpla), caso en el que se reduce exactamente a esa división. Como sigue `expense_total`, en B/C se calcula sobre la base de gasto real. Se **omite** del JSON (`skip_serializing_if`) cuando es `None`: sin base de gasto (`expense_total == 0`) o runway indefinido. El valor `1200` (`MAX_RUNWAY_MONTHS`) **no** es un centinela de infinito sino un **suelo**: significa «al menos 100 años» (el bucle agotó el tope sin cumplir el umbral SWR) y la UI lo pinta «+100 años».
- **Precisión de salida (3.8.0)** — los ratios se sirven **redondeados** (`round_ratio`, en la core;
  nunca en la capa MCP, que devuelve la struct intacta). `savings_rate`,
  `savings_rate_excluding_derived_debt`, `upcoming_coverage_ratio` y `debt_to_assets_ratio` a **6
  decimales de fracción** (= 4 decimales de porcentaje, muy por encima del único decimal que pinta
  la UI); `runway_months` a **1 decimal**, alineado con `simulate_projection`, que ya redondeaba
  así. Antes salían los hasta 28 dígitos que produce cada división de `rust_decimal`
  (`"0.2435991666666666666666666667"`). Es un cambio de **presentación**: el gross-up, el umbral SWR
  y el propio runway se calculan con la precisión completa y solo el resultado publicado se recorta,
  así que ninguna cifra derivada se mueve. Los importes monetarios (4 decimales) no cambian.
- `runway_is_indefinite` (`bool`) — desde **v2.3.0** lo decide el **umbral SWR**, no sobrevivir el cap: `true` ⟺ la retirada anual bruta no supera el SWR sobre el saldo líquido, es decir `gross_up(expense_total × 12) × 100 ≤ liquid_assets_total × swr_pct`, con `swr_pct`/`tax_brackets`/`taxes_enabled` de `installation.fire_settings` (pestaña Jubilación) y el **mismo** `gross_up_net_annual_fire` del target FIRE. Entonces `runway_months` no viaja. Con `swr_pct ≤ 0` nunca es `true`. Con `expense_total == 0` es `false` (no hay base de gasto, no es que esté cubierto). El disparador es deliberadamente independiente de rentabilidad e inflación (que gobiernan solo el caso finito). La UI muestra «Infinito (dentro del SWR 3,5 %)» en el primer caso y oculta la tarjeta en el segundo. **API no breaking**: tipo y nullabilidad de ambos campos son los de v2.2.0.

### Budget (`/v1/budget/`)
Partidas de ingreso/gasto en **una sola lista** (`entries`). Acepta `?view=mine`.

**Fusión de las cuotas de pasivo (3.7.0, API breaking).** Hasta la 3.6.0 las cuotas vivían en un array aparte (`derived_from_liabilities`) que se sumaba por debajo del presupuesto en `totals.expense_derived_monthly_equivalent`. Ahora son **una partida más de `entries`**:

- `source`: `"manual"` (fila de `budget_entries`, editable) | `"liability"` (cuota derivada del plan de pago, **solo lectura**). `PATCH`/`DELETE /v1/budget/entries/{id}` sobre una cuota devuelven 404: se edita con `PATCH /v1/liabilities/{id}`.
- En una cuota: `id` = id del pasivo (los UUID no colisionan entre tablas) y `liability_id` lo repite; `label` = etiqueta del pasivo; `category_id` = su **`expense_category_id`** (la misma categoría de GASTO con la que la comparativa de Movimientos empareja los recibos reales); `amount` = **equivalente mensual** del plan (`weekly` → ×52/12; el importe y la frecuencia crudos siguen en `/v1/liabilities`); `expense_end_date` = fin del plan (`null` = indefinido).
- `category_id` es **opcional** (`skip_serializing_if`): se omite solo en cuotas de pasivos sin `expense_category_id` (anteriores a 3.4.0, y los importados de `.ffbackup` viejos). Esas cuotas **siguen sumando** en los totales — descartarlas bajaría el gasto presupuestado en silencio.
- Totales: `expense_regular_monthly_equivalent` incluye las cuotas y es exactamente la suma de los `entries` de scope `expense`; `expense_total_monthly_equivalent` vale lo mismo (se mantiene por compatibilidad). **`expense_derived_monthly_equivalent` y `derived_from_liabilities` ya no existen.**
- `expense_retirement_monthly_equivalent` cuenta **solo partidas manuales**: una cuota termina con su plan, así que no es gasto post-jubilación. Es el campo que consume la previa FIRE (incidente v1.3.0, divergencia 2–3×).

**Cuota «activa»**: pasivo con plan de pago (`payment_amount` + `payment_frequency`) y `payment_end_date IS NULL OR payment_end_date >= today` — mismo predicado que `/v1/liabilities` y `/v1/summary` (unificado en 3.4.0; antes exigía fecha fin NOT NULL y `>` estricto, y un pasivo sin fecha fin no derivaba línea).

> **La base de gasto del engine NO cambia.** `ledger_regular_monthly_income_and_expense` sigue devolviendo solo lo persistido: el engine cobra la cuota por su lado (`ProjectionLiabilityInput::monthly_payment`, con amortización y fecha fin), así que fundirla también ahí la contaría dos veces en el modo A. `monthly_delta_assumption` de `/v1/projection/series` sigue siendo `income − gasto persistido`. Clavado por `budget_liability_quotas.rs::liability_quota_stays_out_of_the_engine_expense_base`.

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
Histórico de gasto mensual **per-user**: import de CSV bancario (MyInvestor/N26) o efectivo a mano, categorización con reglas aprendidas, y comparativa mes real vs presupuesto vs promedio. Decimal-as-string (importes firmados: negativo = cargo). **Invalidación de la cache de proyección condicionada al modo** (`fire_settings.savings_source`): en modo A (`budget`, default) las transacciones **no son inputs del engine** → ninguna mutación invalida; en los modos que usan transacciones (B `transactions_avg` y C `budget_income_real_expense` → `SavingsSource::uses_transactions()`) el ahorro de la simulación deriva del promedio real 12m → las mutaciones que cambian el conjunto (create/batch/patch/delete, delete import, import confirm, `recurring/materialize`, y desde 3.5.0 conciliar/desconciliar) invalidan la cache vía `invalidate_projection_if_savings_uses_transactions` (best-effort post-commit, jamás convierte una mutación exitosa en 5xx). `rules.rs`, previews, el borrado de una regla recurrente y un pase de conciliación sin pares nuevos nunca invalidan. Regresión (A/B/C + flip + reconcile): `transactions_projection_cache.rs`.

**Promedio 12m que alimenta el engine** (`transactions_12m_avg`, distinto del summary de Movimientos): ventana `[first-of-month(today) − 12m, first-of-month(today))`. El denominador `months_with_data` y las sumas por kind cuentan solo **meses reales** — meses del tramo con ≥1 transacción `recurring_rule_id IS NULL`. Un mes vacío o «pseudovacío» (solo instancias recurrentes materializadas, p. ej. tras un backfill) se excluye **por completo** (ni numerador ni denominador); un mes real cuenta entero, incluidas sus recurrentes. Desde 3.5.0 las **transferencias conciliadas** (`transfer_counterpart_id IS NOT NULL`) quedan igualmente fuera de numerador Y denominador (un mes cuyo único contenido son patas conciliadas es un mes vacío). El `GET /v1/transactions/summary` de la pestaña Movimientos **NO** aplica el filtro de meses reales (cuenta cualquier mes con datos) — divergencia deliberada — pero **SÍ** excluye conciliadas de todos sus buckets. Lecturas: cualquier miembro (`?view=mine` vía `LedgerView` en los GET de listado/comparativa/imports; las **reglas** son siempre own-user, sin `?view`); escrituras siempre `owner_user_id = usuario` y exigen `role_can_write` o **403**. Import limit 16 MiB (`BACKUP_IMPORT_BODY_LIMIT_BYTES`, reutilizado). Códigos 400 estables entre comillas.

| Method | Path | Rol | Notas |
|--------|------|-----|-------|
| GET | `/v1/transactions?view=&month=&kind=&category_id=&import_id=` | lectura | Listado, orden `op_date DESC`. `month` = `YYYY-MM` (inválido → 400). → **200** `[TransactionResponse]`. |
| POST | `/v1/transactions` | write | Alta manual (efectivo, `import_id NULL`, `source='manual'`). Body `{op_date, value_date?, concept, amount, kind, category_id?, linked_asset_id?, linked_liability_id?, notes?, recurrence?}`. **`recurrence: {}`** (opcional, marcador sin campos desde 3.2.0): crea además una regla recurrente-plantilla y deja esta transacción enlazada como instancia de origen (`recurring_rule_id`). Las reglas tienen **resolución mensual** — el legacy `day_of_month` (≤3.1.0) se **ignora** si un cliente viejo lo envía (breaking documentado en CHANGELOG 3.2.0). **Un alta con `op_date` pasada backfillea las instancias de todos los meses CERRADOS intermedios en el MISMO commit** (el mes en curso jamás; ya no depende de una llamada posterior a `/materialize`); `op_date` a más de 10 años atrás → **422** `recurrence_too_old`. 400: `invalid_kind`, `amount_zero`, `savings_no_category`, `category_scope_mismatch`, `linked_asset_not_found`, `linked_liability_not_found`. Huella duplicada → **409**. → **201** `TransactionResponse` (incluye `recurring_rule_id?`). |
| POST | `/v1/transactions/batch` | write | Alta manual multifila (1..=1000). Body `{transactions:[CreateTransactionBody]}`. Cada item acepta `recurrence` (misma semántica que el alta simple, backfill de meses intermedios incluido; item con `op_date` a >10 años → **422** `recurrence_too_old`). Ordinal de huella se avanza dentro del batch. → **201** `[TransactionResponse]`. |
| GET | `/v1/transactions/months?view=` | lectura | Meses con datos (`GROUP BY YYYY-MM`), orden DESC; `is_complete=false` para el mes en curso. → **200** `[MonthEntry]`. |
| GET | `/v1/transactions/category-series?view=&kind=&category_id=&window_months=` | lectura | Serie mensual por categoría (issue #2): para cada categoría del `kind` (`expense`\|`income`, obligatorio) con ≥1 movimiento en la ventana, un punto por mes **cero-relleno** (`{month: "YYYY-MM", total}`; magnitudes ≥ 0 Decimal-string escala 2, misma convención de signos que el summary). `window_months` default 12, clamp 1..=60; el último mes es el actual (parcial). Orden: nombre ASC, pseudo-categoría `null` (sin categorizar) al final. 400 `kind must be…`. → **200** `CategoryMonthlySeriesResponse`. Espejo MCP: `get_category_monthly_series`. |
| GET | `/v1/transactions/summary?view=&year=&month=&avg_window=&avg_months=` | lectura | Comparativa del mes (default: último mes **completo**). **Ventana del promedio** con `avg_window` ∈ {`3`,`6`,`12`,`ytd`,`all`} (default `6`; trim + case-insensitive; inválido → 400 `avg_window must be one of 3, 6, 12, ytd, all`); `avg_months` (1..24) es **alias legado** y `avg_window` gana si vienen ambos. Promedio **ponderado**: denominador = `months_with_data` (meses del tramo `[window_start, selected)` con ≥1 transacción del scope, **no** el nº de meses del tramo) → un historial corto ya no diluye la media a 0. YTD = meses del año del mes seleccionado estrictamente anteriores (enero → tramo vacío); ALL = desde el mes del primer movimiento. Magnitudes ≥0 para comparar con budget (gasto = `−Σ`, ingreso = `+Σ`, ahorro = `−Σ`). **Cuotas atribuidas por categoría (3.4.0)**: cada pasivo activo EN EL MES seleccionado (`payment_end_date IS NULL OR >= primer día del mes`) con `expense_category_id` asignada suma su equivalente mensual al lado **budget** de esa categoría — se empareja con los recibos reales (que ya viven categorizados) y `totals.expense_budget` = Σ budget de categorías de gasto **+ cuotas atribuidas**. Una categoría solo-cuota materializa su fila (budget = plan, actual = 0). Pasivos sin asignar (NULL, pre-3.4.0): sin atribución (comportamiento previo). Sigue **sin** `derived_debt_line` (la fila sintética sin pareja de la v1.6-1.8 no vuelve). Response añade `avg_window: string`, `window_months: u32`, `months_with_data: u32` y ya **no** trae `avg_months` ni `derived_debt_line`. 400: `year`/`month` fuera de rango o desapareados, `avg_window`/`avg_months` inválidos. → **200** `TransactionsSummaryResponse`. |
| POST | `/v1/transactions/import/preview` | write | **Stateless**, sin escrituras. Body `{source (auto\|myinvestor\|n26), file_b64, account_asset_id?}`. Autodetección por cabecera; dedup por huella (estado `new`/`already_imported`), heurísticas de transferencia y savings, matching de reglas. Devuelve `file_sha256` (a reenviar en confirm). 400: `csv_preset_unrecognized`, `csv_date_invalid`, `csv_amount_invalid`, base64 inválido. → **200** `ImportPreviewResponse`. |
| POST | `/v1/transactions/import/confirm` | write | Aplica el import. Body `{source, file_b64, file_sha256, decisions:[ImportDecision] (paralelo por índice a las filas), learn_rules=true, account_asset_id?, original_filename?}`. `file_sha256`/nº de filas deben coincidir con el preview → si no, 400 `preview_confirm_mismatch`. `decision.discard`/`force` por fila; solo EUR (`currency_not_eur`). `learn_rules` hace upsert de una regla por decisión categorizada. Lote vacío → cabecera borrada, `import_id: null`. Doble-confirm concurrente → **409**. Post-commit corre el **pase de auto-conciliación** sobre todo el dataset del owner (la contrapartida puede venir de un lote anterior) — best-effort, reportado en `reconciled_pairs` (0 si falló). → **200** `ImportConfirmResponse {import_id?, imported, skipped_already_imported, discarded, rules_learned, reconciled_pairs}`. |
| GET | `/v1/transactions/imports?view=` | lectura | Lotes de import (orden `created_at DESC`), con `txn_count` y nombre de cuenta origen. → **200** `[ImportBatchResponse]`. |
| DELETE | `/v1/transactions/imports/{id}?confirm=true` | write | Deshace un import (transacciones en cascada). `confirm` debe ser `true` → si no, 400 `confirm_required`. Guardia id+installation+owner → **404** si no es tuyo. → **204**. |
| PATCH | `/v1/transactions/{id}` | write | Edita una transacción (guardia owner → **404**). `op_date`/`amount`/`concept` son **editables en manuales e importadas** (ya no hay `immutable_field`). La diferencia está en la huella de dedup: en **manuales** se recomputa al cambiarlos (tomando un ordinal libre, liberando el anterior); en **importadas** queda **anclada** a la del CSV original y nunca se recomputa → un re-import del mismo archivo sigue detectando el duplicado pese a la edición. Campos `clear_*` para borrar opcionales. Huella duplicada tras recomputar (solo manuales) → **409**. → **200** `TransactionResponse`. |
| DELETE | `/v1/transactions/{id}` | write | Borra (guardia owner → **404**). → **204**. |
| GET | `/v1/transactions/rules` | lectura | Reglas de categorización del usuario (orden `updated_at DESC`). → **200** `[RuleResponse]`. |
| POST | `/v1/transactions/rules` | write | Crea regla. Body `{match_kind? (substring\|prefix\|exact), pattern, source?, assign_kind (requerido), assign_category_id?}`. `(source, pattern)` duplicado → **409**. → **201** `RuleResponse`. |
| PATCH | `/v1/transactions/rules/{id}` | write | Edita (guardia owner → **404**). `clear_source`/`clear_assign_kind`/`clear_assign_category`. Colisión `(source, pattern)` → **409**. → **200** `RuleResponse`. |
| DELETE | `/v1/transactions/rules/{id}` | write | Borra (guardia owner → **404**). → **204**. |
| GET | `/v1/transactions/recurring` | lectura | Reglas recurrentes del usuario (**plantillas**), orden `created_at DESC`. **Siempre own-user** (sin `?view`), como las reglas de categorización. Cada regla trae `category_name`. → **200** `[RecurringRuleResponse]`. |
| POST | `/v1/transactions/recurring/materialize` | write | Genera las copias mensuales pendientes de TODAS las reglas del usuario, desde el cursor `last_materialized_month` (exclusivo) hasta el **último mes cerrado**, una por mes civil (`source='manual'`, `recurring_rule_id` de la regla, `import_id NULL`), cada una fechada el **último día de su mes** (la instancia de M cuenta en las estadísticas de M pero solo existe con el servidor ya en M+1). **El mes en curso jamás se materializa** — ni siquiera en su último día — así el mes abierto no muestra movimientos sintéticos (3.2.0). **Idempotente**: el cursor es la única fuente de idempotencia — re-materializar no duplica ni recrea instancias borradas (el cursor ya pasó ese mes). Jamás crea `op_date` futuro (por construcción: solo meses cerrados). Huella `manual` + ordinal siguiente → **nunca 409**. Invalida la cache de proyección solo en modos B/C (ver cabecera §Transactions). Body vacío. → **200** `{rules_processed, materialized}` (`MaterializeResponse`). |
| DELETE | `/v1/transactions/recurring/{id}` | write | Borra la plantilla (guardia id+installation+owner → **404**). Las instancias ya materializadas **se conservan** (`transactions.recurring_rule_id` es `ON DELETE SET NULL` → quedan como movimientos manuales sueltos). → **204**. |
| POST | `/v1/transactions/reconcile` | write | **Pase explícito de auto-conciliación** (3.5.0) sobre TODO el dataset del owner: empareja importes exactamente opuestos, misma divisa, mismo owner, `\|Δop_date\| ≤ 5 días`, determinista (greedy por Δfecha con orden total) y de **punto fijo** (repetirlo → 0). Nunca re-empareja pares rechazados (`transfer_match_rejections`). Own-user, sin `?view`. Invalida cache COND solo si enlazó algo. → **200** `ReconcileRunResponse {pairs_created, transactions_reconciled}`. |
| POST | `/v1/transactions/{id}/reconcile` | write | **Conciliación manual de un par**: body `{counterpart_id}`. Exige importes exactamente opuestos y misma divisa (conciliar jamás altera el neto) pero **sin** ventana de fecha (SEPA lento, traspaso a caballo de dos meses). Borra un rechazo previo del par; idempotente si ya están conciliadas entre sí. Guardia owner → **404**. 400: `already_reconciled`, `transfer_amounts_not_opposite`, `transfer_currency_mismatch`, `transfer_same_transaction`. → **200** `ReconcilePairResponse {transaction, counterpart}`. |
| DELETE | `/v1/transactions/{id}/reconcile` | write | **Desconcilia** el par de `{id}` (cualquiera de las dos patas) y **persiste el rechazo** — el pase automático no lo resucita. Ambas patas vuelven a contar en los agregados. Guardia owner → **404**. 400 `not_reconciled`. → **200** `ReconcilePairResponse` (ambas ya sueltas). |

**`PATCH /v1/transactions/batch` (3.8.0)** — reclasificación en lote, 1..=200 ids **propios**.
Conjunto de campos **cerrado**: `kind`, `category_id`/`clear_category`, `notes`/`clear_notes`. No
admite `amount`, `op_date`, `concept` ni `value_date`, y ese es justo el punto: ninguno de los
campos admitidos entra en la huella de dedup (`source · op_date · amount · concept`) ni en el
emparejado de transferencias (`op_date`, `amount`), así que el lote no recomputa huellas, no rompe
pares y no dispara el pase de auto-conciliación. El lote **clasifica**; para reescribir está el
PATCH de uno en uno. **Todo o nada** en una única transacción: un id ajeno o inexistente ⇒ 404
nombrando hasta 5 culpables y cero filas tocadas (un resultado parcial obligaría al llamante a
reconciliar estado, que es lo que un lote viene a evitar). **Una sola invalidación COND** al final,
fuera del bucle: 16 recategorizaciones seguidas en modo C tiraban la cache 16 veces. El 404 con
mensaje usa `ApiError::NotFoundWith`, variante nueva que solo nombra ids que el llamante ya envió.

**Backfill de reglas de categorización (3.8.0)** — `POST /v1/transactions/rules/{id}/apply`, y el
eje `apply_to_existing` (`none` default | `uncategorized` | `all`) + `from_month` + `confirm` en
`POST /v1/transactions/rules`. Crear una regla sigue afectando **solo a imports futuros**; aplicarla
al pasado es esta ruta.

- **Precedencia completa, no la regla suelta**: el backfill evalúa `match_rule` sobre el conjunto
  ENTERO de reglas y solo escribe las filas cuya ganadora es `{id}` — el pasado queda como habría
  quedado importando hoy. Las filas donde esta regla casa pero pierde salen en
  `matched_by_other_rule`.
- **`source` se respeta** (una regla de MyInvestor no toca movimientos manuales, igual que en el
  import) y las filas afectadas se reportan en `skipped_by_source`. Sin ese contador, un
  `matched: 0` se leería como «no hay nada que hacer» cuando en realidad es «esta regla no aplica a
  este origen» — el no-op invisible es el modo de fallo caro de este repo.
- **Las patas de transferencia conciliadas se excluyen** (`skipped_reconciled`): están fuera de
  todos los agregados de flujo, recategorizarlas no significa nada.
- **Cache COND, y solo si escribe**: cambiar el `kind` de filas históricas cambia
  `transactions_12m_avg`, input del engine en B/C → `invalidate_projection_if_savings_uses_transactions`
  dentro de la core, condicionada a que haya filas afectadas. **Crear** la regla sigue siendo NONE.
  Los tres casos (crear / preview / backfill, en los tres modos) están en
  `applying_a_rule_invalidates_cond_but_creating_it_still_does_not`. `would_change_kind` en el
  preview es la señal explícita de que la proyección se moverá.
- Por HTTP, `apply_to_existing != "none"` sin `confirm: true` es un **400** (la SPA ya enseña el
  impacto antes de llamar); por MCP la tool devuelve el **preview**, patrón de la casa.
- No recalcula huellas ni toca la conciliación: `kind` y `category_id` no entran en la huella de
  dedup (`source · op_date · amount · concept`) ni en el emparejado (`op_date`, `amount`).

**Filtros de búsqueda de `GET /v1/transactions` (3.8.0)** — aditivos: omitidos, el comportamiento es
el de siempre byte a byte. Viven en `list_transactions_core`, así que HTTP y MCP devuelven los
mismos 400.

- `concept_contains` (1–200): subcadena del concepto, insensible a mayúsculas **y a tildes** —
  `cafe` encuentra `CAFÉ` y viceversa, la misma semántica que el matching de reglas de
  categorización. El plegado se replica en SQL con `translate()` sobre una tabla que incluye
  también `a-z → A-Z`, **no** con `upper()`: `upper()` depende de la collation del cluster (bajo `C`
  no toca los no-ASCII) y esta imagen ya cambió de collation una vez. Como el `concept` se almacena
  sin normalizar, la expresión colapsa además los runs de whitespace con `regexp_replace`. Las dos
  tablas —Rust y SQL— están pinneadas carácter a carácter por `sql_fold_tables_mirror_the_rust_fold`,
  que además barre el latín extendido comprobando que nada que Rust pliegue falte en la tabla SQL.
  Los comodines `%` y `_` del usuario se escapan (`LIKE … ESCAPE '\'`): sin eso, buscar `%`
  devolvería el conjunto entero. Nada de `unaccent`/`pg_trgm` — son extensiones y el Postgres va
  embebido en la imagen.
- `min_amount` / `max_amount`: sobre el importe **con signo**, que es la trampa más probable para un
  cliente. `max_amount=-50` son los gastos de 50 € o más; `min_amount=0`, solo entradas de dinero.
  Banda invertida → 400 explícito en vez de un conjunto vacío silencioso.
- `date_from` / `date_to`: `YYYY-MM-DD` **inclusivos en los dos extremos** («hasta el 31» incluye el
  31; un `<` exclusivo es el off-by-one-day clásico). **Excluyentes con `month`** → 400 si vienen
  juntos: son dos formas de decir lo mismo y cualquier precedencia implícita sería una trampa.
- **Índices**: ninguno nuevo. El scope entra por `transactions_installation_op_date_idx` (household,
  el default) o `transactions_owner_op_date_idx` (`view=mine`), y el `LIKE` se evalúa sobre el
  subconjunto ya acotado. Para el volumen de un hogar es irrelevante; si algún día duele, un GIN
  `pg_trgm` — que hoy no está instalado.

**Conciliación de transferencias — notas (3.5.0)**: un movimiento conciliado (`transfer_counterpart_id` presente en `TransactionResponse`, junto a `transfer_reconciled_at/source` y los denormalizados `transfer_counterpart_concept/op_date`) sigue **visible** en `GET /v1/transactions` y cuenta en `/months`, pero queda **excluido de todos los agregados de flujo**: totales/promedios del summary, `MIN(op_date)` de la ventana «Todo», serie por categoría, promedio real 12m del engine (modos B/C) y `months[]` de `/v1/history/cashflow`. **Asimetría deliberada del cashflow**: la curva `fine` **SÍ** incluye conciliadas — modela el saldo real de cada cuenta y excluirlas la haría divergir de los snapshots anclados (test `reconciled_excluded_from_months_but_not_from_fine_curve`). El pase automático corre post-commit tras toda mutación del conjunto (create/batch/patch de `amount`/`op_date`, delete, delete import, materialize, import confirm, import de backup). Un PATCH que cambia `amount`/`op_date` **rompe el par sin crear rechazo** (revertir el valor re-empareja); borrar una pata desconcilia la otra vía `ON DELETE SET NULL`. El flag `suggested_transfer` del preview de import queda como **hint informativo** (ya no implica descarte).

**Recurrencia — notas**: no hay `PATCH` de plantilla (para cambiarla, bórrala y recréala). Las copias mensuales se crean por dos vías, ambas transaccionales y con la misma semántica de **mes cerrado + fin de mes** (loop compartido `materialize_rule`): (a) el **backfill del alta** con `recurrence` (`POST /v1/transactions` o `/batch`), que rellena en el mismo commit los meses cerrados entre la `op_date` y el mes actual (cota 10 años → 422 `recurrence_too_old`); y (b) `POST /recurring/materialize`, para el avance de mes posterior (lo dispara el frontend al montar Movimientos; no hay cron). Ningún GET muta (los listados nunca generan instancias). El alta con `recurrence` además crea la regla-plantilla y deja enlazada la transacción de origen, que **conserva su `op_date` real** (solo las copias materializadas van a fin de mes).

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

Servidor MCP embebido (v3.0.0; **lectura + simulación + escritura** desde los issues #2/#3), módulo `apps/api/src/mcp/` con el SDK oficial
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
- **Tools de lectura — 19 en total: las 10 iniciales en este bullet, las 9 del issue #2 en el
  siguiente** (la vigésima con `read_only_hint = true` es `simulate_projection`, que tiene bullet
  propio): `get_summary`, `get_projection` (density **hybrid fija**,
  `asset_series` opt-in con `include_asset_series`, comparte la cache de proyección del handler;
  `months` declara su rango real 12..840 en el schema y solo la variante sin `months` sale de
  cache), `get_budget`, `get_transactions_summary`, `list_transactions` (**paginación en SQL**:
  `limit` 1..500 def 100 + `offset`, filtros `month/kind/category_id/import_id` +
  **búsqueda 3.8.0** `concept_contains/min_amount/max_amount/date_from/date_to`, responde
  `{total_count, offset, truncated, transactions}`; el endpoint HTTP conserva su contrato sin
  paginar), `get_history` (`window_months` 1..1200 + `include_asset_series` opt-in default false;
  los mismos knobs existen en `GET /v1/history/series` con `include_asset_series` default true),
  `list_assets`, `list_liabilities`, `list_planning_flows`, `get_settings` (incluye bloque
  `user {id, username, birth_date}` del usuario del token — el endpoint HTTP NO lo lleva). Todas
  menos `get_settings` aceptan `view: "household"|"mine"` (misma semántica que `?view=`).
- **Tools de lectura añadidas en el issue #2 (9)**: `list_allocation_rules` (la cascada como
  reglas, no solo su resultado resuelto), `list_categories` (catálogo id/scope/nombre, filtro
  `scope`, prerrequisito para escribir), `get_category_monthly_series` (serie mensual cero-rellena
  por categoría, magnitudes ≥ 0 Decimal-string; espejo del endpoint nuevo
  `GET /v1/transactions/category-series`), `get_history_cashflow` (`window_months` 1..120,
  `include_curve` opt-in default false, `resolution` weekly|daily), `list_recurring_rules` y
  `list_categorization_rules` (own-user, SIN `view` — el endpoint tampoco lo acepta),
  `list_transaction_months`, `list_snapshots` (`year`, `kind`, `include_items` opt-in default
  false), `list_transaction_imports`. Paridad byte a byte pinneada en
  `new_read_tools_match_http_endpoints`.
- **`simulate_projection` (what-if puro, issue #2)**: simula baseline + escenario con overrides y
  devuelve KPIs (`jubilacion_month_index`, `final_net_worth`, `fire_target_base`, runway) +
  `deltas`; series decimadas opt-in (`include_series`). Desde **3.8.0** cada lado añade la salud
  financiera del **mes 1**: `income_monthly`, `expense_total_monthly`, `debt_service_monthly`,
  `net_monthly` y `savings_rate` (6 dp, misma precisión que `/v1/summary`), con sus cuatro deltas.
  Cuesta **cero simulaciones extra** — son valores que ya vivían en el `ProjectionInput` de cada
  lado y no se serializaban, y esa ausencia obligaba a calcular el impacto a mano desde el chat.
  **Definiciones, que no son las ingenuas**: `expense_total_monthly` = `expense_regular_monthly +
  debt_service_monthly`, la misma base que alimentan el runway y el target FIRE — en modo A la
  cuota de pasivo vive fuera de `expense_regular_monthly` por diseño (`budget.rs`) y entra por el
  servicio de deuda, así que la suma es lo único que cuadra con `expense_total_monthly_equivalent`
  de `/v1/summary` en los tres modos. Y `net_monthly` = `income − expense_total`, que **no** es el
  `net_cash_month` que reparte la cascada: ese lleva además el tramo de planning flows del mes en
  curso. `savings_rate_delta` se recalcula desde los componentes exactos, no restando los dos
  ratios ya redondeados. Identidades pinneadas en
  `sim_kpis_match_summary_financial_health_in_all_three_modes`. Overrides: `one_off_expense`
  (`amount` + exactamente uno de `month_index`/`date`; mismo mapeo fecha→mes que un planning flow
  real), `extra_monthly_expense` (gasto REAL: entra antes del target/caps vía `SimOverrides`
  dentro de `build_installation_projection_input`), `extra_monthly_cash_adjustment` y
  `extra_monthly_savings` (NEUTROS: mecanismo planning-adjustment, no mueven target ni caps),
  `swr_pct` / `annual_inflation_percent` / `retirement_annual_expense` (re-validados con las
  cotas del PATCH real), `asset_return_overrides` (negativos válidos hasta −100 exclusivo),
  `months` 12..840. **Cache-neutral por construcción**: usa `resolve_projection_context` +
  `build_…` + doble `spawn_blocking`, nunca `projection_series_cached`. No persiste nada.
  Regresión: `apps/api/tests/mcp_simulate.rs`.
- **`get_allocation_resolution` (3.8.0, issue #4)**: la cascada resuelta del mes (read-only, cache
  **NONE**). Cierra el _stretch_ pendiente del issue #2 («euros resueltos del mes 1 por regla +
  cuánto acaba en `surplus_cash`») y el hueco de observabilidad que hacía imposible auditar la
  cascada desde el chat. Paridad byte a byte con el GET en
  `get_allocation_resolution_matches_http_endpoint`.
- **`update_transactions` (3.8.0, issue #4)**: reclasificación en lote (1..=200 ids propios) de
  `kind` / categoría / notas. Sin preview/confirm — son ids que el llamante acaba de enumerar
  (criterio del skill §4.5) — pero `destructive_hint = true` e `idempotent_hint = true`. Devuelve
  `resumen` de hasta 20 movimientos + `resumen_truncated`. Cache **COND**, una sola vez por lote.
- **`apply_categorization_rule` (3.8.0, issue #4)**: backfill de una regla sobre el histórico —
  `rule_id`, `apply_to_existing` (`uncategorized` default | `all`), `from_month`, `confirm`. Sin
  `confirm` devuelve preview con `would_match` / `already_correct` / `would_change_kind` /
  `skipped_by_source` / `matched_by_other_rule` / `skipped_reconciled` / `by_current_category` /
  `sample` y el aviso `moves_projection_in_modes_b_and_c`. Cache **COND**. Annotations:
  `destructive_hint = true`, `idempotent_hint = true` — declaradas **a conciencia** en
  `tools_list_exposes_annotations_on_every_tool`, porque el resto del catálogo las deriva del
  prefijo del nombre y `apply_` es un verbo nuevo.
  **Omisión deliberada asociada**: la tool `create_categorization_rule` **no** expone
  `apply_to_existing` (el body HTTP sí, para el round-trip único de la SPA). Dos razones: en el
  momento del preview la regla todavía no existe, así que no hay nada que simular; y un `create_*`
  capaz de reescribir cientos de filas haría mentir a sus propias annotations, que es lo que el
  cliente MCP usa para decidir si pide permiso al humano. Desde el chat: crear y luego aplicar, con
  un único gate de confirmación.
- **Tool annotations**: toda tool declara `annotations` (macro `#[tool(annotations(...))]` de
  rmcp): `title` legible, `open_world_hint = false` (el servidor solo toca su propia DB) y
  `read_only_hint = true` en las lecturas. Sin ellas un cliente conforme al spec asume el peor
  caso (escritura destructiva). Test: `tools_list_exposes_annotations_on_every_tool`.
- **Cero deriva handler↔tool**: cada tool llama a la MISMA core fn que el endpoint HTTP
  (`summary_core`, `projection_series_cached`, `budget_snapshot_core`, `transactions_summary_core`,
  `list_transactions_core`, `history_series_core`, `list_assets_core`, `list_liabilities_core`,
  `list_planning_flows_core`, `installation_access_core`) y serializa el mismo struct serde →
  Decimal-as-string intacto. Paridad congelada en `apps/api/tests/mcp_http.rs`.
- **Errores**: dominio/validación → `CallToolResult{is_error:true}` con el JSON `{error, message}`
  de `ErrorBody`; `Db`/`Unavailable` → `ErrorData::internal_error` sanitizado (detalle a tracing).
- **Tools de escritura (issue #3)** — todas pasan primero por `require_mcp_write` (mcp/auth.rs:
  `role_can_write` con el rol vivo + kill-switch `installation.mcp_write_enabled` leído por
  request; viewer → `forbidden`, toggle apagado → `bad_request` con prefijo `mcp_write_disabled:`),
  llaman a la MISMA core fn de mutación que su handler HTTP (la invalidación de cache vive DENTRO
  de la core, post-commit) y devuelven respuestas compactas `{id, resumen}`. Tramo 1:
  `create_transaction` (con `recurring` opcional; reenvíos idénticos crean OTRO movimiento —
  ordinal de huella, mismo contrato que HTTP), `update_transaction` (owner-guard → `not_found`),
  `capture_snapshot` (upsert por día civil — sobrescribe), `materialize_recurring` (idempotente
  por cursor), `create_planning_flow` / `update_planning_flow` (tri-state `clear_due_date`),
  `create_category`, `create_categorization_rule` (solo imports futuros; conflict con `source`
  concreto duplicado). **Contrato de cache por tool**: COND (`invalidate_projection_if_savings_
  uses_transactions`, solo modos B/C) = transaction C/U + materialize; NONE = capture_snapshot
  (D12), create_category, create_categorization_rule; FULL (`refresh_projection_after_mutation`)
  = planning C/U. Tramo 2: `update_asset_value` (subset current_value + retorno esperado con
  before/after; sin owner-check — contrato del ledger), `create_asset`, `create_liability`
  (principal explícito o `derive_principal_from_plan`), `create_budget_entry` /
  `update_budget_entry` (exclusión `ends_at_retirement` ⊕ `expense_end_date`),
  `update_allocation_rule` (subset amount/cap/enabled — sin create/delete/reorder; la invariante
  del sink vive en la core compartida) y `delete_recurring_rule` (**estrena el patrón
  preview/confirm**: sin `confirm: true` la tool devuelve `{preview, confirm_required, action,
  effects}` como ÉXITO — para un LLM el preview es información, no fallo — y solo ejecuta con
  confirm; NONE). Todos los anteriores excepto delete_recurring_rule invalidan FULL. La capa API
  valida además `expected_annual_return_percent > −100` en create/patch de assets (el engine
  clampa ≤ −100 a pérdida total). Tramo 3 (destructivas, todas preview/confirm):
  `delete_transaction` (preview = el movimiento completo; owner-guard), `delete_planning_flow`,
  `delete_budget_entry`, `delete_asset` (preview con contadores de desvinculación:
  `linked_asset_id`/`account_asset_id` → SET NULL), `delete_liability` (ídem
  `linked_liability_id`), `delete_snapshot` (preview con `items_deleted`; NONE), `delete_import`
  (preview con `transactions_deleted`; cascada; COND — mismo contrato que el `?confirm=true`
  HTTP) y **`update_fire_settings`** (SOLO owner; merge campo a campo vía
  `patch_fire_settings_core` — jamás deserializa a `FireSettings` con su `#[serde(default)]` a
  nivel de struct, el bug del reset silencioso; sin confirm devuelve `{before, after}` validado
  incluyendo `annual_inflation_assumption_percent`; FULL). Conciliación (3.5.0):
  `reconcile_transfers` (pase explícito, idempotente — `reconcile_now_core`; COND solo si enlaza
  algo) y `unreconcile_transfer` (rompe el par + rechazo persistido — `unreconcile_core`; COND;
  sin preview/confirm: no son destructivas). `reconcile_pair` manual se omite a conciencia
  (footgun para un LLM; el registro de omisiones deliberadas vive en la skill
  `futurefin-mcp-parity`). Paridad CRUD del ledger (tras 3.5.0): `update_asset` (body completo
  del PATCH vía la misma `patch_asset_core` — rename, categoría, liquidez, precio de compra con
  `clear_purchase_price` materializando el null del tri-state; `update_asset_value` queda como
  subset de valoración) y `update_liability` (cerraba la única asimetría create/delete-sin-update
  del catálogo, que empujaba al agente al borrar-y-recrear destructivo; merge campo a campo vía
  `patch_liability_core` extraída del PATCH, re-derivación del principal si
  `derive_principal_from_plan` queda activo). Ambas FULL, sin preview/confirm (editar no
  destruye filas). Catálogo total: **47 tools** (20 con `read_only_hint = true` + 27 de
  escritura; recuento reproducible: `grep -c '#\[tool(' apps/api/src/mcp/server.rs`), congelado
  en `tools_list_returns_exactly_the_v1_catalog`. Regresión:
  `apps/api/tests/mcp_write.rs`.
- **NO está en OpenAPI a propósito**: no es un recurso REST — es JSON-RPC cuyo contrato define la
  spec MCP y que se autodescribe vía `tools/list`.
- **Paridad con la API HTTP (norma)**: el catálogo de arriba es superficie derivada de la API —
  cualquier cambio en rutas/handlers obliga a pasar la evaluación de paridad MCP ANTES de
  mergear (¿tool nueva/actualizada, u omisión deliberada registrada?). El criterio de decisión,
  el recipe de añadir/actualizar una tool y el registro de omisiones y gaps pendientes viven en
  [`futurefin-mcp-parity`](skills/futurefin-mcp-parity/SKILL.md); la gate está en
  `futurefin-change-control` §1 (clase "API contract").
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
| GET | `/.well-known/oauth-authorization-server[/mcp]` | RFC 8414. `issuer`, `authorization_endpoint` (`{base}/oauth/authorize`), `token_endpoint`, `registration_endpoint`, `revocation_endpoint`, `code_challenge_methods_supported: ["S256"]` (único), `grant_types_supported: [authorization_code, refresh_token]`, `authorization_response_iss_parameter_supported: true`. **Sin `scopes_supported`** a propósito: el acceso no se granula por scopes sino por el rol vivo del usuario + el toggle `installation.mcp_write_enabled` (las tools de escritura lo comprueban por request) — un scope congelado en el token sería MENOS revocable que el gate vivo. |
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
