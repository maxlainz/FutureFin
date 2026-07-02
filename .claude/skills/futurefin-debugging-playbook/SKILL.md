---
name: futurefin-debugging-playbook
description: >
  Load this skill FIRST whenever a FutureFin session starts from a symptom: wrong or
  implausible projection/FIRE numbers, form preview disagrees with chart, HTTP 409/422/413/403/401
  responses, "VersionMismatch"/checksum error on startup, stale data after an edit, chart wrong
  only at one density or only in dark mode, broken table layout / overlapping action buttons,
  Docker container unhealthy (diagnosing WHY — for operating/restarting/rolling back the stack
  use futurefin-run-and-operate), login loop / session not sticking, Vite dev 404s on /v1. It maps
  each symptom to a first move and a discriminating experiment, and lists the traps that already
  cost real time. Do NOT use it for deep projection-model redesign (futurefin-projection-realism-campaign),
  for writing new tests (futurefin-validation-and-qa), for environment setup from scratch
  (futurefin-build-and-env), or for deploy/upgrade/backup operations (futurefin-run-and-operate).
---

# FutureFin Debugging Playbook

Symptom → triage runbook for FutureFin (self-hosted household finance app: Axum API +
`crates/engine` pure projection math + React/Vite SPA + Postgres 16). Facts verified against
the code as of 2026-07-02, v1.4.3.

Vocabulary you need (one line each; full domain detail in
`.claude/skills/futurefin-fire-domain-reference/SKILL.md`):

- **Installation**: singleton row per deployment; all financial data belongs to it. Users not in
  `installation_memberships` are "pending" and get **403** on data endpoints.
- **View scope**: `?view=mine` filters rows by `owner_user_id = current user`; default is
  `household` (whole installation). Client-side filter, NOT an authorization boundary.
- **Density**: `GET /v1/projection/series?density=hybrid` serializes ~82 non-equidistant points
  (months 0..12 monthly, then 24, 36, … yearly) instead of ~841 monthly points. Same internal
  compute; only serialization differs (`apps/api/src/state.rs`, enum `Density`).
- **Nominal model**: since v1.2.0 the whole simulation runs in nominal euros; only the FIRE
  target grows with inflation: `target(m) = base × (1+inf/100)^(month_index/12)`.
- **FIRE target / SWR / gross-up**: net worth needed to retire; annual need is grossed-up for
  capital-gains tax (closed-form per bracket since v1.3.0) and divided by the Safe Withdrawal Rate.

## Session discipline (read before touching anything)

1. **Reproduce before fixing.** Get the failing request/render on your screen with exact inputs.
   If you cannot reproduce, you are guessing.
2. **Root cause before patching.** The table-CSS saga (v1.0.18 → v1.0.20) burned three releases
   on one bug because two plausible "fixes" shipped before anyone found the mechanism (trap 8;
   full chronicle: futurefin-failure-archaeology §2.3). If your second attempt at the same
   symptom is another CSS/behavior tweak, stop and find the mechanism.
3. **Check both scopes.** Reproduce with `?view=household` (default) AND `?view=mine`. Scope
   bugs historically came from hand-written duplicate SQL branches with inverted bind order
   (live case in `budget.rs`, fixed v1.3.0 by the `LedgerView::scope_where` helpers).
4. **Check both densities.** `?density=monthly` and `?density=hybrid` — index-based math is
   correct on monthly and silently wrong on hybrid (see trap 6).
5. **Check both themes.** Light and dark (`<html data-theme>`, toggle in Ajustes → Datos y
   sistema → Apariencia) before calling any visual bug fixed. Non-negotiable per CLAUDE.md.
6. **Never "fix" a 4xx by loosening validation.** A 422 on `fire_number_mode`, a 409 on a
   unique key, a 413 on body size are deliberate contracts (v1.3.0 made them *stricter* on
   purpose). Fix the caller or take the change through
   `.claude/skills/futurefin-change-control/SKILL.md`.
7. **Reads must never mutate.** If your fix makes a GET handler write, stop (v1.3.0 removed
   exactly that pattern: `purge_expired_liabilities`).

## Master triage table

| Symptom | First move | Likely cause | Detail |
|---|---|---|---|
| Projection numbers plausible but wrong (no error anywhere) | Confirm it is a model question, not a serving bug: same wrongness at both densities, both views, cache cold | Engine economic model (silent-error zone) | Trap 1 → route deep work to `futurefin-projection-realism-campaign` |
| FIRE target in Jubilación form preview ≠ chart/server value | Run both parity suites (commands in trap 2) | Client/server duplicated FIRE math drifted | Trap 2 |
| HTTP 409 on create/update | Identify which unique constraint fired | `From<sqlx::Error>`: SQLSTATE 23505 → 409 Conflict | Trap 3 |
| HTTP 400 "referenced record missing" | Check the FK you sent | SQLSTATE 23503 → 400 | Trap 3 |
| HTTP 422 | Inspect the JSON body you sent | Axum `Json<T>` deserialization rejection (e.g. unknown `fire_number_mode`) | Trap 3 |
| HTTP 413 | Measure request size | Body limits: 1 MiB global, 16 MiB only on `/v1/backup/user-import*` | Trap 3 |
| HTTP 403 on every data endpoint but login works | Check `installation_memberships` for the user | Pending user (registered, not approved) | Trap 3 |
| API refuses to start: migration `VersionMismatch` / checksum error | Read the version number in the error | Edited already-applied migration; auto-repair removed v1.3.0 | Trap 4 |
| Edited an asset/budget line, projection chart unchanged | Grep the mutating handler for `refresh_projection_after_mutation` | Missing cache invalidation | Trap 5 |
| Chart wrong only with `density=hybrid` (deflation, X positions, milestones) | Diff hybrid vs monthly responses | Array-index math on non-equidistant points | Trap 6 |
| Visual bug only in dark mode | Toggle theme, inspect computed CSS | Hardcoded hex instead of `var(--ff-*)`/`var(--proj-*)` token | Trap 7 |
| Table columns overlap / content hidden under action buttons | Inspect the `<td>`'s computed `display` | `display` other than `table-cell` set on a `<td>` | Trap 8 |
| `docker ps` shows container `unhealthy` | `docker compose logs -f futurefin` | Healthcheck exec-form vs shell, or app actually down | Trap 9 |
| Login "succeeds" but next request is 401; login loop | Check `Set-Cookie` in devtools: is `Secure` set? Are you on HTTP? | `COOKIE_SECURE=true` behind non-HTTPS | Trap 10 |
| Split-dev: UI loads but every `/v1/*` call 404s or connection-refused | `curl http://127.0.0.1:8081/v1/health` directly | Vite proxy port mismatch (`.env` at repo root) | Trap 11 |

## Traps, stories, and discriminating experiments

### 1. Wrong projection numbers — the silent-error zone

**Story.** The projection engine is the most-rewritten area of the codebase: inflation model
rewrite #1 in v1.0.12 (2026-05-16, "real pure": deflate returns, everything in today-€) produced
incoherent behavior — assets draining *before* retirement with inflation on — and was replaced
wholesale in v1.2.0 (2026-05-17) by the current all-nominal model where only the FIRE target
inflates. Nothing ever threw an error; the numbers just looked plausible and were wrong. Per the
owner, this is the hardest live problem in the project.

**Discriminating experiment.** First rule out serving bugs (they are cheap to eliminate):

```bash
# Same login cookie; compare monthly vs hybrid at the SAME month_index values.
curl -sb cookies.txt "http://127.0.0.1:8080/v1/projection/series?density=monthly" > /tmp/m.json
curl -sb cookies.txt "http://127.0.0.1:8080/v1/projection/series?density=hybrid"  > /tmp/h.json
```

If hybrid points disagree with the monthly points at matching `month_index` → serving/decimation
bug (trap 6), not the model. If both densities and both views agree and are wrong → engine model.
For the model itself, write a minimal `ProjectionInput` reproduction as a unit test in
`crates/engine/src/projection.rs` (`cargo test -p futurefin-engine`, no DB needed) — the engine
is pure, so every model bug is reducible to one deterministic test. **Deep model work (changing
inflation semantics, cascade, drain, FIRE trigger) is out of scope here** — hand off to
`.claude/skills/futurefin-projection-realism-campaign/SKILL.md`.

### 2. FIRE target: form preview vs chart disagree

**Story (short).** FIRE math is deliberately duplicated client-side (`apps/web/src/lib/fire.ts`,
instant Jubilación preview) and server-side (engine + `handlers/projection.rs`). Two real
divergences shipped: (a) v1.3.0 — `RetirementView` fed the preview `expense_regular_…` where the
server used `expense_retirement_monthly_equivalent` (2–3× divergence); (b) a duplicated-formula
off-by-one, fixed by the single helper `fire_target_at_month_index`. Full chronicle:
futurefin-failure-archaeology §2.4–2.5.

**Discriminating experiment.** Both sides consume one canonical fixture,
`apps/api/tests/fixtures/fire-parity.json`. Run both suites:

```bash
# Server side (needs the test Postgres from CLAUDE.md running on :5433):
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test -p futurefin-api --test fire_parity
# Client side:
npm test --workspace futurefin-web -- fire
```

If exactly one suite fails → that side drifted from the fixture; fix that side. If both pass but
the UI still disagrees → the bug is in what the *view* feeds `lib/fire.ts` (wrong field, wrong
scope), not in the math — re-read the v1.3.0 incident above. If you legitimately change tax
brackets/gross-up, regenerate the fixture's expected values and make BOTH suites pass
(see `.claude/skills/futurefin-validation-and-qa/SKILL.md`).

### 3. HTTP status decoding (409 / 422 / 413 / 403 / 400)

All mappings live in `apps/api/src/error.rs` and `apps/api/src/routes/mod.rs`. Verified table:

| Status | Mechanism | Meaning |
|---|---|---|
| 409 `conflict` | `impl From<sqlx::Error>`: SQLSTATE 23505 (unique violation) → `ApiError::Conflict` | Duplicate key (e.g. username taken). The DB constraint is the contract. |
| 400 `bad_request` "referenced record missing" | SQLSTATE 23503 (FK violation) | You sent an id that doesn't exist (e.g. category_id). |
| 422 | Axum's `Json<T>` extractor rejection (never constructed in `error.rs`) | Body failed deserialization — e.g. `fire_number_mode: "foobar"` (strict since v1.3.0; it used to silently default). |
| 413 | `DefaultBodyLimit`: 1 MiB global; 16 MiB only on `POST /v1/backup/user-import` and `/preview` (base64 inflates ~33%) | Body too large. Covered by `apps/api/tests/body_limits.rs`. |
| 403 `forbidden` | `require_installation_member` (`handlers/installation.rs`): user has a session but no row in `installation_memberships` | **Pending user** awaiting owner approval — this is the #1 cause of "everything is 403". Also: role lacks write permission (`role_can_write`, viewer role). |
| 401 `unauthorized` | `require_session_user` (`handlers/session.rs`): missing/expired `ff_session` cookie | See trap 10 before blaming the session table. |
| 500 `internal` | Any other `sqlx::Error` → `ApiError::Db` | Real bug. Check logs — the v1.0.10 backup-export 500 (2026-05-15) was a SELECT still naming `b.label`/`b.frequency` after migration `20260505180000_budget_entries_monthly_only` dropped them. Lesson: after any column drop, grep handlers for the column name. |

**Discriminating experiment for 403 vs role issues:**

```bash
docker compose exec futurefin-database psql -U futurefin -d futurefin \
  -c "SELECT u.username, m.role FROM users u LEFT JOIN installation_memberships m ON m.user_id = u.id;"
```

Row with `role = NULL` → pending user; approve via the owner's UI (Ajustes) or the
`/v1/installation/pending-users` endpoints. Row with `role = viewer` on a write → expected 403.

### 4. Migration checksum mismatch on startup

**Story.** Until v1.3.0 a 12-round "auto-repair" loop silently patched checksums for a hardcoded
list of migration versions — masking real drift. It was deleted (2026-05-18); now
`sqlx::migrate!().run()` runs straight and a checksum mismatch **fails loud at startup** by
design (`apps/api/src/db.rs`).

**Triage.** The error names the version. Decide which case you are in:

- Someone edited an already-shipped migration file (forbidden — see change-control skill):
  restore the original file content from git history. Never re-edit shipped migrations.
- The change is genuinely idempotent (e.g. a deliberate squash in dev): manually clear the ledger
  row and restart so it re-applies:

```bash
docker compose exec futurefin-database psql -U futurefin -d futurefin \
  -c "SELECT version, description, success FROM _sqlx_migrations ORDER BY version DESC LIMIT 10;"
docker compose exec futurefin-database psql -U futurefin -d futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>;"
```

31 migration files in `apps/api/migrations/` as of 2026-07-02 (`ls apps/api/migrations | wc -l`).

### 5. Stale projection after a mutation (cache invalidation)

**Story.** v1.4.0 (2026-05-19) added an in-memory projection cache: `AppState.projection_cache`,
sliding 60-min TTL, keyed `(installation_id, view, owner_user_id, density)`
(`apps/api/src/state.rs`). Every handler that mutates anything feeding the simulation (assets,
liabilities, budget entries, planning flows, allocation rules, installation FIRE/inflation
settings, `user.birth_date`) must call `refresh_projection_after_mutation(state, iid, uid)`
(`handlers/projection.rs`), which invalidates all entries for the installation. Warm-up runs
**only after login**, deliberately never after mutation: two concurrent post-mutation warm-ups
could finish out of order and leave stale data cached — do not "optimize" this back in.

**Discriminating experiment.** Cache hits/misses are logged at info:

```bash
docker compose logs -f futurefin | grep -E "projection cache (HIT|MISS|invalidated)"
```

Mutate, then GET the series. Expected: `projection cache invalidated by installation` then
`cache MISS, computing`. If you see `cache HIT` right after your mutation → the mutating handler
forgot the invalidation call. Verify:

```bash
grep -rn "refresh_projection_after_mutation" apps/api/src/handlers/ | cut -d: -f1 | sort -u
```

End-to-end check: `bash scripts/smoke-projection-cache.sh` (API on :8080; pass
`SMOKE_USER`/`SMOKE_PASS` if the DB already has users). Also rule out the *frontend* holding
stale state: the API can be fresh while a view didn't reload the series (v1.4.0 added explicit
reload in `saveFireSettingsPatch` for exactly this).

### 6. Chart wrong only with `density=hybrid` — the index vs `month_index` class

**Story (short).** v1.4.2: the chart deflated points by **array index** instead of
`month_index` — invisible at monthly density (index == month), wrong under hybrid's
non-equidistant points. Rule: any `points[i]` arithmetic assuming equidistant points breaks
under decimation; derive everything from `p.month_index` (chart does, see
`ProjectionNetWorthChart.tsx` ~line 191). Full chronicle: futurefin-failure-archaeology §2.8.

**Discriminating experiment.** In browser devtools → Network, the two-phase load fires both
requests; compare rendering right after the hybrid response vs after the monthly one arrives
(throttle network to widen the window). Or fetch both densities with curl (trap 1 commands) and
check your suspect computation by hand at `month_index = 24` (the first non-monthly point).
Server-side note: milestones and FIRE crossover are computed on the **full** 840-month series
before decimation, so if milestones are wrong at hybrid only, suspect *client* math, not the API.

### 7. Dark-mode-only visual bugs

**Story.** v1.4.0 shipped the theme system and immediately hit the pattern: the chart tooltip
had dark text on a dark background because a color was hardcoded rather than tokenized (fixed by
forcing the tooltip theme-independent: `#fafafa` on `rgba(10,10,10,0.92)`).

**Discriminating experiment.** Toggle `data-theme` on `<html>` in devtools (or Ajustes →
Apariencia; `auto` follows `prefers-color-scheme` live). Inspect the broken element's computed
color: if it is a raw hex not present in `apps/web/src/styles/theme.css`, that's the bug.

```bash
grep -rnE "#[0-9a-fA-F]{3,8}" apps/web/src/App.css apps/web/src/components/ apps/web/src/views/ | grep -v theme.css
```

Fix by consuming `var(--ff-*)` / `var(--proj-*)` tokens — never by adding a second hardcoded hex
for dark. Verify BOTH themes before closing.

### 8. Table/CSS layout breakage — the `<td>` display saga

**Story (short).** v1.0.18 → v1.0.20: overlapping action buttons resisted two plausible fixes
because the mechanism was `display: inline-flex` applied **directly to a `<td>`**, ejecting the
cell from the table layout model; the real fix wraps the buttons in an inner `<div>` and leaves
the `<td>` at default `table-cell`. Full chronicle: futurefin-failure-archaeology §2.3.

**Discriminating experiment.** For any table misrendering: devtools → select the `<td>` → check
computed `display`. Anything other than `table-cell` on a `<td>` (or non-`table-row` on `<tr>`)
is the root cause; move the styling to an inner wrapper element. Do not reach for
sticky/z-index/overflow hacks until every cell reports `table-cell`.

### 9. Docker container unhealthy

**Story.** v1.0.2 (2026-05-12): the healthcheck used exec-form `CMD`, so `curl` was not resolved
via shell PATH and the check failed even with a healthy app; changed to `CMD-SHELL` with a bash
`/dev/tcp` fallback for curl-less images (v1.0.1 had already added `curl` to the runtime stage
for this). Same release added `RUST_LOG` to `docker-compose.yml` because container logs were
empty by default — if you see no logs at all, suspect `RUST_LOG` unset, not a dead app.

**Discriminating experiment.**

```bash
docker compose ps                          # which container is unhealthy?
docker compose logs -f futurefin           # app logs (startup milestones: version, DB connected,
                                           # migrations applied, server config)
docker compose exec futurefin sh -c "curl -sf http://localhost:8080/v1/health"  # inside
curl -sf http://127.0.0.1:8080/v1/health   # outside (compose maps APP_PORT, default 8080)
```

Inside-OK / outside-fails → port mapping or host firewall. Both fail with logs showing a
migration error → trap 4. Logs empty → `RUST_LOG` missing from the environment (compose default:
`futurefin_api=info,tower_http=info,sqlx=warn`). DB container unhealthy → its own
`pg_isready`-based healthcheck; check `POSTGRES_PASSWORD` in `.env`.

### 10. Login/session issues — `cookie_secure` behind non-HTTPS

**Mechanism (verified).** `main.rs` reads `COOKIE_SECURE` (default `false`); `auth.rs` sets the
`ff_session` cookie with `.secure(state.cookie_secure)`. If `COOKIE_SECURE=true` and you access
the app over plain HTTP (typical NAS/LAN deploy), the browser **silently drops** the Secure
cookie: login returns 200, but every subsequent request has no cookie → 401 → login loop with no
server-side error whatsoever.

**Discriminating experiment.** Devtools → Network → the `POST /v1/auth/login` response: does
`Set-Cookie: ff_session=...` include `Secure` while the page URL is `http://`? That's it. Also
check devtools → Application → Cookies: is `ff_session` present at all? Server config is logged
at startup: `docker compose logs futurefin | grep "server config"` shows `cookie_secure` and
`session_ttl_days`. If the cookie IS stored and you still get 401, then check expiry in the DB
(`SELECT * FROM sessions WHERE ...`) — sessions have a TTL (`SESSION_TTL_DAYS`).

### 11. Vite proxy 404s in split-dev

**Mechanism (verified in `apps/web/vite.config.ts`).** The dev proxy forwards `/v1`, `/health`,
`/openapi.json` to `http://127.0.0.1:${FUTUREFIN_API_PORT ?? 8081}`. It loads env from the
**repo root** `.env` (not `apps/web/.env`). The API's own port comes from `PORT` in `.env`
(dev convention: `PORT=8081`). Failure modes: (a) API not running or on a different port →
proxy ECONNREFUSED/404; (b) you set `PORT=8080` (API-only mode) while also running Vite on 8080;
(c) edited the wrong `.env`; (d) Vite has `strictPort: false`, so if 8080 is busy it silently
moves to 8081+ and can collide with the API.

**Discriminating experiment.**

```bash
curl -s http://127.0.0.1:8081/v1/health   # API direct — works? API is fine, proxy is the problem
curl -s http://127.0.0.1:8080/v1/health   # through Vite — 404/refused? check .env:
grep -E "^(PORT|FUTUREFIN_API_PORT|WEB_DEV_PORT)" .env
```

Watch the Vite startup banner for the actual port it bound. Full environment recreation:
`.claude/skills/futurefin-build-and-env/SKILL.md`.

## Where the evidence lives

- **API logs**: `RUST_LOG` env filter; default (in `main.rs` and compose)
  `futurefin_api=info,tower_http=info,sqlx=warn`. For SQL statements set `sqlx=debug`; for
  request traces `tower_http=debug`. Docker: `docker compose logs -f futurefin`. Split-dev:
  the `cargo run` terminal. Startup milestones log version, DB connect, migrations, server config.
- **Projection cache**: info-level lines `projection cache HIT` / `MISS, computing` /
  `invalidated by installation` / `warm-up household projection` (in `handlers/projection.rs`
  and `state.rs`). Script: `scripts/smoke-projection-cache.sh`.
- **Migration ledger**: table `_sqlx_migrations` (version, description, checksum, success).
- **Browser devtools**: Network tab shows `content-encoding: gzip` (CompressionLayer, responses
  >1 KB), the `density` field echoed in the projection response body, `Set-Cookie` flags, and
  the two-phase hybrid+monthly request pair. Application tab: `ff_session` cookie, `localStorage`
  theme pref.
- **Database**: `docker compose exec futurefin-database psql -U futurefin -d futurefin`
  (split-dev: `psql "postgres://futurefin:futurefin@127.0.0.1:5432/futurefin"`).
- **History**: `CHANGELOG.md` documents root causes per release — grep it for your symptom before
  re-deriving anything. Full incident chronicle:
  `.claude/skills/futurefin-failure-archaeology/SKILL.md`.

## When NOT to use this skill

- **Deep projection-model work** (inflation semantics, allocation cascade, retirement drain,
  FIRE realism): this playbook only gets you to "it's the model"; the campaign owns the rest →
  `.claude/skills/futurefin-projection-realism-campaign/SKILL.md`.
- **Writing or extending tests**, TestApp harness, parity fixtures →
  `.claude/skills/futurefin-validation-and-qa/SKILL.md`.
- **Setting up dev environment / build failures from scratch** →
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- **Deploy, upgrade, rollback, backups in production** →
  `.claude/skills/futurefin-run-and-operate/SKILL.md`.
- **Env var / flag reference** (what does `COOKIE_SECURE` do, all axes) →
  `.claude/skills/futurefin-config-and-flags/SKILL.md`.
- **Deciding whether a fix is allowed** (migrations, breaking changes, releases) →
  `.claude/skills/futurefin-change-control/SKILL.md`.
- **Measurement recipes beyond triage** (benchmarks, curl instrumentation) →
  `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.

## Provenance and maintenance

Written 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`). All mechanisms verified by reading
code, not by running services. Re-verify before trusting:

- Error mapping (23505→409, 23503→400): `grep -n "23505\|23503" apps/api/src/error.rs`
- Body limits (1 MiB / 16 MiB): `grep -n "BODY_LIMIT" apps/api/src/routes/mod.rs`
- Pending-user 403: `grep -n "Forbidden" apps/api/src/handlers/installation.rs`
- Cache TTL/key/invalidation: `grep -n "PROJECTION_CACHE_TTL\|ProjectionCacheKey\|invalidate_projection" apps/api/src/state.rs`
- No warm-up after mutation rationale: `grep -n -B8 "refresh_projection_after_mutation" apps/api/src/handlers/projection.rs`
- Hybrid density pattern: `grep -n -A8 "density_month_indices" apps/api/src/handlers/projection.rs`
- Chart uses `month_index`: `grep -n "month_index" apps/web/src/views/ProjectionNetWorthChart.tsx`
- Cookie secure flag: `grep -n "COOKIE_SECURE" apps/api/src/main.rs && grep -n "secure(" apps/api/src/handlers/auth.rs`
- Vite proxy ports: `grep -n "FUTUREFIN_API_PORT\|WEB_DEV_PORT" apps/web/vite.config.ts`
- Healthcheck + default RUST_LOG: `grep -n "CMD-SHELL\|RUST_LOG" docker-compose.yml`
- Migration count (31 as of 2026-07-02): `ls apps/api/migrations | wc -l`
- Default log filter: `grep -n "EnvFilter" apps/api/src/main.rs`
- Parity fixture still dual-consumed: `grep -rn "fire-parity.json" apps/api/tests/ apps/web/src/`
- Incident quotes (v1.0.2, v1.0.10, v1.0.12, v1.0.18–20, v1.2.0, v1.3.0, v1.4.0, v1.4.2): `CHANGELOG.md`
- Stale-doc warning: `.claude/data-model.md` / `.claude/engine.md` still mention the removed
  `projection_target_age` (dropped in v1.0.6); trust `handlers/projection.rs` instead.
