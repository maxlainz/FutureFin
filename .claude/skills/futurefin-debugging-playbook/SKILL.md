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
the code as of 2026-07-02, v1.4.3; the container/DB sections re-verified 2026-08-16 for
**v3.0.0** (self-contained image: PostgreSQL 16 embedded in the single `futurefin` container).

Vocabulary you need (one line each; full domain detail in
`.claude/skills/futurefin-fire-domain-reference/SKILL.md`):

- **Installation**: singleton row per deployment; all financial data belongs to it. Users not in
  `installation_memberships` are "pending" and get **403** on data endpoints.
- **Single container (3.0.0)**: production is ONE service, `futurefin`. PostgreSQL 16 runs
  inside it, **socket-only** at `/var/run/postgresql` — no TCP listener, no port, no
  `futurefin-database` service. A bash entrypoint (`apps/api/docker-entrypoint.sh`) supervises
  both processes. DB access is always `docker compose exec futurefin psql -h /var/run/postgresql
  -U futurefin -d futurefin`. Dev Postgres is now a **separate, autonomous** compose:
  `docker compose -f docker-compose.dev.yml up -d` (project `futurefin-dev`, volume `devdata`,
  `127.0.0.1:5432`), replacing the old `docker-compose.split-dev.yml` override.
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
| `docker ps` shows container `unhealthy` | `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/v1/ready` | `/v1/ready` 503 = embedded Postgres really down (3.0.0 dropped the `/dev/tcp` fallback that used to mask it); or `FUTUREFIN_MODE=db-only` | Trap 9 |
| Container exits immediately, FATAL `no persistent volume is mounted at /var/lib/postgresql/data` | `docker inspect -f '{{json .Mounts}}' futurefin` | Deliberate anti-data-loss guard: nothing mounted at `$PGDATA` | Trap 12a |
| FATAL `DATABASE_URL is set and the embedded volume is empty, but the external database does not answer` | `docker compose config \| grep -n DATABASE_URL` | A leftover `DATABASE_URL` (typically a dev `.env` sitting next to the prod compose) with an empty volume | Trap 12b |
| Boxed `DEPRECATED … base de datos EXTERNA` warning, app otherwise fine | `docker inspect -f '{{json .Mounts}}' futurefin` | External-compat mode: 2.x compose running the 3.x image (e.g. after watchtower) | Trap 12c |
| FATAL `pre-migration backup FAILED` | Check free space/permissions on the `ffdata` volume | The automatic pre-migration dump could not be written; startup aborts **on purpose** | Trap 12d |
| FATAL `cannot connect as role 'futurefin'` while adopting a 2.x cluster | Recall the `POSTGRES_USER` of the 2.x install | Adopted cluster whose superuser role is not `futurefin` | Trap 12e |
| `pg_upgrade needed: PostgreSQL 15 -> 16` followed by a failure | Read the pg_upgrade logs inside the `ffdata` volume | Major upgrade aborted; old cluster preserved untouched (nothing is ever deleted) | Trap 12f |
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
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
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
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
  -c "SELECT version, description, success FROM _sqlx_migrations ORDER BY version DESC LIMIT 10;"
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
  -c "DELETE FROM _sqlx_migrations WHERE version = <X>;"
```

34 migration files in `apps/api/migrations/` as of 2026-08-16 (`ls apps/api/migrations | wc -l`;
31 as of 2026-07-02).

**3.0.0 note.** A checksum mismatch now surfaces *after* the entrypoint milestones, and the
automatic pre-migration dump (`/var/lib/futurefin/backups/pre-migration-*.sql.gz`) was already
written before the API tried to migrate — that is your rollback material. If the container is
crash-looping and you need psql without the API, stop it first and start the rescue mode
(PostgreSQL only, API not started): `docker compose stop futurefin && docker compose run --rm
-e FUTUREFIN_MODE=db-only futurefin` — never two postmasters on the same volume.

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

### 9. Docker container unhealthy (single container since 3.0.0)

**Story.** v1.0.2 (2026-05-12): the healthcheck used exec-form `CMD`, so `curl` was not resolved
via shell PATH and the check failed even with a healthy app; changed to `CMD-SHELL` **plus** a
bash `</dev/tcp/...` fallback for curl-less images (v1.0.1 had already added `curl` to the
runtime stage for this). Same release added `RUST_LOG` to `docker-compose.yml` because container
logs were empty by default — if you see no logs at all, suspect `RUST_LOG` unset, not a dead app.

**What changed in 3.0.0.** The `CMD-SHELL` form stays (the v1.0.2 incident is still live), but
the probe now hits **`/v1/ready`, not `/v1/health`**, and the `</dev/tcp/...` fallback was
**removed on purpose**: a TCP-connect fallback answers as long as the Axum listener is alive, so
it would mask a 503 from `/v1/ready` and mark the container *healthy with the database down* —
exactly the failure that matters now that Postgres lives inside the same container. There is no
separate DB healthcheck anymore, because there is no separate DB container. The comment in
`docker-compose.yml` says this out loud: do not re-add the fallback.

Consequences for triage:

- **`unhealthy` now means the embedded Postgres is genuinely down (or unreachable via its socket),
  or the API process died.** It is no longer "the probe is flaky".
- **In rescue mode (`FUTUREFIN_MODE=db-only`) `unhealthy` is the expected state** — that mode
  starts PostgreSQL and deliberately does NOT start the API, so nothing answers `/v1/ready`.
- The first boot after upgrading from 2.x does chown + REINDEX + backup; that is why
  `start_period` is 120 s. `stop_grace_period` is 60 s so the PG checkpoint completes on stop
  (watchtower ignores it — set `WATCHTOWER_TIMEOUT=60s`).

**Discriminating experiment.**

```bash
docker compose ps                            # single service now: futurefin
docker compose logs -f futurefin             # THREE interleaved sources (see below)
curl -s -o /dev/null -w 'ready:%{http_code}\n'  http://127.0.0.1:8080/v1/ready
curl -s -o /dev/null -w 'health:%{http_code}\n' http://127.0.0.1:8080/v1/health
docker compose exec futurefin sh -c 'curl -fsS http://127.0.0.1:8080/v1/ready'   # inside
docker compose exec futurefin pg_isready -h /var/run/postgresql -U futurefin     # embedded PG
```

| Observation | Conclusion |
|---|---|
| `/v1/health` 200 but `/v1/ready` 503 | API alive, embedded Postgres down/not accepting → read the PostgreSQL lines in the log; check the `ffdata`/`pgdata` volumes for disk space |
| Both 503/refused from outside, OK inside the container | Port mapping (`APP_PORT`) or host firewall |
| Both fail, logs show a migration error | Trap 4 |
| No logs at all | `RUST_LOG` missing (compose default `futurefin_api=info,tower_http=info,sqlx=warn`) — the entrypoint and PostgreSQL still log regardless, so *truly* empty logs mean the container never started: read `docker inspect futurefin` |
| Container never reaches healthy and the log ends at a `FATAL:` line | Trap 12 (startup guards) |
| `pg_isready` OK inside, `/v1/ready` still 503 | The API's own pool is broken (custom `DATABASE_URL`?) — see trap 12b/12c |

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

**3.0.0 note.** The split-dev *workflow* is unchanged (`cargo run` + `npm run dev:web`), but its
Postgres no longer comes from `docker-compose.yml -f docker-compose.split-dev.yml`: use the
standalone `docker compose -f docker-compose.dev.yml up -d` (project `futurefin-dev`, container
`futurefin-dev-db`, volume `devdata`, published on `127.0.0.1:5432`). If `cargo run` reports
"connection refused" on 5432 after upgrading, that override is what disappeared.

### 12. Startup FATALs of the self-contained image (3.0.0)

The entrypoint (`apps/api/docker-entrypoint.sh`) prefixes every line with
`[futurefin-entrypoint]` and refuses to start in situations where continuing could lose data.
**These aborts are features.** Golden rule of the entrypoint: it NEVER deletes a cluster — old
or partial clusters are moved aside (`$PGDATA/pgdata_old_<major>`,
`/var/lib/futurefin/failed-automigration-<ts>`), never removed. So a failed start is recoverable
by construction; do not "clean up" volumes to make an error go away.

**12a. `FATAL: no persistent volume is mounted at /var/lib/postgresql/data`.**
First move: `docker inspect -f '{{json .Mounts}}' futurefin`. The guard fires when nothing is
mounted at `$PGDATA`, because the database would then live in the container's writable layer and
die with the container. Discriminating experiment: the same image with
`-v <somevolume>:/var/lib/postgresql/data` starts normally. Fix: mount the volume (the shipped
`docker-compose.yml` does). `FUTUREFIN_ALLOW_EPHEMERAL_DB=1` bypasses it and is **only** for
throwaway containers (CI, a quick `--version` probe); it logs a loud warning. CI pins this
behavior in the "Image sanity (PG majors, label, no-volume guard)" step.

**12b. `FATAL: DATABASE_URL is set and the embedded volume is empty, but the external database
does not answer`.** First move: `docker compose config | grep -n DATABASE_URL` — the usual cause
is a dev `.env` sitting next to the production compose, or a leftover 2.x variable. With a
`DATABASE_URL` pointing outside the container **and** an empty `$PGDATA`, the entrypoint assumes
you are migrating from an external database and waits `FUTUREFIN_EXTERNAL_WAIT_SECS` (60 s) for
it; it refuses to silently start with an empty database. Discriminating experiment: unset
`DATABASE_URL` → the container initializes a fresh cluster (`initializing fresh PostgreSQL 16
cluster`) and starts. Fix: either remove `DATABASE_URL` (fresh install) or bring the external DB
up one last time so the one-shot automigration can dump→restore→detach.

**12c. Boxed `DEPRECATED … base de datos EXTERNA` warning (app works).** First move:
`docker inspect -f '{{json .Mounts}}' futurefin`. You are in external-compat mode: a 2.x compose
(no volume on the app container, `DATABASE_URL` pointing at `futurefin-database`) running the
3.x image — the classic watchtower/`:latest` case. The entrypoint deliberately does NOT migrate
here (writing to the ephemeral layer would be worse) and just runs the API against the external
DB. Discriminating experiment: the log shows the boxed warning and **no** `starting embedded
PostgreSQL` line. It is supported, not broken — but migrate when you can (replace the compose
with the 3.x one; CI covers exactly this transition). The mode disappears in 4.0.0.

**12d. `FATAL: pre-migration backup FAILED`.** First move: check the `ffdata` volume
(`docker run --rm -v futurefin_ffdata:/d alpine df -h /d`) and its permissions. Before applying
migrations for a new app version, the entrypoint writes
`/var/lib/futurefin/backups/pre-migration-<from>-to-<to>-<ts>.sql.gz`; if that dump cannot be
written it **aborts the boot on purpose** rather than migrating without a safety net.
Discriminating experiment: free space (or fix ownership) and restart — the same container boots
and logs `pre-migration backup written: …`. `FUTUREFIN_PREMIGRATION_BACKUP=off` is a deliberate
bypass, never a fix.

**12e. `FATAL: cannot connect as role 'futurefin'` while adopting a cluster.** First move: recall
the `POSTGRES_USER` of your 2.x installation. On an adopted 2.x cluster the superuser role is
whatever `POSTGRES_USER` created it; the 3.x default is `futurefin`. Discriminating experiment:
`docker compose run --rm -e FUTUREFIN_MODE=db-only futurefin` (with the stack stopped) and list
roles — or just set `POSTGRES_USER` back to the old value in the compose and restart. Nothing was
modified: the abort happens before any write.

**12f. `pg_upgrade needed: PostgreSQL 15 -> 16` followed by a failure.** First move: read the
upgrade logs, which live in the `ffdata` volume under `/var/lib/futurefin/pgupgrade/logs`. The
upgrade runs in **copy** mode into a staging directory and is verified by a per-table row census
before the swap, so a failure leaves your old cluster **untouched** — either still at `$PGDATA`
(failure before the swap) or at `$PGDATA/pgdata_old_15` (failure after it; the swap is resumable
and re-runs on the next boot). Discriminating experiment:
`docker compose exec futurefin sh -c 'ls /var/lib/postgresql/data'` — `PG_VERSION` says which
major is live, `pgdata_old_15/` shows the swap already happened. A mandatory
`pre-pgupgrade-15-to-16-*.sql.gz` dump is written before anything is touched. If the image does
not bundle your old major at all the entrypoint says so and names the two escape routes
(stepwise upgrade with an older FutureFin, or dump + external automigration). CI pins the whole
path in the "pg_upgrade 15→16 (seeded PG15 volume)" step.

## Where the evidence lives

- **API logs**: `RUST_LOG` env filter; default (in `main.rs` and compose)
  `futurefin_api=info,tower_http=info,sqlx=warn`. For SQL statements set `sqlx=debug`; for
  request traces `tower_http=debug`. Docker: `docker compose logs -f futurefin`. Split-dev:
  the `cargo run` terminal.
- **Container logs mix THREE sources since 3.0.0.** `docker compose logs -f futurefin`
  interleaves (1) the entrypoint, every line prefixed `[futurefin-entrypoint]`, (2) PostgreSQL
  itself (`logging_collector=off`, so it goes to stdout) and (3) the API. There is no
  `docker compose logs futurefin-database` anymore. Isolate a source with
  `docker compose logs futurefin | grep '^futurefin.*\[futurefin-entrypoint\]'` (entrypoint) or
  `grep -v futurefin-entrypoint` (PG + API).
- **Startup milestones, in order.** Entrypoint first:
  `[futurefin-entrypoint] FutureFin 3.0.0 — mode=… db_mode=… postgres_majors=…`; then exactly one
  cluster path — `initializing fresh PostgreSQL 16 cluster` (new install) OR
  `adopting ownership of PGDATA (uid 70 -> 999)` + `reindexing database after adoption
  (musl->glibc collation)` (upgrade from a 2.x alpine cluster) OR
  `pg_upgrade needed: PostgreSQL 15 -> 16`; then
  `starting embedded PostgreSQL 16 (socket-only at /var/run/postgresql)`, optionally
  `pre-migration backup written: …`, then `starting FutureFin API 3.0.0`. After that the classic
  binary milestones: `futurefin starting` → `database connected` → `migrations applied` →
  `server config` → `serving web UI and API on one port` → `listening on http://…`.
  Shutdown: `shutdown signal received` (logged by both entrypoint and API) → `http server
  stopped` → `database pool closed` → `database system is shut down` (PostgreSQL checkpoint) →
  `clean shutdown complete`. A boot that stops between two of these tells you which stage failed;
  CI greps the shutdown four to prove a clean stop.
- **Projection cache**: info-level lines `projection cache HIT` / `MISS, computing` /
  `invalidated by installation` / `warm-up household projection` (in `handlers/projection.rs`
  and `state.rs`). Script: `scripts/smoke-projection-cache.sh`.
- **Migration ledger**: table `_sqlx_migrations` (version, description, checksum, success).
- **Browser devtools**: Network tab shows `content-encoding: gzip` (CompressionLayer, responses
  >1 KB), the `density` field echoed in the projection response body, `Set-Cookie` flags, and
  the two-phase hybrid+monthly request pair. Application tab: `ff_session` cookie, `localStorage`
  theme pref.
- **Database**: production is socket-only inside the single container —
  `docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin`
  (there is no host port and no TCP listener to connect to). Split-dev, with
  `docker compose -f docker-compose.dev.yml up -d` running:
  `psql "postgres://futurefin:futurefin@127.0.0.1:5432/futurefin"`.
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

Written 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`); traps 9 and 12, the container/DB
evidence sections and the vocabulary entry rewritten 2026-08-16 for **v3.0.0** (self-contained
image). All mechanisms verified by reading code, not by running services. Re-verify before
trusting:

- Error mapping (23505→409, 23503→400): `grep -n "23505\|23503" apps/api/src/error.rs`
- Body limits (1 MiB / 16 MiB): `grep -n "BODY_LIMIT" apps/api/src/routes/mod.rs`
- Pending-user 403: `grep -n "Forbidden" apps/api/src/handlers/installation.rs`
- Cache TTL/key/invalidation: `grep -n "PROJECTION_CACHE_TTL\|ProjectionCacheKey\|invalidate_projection" apps/api/src/state.rs`
- No warm-up after mutation rationale: `grep -n -B8 "refresh_projection_after_mutation" apps/api/src/handlers/projection.rs`
- Hybrid density pattern: `grep -n -A8 "density_month_indices" apps/api/src/handlers/projection.rs`
- Chart uses `month_index`: `grep -n "month_index" apps/web/src/views/ProjectionNetWorthChart.tsx`
- Cookie secure flag: `grep -n "COOKIE_SECURE" apps/api/src/main.rs && grep -n "secure(" apps/api/src/handlers/auth.rs`
- Vite proxy ports: `grep -n "FUTUREFIN_API_PORT\|WEB_DEV_PORT" apps/web/vite.config.ts`
- Healthcheck (CMD-SHELL, `/v1/ready`, no `/dev/tcp` fallback) + default RUST_LOG + grace/start
  periods: `grep -n "CMD-SHELL\|v1/ready\|dev/tcp\|RUST_LOG\|stop_grace_period\|start_period" docker-compose.yml`
- Single service, embedded PG, socket-only:
  `grep -n "services:\|futurefin:\|pgdata\|ffdata" docker-compose.yml` and
  `grep -n "socket-only\|unix_socket_directories\|listen_addresses\|logging_collector" apps/api/docker-entrypoint.sh`
- Startup/shutdown milestones (trap 9, "Where the evidence lives"): entrypoint side
  `grep -n 'log "\|warn "' apps/api/docker-entrypoint.sh`; API side
  `grep -n "futurefin starting\|database connected\|migrations applied\|server config\|serving web UI\|listening\|http server stopped\|database pool closed\|shutdown signal" apps/api/src/main.rs`
- The exact FATAL strings of trap 12:
  `grep -n "no persistent volume\|does not answer\|pre-migration backup FAILED\|cannot connect as role\|pg_upgrade needed\|DEPRECATED" apps/api/docker-entrypoint.sh`
- Nothing is ever deleted (moved aside instead):
  `grep -n "pgdata_old_\|failed-automigration-" apps/api/docker-entrypoint.sh`
- Rescue mode exists: `grep -n "db-only" apps/api/docker-entrypoint.sh`
- Dev Postgres compose (project `futurefin-dev`, 127.0.0.1:5432, volume `devdata`):
  `grep -n "^name:\|5432\|devdata" docker-compose.dev.yml` (and `ls docker-compose*.yml` —
  `docker-compose.split-dev.yml` is gone)
- Container paths behind trap 9/12 are exercised by CI:
  `grep -n "name:" .github/workflows/ci.yml`
- Migration count (34 as of 2026-08-16; 31 as of 2026-07-02): `ls apps/api/migrations | wc -l`
- Default log filter: `grep -n "EnvFilter" apps/api/src/main.rs`
- Parity fixture still dual-consumed: `grep -rn "fire-parity.json" apps/api/tests/ apps/web/src/`
- Incident quotes (v1.0.2, v1.0.10, v1.0.12, v1.0.18–20, v1.2.0, v1.3.0, v1.4.0, v1.4.2, 3.0.0):
  `CHANGELOG.md`
- Doc drift record: the standing-errata table lives in futurefin-docs-and-writing §7 (empty as
  of 2026-07-02); when docs and `handlers/projection.rs` disagree, the code is ground truth.
