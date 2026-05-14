# Changelog

All notable changes to FutureFin will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

## [1.0.9] — 2026-05-14

### Added
- **Backup `.ffbackup` cifrado por usuario**: Sustituye al export ZIP/CSV. Cada usuario exporta solo sus filas (`assets`, `liabilities`, `budget_entries`, `planning_flows` con `owner_user_id = self`) en un contenedor binario versionado cifrado con AES-256-GCM. La clave se deriva de la contraseña de cuenta vía Argon2id (m=19456, t=2, p=1) con sal aleatoria por export; AAD incluye `schema_version`, `user_id` y `exported_at`. Endpoints: `POST /v1/backup/user-export`, `POST /v1/backup/user-import/preview`, `POST /v1/backup/user-import` (replace-only, transaccional). El manifest queda en claro para que el servidor pueda rechazar `schema_version` futuras sin intentar descifrar.
- **Planning — mostrar hitos en el gráfico**: Cada planning flow tiene un nuevo flag `show_in_chart` (solo activable si hay `due_date`). Los hitos marcados se renderizan como líneas verticales en el gráfico de proyección. Migración `20260517120000_planning_flows_show_in_chart.sql`.
- **Per-asset projection series**: `GET /v1/projection/series` ahora devuelve `asset_series[]` (un array por activo con su valor mes a mes, paralelo a `points`). Permite renderizar el desglose por activo sin recalcular en el cliente. El engine deflacta cada serie con el mismo factor que `net_worth` cuando hay inflación activa.

## [1.0.8] — 2026-05-14

### Added
- **Presupuesto — fin de gasto**: Las entradas de gasto recurrente ahora admiten una fecha de fin opcional. Dos modos: "Al jubilarse" (el gasto deja de computarse en la proyección a partir del mes de jubilación) o "Hasta la fecha" (el gasto se cancela a partir del mes indicado). Los gastos que terminan al jubilarse también reducen el objetivo FIRE calculado por el modo `AnnualExpense`.

## [1.0.7] — 2026-05-14

### Changed
- **Docker build**: Node.js build stage upgraded from 22.14 to 24.15 (Active LTS, EOL April 2028). Aligns the production image with CI, which already ran on Node 24.

## [1.0.6] — 2026-05-14

### Improved
- **Projection API**: `GET /v1/projection/series` now returns `jubilacion_month_index` and `jubilacion_target_net_worth` — the FIRE milestone is computed server-side (gross-up + SWR division already run by the engine layer) instead of being duplicated in the browser.

### Fixed
- **Projection engine — FIRE is the sole retirement trigger**: `projection_target_age` has been removed entirely. The engine no longer enters retirement due to age; only reaching the FIRE target net worth triggers the retirement phase. This eliminates the visual gap where the "contributed capital" line stopped growing years before the Jubilación milestone marker.
- **Projection horizon — fixed 90-year lifespan**: The chart horizon is now computed as 90 years from the oldest household member's birth date (clamped 5–70 years, 30-year fallback when no birth date is set), replacing the manual "target age" setting that has been removed.

## [1.0.4] — 2026-05-13

### Added
- **Projection — Jubilación milestone**: The projection chart now marks the month when net worth reaches the FIRE target with an amber vertical line labelled "Jubilación". The "Trayectoria proyectada" panel shows it as a metric card with the target net worth and the estimated date.

### Fixed
- **Projection engine — contributions stop at retirement**: New contributions to `contributed_capital` now stop as soon as the portfolio reaches the FIRE target net worth (or `retirement_start_month`, whichever comes first). Previously, any budget surplus in retirement (e.g. persistent pension income exceeding expenses) was still being invested and counted as new contributed capital. The API computes the FIRE target from `fire_settings` (same SWR + tax gross-up logic as the frontend) and passes it to the engine as `fire_target_net_worth`.

## [1.0.3] — 2026-05-13

### Added
- **Budget — Persiste tras jubilación**: Each income entry now has a "Persists after retirement" toggle (default off). Income items marked as persisting continue to contribute to cash flow after the retirement age; all others stop. This drives a more realistic FIRE projection and a lower FIRE wealth target when passive/pension income is present.
- **Projection engine**: `income_retirement_monthly` field in `ProjectionInput`; simulation loop switches income at `retirement_start_month` instead of keeping it flat for the full horizon. `retirement_monthly_withdrawal` is always 0 — the income drop alone drives the portfolio drain.
- **FIRE target**: Annual need now subtracts persistent retirement income (`max(0, expense − income_retirement) × 12 / SWR` in annual-expense mode; `max(0, income − income_retirement) × 12 / SWR` in current-income mode).
- **Registration**: `birth_date` is now a required field at sign-up (was optional and had to be set separately).
- **Dev workflow**: `docker-compose.local.yml` + CLAUDE.md instructions for full-stack local testing without publishing to Docker Hub.

## [1.0.2] — 2026-05-12

### Fixed
- Docker healthcheck changed from `CMD` (exec form) to `CMD-SHELL` so `curl` resolves correctly via shell PATH; bash `/dev/tcp` fallback for images without `curl`
- `RUST_LOG` added to `docker-compose.yml` so container logs are visible by default

### Improved
- Startup log milestones: version, database connected, migrations applied, server config (port, session TTL, cookie_secure)

## [1.0.1] — 2026-05-12

### Infrastructure
- Single `docker-compose.yml` for production (Docker Hub image, no TLS overlay)
- Only `POSTGRES_PASSWORD` is required; all other vars have sane defaults
- `apps/api/Dockerfile` runtime stage now includes `curl` (required for healthcheck)
- Dev tooling (`CLAUDE.md`, `.claude/`, `.github/`) removed from `main` branch

## [1.0.0] — 2026-05-12

### First public release

**Auth & multi-user**
- Username + password authentication (Argon2id), session cookie `ff_session`
- Singleton installation per deployment; first user becomes owner automatically
- Owner approves pending registrations; roles: `owner`, `member`, `viewer`
- Household view (`default`) and personal view (`?view=mine`) scoped by `owner_user_id`

**Financial ledger**
- Assets: value, purchase price (cost basis Δ), liquidity flag, expected annual return, fixed + weighted contributions, weekly/monthly frequency
- Liabilities: principal (manual or derived from payment plan), APR, weekly/monthly payment schedule, auto-expiry
- Budget: persisted monthly income/expense lines; liability-derived debt payments included in snapshot
- Planning flows: upcoming one-off inflows and outflows with optional due dates
- Categories: CRUD per scope (asset, liability, income, expense)

**Analytics**
- Summary: net worth, total assets/liabilities, debt-to-assets ratio, financial health metrics (savings rate, runway, upcoming coverage), category and type-tag breakdowns
- Projection: monthly net-worth series via `futurefin-engine` (compound growth, debt service, asset contributions, planning cash adjustments, optional inflation deflation)
- FIRE / Jubilación tab: FIRE number modes (manual, annual expense × SWR, current income), capital-gains tax brackets, gap to target

**Infrastructure**
- Axum API, all routes under `/v1/`, OpenAPI at `/openapi.json`
- PostgreSQL + SQLx migrations (auto-run on startup)
- React + TypeScript + Vite SPA embedded in the Docker image
- Docker image: multi-arch (`linux/amd64`, `linux/arm64`), published to GHCR on `vX.Y.Z` tags
- NAS deploy: `docker-compose.yml`, imagen desde Docker Hub
- Backup: `GET /v1/backup/export.zip` (CSV ZIP, owner only)
