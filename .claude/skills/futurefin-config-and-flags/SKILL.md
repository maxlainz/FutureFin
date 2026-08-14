---
name: futurefin-config-and-flags
description: >
  Catalog of every configuration axis in FutureFin: environment variables (PORT, DATABASE_URL,
  SESSION_TTL_DAYS, COOKIE_SECURE, CORS_ORIGINS, WEB_STATIC_ROOT, RUST_LOG, FUTUREFIN_API_PORT,
  WEB_DEV_PORT, POSTGRES_*, FUTUREFIN_IMAGE/TAG, APP_PORT, TEST_DATABASE_URL), the three
  docker-compose files, API query-parameter flags (?view=mine, ?months, ?density=hybrid),
  request-body limits, and per-installation runtime settings (PATCH /v1/installation:
  base_currency, calendar_tz, show_age_mode, annual_inflation_assumption_percent, the
  fire_settings JSONB with swr_pct and tax_brackets bounds). Load this skill when you need to know
  what an option is called, its default, its validation bounds, where it is parsed, whether it is
  prod or dev-only, why a setting change returns 400, why an env var "isn't taking effect"
  (.env precedence), why CORS panics at startup, or when ADDING a new env var / installation
  setting / query param. Do NOT load it for step-by-step environment setup (use
  futurefin-build-and-env), deployment/upgrade/backup operations (futurefin-run-and-operate),
  or the MEANING of the FIRE math these settings feed (futurefin-fire-domain-reference).
---

# FutureFin configuration and flags

All facts verified against the code on 2026-07-02 (v1.4.3, per `apps/api/Cargo.toml`); the
`/v1/history/*` query params were added for v1.5.0 (2026-07-06), with `GET /v1/history/snapshots/prefill`
(`?kind`/`?date`) added for v1.5.1 (2026-07-07) — none introduce new env vars or installation
settings. This skill is the single home for "what can be configured, where, with what bounds".

Vocabulary used below:
- **Installation** — the singleton row in table `installation`; one per deployment; all financial
  data belongs to it. Its columns are the *runtime* settings (changed via API, stored in DB).
- **SWR** — Safe Withdrawal Rate: the % of net worth withdrawn per year in retirement.
  `FIRE number = annual expenses / (SWR/100)`.
- **split-dev** — the two-process dev mode: `cargo run` API on :8081 + Vite dev server on :8080.
- **Nominal vs real** — nominal = future euros; real = deflated to today's purchasing power.

## When NOT to use this skill

- Recreating a working dev environment from scratch, toolchain issues, local image builds →
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- Deploying, upgrading, rollback, backups, logs, smoke tests →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- What `swr_pct`, gross-up, the inflation-growing FIRE target, or the allocation cascade *mean*
  and how the engine consumes them → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- Changing behavior behind a config axis (new semantics, migrations) → gates in
  `.claude/skills/futurefin-change-control/SKILL.md`.
- curl recipes to observe cache hits, densities, etc. →
  `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.

## 1. Environment variable catalog

### API runtime (parsed in `apps/api/src/main.rs`)

| Variable | Default | Bounds / parsing | Prod or dev | Notes |
|---|---|---|---|---|
| `DATABASE_URL` | **none — required** | any Postgres URL; process panics with `expect` if unset | both | In Docker compose it is **composed** from `POSTGRES_*` vars: `postgres://$POSTGRES_USER:$POSTGRES_PASSWORD@futurefin-database:5432/$POSTGRES_DB`. In split-dev set it yourself in `.env` (e.g. `postgres://futurefin:futurefin@127.0.0.1:5432/futurefin`). |
| `PORT` | `8080` | u16; unparseable → silently falls back to 8080 | both | API listen port, binds `0.0.0.0`. Use `8081` in split-dev so Vite can take 8080. Container always runs with `PORT=8080` (Dockerfile `ENV`, restated in compose). |
| `SESSION_TTL_DAYS` | `30` | integer **1–400**; out-of-range or unparseable → **silently** 30 | both | Session cookie/DB row lifetime. Stored in `AppState.session_ttl_days`. |
| `COOKIE_SECURE` | `false` | true only for exact strings `1`, `true`, `TRUE`, `yes`, `YES` (`parse_bool_env`). `True`, `Yes`, `on` etc. parse as **false** | prod (behind HTTPS) | Sets the `Secure` attribute on the `ff_session` cookie. |
| `CORS_ORIGINS` | `http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080` | comma-separated origins, entries trimmed, empties dropped; an unparseable entry **panics at startup**; empty result panics | prod, only for cross-origin API access | `allow_credentials(true)`, methods GET/POST/PATCH/DELETE/OPTIONS, headers `content-type`/`accept`. Same-origin deployments (the normal Docker image) never send CORS preflights, so the default is fine. |
| `WEB_STATIC_ROOT` | unset | path; empty/whitespace value treated as unset; set-but-missing path → startup warning, API-only mode | prod (Docker sets `/app/web`) | When the path exists, the SPA is served from it with `index.html` fallback (single-port mode). Omit in split-dev — Vite serves the UI. |
| `RUST_LOG` | `futurefin_api=info,tower_http=info,sqlx=warn` | tracing `EnvFilter` syntax; invalid filter → the default is used | both | Default is applied in `main.rs` when the env filter can't be built from the env. |

Not env-configurable (hardcoded constants — changing them is a code change):
- DB pool: `max_connections=10, min=1, acquire_timeout=5s, idle_timeout=600s, max_lifetime=1800s` (`apps/api/src/db.rs`).
- Projection cache TTL: 60 min sliding (`PROJECTION_CACHE_TTL`, `apps/api/src/state.rs`).
- Body limits: 1 MiB global, 16 MiB for backup import (`apps/api/src/routes/mod.rs`, see §4).
- Gzip compression for responses >1 KB (`main.rs`, `CompressionLayer`).

### Compose / deployment level (consumed by `docker-compose.yml`, not by the Rust binary)

| Variable | Default | Prod or dev | Notes |
|---|---|---|---|
| `POSTGRES_PASSWORD` | **none — compose fails** (`:?Set POSTGRES_PASSWORD in .env`) | prod | The only variable production strictly requires in `.env`. |
| `POSTGRES_USER` | `futurefin` | prod | Also used in the DB healthcheck. |
| `POSTGRES_DB` | `futurefin` | prod | |
| `FUTUREFIN_IMAGE` | `maxlainz/futurefin` | prod | Set to `futurefin-local` for the local-image test flow (§3). |
| `FUTUREFIN_TAG` | `latest` | prod | Pin to `X.Y.Z` for stability; rollback = change tag + `up -d`. |
| `APP_PORT` | `8080` | prod | **Host** port mapped to the container's fixed internal `:8080`. This is the distinction: `APP_PORT` = host side of the mapping, `PORT` = what the binary listens on inside the container (always 8080 there). |

### Dev-only (Vite, tests, scripts)

| Variable | Default | Consumed where | Notes |
|---|---|---|---|
| `FUTUREFIN_API_PORT` | `8081` | `apps/web/vite.config.ts` | Vite proxy target port for `/v1`, `/health`, `/openapi.json`. Read **without** `VITE_` prefix — the config uses `loadEnv(mode, repoRoot, "")`, i.e. all vars, from the **repo root** `.env` (not `apps/web/.env`). |
| `WEB_DEV_PORT` | `8080` | `apps/web/vite.config.ts` | Vite dev-server port. `strictPort: false` — if 8080 is busy Vite silently picks the next port; check the terminal banner. |
| `TEST_DATABASE_URL` | `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test` | `apps/api/tests/common/mod.rs` | Postgres for integration tests (each test creates its own schema). Not run in CI as of 2026-07-02 — see `.claude/skills/futurefin-validation-and-qa/SKILL.md`. |
| `BASE`, `SMOKE_USER`, `SMOKE_PASS` | `http://127.0.0.1:8080`, auto-registers throwaway user | `scripts/smoke-projection-cache.sh` | Owned by futurefin-diagnostics-and-tooling. |
| `ENV_FILE`, `BACKUP_DIR`, `KEEP_BACKUPS` | `.env.prod`, `./backups`, `30` | `scripts/backup-postgres.sh` | Owned by futurefin-run-and-operate. |

`.env.example` at the repo root is the canonical template: production needs only
`POSTGRES_PASSWORD`; the dev vars (`PORT=8081`, `DATABASE_URL`, `RUST_LOG`) ship commented out.

## 2. `.env` loading order and precedence

API side — `main.rs::load_env()` runs before anything else:
1. `dotenvy::from_filename({CARGO_MANIFEST_DIR}/../../.env)` — the **repo-root** `.env`, resolved
   at compile time relative to `apps/api/Cargo.toml`. This is why `cargo run` from `apps/api`
   still picks up the root `.env`.
2. `dotenvy::dotenv()` — `.env` in the current working directory (a fallback; from repo root this
   is the same file).

dotenvy never overwrites variables already set: **real environment > repo-root `.env` > CWD
`.env`**. If a change to `.env` "isn't taking effect", check for the variable exported in your
shell or injected by compose — that wins. Both loads are `let _ = ... .ok()`: a missing `.env` is
silent, so in Docker (no `.env` in the image) only real env vars apply.

Vite side — `apps/web/vite.config.ts` computes `repoRoot = apps/web/../..` and calls
`loadEnv(mode, repoRoot, "")`. The empty-string third argument disables the `VITE_` prefix filter,
so `FUTUREFIN_API_PORT` / `WEB_DEV_PORT` are plain names in the root `.env`. These are dev-server
settings only; they are not baked into the client bundle.

## 3. Docker Compose file matrix

| File | Scenario | What it does |
|---|---|---|
| `docker-compose.yml` | Production / normal run | `futurefin-database` (postgres:16.4-alpine, digest-pinned, no host port, volume `pgdata`) + `futurefin` (pulled image, host `${APP_PORT:-8080}` → container 8080, healthchecks on both, app waits for `service_healthy`). Sets `PORT=8080`, `WEB_STATIC_ROOT=/app/web`, composed `DATABASE_URL`, `RUST_LOG`. |
| `docker-compose.local.yml` | Test a locally built image without publishing | Override adding **`pull_policy: never`** to service `futurefin` — otherwise compose tries to pull `futurefin-local:dev` from Docker Hub and fails. Use with `FUTUREFIN_IMAGE=futurefin-local`, `FUTUREFIN_TAG=dev` in `.env`: `docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d`. Full recipe: CLAUDE.md "Test local con Docker Desktop". |
| `docker-compose.split-dev.yml` | split-dev (`cargo run` + `npm run dev:web`) | Override exposing Postgres on **`127.0.0.1:5432`** so the host-side API can connect. Not for production (never expose the DB port there). Usage: `docker compose -f docker-compose.yml -f docker-compose.split-dev.yml up -d futurefin-database`. (CLAUDE.md's short form `docker compose up -d futurefin-database` works too but exposes no DB port — then your host API can't reach it; the split-dev override or a manual `docker run` is what actually opens 5432.) |

Overrides are additive: always pass `-f docker-compose.yml -f <override>.yml`, base file first.

## 4. API query-parameter flags and body limits

### `?view=household|mine` — ledger scope (all ledger endpoints)

Defined in `apps/api/src/handlers/person_view.rs` (`LedgerViewQuery::resolve`). The value is
trimmed; exactly `mine` → `LedgerView::Mine` (adds `AND owner_user_id = <session user>`); **any
other value, including typos, silently means `household`** (full installation). Accepted by
assets, liabilities, summary, budget, planning, allocation-rules and projection handlers.
Non-negotiable semantics: this is a client-side display filter, **not** an authorization
boundary — every member sees household data. Handlers must build the WHERE via
`LedgerView::scope_where(alias)` + `bind_scope_as/scalar` (placeholder indices start at
`next_arg_index()`: 2 for household, 3 for mine); never hand-write the two branches.

### `GET /v1/projection/series?months=&density=&view=` (`apps/api/src/handlers/projection.rs`)

| Param | Values | Default | Semantics |
|---|---|---|---|
| `months` | u32, **clamped to 12–840** (no error on out-of-range) | omitted | Horizon override. Omitted → horizon derived from demographics: years until age 90 from ONE resolved birth date (session user's `users.birth_date`, else the first `persons` row by `is_primary DESC, sort_index ASC` — NOT the oldest member), clamped 5–70 years; no birth date at all → 30 years. `horizon_basis` in the response reports which path: `lifespan_90`, `fallback_no_demographics`, or `months_override`. (Implementation: `projection_horizon_months()`, `handlers/projection.rs` ~599–627.) |
| `density` | `monthly` \| `hybrid` (trimmed; anything else → `monthly`) | `monthly` | Serialization-only decimation: `monthly` ≈ one point per month (~841 at max horizon); `hybrid` = months 0..12 monthly + 24, 36, … annually (~82 points, ~5× smaller JSON). The engine always computes the **full** series; milestones/crossover indices are computed pre-decimation, so a `reached_month_index` may not exist as a point in a hybrid response — match by `month_index`, never by array position (the v1.4.2 chart bug). |
| `view` | as above | `household` | Also selects the cache partition. |

Cache-key implications (`apps/api/src/state.rs`): the in-memory projection cache is keyed by
`(installation_id, view, owner_user_id [Some only for mine], density)` with a 60-min sliding TTL.
**Any `?months=` override bypasses the cache entirely** (computed fresh, never stored). Adding a
new query param that changes response content requires either joining `ProjectionCacheKey` or
bypassing the cache — otherwise users get stale cross-contaminated responses. Every mutating
handler invalidates the whole installation's entries (`refresh_projection_after_mutation`).

### `GET /v1/history/*` — snapshot query params (`apps/api/src/handlers/history.rs`, v1.5.0)

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/history/snapshots` | `year` | `i32`, validated **1900–3000** (out of range → 400) | omitted → all years | Filters by a civil-date range (index-friendly), always own-user (no `?view`). |
| `GET /v1/history/snapshots` | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | omitted → both | Note: **stricter than `?view`/`?density`** — an unknown `kind` here **errors 400**, it does not silently fall back. |
| `GET /v1/history/series` | `view` | `household` \| `mine` (standard `LedgerViewQuery::resolve`) | `household` | Standard scope filter (§4.1). `mine` = own series; `household` = server-side sum of every user's interpolated series. |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `kind` | `asset` \| `liability` (anything else → 400 `invalid_kind`) | **required** | Which ledger side to pre-populate the backfill modal with. Always own-user (no `?view`). |
| `GET /v1/history/snapshots/prefill` (v1.5.1) | `date` | civil date `YYYY-MM-DD`; a future date → 400 `snapshot_date_in_future` | **required** | Target date the suggested values are interpolated to (same math as `/v1/history/series`). Each item returns a `value` + `basis` ∈ `interpolated`\|`first_snapshot`\|`live`\|`not_owned`; items that didn't exist yet arrive `value:"0"`, `existed:false`. |
| `GET /v1/history/cashflow` (v1.6.0) | `view` | standard `LedgerViewQuery::resolve` | `household` | Standard scope filter (§4.1) over transactions + snapshots. |
| `GET /v1/history/cashflow` (v1.6.0) | `window_months` | `i64`, **clamped 1..=120** (no error) | `24` | Months of monthly aggregate + fine-grid window. |
| `GET /v1/history/cashflow` (v1.6.0) | `resolution` | `weekly` \| `daily` (trimmed; anything else → `weekly`) | `weekly` | `daily` **requires `window_months <= 6`** → else **400 `daily_window_too_large`** (grid cost). `daily` runs in `spawn_blocking`; `weekly` inline. |

No new **env vars** and no new installation settings ship with the history feature (series,
prefill or cashflow) — it is entirely per-user request/data surface. The series and prefill
endpoints have **no cache** (sub-ms compute) and take no `?months`/`?density`; cashflow is also
uncached.

### `GET /v1/transactions/*` — histórico de gasto query params (`apps/api/src/handlers/transactions/`, v1.6.0)

Most read endpoints accept `?view` (standard §4.1 scope: `GET /v1/transactions`, `/months`,
`/summary`, `/imports`); the **rules** GET is always own-user (no `?view`), and all writes are
`owner_user_id = session user`. Additional filters, all optional unless noted:

| Endpoint | Param | Values | Default | Semantics |
|---|---|---|---|---|
| `GET /v1/transactions` | `month` | `YYYY-MM` (invalid → 400) | omitted → all | Filters `op_date` to that calendar month. Plus `kind` (`expense`\|`income`\|`savings`, invalid → 400), `category_id` (uuid), `import_id` (uuid). |
| `GET /v1/transactions/summary` | `year` + `month` | `year` 1900–3000, `month` 1–12; **provided together or neither** (else 400) | omitted → last **complete** calendar month | Selected month of the comparison. |
| `GET /v1/transactions/summary` | `avg_window` | `3` \| `6` \| `12` \| `ytd` \| `all` (trim + case-insensitive; anything else → **400 `avg_window must be one of 3, 6, 12, ytd, all`**) | `6` | Historical-average window (v1.8.0). Weighted average: denominator = months in the window with ≥1 transaction, not the window width. `ytd` = calendar months of the selected year strictly before the selected month (Jan → empty); `all` = since the first transaction. |
| `GET /v1/transactions/summary` | `avg_months` | u32, **1–24** (out of range → 400) | `6` | **Legacy alias** for `avg_window` (fixed-month window only). `avg_window` wins when both are sent. |
| `DELETE /v1/transactions/imports/{id}` | `confirm` | `bool` | `false` | Must be `true` or **400 `confirm_required`** (undo cascades to the batch's transactions). |

None of these query params (nor any transactions mutation) touch the projection cache — transactions
are not engine inputs (regression `transactions_projection_cache.rs`).

### Body limits (`apps/api/src/routes/mod.rs`)

- Global: `DEFAULT_BODY_LIMIT_BYTES` = 1 MiB (`DefaultBodyLimit` on the outer router).
- `POST /v1/backup/user-import` and `/v1/backup/user-import/preview`, plus (v1.6.0)
  `POST /v1/transactions/import/preview` and `/v1/transactions/import/confirm`:
  `BACKUP_IMPORT_BODY_LIMIT_BYTES` = 16 MiB (base64 `.ffbackup`/CSV payloads inflate ~33%).
- Symptom of hitting the limit: HTTP 413 on an otherwise valid request.

## 5. Per-installation runtime settings (`apps/api/src/handlers/installation.rs`)

Stored on the singleton `installation` row; read back in every `InstallationSnapshot`
(`GET /v1/installation`, `GET /v1/installation/session-context`). Amounts/percentages travel as
**strings** on the wire (Decimal-as-string; never floats).

`PATCH /v1/installation` — **owner role only** (403 otherwise); at least one field required (400
otherwise); a successful PATCH **invalidates** the projection cache (like every mutation — it does
NOT warm it; warm-up happens only after login, see futurefin-architecture-contract D7). Frontend surface:
`apps/web/src/views/SettingsView.tsx` (Ajustes) and `RetirementView.tsx` (FIRE settings).

| Setting | Set where | Validation | Default | Meaning |
|---|---|---|---|---|
| `base_currency` | **setup only** (`POST /v1/installation/setup`); not in the PATCH body → immutable afterwards without a migration | trimmed, exactly 3 ASCII letters, uppercased; MVP whitelist `EUR`/`USD`/`GBP` | `EUR` (auto-bootstrap path) | Display currency. |
| `calendar_tz` | setup + PATCH | trimmed, length 3–64, must parse as an IANA zone via `chrono_tz` (e.g. `Europe/Madrid`, `UTC`); DB CHECK mirrors the length/trim rules | `UTC` (serde default at setup; DB column `DEFAULT 'UTC'`) | Civil "today" for the whole installation (projection anchor month, liability expiry filtering, derive-principal). |
| `show_age_mode` | setup + PATCH | `dates` \| `ages` | `dates` | Whether the projection X axis shows calendar dates or the viewer's age. |
| `annual_inflation_assumption_percent` | PATCH only | sent as a **string** (`"2.5"`); empty string → `0`; must parse as decimal, bounds **0–50** (negative rejected) | `0` (column `NOT NULL DEFAULT 0`) | Annual % applied to the **moving FIRE target only** — `target(month_index) = base × (1+pct/100)^(month_index/12)` (`fire_target_at_month_index`; the engine evaluates month k against the target at index k−1 — see futurefin-fire-domain-reference §4); incomes/expenses/contributions stay nominal. `0` = flat target. Semantics owned by futurefin-fire-domain-reference. |
| `fire_settings` | PATCH only | JSONB, shape below | column nullable; `NULL` → defaults applied on read (`resolve_fire_settings`) | FIRE target computation config. |

### `fire_settings` JSONB shape (as of 2026-07-09; `savings_source` added)

```json
{
  "fire_number_mode": "annual_expense",          // "manual" | "annual_expense" | "current_income"
  "fire_number_manual_amount": null,             // decimal string; REQUIRED and > 0 when mode = "manual"
  "swr_pct": "3.5",                              // decimal string, 0–4 (PERCENT, not ratio)
  "taxes_enabled": true,
  "tax_brackets": [                              // capital-gains schedule used for gross-up
    { "up_to": "6000",   "pct": "19" },
    { "up_to": "50000",  "pct": "21" },
    { "up_to": "200000", "pct": "23" },
    { "up_to": "300000", "pct": "27" },
    { "up_to": null,     "pct": "30" }           // last bracket MUST be open-ended (up_to null)
  ],
  "savings_source": "budget"                     // "budget" (default) | "transactions_avg" | "budget_income_real_expense"
}
```

Validation (`validate_fire_settings` / `validate_tax_brackets`, all 400 on failure):
- `swr_pct` ∈ [0, 4].
- mode `manual` ⇒ `fire_number_manual_amount` present and > 0.
- When `taxes_enabled`: `tax_brackets` non-empty; each `pct` ∈ [0, 99]; only the **last** bracket
  may (and must) have `up_to: null`; non-last `up_to` values must be > 0 and strictly increasing.
- Brackets are **not validated when `taxes_enabled` is false** — stale brackets can sit dormant.

Deserialization details that matter:
- `fire_number_mode` is **strict**: unknown strings → 422. Sole legacy alias
  `annual_expense_adjusted` (old backup schemas) maps to `annual_expense`.
- **`savings_source`** (`SavingsSource` enum, `installation.rs`, `rename_all = "snake_case"`, `Default = Budget`) — source of the simulation's monthly saving, **three modes**:
  - `budget` (mode A, default — from budget entries, historical behavior).
  - `transactions_avg` (mode B — income and expense from the weighted average of the last 12 complete calendar months of transactions, with a hybrid subtraction of each active liability's payment).
  - `budget_income_real_expense` (mode C — income from the **budget** + **real** expense, same `expense_eff` as B; FIRE target `annual_expense` uses the real expense, `current_income` uses budget income).

  The 12m average that feeds the engine (`transactions_12m_avg`) counts only **real months** (≥1 transaction with `recurring_rule_id IS NULL`); pseudo-empty / recurring-only months are excluded entirely. Strict deserialize like `FireNumberMode`: unknown value → **422** (error lists all three valid variants); absent → `budget` (via the struct-level `#[serde(default)]`; old backups load). No extra `validate_fire_settings` bound — any of the three enum values is accepted. **What it affects** (modes B and C, gated by `SavingsSource::uses_transactions()`): `GET /v1/projection/*` (engine income/expense + FIRE target base), `GET /v1/summary` `financial_health` (income/expense/net/savings_rate + fields `savings_source`, `savings_source_months_with_data`; in mode C income stays the budget income; since v2.2.0 also `expense_derived`/`expense_total` — derived = debt service, total = `expense_eff + debt_service` — and therefore `runway_months`), `GET /v1/assets` (`contribution_nominal_monthly` **and**, since v2.2.0, the `months_expense`/`income_multiple` caps behind `contribution_target_amount`), `GET /v1/projection/series` (echoes the effective mode in `savings_source` + `savings_source_months_with_data`), and — crucially — the **projection-cache invalidation contract**: in B/C transaction mutations invalidate the cache (`invalidate_projection_if_savings_uses_transactions`), in mode A they never do (D12/D12a in futurefin-architecture-contract). Read without a round-trip via `projection_savings_source(pool, iid)`. FIRE-math meaning owned by futurefin-fire-domain-reference.
- The struct has `#[serde(default)]`: omitted fields fill with defaults
  (mode `annual_expense`, `swr_pct` 3.5, `taxes_enabled` true, Spanish IRPF brackets above).
- In the PATCH body `fire_settings` is `Option<Option<FireSettings>>`: **omit** = unchanged,
  JSON **`null`** = clear stored JSON (defaults apply on read), **object** = validate + replace
  wholesale (no deep merge — send the full object).

Note: `installation.projection_target_age` no longer exists — dropped by migration
`20260516120000_drop_projection_target_age.sql` (v1.0.6). The FIRE crossover is the sole
retirement trigger; do not reintroduce an age setting.

## 6. How to add a new configuration axis

First decide the layer: **env var** = per-deployment, operator-set, needs restart;
**installation setting** = per-household runtime data, owner-set via UI, persisted in DB;
**query param** = per-request presentation/scoping only. Behavior changes ride
futurefin-change-control gates regardless of layer.

### New env var — checklist
1. `apps/api/src/main.rs` — parse next to the existing helpers (`parse_bool_env`, `port()`).
   Follow house style: explicit default, bounds via `.filter(...)`, never panic except for
   truly required values (only `DATABASE_URL` and bad `CORS_ORIGINS` panic today).
2. If handlers need it at request time: add a field to `AppState` (`apps/api/src/state.rs`) and
   thread it through `AppState::new(...)` in `main.rs`.
3. Log it in the startup `tracing::info!(... , "server config")` line so deployments are auditable.
4. `.env.example` — add it, commented out unless production-required, with the default noted.
5. If production-relevant: `docker-compose.yml` `environment:` block (and decide compose default).
6. Docs of record: `.claude/env-and-config.md` table + `README.md` "Environment variables" table.
7. If integration tests need it: `apps/api/tests/common/mod.rs` follows the
   default-with-override pattern (`TEST_DATABASE_URL`).

### New installation setting — checklist
1. Migration `apps/api/migrations/YYYYMMDDHHMMSS_description.sql`: `ALTER TABLE installation ADD
   COLUMN ... NOT NULL DEFAULT ...` (a default keeps the existing singleton row valid). Never edit
   a shipped migration; data-losing migrations need explicit owner sign-off (change-control rule).
2. `apps/api/src/handlers/installation.rs`: add the field to `InstallationMemberRow`,
   `InstallationSnapshot`, `PatchInstallationBody` (as `Option<...>`, omit = unchanged); write a
   `validate_*` function with explicit bounds; extend the `UPDATE` in `patch_my_installation`, the
   "at least one field" guard, and **all three** `SELECT i.id, i.base_currency, ...` queries (they
   are duplicated in session-context / get / patch — miss one and reads return stale shape).
   Also update `setup_installation`'s hardcoded response snapshot if the field has a setup default.
3. New struct types → register in the schema list in `apps/api/src/openapi.rs`; utoipa path
   annotations pick up body changes automatically.
4. Frontend: `apps/web/src/api/types.ts` (snapshot + patch types) and the editing UI in
   `apps/web/src/views/SettingsView.tsx` (Ajustes) or `RetirementView.tsx` for FIRE-related knobs.
5. If projection math consumes it: thread through `build_installation_projection_input` and
   remember PATCH already invalidates the projection cache.
6. Integration test in `apps/api/tests/` covering the validation bounds (accept boundary, reject
   out-of-range) — see futurefin-validation-and-qa for the TestApp harness.
7. Docs of record: `.claude/data-model.md` (installation row + invariants) and
   `.claude/api-routes.md`; CHANGELOG entry per futurefin-docs-and-writing.

### New query param — checklist
Add the field to the handler's `#[derive(Deserialize)]` query struct with `#[serde(default)]`,
resolve with the trim-then-match pattern (unknown values fall back to the default, they don't
error — match existing `view`/`density` behavior), document it in the `#[utoipa::path]` `params`,
and if the endpoint is the cached projection route, extend `ProjectionCacheKey` in
`apps/api/src/state.rs` or bypass the cache (see §4). Update `.claude/api-routes.md`.

## Provenance and maintenance

Every table row above is re-verifiable; run these from the repo root when auditing for drift:

- Env parsing, defaults, bounds, load order: `grep -n "env::var\|unwrap_or\|contains(&d)\|load_env" apps/api/src/main.rs`
- CORS default origin list + panic: `grep -n "CORS_ORIGINS" -A 6 apps/api/src/main.rs`
- Pool constants: `grep -n "connections\|timeout\|lifetime" apps/api/src/db.rs`
- Cache TTL + key + Density docs: `grep -n "PROJECTION_CACHE_TTL\|pub enum Density\|ProjectionCacheKey" -A 6 apps/api/src/state.rs`
- Body limits: `grep -n "BODY_LIMIT" apps/api/src/routes/mod.rs`
- `?months` clamp + horizon: `grep -n "clamp(12, 840)\|LIFESPAN_AGE\|FALLBACK_YEARS\|lifespan_90" apps/api/src/handlers/projection.rs`
- `?density` / hybrid indices: `grep -n "resolve_density\|density_month_indices" -A 10 apps/api/src/handlers/projection.rs`
- `?view` resolution: `grep -n "fn resolve" -A 5 apps/api/src/handlers/person_view.rs`
- Installation validation bounds: `grep -n "normalize_currency\|validate_show_age_mode\|validate_annual_inflation\|normalize_calendar_tz\|swr_pct\|from(99u32)" apps/api/src/handlers/installation.rs`
- fire_settings defaults + legacy alias: `grep -n "default_fire_settings\|annual_expense_adjusted" -A 8 apps/api/src/handlers/installation.rs`
- `savings_source` enum + reader + conditional cache gating: `grep -n "enum SavingsSource\|savings_source\|projection_savings_source" apps/api/src/handlers/installation.rs apps/api/src/handlers/transactions/mod.rs`
- Compose defaults + pull_policy + split-dev port: `grep -n ":-\|:?\|pull_policy\|5432:5432" docker-compose*.yml`
- Vite env reading: `grep -n "loadEnv\|FUTUREFIN_API_PORT\|WEB_DEV_PORT\|strictPort" apps/web/vite.config.ts`
- Dockerfile env: `grep -n "^ENV" apps/api/Dockerfile`
- Test DB default: `grep -n "TEST_DATABASE_URL" apps/api/tests/common/mod.rs`
- Version stamp: `grep -n "^version" apps/api/Cargo.toml`

(The previously stale docs on these topics — data-model.md's `projection_target_age`,
env-and-config.md's fake `DATABASE_URL` "default", and the `mac_*` `horizon_basis` doc comment in
`handlers/projection.rs` — were all fixed on 2026-07-02. The standing-errata record lives in
futurefin-docs-and-writing §7.)

When you change anything cataloged here, update this file in the same change, plus the matching
doc of record (`.claude/env-and-config.md`, `.claude/data-model.md`, `README.md`).
