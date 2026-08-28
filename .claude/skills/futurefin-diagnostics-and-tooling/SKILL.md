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
**v3.0.0** (34 migration files, self-contained image). The history/cash-flow payload
sections, the MCP-catalog context-cost recipe and the projection `events` field were
added/verified 2026-08-28 against branch `feat/mcp-fase-5-contexto` (issue #86, Fase 5
of the 4.4.0 MCP-audit train — unreleased at verification time, `Cargo.toml` still
4.3.1). All commands run from the repo
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
| `GET /openapi.json` | none | Full utoipa-generated contract. Snapshot it before/after a change: `curl -s $BASE/openapi.json \| python3 -m json.tool > /tmp/openapi-before.json` and `diff` later. Any route/field drift shows up here without reading Rust. |

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

**`?months=N`** (12–840; fuera de rango **rechaza** con 400 `months_out_of_range` desde 4.4.0 — antes clampaba en silencio): bypasses the cache entirely (`q.months.is_none()`
gate in `get_projection_series`) — use it to force a true compute measurement.

**`events` / `events_truncated`** (Fase 5, issue #86): `GET /v1/projection/series` now
also carries dated Próximos (planning flows with a `payment_start_date`) that fall
inside the horizon — `month_index`, `date_ymd`, `title`, `amount` (≥ 0),
`direction` (`inflow`|`outflow`) — capped at `PROJECTION_EVENTS_MAX = 100`
(`apps/api/src/handlers/projection.rs`), month ASC then amount DESC, with
`events_truncated = true` past the cap. No new query — it reuses the existing planning-
flows join. Budget it as **~90 bytes per event**, not per NW point. This field is the
answer to "the hybrid density hides a jump" (in the Fase-5 audit: a ~98 k€ drop between
two annual `hybrid` points with nothing in the response explaining it) — **do not**
"fix" that symptom by raising `density`, which was deliberately rejected as a query
param: it would bring back all 841 monthly points and still only say *where* the curve
moved, not *why*. `events` says why without changing the density at all.

**`view` echoed at the response root** (Fase 5): `GET /v1/summary`, `/v1/budget`,
`/v1/projection/series` and `/v1/allocation-rules/resolution` now all carry
`"view": "household" | "mine"` at the top level, put there by the core (so it is on the
HTTP response and therefore on `openapi.json` too), via `LedgerView::as_str()`
(`apps/api/src/handlers/person_view.rs`). Before this, `?view=mine` and omitting the
param could return **byte-identical** payloads on a single-user installation, so there
was no way to tell "mine == household" from "the param was silently ignored". **Read
this field first** whenever two measurements that are supposed to differ by scope
don't — it tells you which scope the server actually applied before you spend time
diffing bytes.

**view=mine vs household diff**: use `projection-diff.sh "" "view=mine"` (below), or
manually fetch both and compare `starting_net_worth` / `asset_series` — they SHOULD
differ if members own different rows; `mine` has its own cache key.

### History and cash-flow payload sizes (Fase 5, issue #86 — defaults changed under you)

`GET /v1/history/series` **no longer defaults to "the whole history"**. Omitting
`window_months` now means `DEFAULT_HISTORY_WINDOW_MONTHS = 120` months (10 years);
`window_months=1200` (`MAX_HISTORY_WINDOW_MONTHS`) is the new spelling of "everything"
(nothing can exist further back — the window cap itself is 100 years). **Any recipe
written before Fase 5 that measured "`/v1/history/series` with no params" as the
worst-case payload is now measuring a smaller, bounded response** — re-baseline before
comparing before/after numbers. Worst case measured in the Fase-5 audit: **53.6 KB →
16.1 KB (−70 %)**, because the old unbounded default walked all the way back to the
user's `birth_date` — for a young installation that is ~290 points, the first ~200 of
them interpolating between €0 and a few hundred euros at 15 decimal places.

Chart numerics are now rounded at **publication** time, not computation time:
`CHART_DP = 2` decimals for every series value, `month_fraction` to 4 decimals
(`MONTH_FRACTION_DP`, `(f * 10_000.0).round() / 10_000.0`). Applies to both
`/v1/history/series` and the cash-flow fine curve. The interpolation itself
(`crates/engine/src/history.rs`) stays exact `Decimal` math — this is publication
rounding only, same family as `money_out`/`round_ratio`. If you are diffing payload
bytes before/after some unrelated refactor, remember this rounding alone trims
trailing digits and will show up as a byte-size delta that has nothing to do with your
change.

`GET /v1/history/cashflow`'s fine (sub-monthly) curve is capped at
`MAX_FINE_CURVE_WINDOW_MONTHS = 36` months. Asking for a wider window is **not a 400**:
the monthly aggregate `months[]` still comes back in full (up to the 120-month window
cap), `fine` is `null`, and `fine_absent_reason = "window_too_large_for_curve"` says
why — the other three values are `not_requested`, `no_asset_linked_transactions`,
`no_snapshots_to_anchor`; `null` iff `fine` actually travels. Worst case measured:
**64 KB → 20 KB (−69 %)**.

Measure it:

```bash
curl -sf -b "$JAR" -o /dev/null -w 'default (120mo): %{size_download} B\n' "$BASE/v1/history/series"
curl -sf -b "$JAR" -o /dev/null -w 'all (1200mo):    %{size_download} B\n' "$BASE/v1/history/series?window_months=1200"
curl -sf -b "$JAR" "$BASE/v1/history/cashflow?window_months=48" | python3 -c \
  'import json,sys; d=json.load(sys.stdin); print("fine present:", d["fine"] is not None, "reason:", d["fine_absent_reason"])'
```

### MCP catalog context cost (Fase 5, issue #86)

Cheapest measurement — no server needed, just read the fixture the catalog-freeze test
writes:

```bash
python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];l=[x['description_len'] for x in t];print('tools',len(t),'chars',sum(l),'max',max(l))"
```

As of Fase 5: `tools 52 chars 21319 max 596`. Before Fase 5 (verified by diffing
`apps/api/src/mcp/server.rs` against `main`): 37,214 chars total, max 3,821, and **26**
descriptions over 600 chars — not 12; that number appears in the CHANGELOG but does not
match a direct count of the pre-Fase-5 source. Re-derive rather than trust either
figure:

```bash
python3 - <<'EOF'
import re
src = open('apps/api/src/mcp/server.rs', encoding='utf-8').read()
d = [x.replace('\\"', '"').replace('\\n', '\n')
     for x in re.findall(r'^\s+description = "((?:[^"\\]|\\.)*)",\s*$', src, re.M)]
print(len(d), sum(map(len, d)), max(map(len, d)), 'over600:', sum(1 for x in d if len(x) > 600))
EOF
```

The fixture regenerates with:

```bash
UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http -- tools_list_freezes_the_input_contract
```

The merge gate is `apps/api/tests/mcp_http.rs::tool_descriptions_stay_within_the_context_budget`
(`PER_TOOL_MAX = 600`, `TOTAL_BUDGET = 24_000`) — see `futurefin-validation-and-qa` for
what to do when it fails (never raise the constant).

**Descriptions are no longer the dominant cost.** Measured in the Fase-5 audit, the
`inputSchema` of the 52-tool catalog is **~55 KB — about 2.7× the descriptions**. That
number lives only in CHANGELOG prose, not in a fixture or a test assertion: treat it as
a one-time audit measurement, not a frozen constant, and re-derive it against a live
server rather than quoting 55 KB going forward:

```bash
BASE=http://127.0.0.1:8080
curl -s -X POST "$BASE/mcp" \
  -H "Authorization: Bearer ffp_…" -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
| python3 -c '
import json, sys
d = json.load(sys.stdin)
tools = d["result"]["tools"]
schema_bytes = sum(len(json.dumps(t["inputSchema"])) for t in tools)
desc_bytes = sum(len(t.get("description", "")) for t in tools)
print("tools", len(tools), "schema_bytes", schema_bytes, "desc_bytes", desc_bytes)
'
```

(`/mcp` is Streamable HTTP over SSE, hence the `Accept` header carrying both content
types; a Bearer API token from Ajustes → Integraciones or an OAuth access token both
work.) If a future change needs to cut context further, this is where the remaining
budget is — the ~250 parameter doc-comments that `schemars` publishes as each field's
own `description` inside `inputSchema`, not the tool-level `description` string this
Fase 5 already cut.

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
- Density pattern + months clamp + cache bypass: `grep -n 'density_month_indices\|validate_months_override\|q.months.is_none' apps/api/src/handlers/projection.rs`
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
- History window default/max + chart rounding (Fase 5): `grep -n 'DEFAULT_HISTORY_WINDOW_MONTHS\|MAX_HISTORY_WINDOW_MONTHS\|CHART_DP\|MONTH_FRACTION_DP' apps/api/src/handlers/history.rs`
- Cash-flow fine-curve cap + absence reasons (Fase 5): `grep -n 'MAX_FINE_CURVE_WINDOW_MONTHS\|fine_absent_reason = Some' apps/api/src/handlers/history.rs`
- `events`/`events_truncated` cap (Fase 5): `grep -n 'PROJECTION_EVENTS_MAX' apps/api/src/handlers/projection.rs`
- `view` echoed at the response root (Fase 5): `grep -rn 'view: view.as_str()' apps/api/src/handlers/{summary,budget,projection,allocation_rules}.rs`
- MCP catalog description budget + fixture (Fase 5): `grep -n 'PER_TOOL_MAX\|TOTAL_BUDGET\|UPDATE_MCP_CATALOG' apps/api/tests/mcp_http.rs`; the fixture itself is `apps/api/tests/fixtures/mcp-catalog.json`
- Smoke script env/behavior: `sed -n '1,45p' scripts/smoke-projection-cache.sh`
- Size/timing norms (~260→30 KB, ~5 KB hybrid, ~82 vs ~841 points, sub-ms hit,
  ~500 ms compute): `.claude/api-routes.md` §Projection + CHANGELOG.md v1.4.0.
- If any of these drift, update this SKILL.md AND the script headers (they claim
  "as of 2026-07-02").
