# CLAUDE.md

This file provides guidance to any AI model (or human) working with code in this repository. It is the **single entry point**: everything else — reference docs, runbooks, history — is reachable from here.

## Start here — route your task

FutureFin is a self-hosted household finance + FIRE-planning app: Rust/Axum API (`apps/api`), pure-Decimal projection engine (`crates/engine`), React 19 SPA (`apps/web`), PostgreSQL — **embebido en la propia imagen Docker desde 3.0.0** (un solo contenedor en producción; en dev sigue siendo un Postgres aparte). Money is NEVER `f64` in domain code. UI copy en español; código e identificadores en inglés.

The repo carries three documentation layers. Consult them in this order:

1. **This file** — commands, conventions, delegation norm, architecture summary, git workflow.
2. **Skills** (`.claude/skills/*/SKILL.md`) — task-shaped runbooks with verified commands, the project's history and its discipline. **Pick by task type** (table below).
3. **Reference docs** (`.claude/*.md`) — per-area fact sheets (routes, schema, engine, env…).

### Skill routing (load BEFORE starting the matching task)

| Your task looks like… | Load |
|---|---|
| Any change you plan to merge (gates, migration/release rules, pre-merge checklist) | [`futurefin-change-control`](.claude/skills/futurefin-change-control/SKILL.md) |
| A symptom: wrong numbers, HTTP errors, unhealthy container, layout breakage | [`futurefin-debugging-playbook`](.claude/skills/futurefin-debugging-playbook/SKILL.md) |
| "Why is X designed this way?" / touching cache, auth, scoping, serialization | [`futurefin-architecture-contract`](.claude/skills/futurefin-architecture-contract/SKILL.md) |
| Understanding the FIRE/projection math (SWR, gross-up, cascade, inflación) | [`futurefin-fire-domain-reference`](.claude/skills/futurefin-fire-domain-reference/SKILL.md) |
| About to (re)introduce an old idea — check what was already tried and rejected | [`futurefin-failure-archaeology`](.claude/skills/futurefin-failure-archaeology/SKILL.md) |
| Añadir/cambiar rutas o handlers (¿tool MCP?), añadir/actualizar una tool MCP, deriva del catálogo `/mcp` | [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md) |
| Tocar una métrica o un KPI: su base, su ventana, su nombre, o añadir/retirar uno | [`futurefin-metric-definitions`](.claude/skills/futurefin-metric-definitions/SKILL.md) |
| Env vars, compose files, query params, fire_settings axes; adding a config axis | [`futurefin-config-and-flags`](.claude/skills/futurefin-config-and-flags/SKILL.md) |
| Setting up / building / dev-environment failures | [`futurefin-build-and-env`](.claude/skills/futurefin-build-and-env/SKILL.md) |
| Deploy, upgrade, rollback, backups, logs, production ops | [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md) |
| Measuring: timings, cache hits, payload sizes, DB state (ships scripts) | [`futurefin-diagnostics-and-tooling`](.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md) |
| Running or writing tests; what evidence a change needs; fire-parity fixture | [`futurefin-validation-and-qa`](.claude/skills/futurefin-validation-and-qa/SKILL.md) |
| Updating CHANGELOG/README/docs; doc drift; house style; templates | [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md) |
| Improving projection realism/correctness (Monte Carlo, taxes, invariants…) | [`futurefin-projection-realism-campaign`](.claude/skills/futurefin-projection-realism-campaign/SKILL.md) |
| Numeric analysis: closed forms, index proofs, f64 safety, determinism audits | [`futurefin-proof-and-analysis-toolkit`](.claude/skills/futurefin-proof-and-analysis-toolkit/SKILL.md) |
| "What should we build next?" / public capability claims | [`futurefin-research-frontier`](.claude/skills/futurefin-research-frontier/SKILL.md) |
| Turning a hypothesis into an accepted change (evidence bar, predict-then-run) | [`futurefin-research-methodology`](.claude/skills/futurefin-research-methodology/SKILL.md) |

## Norm — delegate to Opus subagents whenever possible

**Default posture: delegate.** Any unit of work that can be handed to a subagent should be, and every subagent runs on **Opus** (`Agent` tool with `model: "opus"`). Never let delegated work silently fall to a smaller model: this repo's failure modes are *silent wrong numbers* (projection/FIRE math, Decimal handling, index arithmetic), and a cheaper model reads as confident on exactly those.

**Delegate by default:**
- Exploration and search that spans many files or naming conventions (`Explore` / `general-purpose`).
- Per-area research feeding a change (routes, schema, engine, frontend structure).
- Independent work that can run concurrently — launch those subagents **in a single message** so they run in parallel.
- Reviews, audits and adversarial verification of a finding before you act on it.
- Anything whose raw output (file dumps, long logs, test noise) would flood the main context; the subagent returns the conclusion, not the dump.

**Keep in the main session** (do NOT delegate): git operations (commit, push, tag, merge to `main`), migrations and releases, anything destructive or irreversible, and small edits to a file you already have open — delegation there costs more than it saves.

**How to delegate well:**
- Subagents start with **fresh context**: they do not inherit yours. State the task, the files/paths involved, and **which skill they must load** from the routing table above.
- Ask for a conclusion plus the evidence for it (paths, line numbers, command output) — never a bare verdict.
- Never take a subagent's claim at face value on money math or invariants: the main session stays the owner and re-verifies with the gates in [`futurefin-change-control`](.claude/skills/futurefin-change-control/SKILL.md) before committing.

## Reference docs (`.claude/`)

Extended reference — read these before working on the relevant area:

| File | Contents |
|------|----------|
| [`.claude/api-routes.md`](.claude/api-routes.md) | Full route map with auth patterns |
| [`.claude/data-model.md`](.claude/data-model.md) | DB schema, table invariants, FIRE JSONB shape |
| [`.claude/engine.md`](.claude/engine.md) | Projection engine public API and simulation loop |
| [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md) | Auth flow, roles, cookie, pending users |
| [`.claude/env-and-config.md`](.claude/env-and-config.md) | All env vars, `.env` loading order, Vite config |
| [`.claude/adding-handler.md`](.claude/adding-handler.md) | Step-by-step pattern for adding a new API handler |
| [`.claude/frontend-structure.md`](.claude/frontend-structure.md) | SPA layout post-refactor (lib/, api/, components/, views/, auth/) and where to put what |
| [`.claude/design-system.md`](.claude/design-system.md) | V1 redesign — tokens, paleta, reglas para añadir UI nueva (LEE ANTES de tocar estilos) |
| [`.claude/tests.md`](.claude/tests.md) | How to run + write backend integration tests (Postgres schemas) and frontend Vitest tests |

**Keep these files up to date** whenever the corresponding area changes (routes, schema, env vars, etc.). The same applies to the skills: each `SKILL.md` ends with a "Provenance and maintenance" section listing one-line re-verification commands — if your change makes one of those facts stale, update the skill in the same PR. If you find a doc/code disagreement you cannot fix in the same change, record it in the standing-errata table of [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md); the code is ground truth.

## Commands

### Development (split-dev: API + Vite hot reload)
```bash
cp .env.example .env
# Uncomment the dev vars in .env (PORT, DATABASE_URL, RUST_LOG)
# Postgres de desarrollo — compose autónomo que expone 127.0.0.1:5432 (imprescindible para cargo run):
docker compose -f docker-compose.dev.yml up -d

# Terminal 1 — API at :8081 (auto-migrates DB on start)
cd apps/api && cargo run

# Terminal 2 — UI at :8080 with proxy to API
npm install
npm run dev:web
```
Open `http://127.0.0.1:8080`. The Vite proxy routes `/v1`, `/health`, `/openapi.json` to the API port.

### API only (no Vite)
Set `PORT=8080` in `.env`, then `cd apps/api && cargo run`.

### Test local con Docker Desktop (sin publicar imagen)
Útil para validar el stack completo (API + frontend + DB) exactamente como en producción, sin esperar a que CI publique una imagen.

```bash
# 1. Construir la imagen localmente (tarda la primera vez; usa caché en rebuilds)
#    --load es obligatorio con BuildKit para que quede en el store local de Docker
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .

# 2. Asegúrate de que .env tiene (ya no hace falta POSTGRES_PASSWORD):
#      FUTUREFIN_IMAGE=futurefin-local
#      FUTUREFIN_TAG=dev
#    OJO: sin DATABASE_URL descomentada — la imagen la interpretaría como DB externa.

# 3. Arrancar el stack con el override local (evita que Compose haga pull de la imagen local)
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d

# 4. Smoke test — /v1/ready valida también el Postgres embebido
curl -sf http://127.0.0.1:8080/v1/ready

# 5. Rebuild tras cambios (la caché de Docker reutiliza las capas sin cambios)
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev . \
  && docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env \
     up -d --no-deps futurefin
```

> `docker-compose.local.yml` añade `pull_policy: never` al servicio `futurefin` para que Compose no intente hacer pull de una imagen que solo existe en local.

### Production stack (maintenance)
```bash
docker compose logs -f futurefin          # logs (un solo flujo: entrypoint + PostgreSQL + API)
docker compose down --remove-orphans      # stop (los datos quedan en los volúmenes pgdata/ffdata)
curl -sf http://127.0.0.1:8080/v1/ready   # smoke test (valida también el PG embebido)
# psql contra la base embebida (socket-only, sin TCP):
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin
# Modo rescate (solo PostgreSQL, sin API — restore/inspección):
#   FUTUREFIN_MODE=db-only  →  ver scripts/restore-postgres.sh
```

### Rust
```bash
cd apps/api && cargo build
cargo test -p futurefin-engine           # engine unit tests only (no DB)
cargo test -p futurefin-engine -- <name> # single test

# Integration tests (require a running Postgres):
# 1) Start a dedicated test DB once (port 5433 to avoid clashing with dev):
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine
# 2) Run the full workspace test suite (each test gets its own schema, see .claude/tests.md):
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

### Frontend
```bash
npm run typecheck:web   # tsc --noEmit
npm run lint:web        # eslint
npm run build:web       # Vite production build → apps/web/dist/
npm test --workspace futurefin-web   # Vitest run (pure-function tests)
```

### Production deploy
```bash
docker compose --env-file .env up -d
```

## Architecture

### Workspace layout
```
Cargo workspace: apps/api + crates/domain + crates/engine
npm workspace:   apps/web (futurefin-web)
```

**crates/domain** — shared primitives: `UserId` (newtype over `Uuid`), re-exports `Decimal` and `Uuid`. No `f64` for monetary values anywhere in the domain.

**crates/engine** — pure projection math (`project_net_worth_series`, `first_month_per_asset_contribution_nominals`) plus historical-snapshot interpolation (`history.rs`: `evaluate_timeline`, linear-for-assets / French-amortization-for-liabilities, `month_index_of` / `add_months_signed`) y el runway de liquidez (`runway.rs`: `liquid_runway_months` — meses que cubren los activos líquidos componiendo su rentabilidad esperada y la inflación del gasto; lo consume `GET /v1/summary`. Desde v2.3.0 el caso **infinito** lo decide el **SWR**, no sobrevivir el tope de 1.200 meses: es indefinido ⟺ el gasto anual **grosseado** (mismo `gross_up_net_annual_fire` del target FIRE) ≤ `swr_pct` × líquidos; sobrevivir el tope devuelve `Months(1200)` como **suelo** («+100 años» en la UI)). No I/O, no DB; only `Decimal` arithmetic. Has unit tests.

**apps/api** — Axum HTTP server. Entry point: `main.rs` (bin), with shared crate modules in `lib.rs`. Key modules:
- `routes/mod.rs` — full route map; all routes under `/v1/` except `/health` and `/openapi.json`. `DefaultBodyLimit` caps requests at 1 MB globally, 16 MB on `/backup/user-import*`.
- `state.rs` — `AppState` (pool, cookie_secure, session_ttl_days, version)
- `error.rs` — `ApiError` → `(StatusCode, JSON {error, message})` via `IntoResponse`. `impl From<sqlx::Error>` detects SQLSTATE 23505 → `Conflict` (409), 23503 → `BadRequest`; handlers can just `?` any `sqlx::Error` without manual mapping.
- `auth/` — password hashing (Argon2id)
- `handlers/session.rs` — `require_session_user` reads cookie `ff_session` → validates against `sessions` table
- `handlers/api_tokens.rs` — tokens de API por usuario (Bearer `ffp_…`, solo se persiste el SHA-256; CRUD `/v1/api-tokens` por cookie) + `require_api_token`, la credencial del servidor MCP
- `mcp/` — servidor MCP embebido (`/mcp`, Streamable HTTP, rmcp 3.1): **50 tools de lectura, simulación y escritura** que llaman a las mismas core fns `*_core` que los handlers HTTP (cero deriva, Decimal-as-string intacto; la invalidación de cache vive dentro de las cores de mutación). Auth = middleware Bearer (`mcp/auth.rs`) con identidad y rol vivos por request; toda tool de escritura pasa por `require_mcp_write` (rol + toggle `installation.mcp_write_enabled`, editable en Ajustes → MCP) y las destructivas piden `confirm: true` (sin él devuelven un preview). `FUTUREFIN_MCP_ENABLED=0` lo desmonta
- `handlers/installation.rs` — singleton installation, FIRE settings, `require_installation_member`
- `handlers/membership.rs` — roles: `owner`, `member`, `viewer`; `role_can_write` used by handlers
- `handlers/person_view.rs` — `LedgerView` enum (`Household` / `Mine`) **plus helpers** `scope_where(table_alias)`, `next_arg_index()`, `bind_scope_as`, `bind_scope_scalar`. Use them instead of duplicating `match view { Household | Mine }` blocks — they enforce consistent placeholder ordering across both branches.
- `handlers/history.rs` — per-user net-worth **snapshots** under `/v1/history` (capture / backfill CRUD / interpolated series + `GET /v1/history/cashflow` tier-2). Manual snapshots of the user's asset + liability items; the engine (`history.rs`) reconstructs the past series between them. Snapshots are NOT projection inputs → their mutations do **not** invalidate the projection cache.
- `handlers/transactions/` — per-user **histórico de gasto mensual** under `/v1/transactions` (import CSV MyInvestor/N26, movimientos manuales, reglas de categorización, comparativa mes vs budget vs promedio ponderado, y **movimientos recurrentes**). Modules: `crud.rs`, `import.rs` (preview→confirm stateless, presets en `csv_presets.rs`), `reconcile.rs` (conciliación de transferencias, 3.5.0: pase automático determinista de importes opuestos a ≤5 días + par/desconciliación manual — un movimiento **conciliado** sigue visible pero queda fuera de TODOS los agregados de flujo), `rules.rs`, `summary.rs` (incluye el helper `transactions_12m_avg` que consumen los modos B y C, contando solo «meses reales»: los meses solo-recurrentes y las transferencias conciliadas se excluyen), `recurring.rs` (plantillas recurrentes + **convergencia**: desde 3.9.0 las instancias existen exactamente en los meses con datos reales, sin cursor), `schema.rs`. Las transacciones son inputs del engine **solo en los modos que usan transacciones** (`fire_settings.savings_source ∈ {transactions_avg (B), budget_income_real_expense (C)}`, gate `SavingsSource::uses_transactions()`; desde 3.9.0 las **ventanas del promedio son configurables por lado** — ingreso y gasto, meses + semántica): en esos casos las mutaciones invalidan la cache de proyección vía `invalidate_projection_if_savings_uses_transactions` (best-effort post-commit); con `savings_source = budget` (default, modo A) **ningún handler invalida** (contrato histórico intacto). `rules.rs`, previews y el borrado de una regla recurrente nunca invalidan (regresión: `transactions_projection_cache.rs`).
- `db.rs` — pool setup (`max=10, min=1, idle_timeout=10min, max_lifetime=30min`) + `sqlx::migrate!` runner. No more auto-repair loop; if a checksum mismatches in dev, fix manually via `DELETE FROM _sqlx_migrations WHERE version = X` and rerun.
- **`tests/`** — integration tests against a real Postgres (schema-isolated per test). See [`.claude/tests.md`](.claude/tests.md).

**apps/web** — React 19 + TypeScript + Vite. `App.tsx` is the composition root (auth gate + global state + route → view dispatch). All views, components, helpers and types live in separate modules — see [`.claude/frontend-structure.md`](.claude/frontend-structure.md).

### Key design decisions

**Authentication**: cookie `ff_session` (UUID), `HttpOnly`, `SameSite=Lax`. Session stored in DB with expiry. First user to register becomes installation owner automatically (`bootstrap_installation_as_owner_if_empty`).

**Installation singleton**: one row in `installation` per deployment. All financial data belongs to it. Users who register but aren't in `installation_memberships` are "pending" — they see no data until the owner approves them.

**Money**: always `rust_decimal::Decimal`. API serializes amounts as decimal strings (`serde-with-str`). The frontend receives and sends strings, never floats. Never use `f64` for financial values.

**Dual-port dev**: Vite `:8080`, API `:8081` (set in `.env.example`). `vite.config.ts` reads `FUTUREFIN_API_PORT` and `WEB_DEV_PORT` from repo-root `.env`. Docker image serves both on `:8080` via `WEB_STATIC_ROOT=/app/web`.

**Imagen autocontenida (3.0.0)**: en producción PostgreSQL 16 corre **dentro** del contenedor `futurefin` (socket Unix, sin TCP), supervisado por `apps/api/docker-entrypoint.sh` (PID 1): adopción de volúmenes 2.x (chown + REINDEX una vez), backup automático pre-migración con retención, auto-`pg_upgrade` (15+16 empaquetados), automigración one-shot desde una `DATABASE_URL` externa (modo deprecado, se elimina en 4.0.0) y apagado ordenado (API con graceful shutdown primero, después **SIGINT** al postmaster — nunca SIGTERM). El entrypoint **jamás borra un cluster** (solo `mv`), la imagen **no declara `VOLUME`** y aborta si no hay volumen montado en `PGDATA`. Runbook completo: skill `futurefin-run-and-operate`; trampas de la migración: `futurefin-failure-archaeology`.

**View scoping**: all ledger endpoints accept `?view=mine` to filter by `owner_user_id = current_user`. Default is `household` (full installation scope). This is a client-side filter, not an authorization boundary. Handlers must use `LedgerView::scope_where` + `bind_scope_as/scalar` so the two branches stay in sync.

**Reads never mutate**: liabilities with `payment_end_date < today` are **filtered** out of `GET /v1/liabilities`, `/summary`, `/budget` (derived lines), `/assets`, `/projection` via `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. They are **not** physically deleted. The legacy `purge_expired_liabilities` function was removed in May 2026 — GET handlers were silently issuing `DELETE` statements, violating HTTP semantics and impeding caching.

**Histórico por snapshots**: cada usuario guarda **snapshots manuales** (per-user) de sus activos y pasivos; el servidor interpola la serie histórica de patrimonio entre ellos (lineal para activos, amortización francesa para pasivos) y la sirve lista para pintar en `GET /v1/history/series`, unida a la proyección en un único chart temporal. Los snapshots **no son inputs del engine de proyección**: sus mutaciones (`/v1/history/*`) nunca llaman a `refresh_projection_after_mutation`, así que jamás invalidan la cache de proyección (test de regresión: `snapshot_mutations_do_not_touch_projection_cache`). El household histórico es la **suma** de las series interpoladas de cada usuario; `?view=mine` devuelve solo la propia. **Cash-flow tier-2** (v1.6.0, `GET /v1/history/cashflow`): los snapshots siguen siendo **la verdad** (la curva anclada pasa exacta por ellos), pero entre snapshots los deltas de las transacciones vinculadas a un asset **moldean** una curva fina (weekly/daily); si no hay transacciones o falla, el pasado queda idéntico a la serie de snapshots. Incluidos en `.ffbackup` v6.

**Transactions (histórico de gasto mensual, v1.6.0; pestaña «Movimientos» + recurrencia en v1.8.0)**: cada usuario importa CSV bancarios (MyInvestor/N26, autodetección + dedup por huella canónica) o mete efectivo a mano, los categoriza con reglas aprendidas al confirmar un import, y ve la comparativa mes real vs presupuesto vs **promedio ponderado** (denominador = meses con datos; ventanas `avg_window` 3/6/12/YTD/Todo) (`/v1/transactions/*`). Los **movimientos recurrentes** son plantillas per-user que `POST /v1/transactions/recurring/materialize` convierte en instancias mensuales (cursor de idempotencia, sin fechas futuras). Per-user (`owner_user_id`; lecturas con `?view=mine`). **Contrato de cache condicionado al modo (`fire_settings.savings_source`)**: en modo A (`budget`, default) las transacciones **no son inputs del engine** → sus mutaciones **nunca invalidan la cache de proyección**; en modo B (`transactions_avg`, v2.0.0) y modo C (`budget_income_real_expense`, income de presupuesto + gasto real) el engine deriva el ahorro mensual del **promedio ponderado 12m** de las transacciones (solo «meses reales»; los meses solo-recurrentes y las conciliadas se excluyen) → **sí son inputs**, y toda mutación que cambie el conjunto (conciliar/desconciliar incluido) invalida la cache (gating `SavingsSource::uses_transactions()` en `invalidate_projection_if_savings_uses_transactions`). Regresión de los tres casos: `transactions_projection_cache.rs`. **Conciliación de transferencias (3.5.0)**: nada se descarta en el import — un movimiento con pinta de transferencia entra con su kind natural y solo deja de contar como gasto/ingreso cuando el pase automático (importes exactamente opuestos, misma divisa, mismo owner, ≤5 días, cruzando toda la BD) o el usuario lo **concilian** con su contrapartida (`transactions.transfer_counterpart_id`, self-FK `ON DELETE SET NULL`; desconciliar persiste un rechazo anti-resurrección en `transfer_match_rejections`). Conciliado = visible con badge, excluido de todos los agregados de flujo (la curva fina del cashflow NO — modela saldo real). Cinco tablas (`transaction_imports`, `transactions`, `categorization_rules`, `recurring_transaction_rules`, `transfer_match_rejections`) incluidas en `.ffbackup` v8.

**Servidor MCP (v3.0.0; lectura+escritura desde los issues #2/#3)**: endpoint `/mcp` (Streamable HTTP, SDK oficial `rmcp`) en el mismo binario y puerto — 50 tools: lectura, `simulate_projection` (what-if puro, cache-neutral) y escritura con preview/confirm en las destructivas (desde 3.5.0 también `reconcile_transfers`/`unreconcile_transfer`; tras 3.5.0, `update_asset`/`update_liability` cierran la paridad CRUD del ledger). Dos credenciales, despachadas por prefijo del Bearer: **tokens de API** por usuario (`ffp_…`, Ajustes → MCP) y **access tokens OAuth** (`ffo_…`, v3.1.0); ambas pueden escribir. Ninguna congela nada — cada request re-resuelve membership y rol (revocar = corte inmediato, misma filosofía que las sesiones en DB), y la escritura exige además `role_can_write` + el toggle vivo `installation.mcp_write_enabled` (`require_mcp_write`; owner-only para `update_fire_settings`). Las tools reutilizan las core fns de los handlers (`summary_core`, `projection_series_cached`, cores de mutación con la invalidación FULL/COND/NONE dentro) — nunca dupliques SQL, validación ni tipos de respuesta en una tool. **La superficie MCP es derivada de la API HTTP y se mantiene en paridad**: todo cambio de rutas/handlers pasa la evaluación de paridad de [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md) antes de mergear (tool nueva/actualizada u omisión deliberada registrada). `/mcp` no va en OpenAPI (se autodescribe vía `tools/list`). Detalles: [`.claude/api-routes.md`](.claude/api-routes.md) sección MCP.

**OAuth 2.1 embebido (v3.1.0)**: el binario es también **authorization server + resource server** de `/mcp` — lo que exige el conector de claude.ai web. Módulo `apps/api/src/oauth/` (protocolo: metadata RFC 8414/9728 en `/.well-known/*` **con y sin sufijo `/mcp`**, DCR RFC 7591 abierto en `POST /oauth/register`, token endpoint con PKCE S256-only + refresh rotation + reuse-detection, revocación RFC 7009) y `handlers/oauth_consent.rs` (`/v1/oauth/*`, cookie: pantalla de consentimiento de la SPA + panel «Conexiones»). Tokens opacos hash-only con rol vivo (D14); el grant (una fila por app+usuario) es la unidad de revocación. **Jamás registres una ruta backend en `/oauth/authorize`** (la sirve el fallback SPA; un 405 no cae al fallback). Todo el protocolo cuelga del kill-switch `FUTUREFIN_MCP_ENABLED` **excepto** `/v1/oauth/connections` (siempre montado). Issuer derivado del request; override opcional `FUTUREFIN_PUBLIC_URL`. El login de la app NO cambia: OAuth delega acceso, no inicia sesión (la fila «OAuth login» de la arqueología sigue vigente para login-con-IdP). Detalles: [`.claude/api-routes.md`](.claude/api-routes.md) sección OAuth.

**OpenAPI**: generated via `utoipa`, served at `GET /openapi.json`. All handler structs annotated with `#[utoipa::path]`.

**CORS**: `CORS_ORIGINS` env var (comma-separated). Not required — defaults to localhost origins. Set explicitly only for cross-origin API access.

### Migrations
SQLx embed migrations in `apps/api/migrations/`. Run automatically on startup via `db::run_migrations`. Filenames: `YYYYMMDDHHMMSS_description.sql`. No auto-repair: a checksum mismatch fails loud and must be resolved by hand (e.g. `psql -c "DELETE FROM _sqlx_migrations WHERE version = X"` if the change is genuinely idempotent; en producción el `psql` es `docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin`). Antes de aplicar migraciones nuevas, el entrypoint escribe un backup automático `pre-migration-*.sql.gz` en el volumen `ffdata`.

## UI conventions

- **Monetary amounts**: no decimals, currency symbol after the number (`1.234 €`). Use `formatCurrencyAmount` / `formatCurrencyNumber` — never `toString()` or manual concatenation.
- **Percentages**: exactly one decimal, suffix ` %` (`3,5 %`). Use `formatPercentAmount` / `formatPercentDisplay`. The function already includes the suffix.
- **MetricCard additional info**: always goes in the `parenthetical` prop, not `suffix`. El paren-slot se reserva siempre (con `&nbsp;` cuando está vacío) para que las KPIs en la misma fila tengan baseline alineada.
- **Copy**: minimal — prefer short labels, empty states in a few words (`Sin datos.`).
- **Palette (V1 redesign)**: base monocromática (zinc) + único acento periwinkle. Verde/rojo **solo en cifras delta**, nunca en chrome decorativo. Las gráficas son la única zona donde se aceptan varios colores funcionales. **Nunca uses hex hardcoded en `App.css` o componentes — consume `var(--ff-*)`** definidos en [`apps/web/src/styles/theme.css`](apps/web/src/styles/theme.css). Detalles completos: [`.claude/design-system.md`](.claude/design-system.md).
- **Tema**: claro / oscuro / auto, controlado por `<html data-theme>`. Estado en `App.tsx` (`themePref`), helpers en [`apps/web/src/lib/theme.ts`](apps/web/src/lib/theme.ts), toggle en `Ajustes → Datos y sistema → Apariencia`. **Verifica claro y oscuro antes de mergear cualquier cambio visual.**
- **Iconografía**: set unificado en [`apps/web/src/components/icons.tsx`](apps/web/src/components/icons.tsx) — viewBox 16×16, `stroke="currentColor"`, `strokeWidth=1.5`. No introduzcas SVG nuevo fuera de ese archivo.
- **Charts pequeños**: usa [`MiniProjection`](apps/web/src/components/charts/MiniProjection.tsx) en lugar de SVG custom — comparte tokens con el chart grande y soporta `zoomY`, `clampToMonth`, `xAxis`, áreas escaladas al NW.

## Git workflow

**Branches:**
- `main` — rama de producción y de publicación. La imagen Docker se construye y publica **desde `main`** (el workflow `publish-image.yml` vive aquí). Es la rama por defecto.
- `dev` — desarrollo activo, **ramificada de `main`**. `main` es un **espejo completo de `dev`**: contiene exactamente lo mismo (CLAUDE.md, .claude/, .github/, workflows, código). No hay divergencia de `.gitignore` ni archivos exclusivos de una rama.

**Releases:**
1. Desarrollar en `dev`, hacer commit y push.
2. Bumpar versión en `apps/api/Cargo.toml` (sincronizar `Cargo.lock`) y añadir entrada en `CHANGELOG.md`.
3. **Merge completo `dev` → `main`** (`git checkout main && git merge dev`). Nunca copias parciales de archivos: `main` debe quedar idéntico a `dev`.
4. Push tag `vX.Y.Z` **desde `main`** → el workflow `publish-image.yml` (que vive en `main`) publica la imagen.
5. Volver a `dev` (`git checkout dev`) y seguir; mantener `dev` al día con `main`.

Tags published: `:X.Y.Z`, `:X.Y`, `:X`, `:latest`. Requiere secrets `DOCKERHUB_USERNAME` + `DOCKERHUB_TOKEN` en GitHub repo.

Before resuming work: `git pull --ff-only`. After push: pull again.
