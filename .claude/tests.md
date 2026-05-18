# Tests

Test setup post-refactor (May 2026). Before: 22 engine unit tests, nothing else. After: **146 tests** across backend + frontend.

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

### Engine unit tests (`crates/engine/src/projection.rs`)
- 22+ tests in `mod tests` covering cascade allocation, retirement drain, FIRE target inflation, off-by-one between `fire_target_at_month_index(k+1)` and the handler's series.
- Pure: no Postgres, no env. `cargo test -p futurefin-engine` is enough.

### Integration tests (`apps/api/tests/`)
- Each test spins up the full Axum router (`routes::app_router()`) and drives it via `tower::ServiceExt::oneshot` against a real Postgres.
- **Schema-isolated per test**: `common::isolated_pool()` creates `ff_test_<uuid>`, sets `search_path`, applies all 33 migrations, returns the pool. Schemas are leaked intentionally — drop them with `psql -c "DROP SCHEMA ff_test_<id> CASCADE"` or wipe the test DB.

### Test infrastructure (`apps/api/tests/common/mod.rs`)
- `TestApp::spawn() -> TestApp { router, pool, schema }` — fresh schema + axum router wired with cookie cookies.
- Convenience methods on `TestApp`:
  - `register_and_login_owner("alice") -> LoggedInOwner { username, cookie }` — first user becomes owner via bootstrap.
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
- Pure-function tests only. We have 72 across:
  - `lib/format.test.ts` (29) — Intl formatting in es-ES, edge cases (null/NaN/empty), Decimal string preservation.
  - `lib/dates.test.ts` (26) — civil calendar (leap years, day clamping, age before/after birthday), TZ fallback, payment intervals.
  - `api/client.test.ts` (10) — `fetch` mocks: credentials, body serialization, 4xx error propagation, 204 handling.
  - `lib/fire.test.ts` (7) — **FIRE target parity** vs server: loads the same `apps/api/tests/fixtures/fire-parity.json` and asserts `grossUpNetAnnualFire(computeFireAnnualNeedNetEur(...)) / (swr/100)` matches `expected_target_nw` (± 1 €).

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

There is no CI yet. Recommended GitHub Action (not committed):
```yaml
- run: docker run -d --name ff-test-db -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine
- run: until docker exec ff-test-db pg_isready -U futurefin; do sleep 1; done
- run: cargo test --workspace
  env:
    TEST_DATABASE_URL: postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test
- run: npm install && npm test --workspace futurefin-web && npm run typecheck:web && npm run build:web
```
