# Changelog

All notable changes to FutureFin will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

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
