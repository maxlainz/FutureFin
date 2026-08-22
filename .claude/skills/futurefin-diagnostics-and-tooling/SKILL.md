---
name: futurefin-diagnostics-and-tooling
description: >
  Measuring FutureFin instead of eyeballing it: timing endpoints with curl,
  proving projection-cache HIT vs MISS, checking gzip payload sizes and
  density point counts, diffing projection responses before/after a change,
  reading row counts and _sqlx_migrations straight from Postgres, and reading
  frontend bundle/network evidence. Load this skill when the task says:
  "is the cache working?", "how slow is /v1/projection/series?", "measure it",
  "did my refactor change the numbers?", "how big is the response?",
  "why is the first GET slow?", "hybrid vs monthly", "point counts",
  "row counts", "which migrations are applied?", "EXPLAIN ANALYZE",
  "smoke test the projection cache", or before/after evidence is needed for a
  performance or no-behavior-change claim. Do NOT load it to triage a bug from
  a symptom (futurefin-debugging-playbook), to write or run the test suites
  (futurefin-validation-and-qa), to deploy/upgrade/backup
  (futurefin-run-and-operate), or to set up the dev environment
  (futurefin-build-and-env).
---

# FutureFin diagnostics and tooling — measure, don't eyeball

Everything here is verified against the code as of 2026-07-02 (v1.4.3, 31 migration
files); the container/DB sections and `db-stats.sh` were re-verified 2026-08-16 for
**v3.0.0** (34 migration files, self-contained image). All commands run from the repo
root. The three scripts shipped with this skill live in
`scripts/diagnostics/` and were **written against
the API contract as of 2026-07-02, verified by code reading**
(`apps/api/src/routes/mod.rs`, `apps/api/src/handlers/projection.rs`,
`apps/api/src/state.rs`) — not by execution. They check preconditions and fail with
actionable errors when the API/DB is not running. They are shellcheck-clean and CI
enforces it (`docker-stack` job, step "shellcheck (entrypoint + scripts)").

**3.0.0 topology, once.** Production is a **single container** (`futurefin`) with
PostgreSQL 16 embedded, listening **only on the Unix socket** `/var/run/postgresql` —
no TCP listener, no host port, no `futurefin-database` service. Every DB measurement
below therefore goes through `docker compose exec futurefin psql -h /var/run/postgresql
…`. The development Postgres is a separate, autonomous compose
(`docker compose -f docker-compose.dev.yml up -d`, project `futurefin-dev`, volume
`devdata`, published on `127.0.0.1:5432`), which is what split-dev `cargo run` talks
to. The old `docker-compose.split-dev.yml` override no longer exists.

## Vocabulary (once)

| Term | Meaning here |
|---|---|
| installation | The singleton row in table `installation`; all financial data belongs to it. |
| view scoping | `?view=mine` filters rows to `owner_user_id = current_user`. Client-side filter, NOT an authz boundary. Default `household`. |
| density | `/v1/projection/series?density=hybrid` decimates the big arrays at serialization time (months 0..12 monthly, then 24, 36, … annual). `monthly` (default) serializes every month. The engine compute is identical for both. |
| projection cache | In-memory `HashMap` in `AppState`, sliding 60-min TTL (`PROJECTION_CACHE_TTL`, `apps/api/src/state.rs`), keyed `(installation_id, view, owner_user_id, density)`. Invalidated by every mutating handler; warmed only after login. |
| warm-up | After `POST /v1/auth/login`, a `tokio::spawn` recomputes `view=household` for BOTH densities so the first GET is a hit (`warm_up_household_projection`). There is deliberately NO warm-up after mutations (race condition — see futurefin-failure-archaeology). |
| nominal vs real | Engine simulates in nominal euros; only the FIRE target grows with inflation. "Real" (deflated, today-euros) values are derived for display/milestones. Math details: futurefin-fire-domain-reference. |

## 1. Existing tool inventory

### `scripts/smoke-projection-cache.sh` (repo root — predates this skill)

End-to-end cache smoke: login (or register+setup on a virgin DB) → wait 1.5 s for
warm-up → 2× GET hybrid → 2× GET monthly → **create a category + asset (real
mutation!)** → 2× GET to show MISS→HIT after invalidation → logout.

- Env: `BASE` (default `http://127.0.0.1:8080`), `SMOKE_USER`/`SMOKE_PASS`
  (existing member credentials; without them it registers a throwaway user, which
  only yields a working member on a **virgin** DB — the first registered user
  auto-becomes owner).
- Reading the output: all four initial GETs should be fast (warm-up covers
  household×both densities). After the mutation, `GET #1` is slow (cache MISS →
  full compute) and `GET #2` fast again (HIT). If `GET #2` is also slow, cache
  insert/invalidation is broken.
- Caveats (verified in the script): it does **not** pre-check that the API is up
  (`set -euo pipefail` + `curl -sf` just dies on the first call), and its mutation
  section leaves a `Smoke Cache Cash` category and a `Smoke Cache Asset <ts>`
  asset in the DB. Don't run it against data you care about without cleaning up.
  For a mutation-free timing pass use `api-timing.sh` (below).

### Probes and contract snapshot

| Endpoint | Auth | What it proves |
|---|---|---|
| `GET /health` and `GET /v1/health` | none | Process is up; returns `{status, service, version}` — read `version` to confirm which build is running. |
| `GET /v1/ready` | none | DB reachable (`SELECT 1`); 503 (`Unavailable`) if not. Distinguishes "app up, DB down" from "app down". **Since 3.0.0 this is also the compose healthcheck** (it replaced `/v1/health`, and the old `</dev/tcp/…` fallback was deliberately removed so a 503 can no longer be masked) — so the `docker ps` health state and this probe now say the same thing. |
| `GET /openapi.json` | none | Full utoipa-generated contract. Snapshot it before/after a change: `curl -s $BASE/openapi.json | python3 -m json.tool > /tmp/openapi-before.json` and `diff` later. Any route/field drift shows up here without reading Rust. |

### Log axes (`RUST_LOG`)

Default filter (set in `apps/api/src/main.rs` and `docker-compose.yml`):
`futurefin_api=info,tower_http=info,sqlx=warn`.

- `futurefin_api=info` — cache telemetry. Grep the logs for these exact messages
  (emitted in `handlers/projection.rs` / `state.rs`): `projection cache HIT`,
  `projection cache MISS, computing`, `projection compute done, inserting in cache`
  (includes `ms=` field — the authoritative compute time, network excluded),
  `warm-up household projection start`, `warm-up done`, `warm-up failed`,
  `projection cache invalidated by installation`, `... by user (logout)`.
- `tower_http=debug` — per-request method/path/status/latency (TraceLayer).
- `sqlx=debug` — every SQL statement with timing. Very noisy; use for "which
  queries does this endpoint actually run".

Apply: in split-dev, `RUST_LOG=futurefin_api=info,sqlx=debug cargo run` (or `.env`);
in Docker, edit `RUST_LOG` in `docker-compose.yml` env and
`docker compose up -d futurefin`. Read with `docker compose logs -f futurefin`.

### Container state

```bash
docker compose ps                      # ONE service since 3.0.0: futurefin "running (healthy)"?
docker compose logs -f futurefin       # entrypoint + PostgreSQL + API, interleaved
# Isolate a source when the noise gets in the way:
docker compose logs futurefin | grep    '\[futurefin-entrypoint\]'   # boot/migration/backup path
docker compose logs futurefin | grep -v '\[futurefin-entrypoint\]'   # PostgreSQL + API
docker compose logs futurefin | grep -E "projection cache (HIT|MISS|invalidated)"
```

There is no `docker compose logs futurefin-database` anymore: PostgreSQL runs inside
the same container with `logging_collector=off`, so its lines land on the same stdout
as the API's. The `[futurefin-entrypoint]` prefix is the reliable discriminator.

## 2. Curl cookbook (verified against `routes/mod.rs`)

All ledger routes need the `ff_session` cookie. Use a jar:

```bash
BASE=http://127.0.0.1:8080          # split-dev API: http://127.0.0.1:8081
JAR=$(mktemp)
# Existing user:
curl -sf -c "$JAR" -X POST "$BASE/v1/auth/login" \
  -H 'content-type: application/json' \
  -d '{"username":"alice","password":"secret"}'
# Virgin DB only (first user auto-becomes installation owner; birth_date required):
curl -sf -c "$JAR" -X POST "$BASE/v1/auth/register" \
  -H 'content-type: application/json' \
  -d '{"username":"probe","password":"probe-pass-1234","birth_date":"1990-01-01"}'
# Membership check (pending users 403 on everything else): "access":null = pending
curl -sf -b "$JAR" "$BASE/v1/installation/session-context"
```

**Timed GET** (wall-clock; server compute time is in the `projection compute done ms=` log line):

```bash
curl -sf -b "$JAR" -o /dev/null -w '%{time_total}\n' "$BASE/v1/projection/series"
```

**Gzip and payload size** (compression is `tower_http` `CompressionLayer::new().gzip(true)` in `main.rs`, applied to all endpoints):

```bash
curl -sf -b "$JAR" -o /dev/null -w 'raw:  %{size_download} B\n' "$BASE/v1/projection/series"
curl -sf -b "$JAR" -H 'Accept-Encoding: gzip' -o /dev/null \
  -w 'gzip: %{size_download} B\n' "$BASE/v1/projection/series"
# Confirm the header is actually set:
curl -s -b "$JAR" -H 'Accept-Encoding: gzip' -o /dev/null -D - \
  "$BASE/v1/projection/series" | grep -i '^content-encoding'
```

**Density point counts** (one-liner):

```bash
for D in "" "?density=hybrid"; do
  curl -sf -b "$JAR" "$BASE/v1/projection/series$D" | python3 -c \
    'import json,sys; d=json.load(sys.stdin); print(d["density"], "points:", len(d["points"]), "months:", d["months"], "basis:", d["horizon_basis"])'
done
```

Expected: `monthly points == months + 1` — the series includes index 0 (today's
state) plus one point per simulated month, so 841 points for the 840-month
(70-year) horizon and 361 for the 30-year no-demographics fallback (pinned by
`apps/api/tests/projection_marker.rs`: horizon 24 → 25 points, indices 0..=24);
`hybrid` = 13 points (months 0–12) + one per year from month 24 (24, 36, …, 840)
→ 82 points at months=840.
`horizon_basis` is one of `lifespan_90 | fallback_no_demographics |
months_override`.

**`?months=N`** (12–840, clamped): bypasses the cache entirely (`q.months.is_none()`
gate in `get_projection_series`) — use it to force a true compute measurement.

**view=mine vs household diff**: use `projection-diff.sh "" "view=mine"` (below), or
manually fetch both and compare `starting_net_worth` / `asset_series` — they SHOULD
differ if members own different rows; `mine` has its own cache key.

## 3. DB-level diagnosis

Since 3.0.0 the prod stack does **not** expose Postgres at all — not on the host and not
even on TCP inside the container: the embedded server is started with an empty
`listen_addresses` and `unix_socket_directories=/var/run/postgresql`, so the socket is
the only way in. There is nothing to port-forward and no password to find. Go through
the single container:

```bash
docker compose exec -T futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
  -P pager=off -c '<SQL>'
```

(Development instead has a real TCP port, from its own compose:
`docker compose -f docker-compose.dev.yml up -d` then
`psql "postgres://futurefin:futurefin@127.0.0.1:5432/futurefin" -c '<SQL>'`.)

Useful queries (tables as of 2026-08-16: `users, sessions, installation,
installation_memberships, persons, categories, assets, liabilities, budget_entries,
planning_flows, allocation_rules, history_snapshots, history_snapshot_items,
transaction_imports, transactions, categorization_rules, recurring_transaction_rules,
_sqlx_migrations` — enumerate them with the `db-stats.sh` row-count query rather than
trusting this list):

```sql
-- Which server + which collation: an adopted 2.x cluster (created by postgres:16-alpine,
-- musl) shows a different datcollate than a cluster initdb'd by the 3.x image (C.UTF-8).
SHOW server_version;
SELECT datname, datcollate, datcollversion FROM pg_database WHERE datname = current_database();
-- Migration state: applied count must equal `ls apps/api/migrations/*.sql | wc -l` (34 as of 2026-08-16)
SELECT count(*), max(version), bool_and(success) FROM _sqlx_migrations;
-- Expired liabilities STILL stored is NORMAL (reads never mutate, v1.3.0):
SELECT count(*) FROM liabilities WHERE payment_end_date < CURRENT_DATE;
-- Sessions: active vs expired
SELECT count(*) FILTER (WHERE expires_at > now()) AS active,
       count(*) FILTER (WHERE expires_at <= now()) AS expired FROM sessions;
-- Pending users (403 everywhere): registered but no membership
SELECT u.username FROM users u
WHERE NOT EXISTS (SELECT 1 FROM installation_memberships m WHERE m.user_id = u.id);
```

**EXPLAIN ANALYZE pattern** — take the SQL from the handler (they are plain strings,
e.g. the assets query in `build_installation_projection_input`), substitute the
`$1`/`$2` binds with literals, and:

```bash
docker compose exec -T futurefin psql -h /var/run/postgresql -U futurefin -d futurefin -c \
  "EXPLAIN (ANALYZE, BUFFERS) SELECT id, name, current_value FROM assets
   WHERE installation_id = '<uuid>' ORDER BY sort_index ASC, name ASC;"
```

At household scale (tens of rows) seq scans are normal and fine; investigate only
if `actual time` is in the tens of ms.

Or just run `db-stats.sh` (section 5) which packages the read-only queries.

## 4. Frontend measurement

- **Bundle sizes**: `npm run build:web` (Vite) prints one line per chunk with raw
  and gzip sizes (`apps/web/dist/assets/<Chunk>-<hash>.js … kB │ gzip: … kB`).
  Each view is a lazy chunk (`React.lazy` in `App.tsx`; `SummaryView` stays eager).
  The heavy projection chart (`ProjectionNetWorthChart`, ~1,000 LOC + 
  `lib/projection-chart.ts`) ships **inside the `ProjectionView` chunk** — it is a
  static import there, not an inner lazy (see the comment at `App.tsx` near
  `prefetchOtherViews`; CHANGELOG v1.4.0's "own chunk" wording is outdated).
  Compare chunk gzip sizes before/after a change instead of guessing.
- **Two-phase loading in DevTools → Network**: filter `projection/series`. You
  should see TWO parallel requests: `?density=hybrid` (~5 KB gzipped, rendered
  first) and the default monthly (~30 KB gzipped, replaces the data inside
  `startTransition`) — implemented in `fetchProjectionTwoPhase` (`App.tsx`). With a
  warm server cache both return in <10 ms and the swap is invisible. Check the
  `Content-Encoding: gzip` response header here too.
- Frontend Vitest tests (`npm test --workspace futurefin-web`) are owned by
  futurefin-validation-and-qa.

## 5. Shipped scripts

Run from the repo root. Common env: `BASE` (default `http://127.0.0.1:8080`;
split-dev API is `:8081`), `SMOKE_USER`/`SMOKE_PASS` (same convention as
`scripts/smoke-projection-cache.sh`). All three are read-mostly: `api-timing.sh`
and `projection-diff.sh` never mutate ledger data (worst case `api-timing.sh`
registers a throwaway user on a virgin DB; `projection-diff.sh` only logs in);
`db-stats.sh` is SELECT-only.

### `scripts/api-timing.sh`

```bash
SMOKE_USER=alice SMOKE_PASS=secret \
  bash scripts/diagnostics/api-timing.sh
```

Measures: health probes, login latency, projection GETs (2 hits per density, plus
a `?months=840` cache-bypass and a `view=mine`), the uncached data endpoints, and
raw-vs-gzip sizes. Aborts with a clear message if the API is down or the logged-in
user has no installation membership (pending users would 403 everywhere).

Interpretation (localhost; sources: `.claude/api-routes.md` §Cache, CHANGELOG
v1.4.0, `scripts/smoke-projection-cache.sh` comments):

| Line | NORMAL | Suspicious |
|---|---|---|
| `/v1/health`, `/v1/ready` | 1–10 ms | `/v1/ready` FAILED → DB down/unreachable |
| login | 100–500 ms (Argon2id verify dominates) | multi-second → DB or CPU starved |
| series #1 (either density) | <10 ms if warm-up finished within the 1.5 s wait; up to ~500 ms if not | — |
| series #2 | <10 ms — MUST be a hit | ~same as #1 → cache insert broken; check `projection cache HIT` in logs |
| `?months=840` | ~200–500 ms always (bypasses cache = true compute cost) | multi-second → engine or query regression |
| `view=mine` #1 | MISS (~500 ms) — warm-up only covers household | — |
| summary/assets/budget/liabilities | 5–50 ms (plain queries, no cache) | — |
| sizes monthly | ~260 KB raw / ~30 KB gzip | gzip == raw → compression layer gone |
| sizes hybrid | ~5 KB gzip | — |

### `scripts/projection-diff.sh`

```bash
S=scripts/diagnostics/projection-diff.sh
SMOKE_USER=alice SMOKE_PASS=secret bash $S                     # monthly vs hybrid
SMOKE_USER=... SMOKE_PASS=... bash $S "" "view=mine"           # household vs mine
SMOKE_USER=... SMOKE_PASS=... bash $S --save /tmp/base.json    # BEFORE a change
SMOKE_USER=... SMOKE_PASS=... bash $S --compare /tmp/base.json # AFTER
```

Fetches `/v1/projection/series` twice (positional args = raw query strings) and
diffs: scalars (`months`, `horizon_*`, `starting_net_worth`,
`monthly_delta_assumption`), KPIs (`jubilacion_month_index`,
`jubilacion_target_net_worth`, `compound_outpaces_true_savings_month_index`),
`milestones`/`milestones_real`, array lengths, and every value at **shared
month_index points**. Exit 0 = no forbidden divergence; exit 1 = real DIFF.

Interpretation:
- **Densities differ (default run)**: `points_len`/`fire_target_series_len`/last
  `month_index` differences are labeled `diff (expected)` (decimation artifacts).
  Note the series is `months + 1` points long (index 0 + one per month), and at
  the default 840-month horizon BOTH densities end at `month_index = 840` (it is
  a multiple of 12); the last kept indices only diverge with a
  non-multiple-of-12 `?months=` override (e.g. months=839 → monthly ends at 839,
  hybrid at 828).
  Everything else must be `same`: KPIs and milestones are computed server-side on
  the FULL series (`points_full` in `handlers/projection.rs`) and are
  density-invariant. A DIFF here is the v1.4.2 regression class (decimated series
  breaking index math).
- **Shared-index values**: hybrid indices are a subset of monthly indices and both
  serialize the same deterministic compute → f64 values must be bit-identical.
  Mismatches mean two different computes saw different data (did something mutate
  between the two GETs?) or a serialization change.
- **`--save`/`--compare`**: the refactor-evidence workflow ("must not change
  output"). Snapshot before, compare after; with identical query and data,
  EVERYTHING must be `same`. Pair with futurefin-proof-and-analysis-toolkit.

### `scripts/db-stats.sh`

```bash
bash scripts/diagnostics/db-stats.sh
# non-default credentials: POSTGRES_USER=... POSTGRES_DB=... DB_SERVICE=... bash .../db-stats.sh
```

SELECT-only, via `docker compose exec -T $DB_SERVICE psql -h /var/run/postgresql`
(`DB_SERVICE` defaults to **`futurefin`** — the single 3.0.0 service; it was
`futurefin-database` in 2.x, hence the env var). It aborts with an actionable message
if `docker` is missing or that compose service is not running, and warns if you are
not in the repo root. Prints, in order: **server version and database collation**,
applied `_sqlx_migrations` (count/latest/`all_success` + last 5) next to the repo file
count, exact row counts for every `public` table, active-vs-expired sessions,
expired-but-still-stored liabilities, pending users, and membership roles.

Interpretation:
- `SHOW server_version` / `datcollate` + `datcollversion` (added in 3.0.0): they tell
  you **which cluster you are on**. A cluster created by the 3.x image reports the
  `C.UTF-8` locale it was `initdb`'d with; a cluster **adopted** from a 2.x
  `postgres:16-alpine` volume carries the old musl locale (e.g. `en_US.utf8`) — which
  is exactly why the entrypoint runs a one-time `REINDEX DATABASE` on adoption. A
  non-empty `datcollversion` mismatch warning in the PG logs plus an adopted collation
  is the signature to look for when unique indexes behave oddly after an upgrade.
  `server_version` also confirms a `pg_upgrade` really landed (16.x, not 15.x).
- `applied` ≠ repo file count → the running image and your checkout disagree
  (old image, or migration added but API not restarted). `all_success = f` →
  a migration failed; startup should have failed loud.
- `expired_liabilities_still_stored > 0` is **NORMAL** (reads never mutate;
  v1.3.0 removed the purge). Do not "fix" it.
- `pending_users > 0` → those users see no data and get 403s — expected until the
  owner approves them (explains "user can't see anything" reports).
- Expired sessions accumulating is harmless (validated on read).

## 6. When NOT to use this skill

- **You start from a symptom** ("numbers look wrong", 409/422, login loop,
  container unhealthy) → `futurefin-debugging-playbook` first; come back here when
  it tells you to measure something.
- **You need test evidence** (unit/integration/parity suites, TestApp harness,
  what CI covers) → `futurefin-validation-and-qa`.
- **Deploy/upgrade/rollback/backups/production logs** → `futurefin-run-and-operate`.
- **Getting the stack running at all** → `futurefin-build-and-env`.
- **What a flag/env var means** → `futurefin-config-and-flags`.
- **Why the cache/warm-up is designed this way** → `futurefin-architecture-contract`
  (and `futurefin-failure-archaeology` for the rejected warm-up-after-mutation).
- Changing behavior based on what you measured → gates in `futurefin-change-control`.

## Provenance and maintenance

Facts date-stamped 2026-07-02 (v1.4.3); container/DB access, `db-stats.sh` and the
table list re-verified 2026-08-16 (**v3.0.0**, 34 migration files). Re-verify before
trusting:

- Route paths / new endpoints: `grep -n 'route(' apps/api/src/routes/mod.rs`
- Cache TTL + key shape: `grep -n 'PROJECTION_CACHE_TTL\|pub struct ProjectionCacheKey' -A6 apps/api/src/state.rs`
- Density pattern + months clamp + cache bypass: `grep -n 'density_month_indices\|clamp(12, 840)\|q.months.is_none' apps/api/src/handlers/projection.rs`
- horizon_basis values: `grep -n '"months_override"\|lifespan_90\|fallback_no_demographics' apps/api/src/handlers/projection.rs`
- Cache log messages grepped in §1: `grep -rn '"projection cache\|warm-up' apps/api/src/handlers/projection.rs apps/api/src/state.rs`
- gzip layer: `grep -n 'CompressionLayer' apps/api/src/main.rs`
- Default RUST_LOG: `grep -n 'futurefin_api=info' apps/api/src/main.rs docker-compose.yml`
- Migration count (34 as of 2026-08-16; 31 as of 2026-07-02): `ls apps/api/migrations/*.sql | wc -l`
- Table list: `grep -rn 'CREATE TABLE\|DROP TABLE\|RENAME TO' apps/api/migrations/`
  (careful: `persons` is dropped and later re-created — the live list is what
  `db-stats.sh`'s row-count query returns)
- Postgres is socket-only, no TCP at all:
  `grep -n 'listen_addresses\|unix_socket_directories\|socket-only' apps/api/docker-entrypoint.sh`
  and `grep -n 'ports:' -A2 docker-compose.yml` (only `APP_PORT:8080`, no 5432)
- Single service + volumes: `grep -n 'services:\|futurefin:\|pgdata\|ffdata' docker-compose.yml`
- Dev DB on 127.0.0.1:5432 lives in its own compose:
  `grep -n '^name:\|5432\|devdata' docker-compose.dev.yml` (and `ls docker-compose*.yml` —
  `docker-compose.split-dev.yml` no longer exists)
- Readiness probe is the healthcheck, without a `/dev/tcp` fallback:
  `grep -n 'CMD-SHELL\|v1/ready\|dev/tcp' docker-compose.yml`
- Entrypoint log prefix used to split the three log sources:
  `grep -n 'futurefin-entrypoint' apps/api/docker-entrypoint.sh | head -3`
- `db-stats.sh` defaults and its two 3.0.0 queries:
  `grep -n 'DB_SERVICE\|/var/run/postgresql\|server_version\|datcollate' scripts/diagnostics/db-stats.sh`
- Scripts stay shellcheck-clean (CI enforces):
  `shellcheck -S warning scripts/diagnostics/*.sh`
- Two-phase fetch + chunking: `grep -n 'fetchProjectionTwoPhase\|density=hybrid\|import("./views/ProjectionView")' apps/web/src/App.tsx`
- Smoke script env/behavior: `sed -n '1,45p' scripts/smoke-projection-cache.sh`
- Size/timing norms (~260→30 KB, ~5 KB hybrid, ~82 vs ~841 points, sub-ms hit,
  ~500 ms compute): `.claude/api-routes.md` §Projection + CHANGELOG.md v1.4.0.
- If any of these drift, update this SKILL.md AND the script headers (they claim
  "as of 2026-07-02").
