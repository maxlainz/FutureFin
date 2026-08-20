# Tests

Test setup post-refactor (May 2026). Before: 22 engine unit tests, nothing else. Ahora hay suites de engine (unit), API (integración contra Postgres real) y frontend (Vitest). No congeles totales en docs — cuéntalos con: `cargo test --workspace 2>/dev/null | grep "test result"` y `npm test --workspace futurefin-web 2>&1 | grep Tests`.

> **3.0.0 (2026-08-16) no cambia nada de esta página salvo la fila `docker-stack` de la matriz de CI.** La imagen de producción ahora lleva PostgreSQL embebido, pero la base de tests sigue siendo el contenedor **`ff-test-db` en el puerto 5433**, por TCP, con un schema `ff_test_<uuid>` por test. **No** apuntes la suite al Postgres embebido de un contenedor `futurefin` en marcha: contiene datos reales y no expone TCP (solo socket Unix).

## TL;DR

```bash
# Backend (engine + integration)
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace

# Frontend
npm test --workspace futurefin-web
```

`TEST_DATABASE_URL` admite además la forma **socket Unix** de libpq —
`postgres:///futurefin?host=/ruta/al/sock&user=futurefin` — soportada por sqlx 0.8 y verificada
corriendo la suite completa (es exactamente la forma que el entrypoint 3.0.0 usa para enchufar la
API al Postgres embebido). Útil si ya tienes un Postgres local por socket y no quieres publicar
puerto; el default documentado sigue siendo el TCP de 5433.

## Backend

### Engine unit tests (`crates/engine/src/{projection.rs,history.rs,runway.rs}`)
- `projection.rs` `mod tests`: 22 tests covering cascade allocation, retirement drain, FIRE target inflation, off-by-one between `fire_target_at_month_index(k+1)` and the handler's series.
- `history.rs` `mod tests`: 21 tests (v1.5.0 + cash-flow tier-2) covering linear interpolation (midpoint exact, endpoints), the French-amortization curve (matches a pure schedule when the residual is 0, residual correction passes through both endpoints, midpoint above the linear chord, fallbacks when payment ≤ interest / no terms / apr=0, clamp ≥ 0), timeline rules (item absent from an intermediate snapshot = 0 between pairs, appears/disappears when present in only one, first-month clamp, virtual-today join with a deleted item → 0), and `month_index_of` / `add_months_signed` with negative deltas across a year boundary.
- `runway.rs` `mod tests`: 13 tests (v2.3.0; 8 in v2.2.0) covering `liquid_runway_months` — exact reduction to `A/g` with no return/inflation (no tolerances), return extends / inflation shortens, `Indefinite` via the **SWR threshold** (`withdrawal_within_swr_is_indefinite`, renamed from `return_covering_expense_is_indefinite`), value-weighted multiplier (vs the naive per-asset mean), negative return **shortens** the runway (losses compound since the `monthly_multiplier` fix — before it behaved as zero growth), `NoExpenseBase` with zero expense (which also pins the check *order* against the threshold), `Months(0)` with zero balance. The five added in v2.3.0: `swr_threshold_exact_equality_is_indefinite` (exact `Decimal` boundary, 300.000 @ 4 %), `just_below_swr_threshold_is_finite` (one euro under → finite, `A/g` intact), `swr_zero_never_indefinite` (also defensive with `swr < 0`), `cap_reached_without_swr_is_months_at_cap` (surviving the cap is the `Months(1200)` **floor**, not `Indefinite`), `grossed_expense_raises_threshold` (the 5th parameter participates; the engine never recomputes `12 × monthly_expense`). Predicted values in each test's doc comment.
- Engine total: **61** as of 2026-08-18 (27 + 21 + 13; los 5 nuevos cubren `monthly_multiplier` con tasas negativas — composición, clamp ≤ −100, pin de las positivas y decaimiento en simulación) — recuéntalo con `cargo test -p futurefin-engine 2>&1 | grep "test result"` o, sin compilar, `grep -c '#\[test\]' crates/engine/src/{projection,history,runway}.rs`.
- Pure: no Postgres, no env. `cargo test -p futurefin-engine` runs both (and they run in **CI**, unlike the integration suite).

### Integration tests (`apps/api/tests/`)
- Each test spins up the full Axum router (`routes::app_router()`) and drives it via `tower::ServiceExt::oneshot` against a real Postgres.
- **Schema-isolated per test**: `common::isolated_pool()` creates `ff_test_<uuid>`, sets `search_path`, applies every migration in `apps/api/migrations/` (count them with `ls apps/api/migrations | wc -l`), returns the pool. Schemas are leaked intentionally — drop them with `psql -c "DROP SCHEMA ff_test_<id> CASCADE"` or wipe the test DB.

### Test infrastructure (`apps/api/tests/common/mod.rs`)
- `TestApp::spawn() -> TestApp { router, pool, schema, state }` — fresh schema + axum router wired with cookie cookies. Los cuatro campos son `pub`, lo que permite construir un `TestApp` a mano con otro `AppState` (es como se prueba el kill-switch, ver abajo).
- Convenience methods on `TestApp`:
  - `register_and_login_owner("alice") -> LoggedInOwner { username, cookie, user_id }` — first user becomes owner via bootstrap.
  - `register_and_approve_member("bob") -> LoggedInOwner` (v1.5.0) — registers a second user and has the owner approve them as a writable member; used by the household-aggregation and cross-user tests (`history_series.rs`, `history_snapshots.rs`).
  - `create_category(&owner, "asset", "Bolsa") -> id`
  - `count_rows("liabilities") -> i64` — query against the test schema.
  - `get(uri)`, `get_with_cookie(uri, cookie)`, `post_json(uri, body)`, `post_json_with_cookie`, `patch_json_with_cookie`, `delete_with_cookie` — return `ResponseParts { status, headers, body: Vec<u8> }`.
  - `ResponseParts::json()` parses body as `serde_json::Value`; `.session_cookie()` extracts `ff_session=…` from Set-Cookie.
- **Helpers añadidos en v3.1.0** (los estrena `oauth_flow.rs`; el token endpoint OAuth habla `form`, no JSON):
  - `post_form(uri, form: &[(&str, &str)])` — POST `application/x-www-form-urlencoded`, percent-encodeando clave y valor con el `urlencode` privado del módulo (set no reservado `A-Za-z0-9-_.~`). Para `/oauth/token` y `/oauth/revoke`.
  - `post_form_with_basic_auth(uri, form, client_id, secret)` — igual + `Authorization: Basic base64(client_id:secret)` (base64 **estándar con padding**, no URL-safe), para `client_secret_basic`.
  - `get_with_headers(uri, headers: &[(&str, &str)])` — GET aplicando headers verbatim. Es la vía para inyectar `x-forwarded-host` / `x-forwarded-proto` / un `Host` malformado y probar la derivación del issuer.
  - `mcp_initialize(bearer: Option<&str>)` — POST `initialize` mínimo a `/mcp` (protocolVersion `2026-07-28`, headers `MCP-Protocol-Version`/`Mcp-Method`), con `Authorization: Bearer …` solo si `bearer` es `Some`. 200 = credencial válida, 401/403 = rechazada. Es el helper más usado de `oauth_flow.rs`. **Trampa**: `api_tokens.rs` conserva su propio duplicado privado del mismo nombre (protocolVersion `2025-06-18`, sin esos headers) — el helper nuevo **no** lo sustituyó; si tocas uno, mira el otro.
- **`request()` inyecta `Host: futurefin.test` si falta** (insert-if-absent, así que un `Host` explícito de `get_with_headers` gana):
  ```rust
  if !req.headers().contains_key(http::header::HOST) {
      req.headers_mut().insert(
          http::header::HOST,
          http::HeaderValue::from_static("futurefin.test"),
      );
  }
  ```
  **Por qué**: `tower::ServiceExt::oneshot` no pasa por la capa HTTP/1.1, así que no sintetiza `Host`. Los endpoints OAuth derivan el origen público del request (`oauth/url.rs::public_base_url`, con `AppState.public_url = None` en tests) → sin `Host` devolverían un **400 irreal** y cualquier test que roce `/oauth/*` o `/v1/oauth/*` fallaría por el motivo equivocado. El literal `"futurefin.test"` está en **dos** sitios que deben coincidir (`request()` y `mcp_initialize()`) y es el origen que asertan las expectativas de `oauth_flow.rs` (`issuer == "http://futurefin.test"`).
- **Kill-switch: sin `set_var`.** Ni `oauth_flow.rs` ni `mcp_http.rs` tocan el entorno. `FUTUREFIN_MCP_ENABLED=0` se simula construyendo el router a mano con `AppState::new(…, /*mcp_enabled*/ false, /*public_url*/ None)` y montando un `TestApp` literal (patrón de `mcp_http.rs::mcp_disabled_returns_404`). `AppState::new` ganó en 3.1.0 un 6º parámetro `public_url: Option<String>`; `spawn()` pasa `None` **a propósito**, para que el issuer siempre se derive del request y `FUTUREFIN_PUBLIC_URL` no haga falta en tests.

### Tests checked in
| File | Covers |
|---|---|
| `smoke.rs` | health/ready, 401 on unauth, register→login→me, first-user bootstrap |
| `liabilities_purge.rs` | expired liabilities hidden from GET listings + summary totals but **persist in DB**; since 3.4.0 also `projection_excludes_expired_liability_principal` (la proyección filtra vencidos: `starting_net_worth == summary.net_worth`), `liability_create_requires_expense_category` (obligatoria + scope expense) y `expense_category_remap_and_set_null` (remap la arrastra; borrado sin refs no se bloquea → NULL) |
| `budget_liability_quotas.rs` (10) | Cuotas de pasivo dentro de `GET /v1/budget` (renombrado desde `budget_derived.rs` en 3.7.0, cuando la cuota pasó a ser una partida más de `entries`): forma de la partida (`source`, `liability_id`, `label`, categoría de gasto) y convivencia con la partida manual de la misma categoría; totales (`expense_regular` = suma de los `entries` de gasto, sin `expense_derived`); la cuota fuera de `expense_retirement_*`; **la cuota fuera de la base de gasto del engine** (`monthly_delta_assumption`, regresión del doble conteo con cifras predichas a mano); predicado de pasivo activo (fecha fin NULL SÍ, vencido no, borde `>=`, sin plan no), semanal ×52/12, scoping household/mine, y la cuota sin `expense_category_id` que sigue sumando |
| `body_limits.rs` | 1 MB cap on normal endpoints (413), 16 MB cap on `/backup/user-import` |
| `installation_patch.rs` | unknown `fire_number_mode` rejected (422); legacy `annual_expense_adjusted` alias still accepted |
| `unique_violation.rs` | duplicate username + duplicate category name → 409 via central `From<sqlx::Error>` |
| `projection_marker.rs` | `compound_outpaces_true_savings_month_index` stable across the perf refactor (regression for spawn_blocking + tokio::join) |
| `fire_parity.rs` | **FIRE target parity** — for each case in `fixtures/fire-parity.json`, seeds installation + budget + assets and asserts `jubilacion_target_net_worth` matches the canonical expected value (± 1 €). |
| `projection_cache.rs` | Cache de proyección: hit tras GET, invalidación tras mutación, aislamiento por vista/densidad, `?months=` bypassa el cache. |
| `history_snapshots.rs` (20) | Snapshots CRUD: captura con términos copiados, upsert mismo día reemplaza items, excluye filas compartidas/expiradas, backfill roundtrip con filtro `year` y cascade, validaciones 400 (futuro, `duplicate_item_id`, términos en asset), 409 fecha ocupada, 404 cross-user, 403 viewer en toda mutación, GET nunca muta, y `snapshot_mutations_do_not_touch_projection_cache` (la cache de proyección sigue HIT — history NO es input del engine). **Prefill** (`GET /v1/history/snapshots/prefill`, v1.5.1, ~7): interpolación idéntica a la serie, `first_snapshot`, `live`, `not_owned` (0 + `existed:false`), validaciones (fecha futura / `invalid_kind` → 400), viewer. |
| `history_series.rs` (7) | `GET /v1/history/series`: vacío→200, interpolación lineal exacta entre dos snapshots de asset, join a valores vivos (asset borrado→0 en k=0), curva de amortización por encima de la cuerda con extremos exactos, household suma dos usuarios + `?view=mine` filtra, markers con fecha/kind/total, snapshot único de hoy. Números predichos antes de ejecutar. |
| `backup_user_roundtrip.rs` (13) | `.ffbackup` v4/v5/v6: roundtrip con serie histórica idéntica, re-link de items a los UUIDs frescos de assets, `ledger_index` null conserva `item_key`, v3 sigue importando (0 snapshots), índice fuera de rango → 400 con rollback, import invalida la cache de proyección (fix del bug preexistente), preview reporta counts de snapshots/items, viewer 403, **v5** (v1.6.0): roundtrip de transactions/imports/rules con re-link por índice y `fingerprint_ordinal` preservado, y **v6** (v1.8.0): roundtrip de `recurring_transaction_rules` con `recurring_rule_index` re-enlazado y `last_materialized_month` preservado. **v8** (3.5.0): roundtrip de pares conciliados + rechazos (aserción anti-resurrección tras el restore) y un v7 importa limpio con el pase retro post-import re-conciliando sus pares. |
| `transactions_import.rs` (15) | Import CSV: autodetección MyInvestor/N26 por cabecera, preview marca `already_imported` y los omite por defecto, confirm inserta con ordinales, re-confirm mismo archivo → 0 nuevos, `force` añade ordinal nuevo, heurística de transferencia interna (desde 3.5.0 **hint informativo**: el default importa TODAS las filas y el confirm reporta `reconciled_pairs`), regla aprendida pre-asigna en el siguiente preview, no-EUR rechazado en confirm, viewer 403, sha preview↔confirm distinto → 400. **Fold de acentos** (post-2.0.0): `savings_hint_accent_insensitive_*` (hint de ahorro con «Aportación…» con/sin cartera), `learned_rule_matches_accent_insensitive*` (regla acentuada matchea concepto sin tilde y viceversa), precedencia regla-aprendida vs hint. |
| `transactions_crud.rs` (26) | CRUD de movimientos: alta manual individual/batch, `savings` exige categoría NULL, validación de scope income/expense, **PATCH de importadas edita op_date/amount/concept con huella anclada al CSV** (`patch_imported_fields_editable_fingerprint_anchored`, antes `patch_imported_op_date_is_immutable`; ya no hay `immutable_field`) y en manuales recomputa la huella liberando el ordinal (`patch_manual_op_date_recomputes_and_allows_reuse`), borrar asset/liability vinculado → SET NULL conservando el movimiento, remap al borrar categoría, viewer 403. |
| `transactions_summary.rs` (15) | `GET /v1/transactions/summary`: números Decimal exactos por categoría (real/budget/avg), **promedio ponderado** (denominador = `months_with_data`, no el nº de meses del tramo → historial corto no diluye a 0), ventanas `avg_window` 3/6/12/`ytd`/`all` + alias legado `avg_months`, `avg_window` inválido → 400, mes parcial marcado, savings excluido del gasto, bucket «Sin categoría». **Ya no** hay línea derivada de cuotas de pasivo: `totals.expense_budget` = Σ budget de categorías de gasto. 3.5.0: +3 de conciliación — par conciliado fuera de los totales del mes (y vuelve al desconciliar con neto invariante), mes solo-conciliadas fuera de `months_with_data`, serie por categoría excluye conciliadas. |
| `transactions_projection_cache.rs` (9) | Contrato de cache **condicionado al modo** (`fire_settings.savings_source`): `mode_a_mutations_do_not_touch_projection_cache` (modo `budget`: la cache sigue HIT tras import/create/edit/delete y los endpoints recurrentes — transacciones NO son inputs; la «regla» de esta batería es la **recurrente**, no la de categorización), `mode_b_each_mutation_invalidates_projection_cache` (modo `transactions_avg`: cada mutación invalida), `mode_c_mutation_invalidates_projection_cache` (modo `budget_income_real_expense`: paridad con B sobre **create + patch + delete**) y `flipping_savings_source_invalidates_projection_cache` (cambiar de modo invalida). 3.5.0: +2 para las rutas de conciliación (modo A no invalida; modo B sí, y un pase sin pares nuevos no tira la cache caliente). 3.8.0: `creating_a_categorization_rule_never_invalidates_projection_cache` cierra el hueco que tres documentos daban por pinneado sin estarlo — crear una regla solo hace INSERT, así que **no invalida en ninguno de los tres modos**; el backfill retroactivo es otra ruta y sí invalida COND. |
| `transactions_reconcile.rs` (19) | **Conciliación de transferencias (3.5.0)**: pase automático (ventana ±5 días con borde exacto 5/6, par cross-import — el caso del bug —, greedy determinista con candidatos múltiples, punto fijo/idempotencia, savings participa, owners distintos jamás), desconciliar persiste rechazo anti-resurrección, PATCH de `amount`/`op_date` rompe el par SIN rechazo (revertir re-empareja), borrar pata/lote desconcilia a la superviviente, conciliación manual sin ventana (+ 400 `transfer_amounts_not_opposite`/`already_reconciled`/`not_reconciled`/`transfer_same_transaction`), viewer 403 y owner-guard 404. |
| `allocation_resolution.rs` (3) | **Cascada resuelta (3.8.0)**, `GET /v1/allocation-rules/resolution`: identidades `base_cash = recurring_net + planning_component` y `Σ per_asset + leftover = base_cash` con números predichos (3000/1000 → 2000; fijo a un activo ya en su techo → `cap_full` con `cap_room` 0; 40 % → 800; sumidero → 1200); un planning flow SIN fecha activa `base_includes_transient` y el tramo resulta múltiplo exacto de 900/90 = 10 €/día, cuadrando con la diferencia `contribution_nominal_monthly − contribution_recurring_monthly` de `/v1/assets`; y las reglas tras el corte por caja se reportan `not_reached` (distinto de `no_cash`) mientras la que agotó la caja aparece **recortada** (intent 500, resolved 200) y no saltada. |
| `savings_source.rs` (23) | Toggle `savings_source` observado vía la proyección (`monthly_delta_assumption`, `?months=240` para saltar la cache). Serde/PATCH: default `budget`, roundtrip de `transactions_avg` y `budget_income_real_expense`, valor desconocido → 422 listando las 3 variantes. **Modo B** (`transactions_avg`): promedio ponderado excluye `savings` y el mes parcial, `months_with_data==0` → fallback a presupuesto, promedio CRUDO ignora vínculos y cuotas de liabilities (reforma 3.4.0), resta estática de NW (`mode_b_liability_static_nw_subtraction`: NW(k) = k·delta − principal, sin amortización ni escalones), sin step-up al vencer el plan (`mode_b_no_step_up_at_liability_end`, pin del coste aceptado), target `annual_expense` usa `expense_avg`, scoping household/mine, `GET /v1/assets` sigue el modo. **Meses reales**: `pseudo_empty_month_excluded_from_avg` (mes real 2000 + mes solo-recurrente 3000 → months=1, avg=2000), `real_month_counts_recurring_too` (un mes real cuenta su recurrente → avg 5000), `mode_b_all_pseudo_empty_falls_back_to_budget` (backfill entero → 0 meses reales → presupuesto). **Modo C** (`budget_income_real_expense`): `mode_c_income_budget_expense_real` (income presupuesto 5000 − gasto real 800 = 4200), target `annual_expense` usa `expense_avg`, target `current_income` usa el income del presupuesto, `months==0` → fallback. **v2.2.0**: `assets_cap_targets_follow_savings_source_mode` (los caps `months_expense`/`income_multiple` de `GET /v1/assets` valen 18.000/10.000 en modo A y 6.000/8.000 en modo B — falla contra el código anterior) y `projection_series_reports_effective_savings_source` (los dos campos nuevos de `/v1/projection/series`: budget/0, fallback budget/0 y transactions_avg/1). Números predichos antes de ejecutar. |
| `summary_savings_source.rs` (6) | `GET /v1/summary` `financial_health` siguiendo `savings_source`: modo A = presupuesto, modo B usa el promedio real crudo (cuotas dentro del gasto, `expense_derived = 0`), `months==0` → fallback, scoping household/mine, `mode_b_summary_pseudo_empty_month_excluded` (meses solo-recurrentes no cuentan en `savings_source_months_with_data`) y `mode_c_income_not_overwritten` (en modo C `income_monthly_equivalent` conserva el income del presupuesto). |
| `summary_runway.rs` (10) | `financial_health` de `GET /v1/summary`: **captura de regresión previa a 2.2.0** que sigue verde después (`runway_months == liquid_assets_total / expense_total` exacto en modo A sin retorno ni inflación; `expense_derived` = cuotas activas; sin gasto el campo `runway_months` **no se serializa**), más el comportamiento nuevo — la rentabilidad alarga (12.000 al 5 % con gasto 1.200 → >10 y <11), la inflación acorta (3 % → <10 y >9), `runway_indefinite_when_withdrawal_within_swr` (renombrado desde `..._when_returns_cover_expense`; 1M al 7 % vs 1.000 €/mes → `runway_months` null + flag true), modo B con las identidades `expense_total = expense_reg + expense_der` y `net = income − expense_total` y runway sobre la base efectiva (16.000/1.600 = 10; en modo A serían 16.000/8.800), y el fallback `months_with_data == 0` → bloque idéntico al de modo A. Los tres añadidos en v2.3.0 cubren el **umbral SWR** extremo a extremo: `runway_indefinite_at_exact_swr_threshold` (taxes off, frontera exacta 240.000 € / 700 €/mes / SWR 3,5 %), `runway_gross_up_raises_threshold` (270.000 € / 700 €/mes: el flag se invierte al activar `taxes_enabled`, probando que el handler pasa el gasto grosseado) y `runway_swr_zero_never_indefinite` (con `swr_pct = 0` el flag es false y `runway_months` viaja con el suelo 1200). Números predichos en el comentario de cada test. |
| `transactions_recurring.rs` (16) | Movimientos recurrentes (v1.8.0): alta con `recurrence` crea regla + instancia de origen enlazada, `materialize` idempotente (2ª llamada → 0), no genera `op_date` futuro (mes en curso solo si el día ya llegó), clamp de `day_of_month` a fin de mes, borrar una instancia NO la recrea al re-materializar (cursor), `DELETE` de regla conserva las instancias (SET NULL), viewer 403 en materialize/delete, `recurrence.day_of_month` fuera de rango → 400. **Backfill del alta** (post-2.0.0): `create_with_past_date_backfills_instances` (fecha pasada rellena los meses intermedios en el mismo commit), `recurrence_op_date_within_bound_created`, y la cota `recurrence_op_date_too_old_*` → 422 `recurrence_too_old`. |
| `history_cashflow.rs` (6) | `GET /v1/history/cashflow`: agregados mensuales exactos (Decimal-string, household y mine), la serie fina pasa por los snapshots, **`/v1/history/series` idéntico byte a byte con y sin transacciones** (regresión tier-1), `daily` con ventana >6m → 400, `fine` ausente sin vínculos. 3.5.0: `reconciled_excluded_from_months_but_not_from_fine_curve` fija la **asimetría deliberada** — `months[]` excluye conciliadas, la curva fina NO (mismos 1187.5/750 que el caso sin conciliar). |
| `api_tokens.rs` (8) | Tokens de API (v3.0.0, `/v1/api-tokens` + gate Bearer de `/mcp`): 201 con secreto `ffp_` una sola vez y `token_prefix` coherente, listado sin secreto/hash, revocado y expirado → 401 (con `WWW-Authenticate`), Bearer malformado/prefijo ajeno/secreto random → mismo 401, usuario pending → 403 al crear, viewer puede crear y su token autentica, aislamiento entre usuarios (listado y DELETE ajeno → 404), validaciones 400 (label/`expires_in_days`) y límite de 10 activos (`token_limit_reached`) que se libera al revocar. |
| `mcp_write.rs` (19) | Tools MCP de escritura (tramo 1): viewer → `forbidden`; **toggle vivo** (`PATCH mcp_write_enabled=false` por cookie corta la siguiente escritura MCP con `mcp_write_disabled`, las lecturas siguen, re-activar la devuelve — sin reinicio); la fila creada por MCP es indistinguible por HTTP y los reenvíos idénticos crean otro movimiento (ordinal, contrato HTTP); **contrato de cache por el camino MCP** (modo A no invalida, `capture_snapshot` NONE, `create_planning_flow` FULL, modo B sí invalida — helpers `warm`/`assert_invalidated` espejo de `transactions_projection_cache.rs`); recurrencia con backfill en el alta + `materialize` idempotente; conflicts de categoría y regla (source concreto); tri-state `clear_due_date` y 400 compartidos con HTTP. Tramo 2: create/update de asset (FULL + cota `> −100`), liability con principal derivado, budget (FULL + exclusión mutua), `update_allocation_rule` respeta la invariante del sink (`remainder_required` vía MCP) con before/after, y `delete_recurring_rule` estrena preview/confirm (el preview no borra; confirm borra la plantilla y las instancias sobreviven; repetir → `not_found`). Tramo 3: `update_fire_settings` cambia solo `swr_pct` y los `tax_brackets` personalizados sobreviven (el test que caza el reset del `#[serde(default)]`), preview no persiste, cotas re-aplicadas, member → `forbidden` (gate Owner) y cambiar `savings_source` invalida FULL; los deletes destructivos hacen preview con efectos (SET NULL contado en asset, `items_deleted` en snapshot, `transactions_deleted` en import) y solo ejecutan con confirm (cascada del lote verificada). Paridad CRUD (post-3.5.0): `update_asset_and_update_liability_share_cores_and_invalidate_full` — body completo de asset (rename + recategorizar + `clear_purchase_price` borra el precio, contradicción con `purchase_price` → 400), edición de liability sobre la MISMA fila (TAE + plan, `patch_liability_core`), ambas FULL, el 400 «at least one field» compartido con el PATCH y el toggle cortándolas en vivo. **3.8.0**: cuarteto de `apply_categorization_rule` (preview no escribe ni invalida, confirm reescribe y en modo C invalida, el toggle corta y un viewer no puede) y de `update_transactions` (lote indistinguible del PATCH HTTP, todo-o-nada con un id inventado dejando cero filas y el error nombrándolo, COND en modo B). |
| `mcp_simulate.rs` (7) | Tool `simulate_projection`: baseline sin overrides ≡ `get_projection` (jubilación + patrimonio final del último punto, tolerancia f64) y escenario ≡ baseline con deltas 0; el **par discriminador** `extra_monthly_expense` (mueve `fire_target_base` y retrasa la jubilación) vs `extra_monthly_cash_adjustment` (target intacto) vs `extra_monthly_savings` (adelanta la jubilación); `one_off_expense` por `date` ≡ por `month_index` equivalente + series opt-in; override de retorno −30 % hunde el patrimonio final (post-fix del engine); cotas (swr 5, months 6, asset ajeno, retorno ≤ −100, negativos, one_off malformado, inflación 60, retirement 0 → `bad_request` con mensaje); y **neutralidad de cache** (simular no crea entradas ni toca las existentes). **3.8.0**: `sim_kpis_match_summary_financial_health_in_all_three_modes` — los KPIs de salud del mes 1 (income, gasto total, neto, tasa) coinciden con `financial_health` de `/v1/summary` en los tres modos, con un pasivo de 400 €/mes activo; definir el gasto como `expense_regular_monthly` a secas hace fallar el modo A por esos 400 € exactos. |
| `mcp_http.rs` (18) | Flujo MCP end-to-end sobre `/mcp` (stateless 2026-07-28 con headers SEP-2243 `Mcp-Method`/`Mcp-Name` + `_meta` por request): initialize → `serverInfo.name == "futurefin"`, `tools/list` congela el **catálogo completo (50 tools — lectura, simulación y escritura; `tools_list_returns_exactly_the_v1_catalog`)** y asserta las **annotations** de cada una (`title` + `readOnlyHint`/`destructiveHint`/`idempotentHint` derivados del prefijo del nombre + `openWorldHint:false`), **paridad byte a byte** `get_summary` vs `GET /v1/summary` y de 8 tools de lectura nuevas vs su GET (`new_read_tools_match_http_endpoints`), `get_projection` hybrid fijo + `asset_series` opt-in + **pobla la misma cache** que el handler, `get_history` con `window_months`/`include_asset_series` opt-in, `get_settings` devuelve installation + rol + bloque `user`, `list_transactions` **pagina en SQL** (`offset` + `total_count/truncated`), `get_category_monthly_series` cero-rellena y valida `kind`, `list_snapshots` con `include_items` opt-in y `year` validado, error de validación → `is_error: true` con el JSON `{error, message}` del wire HTTP, `view: "mine"` filtra al usuario del token, y `mcp_enabled=false` → `/mcp` 404 (los tests construyen `AppState` a mano para ese caso). |
| `oauth_flow.rs` (30) | **OAuth 2.1 embebido (v3.1.0)** — la suite más grande por nº de tests. **Metadata/descubrimiento**: los 4 paths `.well-known` devuelven JSON y no el fallback SPA (si sale `text/html`, el fallback se los está tragando), `issuer`/`resource`/`authorization_servers` exactos, el issuer sigue `x-forwarded-proto`/`x-forwarded-host` y un `Host` malformado → 400. **`WWW-Authenticate`**: el 401 de `/mcp` anuncia `resource_metadata="…/oauth-protected-resource/mcp"` y el **403** (token vivo cuyo usuario pierde la membership) **no lleva header** — anti-bucle. **DCR**: cliente público (`ffc_`, sin secreto) vs confidencial (`ffcs_`, `client_secret_expires_at: 0`, default `client_secret_basic` al omitir el método), 8 `redirect_uris` rechazados (http no-loopback, `ftp:`, fragmento, userinfo, relativa, >5, ausente, vacío) vs loopback con puerto dinámico aceptado, y metadata desconocida ignorada (RFC 7591 §3.1). **Canje**: happy path code → `ffo_`/`ffr_` → `/mcp` 200 con `expires_in: 3600`, `token_type: Bearer`, `Cache-Control: no-store`, `state` eco literal + `iss` (RFC 9207); verifier PKCE incorrecto, `redirect_uri` distinto al del authorize, code expirado (forzado por SQL), `client_id` desconocido → **401 `invalid_client`** (la señal de re-registro de claude), Basic con secreto malo → 401 dejando el code **vivo**. **Reuse-detection**: reusar un code o un refresh ya consumido revoca el grant entero y deja `revoked_reason` = `code_reuse` / `refresh_token_reuse` (leído de la columna), matando el access token en curso; refresh de otro cliente rechazado; access expirado → 401. **Consentimiento**: errores fatales nunca traen `redirect_to` (anti-open-redirect, y son **200** con `status: invalid_request`), `plain` PKCE y un `resource` ajeno son **redirigibles** (`error=invalid_request` / `invalid_target` con `state`), `authorize-details` funciona **sin sesión** y reporta `already_connected`, deny → `access_denied` sin crear grant, sin sesión → 401, usuario pending → 403 sin grant, re-consentir la misma app deja **1 sola fila** en `oauth_grants`. **Revocación**: el panel corta `/mcp` al instante (y el refresh), aislamiento entre usuarios (listado vacío, DELETE ajeno → 404), RFC 7009 con `ffr_` mata el grant y un token desconocido sigue devolviendo 200. **Kill-switch**: con `mcp_enabled=false` las rutas de protocolo y `authorize-details` dan 404 pero `GET /v1/oauth/connections` sigue **200**. **Guardias**: `get_oauth_authorize_is_not_handled_by_the_api` (404 esperado; un 405 sería la señal de que alguien registró la ruta y rompió la pantalla en producción), `backup_export_works_with_oauth_grants_present` (anti-deriva SQL del incidente v1.0.10 — las tablas OAuth quedan fuera del `.ffbackup` por construcción) y `oauth_and_api_token_schemes_do_not_cross` (un `ffr_` como Bearer o un `ffo_` con un carácter alterado → 401). PKCE real con `OsRng` + SHA-256; caducidades forzadas por `UPDATE … expires_at = now() - interval '1 minute'` (no hay mock de reloj). |

### API lib unit tests (run under `cargo test --workspace`)

Besides the integration files above, the API crate carries `#[cfg(test)]` unit tests in library
modules. Notably `apps/api/src/handlers/backup_user/schema.rs` `mod tests` (10 tests; 2 added in
v1.6.0 for `.ffbackup` v5, 2 more in v1.8.0 for v6): reject a future `schema_version`, migrate v1
dropping legacy contribution fields, v3-with-rules round-trip, migrate v3→v4 filling empty
`snapshots`, v4 snapshot-items round-trip, migrate v4 filling empty transactions, v5 transactions
round-trip, migrate v5 filling empty `recurring_transaction_rules`, v6 recurring-rules round-trip,
and the full v1→v6 migration chain. The `handlers/transactions/` module adds unit tests for the
CSV presets (separators, ES/US decimals, BOM/Windows-1252, header autodetection), the
fingerprint/ordinal grouping and the rule-precedence logic. They need no Postgres.

**Excepción consciente (v3.1.0)**: el módulo `apps/api/src/oauth/` (9 ficheros), `auth/secret.rs` y
`handlers/oauth_consent.rs` **no llevan ni un `#[test]`**. Su cobertura es 100 % de integración
(`oauth_flow.rs`), porque casi todo lo que hay que probar cruza la DB (transacciones con
`FOR UPDATE`, expiries calculadas por Postgres, revocación por JOIN) o el router entero (fallback
SPA, kill-switch, headers). Si añades un helper **puro** ahí (parsing, validación de charset), ese sí
merece un unit test local.

### Writing a new integration test

```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn my_endpoint_does_x() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // arrange via API
    let cat = app.create_category(&owner, "asset", "Cash").await;

    // act
    let resp = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "EUR", "current_value": "1000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;

    // assert
    assert_eq!(resp.status, http::StatusCode::CREATED);
    let body = resp.json();
    assert_eq!(body["name"], "EUR");
}
```

Rules of thumb:
- One test per surprising behavior. Don't bundle multiple "while we're here" assertions.
- Compare Decimal fields via `f64::parse + abs() < tol` — the API serializes `"1000.0000"`, not `"1000"`.
- For regression tests on a refactor, capture the value first (test will fail), then commit the expected value once green.

## Frontend (Vitest)

- Config: `apps/web/vitest.config.ts` — `node` environment (no jsdom). Add `happy-dom` if you ever add component render tests.
- Pure-function tests only. **321 en 13 ficheros a 2026-08-19** (medido, no estimado: `npm test --workspace futurefin-web 2>&1 | grep Tests`). Ojo al contar a mano: `chart-gestures.test.ts` y `fire.test.ts` generan tests en bucles, así que el nº de `it(` en el fichero es menor que el que reporta Vitest (10 → 77 y 2 → 8 respectivamente). Esta lista omite `responsive.test.ts` (3) y `chart-gestures.test.ts` (77):
  - `lib/format.test.ts` (38) — Intl formatting in es-ES, edge cases (null/NaN/empty), Decimal string preservation; `formatMonthsRough` en años + meses desde 24 y `formatRunwayValue` («Infinito» si el runway es indefinido; «+100 años» para el suelo `months ≥ 1200`) (v2.3.0).
  - `lib/fire.normalize.test.ts` (21) — normalizadores y gating de `savings_source`, incluido `savingsAvgParenthetical` (paréntesis «promedio de N meses», singular, `undefined` en modo A / fallback) (v2.2.0).
  - `lib/dates.test.ts` (32) — civil calendar (leap years, day clamping, age before/after birthday), TZ fallback, payment intervals, `addMonthsCivil` con deltas **negativos** (v1.5.0).
  - `api/client.test.ts` (10) — `fetch` mocks: credentials, body serialization, 4xx error propagation, 204 handling.
  - `lib/fire.test.ts` (8) — **FIRE target parity** vs server: loads the same `apps/api/tests/fixtures/fire-parity.json` and asserts `grossUpNetAnnualFire(computeFireAnnualNeedNetEur(...)) / (swr/100)` matches `expected_target_nw` (± 1 €).
  - `lib/history-merge.test.ts` (12) — `mergeProjectionWithHistory`: identidad por referencia (history null/vacío/anchor distinto → render byte-idéntico), descarta puntos `month_index ≥ 0`, unión de asset series por `asset_id`, offset del futuro.
  - `lib/projection-chart.test.ts` (10) — `deflationFactorAt` (0 / ±12 meses / inflación 0) y los tick-builders con `startMonth=-24` + regresión `startMonth=0` idéntica al comportamiento previo.
  - `lib/snapshot-tracker.test.ts` (8) — `liquidCoverageComplete` (vacío→false, cobertura completa→true, stale tras `pruneEditLog`→false, asset nuevo dentro de la ventana).
  - `lib/expenses.test.ts` (89) — helpers puros de la pestaña «Movimientos» (v1.6.0, ampliado en v1.8.0): labels de mes, `defaultSelectedMonth` (último completo), `categoriesForKind` (savings sin categoría), `buildConfirmDecisions` paralelo por índice, filtros del preview, tonos de delta, y (v1.8.0) `significanceThreshold`/`trendArrow`/`significantDeltaTone` (umbral 1% del ingreso real), `AVG_WINDOWS`/`avgWindowLabel`, `capitalizeSource`.
  - `lib/oauth.test.ts` (8) — helpers de la pantalla de consentimiento (v3.1.0): `parseAuthorizeParams` (query completa con opcionales URL-decoded, `null` si falta cualquiera de los 5 obligatorios, opcionales ausentes **no** se inventan, y `code_challenge_method=plain` **SÍ** parsea — el rechazo es del servidor, no del cliente: la división de responsabilidades queda congelada en un test), `redirectHostLabel` (host con y sin puerto; string no-URL devuelto tal cual) y `authorizeErrorMessage` (códigos conocidos vs default legible).
  - `lib/navigation.test.ts` (5) — sub-tabs de Ajustes (v3.0.0/3.1.0): la sub-tab `mcp` tiene slug y label propios, `access` se renombró a «Usuarios» **conservando el slug histórico** (los enlaces guardados siguen vivos), slug desconocido → `null` (App cae a la sub-tab por defecto), cualquier `/ajustes/*` resuelve a la pestaña `settings`, y todos los slugs son únicos y redondean por `settingsSubTabPath`.

### Writing a new frontend test
Colocate beside the module: `lib/foo.ts` ↔ `lib/foo.test.ts`. Use `vi.mocked(globalThis.fetch)` if you need to stub fetch.

```ts
import { describe, expect, it } from "vitest";
import { myHelper } from "./foo";

describe("myHelper", () => {
  it("handles edge case", () => {
    expect(myHelper("input")).toBe("output");
  });
});
```

Run: `npm test --workspace futurefin-web`.

## Shared fixtures (cliente ↔ servidor)

Algunos cálculos viven duplicados a propósito (e.g. FIRE math — el cliente alimenta un preview en vivo del formulario sin round-trip; el servidor es source of truth en `/v1/projection/series`). Para evitar drift hay un JSON canónico que **ambos lados consumen**:

| Fixture | Backend test | Frontend test |
|---|---|---|
| `apps/api/tests/fixtures/fire-parity.json` | `apps/api/tests/fire_parity.rs` ejecuta `/v1/projection/series` con cada `fire_settings` + `monthly` y verifica `jubilacion_target_net_worth`. | `apps/web/src/lib/fire.test.ts` carga el mismo path absoluto vía `fs.readFileSync` y aplica los helpers TS para llegar al mismo target. |

**Regla**: si cambias `tax_brackets`, la fórmula de gross-up o el contrato de `compute_fire_target_nw` en un lado, regenera los `expected_target_nw` del JSON (usar `python3` o el motor Rust como referencia) y los dos suites deben seguir verdes. Un fallo en un solo lado indica drift.

**Cómo añadir un caso nuevo**: añade un objeto al array `cases[]` con `name`, `fire_settings`, `monthly` y `expected_target_nw`. Re-corre ambos suites. Tip: `_calc_note` documenta cómo se derivó el valor.

## CI

CI existe: `.github/workflows/ci.yml` corre en push/PR a `main`/`dev`:

| Job | Corre | NO corre |
|---|---|---|
| `rust` | `cargo build -p futurefin-api --locked`, `cargo test -p futurefin-engine --locked` | los tests de integración (`apps/api/tests/`) — necesitan `TEST_DATABASE_URL` |
| `web` | `npm run typecheck:web`, `npm run build:web` | Vitest (`npm test`), `npm run lint:web` |
| `docker-stack` (3.0.0) | shellcheck (`docker-entrypoint.sh`, `scripts/*.sh`, scripts de skills) + build de la imagen autocontenida + **sanity** (majors PG 15 y 16 presentes, label `com.futurefin.postgres.majors=15,16`, arranque **sin volumen** debe ABORTAR) + **instalación nueva** (volumen virgen → `initializing fresh PostgreSQL 16`, `/v1/ready`, alta + login + categoría «Ácido Ñandú» vía API) + **recreate estilo watchtower** conservando datos + **apagado limpio** (greps de `shutdown signal received`, `database pool closed`, `database system is shut down`, `clean shutdown complete` y `ExitCode == 0`) + **stack 2.3.0 REAL con datos** → **imagen V3 sobre el compose V2** (modo compat externa: warning `DEPRECATED`, login y datos intactos) → **migración al compose V3 reutilizando el volumen** (`adopting ownership of PGDATA`, `reindexing database after adoption`, login idéntico, categoría con Ñ/acentos intacta, username duplicado → 409/422, backup `pre-migration-*.sql.gz` presente) + **automigración desde DB externa** (dump → embebida → `automigration completed` → se apaga la externa y todo sigue vivo) + **pg_upgrade 15→16** (marker row sobrevive, `SHOW server_version` empieza por 16, `pgdata_old_15/`, backup `pre-pgupgrade-15-to-16-*.sql.gz`) | UI real (no hay E2E de navegador); las guardas de arranque que abortan (`pre-migration backup FAILED`, rol inexistente, swap de pg_upgrade interrumpido) salvo la de «sin volumen» |

El `docker-stack` dejó de ser un smoke de arranque: desde 3.0.0 es **la única evidencia automatizada de «sin pérdida de datos»** (así lo dice el comentario del job: «no debilitar»). Sus entradas congeladas viven en `.github/testdata/docker-compose.{v2,v2-app-v3,automigrate}.yml`; **`docker-compose.v2.yml` NO se actualiza** cuando evolucione el compose de producción — su valor es ser la topología 2.x exacta (dos servicios, imagen pineada a 2.3.0), igual que un fixture. Si tocas `apps/api/docker-entrypoint.sh`, `apps/api/Dockerfile`, `docker-compose.yml` o esos ficheros, ese job **es** tu suite: léelo, no lo debilites, y añade un paso nuevo si añades un camino de arranque nuevo.

**Consecuencia**: antes de mergear tienes que correr EN LOCAL lo que CI no cubre:
```bash
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" cargo test --workspace
npm test --workspace futurefin-web
npm run lint:web
```
(Checklist completo: [`.claude/skills/futurefin-change-control/SKILL.md`](skills/futurefin-change-control/SKILL.md).)

## Procedencia y re-verificación

Contenido verificado leyendo el código; la tabla de CI y las notas de contenedor se
re-verificaron el **2026-08-16 (v3.0.0)**, y la suite `oauth_flow.rs`, los helpers nuevos de
`common/mod.rs` y los recuentos de Vitest el **2026-08-17 (v3.1.0)**. Comandos para comprobar que
esta página no ha derivado (todos desde la raíz del repo):

```bash
# Jobs y pasos de CI (la fila docker-stack = un paso por línea)
grep -n "^  [a-z-]*:$\|      - name:" .github/workflows/ci.yml
# Aserciones de «sin pérdida de datos» que cita la tabla
grep -n "no persistent volume\|initializing fresh PostgreSQL 16\|Ácido Ñandú\|adopting ownership\|reindexing database after adoption\|automigration completed\|pg_upgrade needed\|pgdata_old_15\|pre-migration-\|pre-pgupgrade-\|clean shutdown complete\|ExitCode" .github/workflows/ci.yml
# Integración y Vitest siguen FUERA de CI (ambos deben no imprimir nada)
grep -n "TEST_DATABASE_URL\|5433" .github/workflows/ci.yml
grep -n "npm test\|vitest\|lint:web" .github/workflows/ci.yml
# Fixtures congelados (v2 = topología 2.x, imagen pineada)
ls .github/testdata/ && grep -n "image:\|services:" .github/testdata/docker-compose.v2.yml
# La base de tests sigue en 5433 por TCP
grep -n "5433" apps/api/tests/common/mod.rs
# Recuentos sin compilar
grep -c '#\[test\]' crates/engine/src/{projection,history,runway}.rs
grep -c "#\[tokio::test\]" apps/api/tests/*.rs | awk -F: '{s+=$2} END {print s}'
                                        # 284 en 27 suites a 2026-08-19 (transactions_reconcile.rs aporta 19; +1 post-3.5.0 por la paridad CRUD MCP)
ls apps/api/tests/*.rs | wc -l          # 27 a 2026-08-19
ls apps/api/migrations | wc -l          # 40 a 2026-08-19 (3.5.0 añade 20260819120000_transactions_transfer_reconciliation.sql)
# Vitest: el nº de `it(` NO es el total (hay bucles) — el autoritativo es el runner
npm test --workspace futurefin-web 2>&1 | grep "Tests "   # 321 en 13 ficheros a 2026-08-19
# El Host por defecto del harness (sin él, todo /oauth/* daría 400 irreal)
grep -n "futurefin.test" apps/api/tests/common/mod.rs      # 2 hits: request() y mcp_initialize()
```
