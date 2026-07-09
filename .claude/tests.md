# Tests

Test setup post-refactor (May 2026). Before: 22 engine unit tests, nothing else. Ahora hay suites de engine (unit), API (integración contra Postgres real) y frontend (Vitest). No congeles totales en docs — cuéntalos con: `cargo test --workspace 2>/dev/null | grep "test result"` y `npm test --workspace futurefin-web 2>&1 | grep Tests`.

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

## Backend

### Engine unit tests (`crates/engine/src/{projection.rs,history.rs}`)
- `projection.rs` `mod tests`: 22 tests covering cascade allocation, retirement drain, FIRE target inflation, off-by-one between `fire_target_at_month_index(k+1)` and the handler's series.
- `history.rs` `mod tests`: 14 tests (v1.5.0) covering linear interpolation (midpoint exact, endpoints), the French-amortization curve (matches a pure schedule when the residual is 0, residual correction passes through both endpoints, midpoint above the linear chord, fallbacks when payment ≤ interest / no terms / apr=0, clamp ≥ 0), timeline rules (item absent from an intermediate snapshot = 0 between pairs, appears/disappears when present in only one, first-month clamp, virtual-today join with a deleted item → 0), and `month_index_of` / `add_months_signed` with negative deltas across a year boundary. Engine total: **36**.
- Pure: no Postgres, no env. `cargo test -p futurefin-engine` runs both (and they run in **CI**, unlike the integration suite).

### Integration tests (`apps/api/tests/`)
- Each test spins up the full Axum router (`routes::app_router()`) and drives it via `tower::ServiceExt::oneshot` against a real Postgres.
- **Schema-isolated per test**: `common::isolated_pool()` creates `ff_test_<uuid>`, sets `search_path`, applies every migration in `apps/api/migrations/` (count them with `ls apps/api/migrations | wc -l`), returns the pool. Schemas are leaked intentionally — drop them with `psql -c "DROP SCHEMA ff_test_<id> CASCADE"` or wipe the test DB.

### Test infrastructure (`apps/api/tests/common/mod.rs`)
- `TestApp::spawn() -> TestApp { router, pool, schema }` — fresh schema + axum router wired with cookie cookies.
- Convenience methods on `TestApp`:
  - `register_and_login_owner("alice") -> LoggedInOwner { username, cookie }` — first user becomes owner via bootstrap.
  - `register_and_approve_member("bob") -> LoggedInOwner` (v1.5.0) — registers a second user and has the owner approve them as a writable member; used by the household-aggregation and cross-user tests (`history_series.rs`, `history_snapshots.rs`).
  - `create_category(&owner, "asset", "Bolsa") -> id`
  - `count_rows("liabilities") -> i64` — query against the test schema.
  - `get(uri)`, `get_with_cookie(uri, cookie)`, `post_json(uri, body)`, `post_json_with_cookie`, `patch_json_with_cookie`, `delete_with_cookie` — return `ResponseParts { status, headers, body: Vec<u8> }`.
  - `ResponseParts::json()` parses body as `serde_json::Value`; `.session_cookie()` extracts `ff_session=…` from Set-Cookie.

### Tests checked in
| File | Covers |
|---|---|
| `smoke.rs` | health/ready, 401 on unauth, register→login→me, first-user bootstrap |
| `liabilities_purge.rs` | expired liabilities hidden from GET listings + summary totals but **persist in DB** |
| `body_limits.rs` | 1 MB cap on normal endpoints (413), 16 MB cap on `/backup/user-import` |
| `installation_patch.rs` | unknown `fire_number_mode` rejected (422); legacy `annual_expense_adjusted` alias still accepted |
| `unique_violation.rs` | duplicate username + duplicate category name → 409 via central `From<sqlx::Error>` |
| `projection_marker.rs` | `compound_outpaces_true_savings_month_index` stable across the perf refactor (regression for spawn_blocking + tokio::join) |
| `fire_parity.rs` | **FIRE target parity** — for each case in `fixtures/fire-parity.json`, seeds installation + budget + assets and asserts `jubilacion_target_net_worth` matches the canonical expected value (± 1 €). |
| `projection_cache.rs` | Cache de proyección: hit tras GET, invalidación tras mutación, aislamiento por vista/densidad, `?months=` bypassa el cache. |
| `history_snapshots.rs` (20) | Snapshots CRUD: captura con términos copiados, upsert mismo día reemplaza items, excluye filas compartidas/expiradas, backfill roundtrip con filtro `year` y cascade, validaciones 400 (futuro, `duplicate_item_id`, términos en asset), 409 fecha ocupada, 404 cross-user, 403 viewer en toda mutación, GET nunca muta, y `snapshot_mutations_do_not_touch_projection_cache` (la cache de proyección sigue HIT — history NO es input del engine). **Prefill** (`GET /v1/history/snapshots/prefill`, v1.5.1, ~7): interpolación idéntica a la serie, `first_snapshot`, `live`, `not_owned` (0 + `existed:false`), validaciones (fecha futura / `invalid_kind` → 400), viewer. |
| `history_series.rs` (7) | `GET /v1/history/series`: vacío→200, interpolación lineal exacta entre dos snapshots de asset, join a valores vivos (asset borrado→0 en k=0), curva de amortización por encima de la cuerda con extremos exactos, household suma dos usuarios + `?view=mine` filtra, markers con fecha/kind/total, snapshot único de hoy. Números predichos antes de ejecutar. |
| `backup_user_roundtrip.rs` (11) | `.ffbackup` v4/v5/v6: roundtrip con serie histórica idéntica, re-link de items a los UUIDs frescos de assets, `ledger_index` null conserva `item_key`, v3 sigue importando (0 snapshots), índice fuera de rango → 400 con rollback, import invalida la cache de proyección (fix del bug preexistente), preview reporta counts de snapshots/items, viewer 403, **v5** (v1.6.0): roundtrip de transactions/imports/rules con re-link por índice y `fingerprint_ordinal` preservado, y **v6** (v1.8.0): roundtrip de `recurring_transaction_rules` con `recurring_rule_index` re-enlazado y `last_materialized_month` preservado. |
| `transactions_import.rs` (15) | Import CSV: autodetección MyInvestor/N26 por cabecera, preview marca `already_imported` y los omite por defecto, confirm inserta con ordinales, re-confirm mismo archivo → 0 nuevos, `force` añade ordinal nuevo, heurística de transferencia interna, regla aprendida pre-asigna en el siguiente preview, no-EUR rechazado en confirm, viewer 403, sha preview↔confirm distinto → 400. **Fold de acentos** (post-2.0.0): `savings_hint_accent_insensitive_*` (hint de ahorro con «Aportación…» con/sin cartera), `learned_rule_matches_accent_insensitive*` (regla acentuada matchea concepto sin tilde y viceversa), precedencia regla-aprendida vs hint. |
| `transactions_crud.rs` (14) | CRUD de movimientos: alta manual individual/batch, `savings` exige categoría NULL, validación de scope income/expense, **PATCH de importadas edita op_date/amount/concept con huella anclada al CSV** (`patch_imported_fields_editable_fingerprint_anchored`, antes `patch_imported_op_date_is_immutable`; ya no hay `immutable_field`) y en manuales recomputa la huella liberando el ordinal (`patch_manual_op_date_recomputes_and_allows_reuse`), borrar asset/liability vinculado → SET NULL conservando el movimiento, remap al borrar categoría, viewer 403. |
| `transactions_summary.rs` (9) | `GET /v1/transactions/summary`: números Decimal exactos por categoría (real/budget/avg), **promedio ponderado** (denominador = `months_with_data`, no el nº de meses del tramo → historial corto no diluye a 0), ventanas `avg_window` 3/6/12/`ytd`/`all` + alias legado `avg_months`, `avg_window` inválido → 400, mes parcial marcado, savings excluido del gasto, bucket «Sin categoría». **Ya no** hay línea derivada de cuotas de pasivo: `totals.expense_budget` = Σ budget de categorías de gasto. |
| `transactions_projection_cache.rs` (3) | Contrato de cache **condicionado al modo** (`fire_settings.savings_source`): `mode_a_mutations_do_not_touch_projection_cache` (modo `budget`: la cache sigue HIT tras import/create/edit/delete/regla y los endpoints recurrentes — transacciones NO son inputs), `mode_b_each_mutation_invalidates_projection_cache` (modo `transactions_avg`: cada mutación invalida) y `flipping_savings_source_invalidates_projection_cache` (cambiar de modo invalida). |
| `transactions_recurring.rs` (16) | Movimientos recurrentes (v1.8.0): alta con `recurrence` crea regla + instancia de origen enlazada, `materialize` idempotente (2ª llamada → 0), no genera `op_date` futuro (mes en curso solo si el día ya llegó), clamp de `day_of_month` a fin de mes, borrar una instancia NO la recrea al re-materializar (cursor), `DELETE` de regla conserva las instancias (SET NULL), viewer 403 en materialize/delete, `recurrence.day_of_month` fuera de rango → 400. **Backfill del alta** (post-2.0.0): `create_with_past_date_backfills_instances` (fecha pasada rellena los meses intermedios en el mismo commit), `recurrence_op_date_within_bound_created`, y la cota `recurrence_op_date_too_old_*` → 422 `recurrence_too_old`. |
| `history_cashflow.rs` (5) | `GET /v1/history/cashflow`: agregados mensuales exactos (Decimal-string, household y mine), la serie fina pasa por los snapshots, **`/v1/history/series` idéntico byte a byte con y sin transacciones** (regresión tier-1), `daily` con ventana >6m → 400, `fine` ausente sin vínculos. |

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
- Pure-function tests only. We have 154 across (this list omits `responsive.test.ts` and `chart-gestures.test.ts`; run `npm test --workspace futurefin-web` for the authoritative total):
  - `lib/format.test.ts` (29) — Intl formatting in es-ES, edge cases (null/NaN/empty), Decimal string preservation.
  - `lib/dates.test.ts` (29) — civil calendar (leap years, day clamping, age before/after birthday), TZ fallback, payment intervals, `addMonthsCivil` con deltas **negativos** (v1.5.0).
  - `api/client.test.ts` (10) — `fetch` mocks: credentials, body serialization, 4xx error propagation, 204 handling.
  - `lib/fire.test.ts` (7) — **FIRE target parity** vs server: loads the same `apps/api/tests/fixtures/fire-parity.json` and asserts `grossUpNetAnnualFire(computeFireAnnualNeedNetEur(...)) / (swr/100)` matches `expected_target_nw` (± 1 €).
  - `lib/history-merge.test.ts` (12) — `mergeProjectionWithHistory`: identidad por referencia (history null/vacío/anchor distinto → render byte-idéntico), descarta puntos `month_index ≥ 0`, unión de asset series por `asset_id`, offset del futuro.
  - `lib/projection-chart.test.ts` (10) — `deflationFactorAt` (0 / ±12 meses / inflación 0) y los tick-builders con `startMonth=-24` + regresión `startMonth=0` idéntica al comportamiento previo.
  - `lib/snapshot-tracker.test.ts` (8) — `liquidCoverageComplete` (vacío→false, cobertura completa→true, stale tras `pruneEditLog`→false, asset nuevo dentro de la ventana).
  - `lib/expenses.test.ts` (49) — helpers puros de la pestaña «Movimientos» (v1.6.0, ampliado en v1.8.0): labels de mes, `defaultSelectedMonth` (último completo), `categoriesForKind` (savings sin categoría), `buildConfirmDecisions` paralelo por índice, filtros del preview, tonos de delta, y (v1.8.0) `significanceThreshold`/`trendArrow`/`significantDeltaTone` (umbral 1% del ingreso real), `AVG_WINDOWS`/`avgWindowLabel`, `capitalizeSource`.

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
| `docker-stack` | build de imagen + compose up + smoke `/v1/health` | — |

**Consecuencia**: antes de mergear tienes que correr EN LOCAL lo que CI no cubre:
```bash
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" cargo test --workspace
npm test --workspace futurefin-web
npm run lint:web
```
(Checklist completo: [`.claude/skills/futurefin-change-control/SKILL.md`](skills/futurefin-change-control/SKILL.md).)
