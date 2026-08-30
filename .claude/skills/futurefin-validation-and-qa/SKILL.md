---
name: futurefin-validation-and-qa
description: >
  Load this skill whenever you need to PROVE a FutureFin change is correct: running or adding
  tests, deciding what evidence a change needs before merge, writing backend integration tests
  (TestApp harness), engine unit tests, or frontend Vitest tests; regenerating or extending the
  fire-parity.json cross-language fixture; capturing regression values before a refactor; or
  answering "did CI cover this?" / "which tests must I run locally?". Symptom keywords: test
  fails only locally, cargo test hangs on TEST_DATABASE_URL, schema ff_test_* piling up, parity
  test fails on one side only, Decimal string "1000.0000" vs "1000" assertion mismatch, 146/156
  test count confusion. Do NOT use for: getting the app running or a
  dev environment (futurefin-build-and-env), measuring live behavior with curl/scripts and
  interpreting the numbers (futurefin-diagnostics-and-tooling), deciding whether a change is
  allowed at all (futurefin-change-control), or the FIRE math itself
  (futurefin-fire-domain-reference).
---

# FutureFin — Validation & QA

How to prove things in this repo: what counts as evidence, the exact test inventory and
harness, and how to add tests. Verified against the code on 2026-07-02 (v1.4.3); counts and the
test-file inventory refreshed on 2026-08-14 for v2.2.0 (new `summary_runway.rs`, engine `runway.rs`)
and on 2026-08-15 for v2.3.0 (runway SWR threshold: engine `runway.rs` 8 → 13, `summary_runway.rs`
7 → 10); § 3 (CI reality) and § 6 (coverage gaps) rewritten on 2026-08-16 for **v3.0.0**, whose
`docker-stack` job grew from a boot smoke test into the project's only automated *no-data-loss*
evidence. The `docker-stack` job aside, the cargo test suites were **unchanged** by the image work
itself. Counts and the test-file inventory re-counted **2026-08-17 for v3.1.0**: the three suites
that the MCP/OAuth releases added (`api_tokens.rs`, `mcp_http.rs`, `oauth_flow.rs`) had been missing
from § 2's inventory, and every total here was one release behind. Re-synced **2026-08-19
(post-3.5.0)**: the 3.4.0/3.5.0 trains had repeated the same pattern (four rows missing —
`budget_liability_quotas.rs`, `transactions_reconcile.rs`, `mcp_write.rs`, `mcp_simulate.rs` — and a
stale `mcp_http.rs` row still describing the 10-tool catalog); all rows and counts now match
the code, and § 2's table carries a standing "tests.md wins on disagreement" note. **Re-synced again
2026-08-22 (4.0.0)**: this is the release that put the integration suite, ESLint and Vitest **inside**
CI, so § 3 and § 6 no longer describe them as a local-only obligation; two suites were missing from § 2's inventory
(`account_and_members.rs`, `openapi_contract.rs`; the train's other three additions —
`query_param_validation.rs`, `error_codes_parity.rs`, `fixtures_shape.rs` — were already listed); every count moved; and a claim about
`handlers/transactions/` unit tests that had never been true was retracted — a money bug had been
living in exactly that hole.

Why this matters here: the hardest live problem in FutureFin is **projection correctness** —
errors are silent (numbers look plausible but wrong). Eyeballing a chart is never acceptance.

## 1. Evidence standards — what counts as proof

| Situation | Required evidence | Not acceptable |
|---|---|---|
| Refactor that must not change output | **Bit-exact regression capture**: write the test asserting the value FIRST (it fails or you print the actual), run against pre-refactor code, commit the captured expected value, then refactor until green. Example: `apps/api/tests/projection_marker.rs` (captured `compound_outpaces_true_savings_month_index == Some(1)` before the spawn_blocking/tokio::join perf refactor). | "The chart looks the same" |
| Model/behavior change (engine math, FIRE formula) | **Predict-then-measure**: write down the expected number (hand calculation, `python3`, or independent derivation) BEFORE running, then assert against it. Full discipline: `.claude/skills/futurefin-research-methodology/SKILL.md`. | Running first and asserting whatever came out |
| Logic duplicated client & server (FIRE target) | **Parity fixture**: one canonical JSON both suites consume (`apps/api/tests/fixtures/fire-parity.json`). A failure on one side only = drift. | Updating one side and re-deriving the other "by inspection" |
| Bug fix | One test that fails on the old code and passes on the new. One test per surprising behavior — no "while we're here" assertion bundles. | Fix without a pinning test |
| Comparing money values in tests | API serializes `Decimal` as strings like `"1000.0000"`, never `"1000"`. Parse to `f64` and compare with tolerance: `let v: f64 = body["x"].as_str().unwrap().parse().unwrap(); assert!((v - 1000.0).abs() < 0.01)`. Or `starts_with("15000")` for coarse checks. | `assert_eq!(body["x"], "1000")` — will fail on scale |
| Visual change | Verify light AND dark theme manually (`<html data-theme>`); there are no rendering tests. | Checking one theme |
| Asserting an **absence** (a route that does not exist, a feature that is off) | Mount the same fallback stack the published image has (`TestConfig::web_static_root` → `spa::mount_static_spa`, the exact function `main.rs` calls). Absence is only observable against the real fallback. | A stripped-down test router with no static/SPA fallback — it silently proves the wrong thing. Lesson from issue #85 (MCP Fase 4, 4.4.0): the old kill-switch test built its router by hand with no fallback and confirmed a 404 the shipped image never returned — in production `FUTUREFIN_MCP_ENABLED=0` gave a bare 405 to `POST /mcp` and the SPA shell (200 `text/html`) to a `GET .well-known` route, because `ServeDir` only calls its fallback for GET/HEAD |
| Adding a **context field** (provenance, window, absence-reason — anything meant to tell a caller *where a number came from* or *why something is missing*) | A test that demonstrates the field **distinguishes the two cases that used to be indistinguishable**. Existence is not evidence: `financial_health.basis` is not proven by a test that reads `"plan"` once, it's proven by a test that also drives the field to `"actual"`/`"mixed"` and shows the two are different. Example family: `apps/api/tests/context_fields.rs` (issue #86, Fase 5, 4.4.0) — every one of its 11 tests is shaped this way (`markers_declare_capture_versus_backfill` proves a `capture` snapshot and a `backfill` snapshot produce different `source` values; `snapshots_distinguish_suppressed_detail_from_an_empty_snapshot` proves `item_count: 0, items_included: false` reads differently from an actually-empty snapshot; etc.) | A test that only asserts the field is present, or that its one observed value matches its default — that proves serialization, not meaning |

**Norm this table's last row generalizes**: a context field is not "done" because it compiles and shows up in a response once. It is done when a test forces the field through both branches of the ambiguity it exists to resolve, and shows the reader can tell them apart. Write the two-case test before calling the field shipped.

Jargon used below, defined once: **SWR** = safe withdrawal rate (annual % of net worth
withdrawn in retirement); **gross-up** = inflating a net annual need to the pre-tax gross
amount using progressive tax brackets; **installation** = the singleton row all data belongs
to; **cascade** = the ordered allocation-rules pipeline distributing monthly surplus to assets.

## 2. Test inventory (as of 2026-08-19, post-3.5.0)

Three suites. None share infrastructure; run all three before merging. Counts below are date-stamped,
not authoritative — recount with the commands in "Provenance and maintenance".

| Suite | Location | Needs | Command (from repo root) |
|---|---|---|---|
| Engine unit tests (**67** as of 2026-08-22) | `crates/engine/src/{projection.rs (32), history.rs (22), runway.rs (13)}` `mod tests` | Nothing (pure `Decimal` math, no I/O) | `cargo test -p futurefin-engine` |
| Backend integration (**43 files on 2026-08-27**; 33 files / 375 attributes on 2026-08-22) | `apps/api/tests/*.rs` | Postgres reachable via `TEST_DATABASE_URL` | See below |
| Frontend Vitest (**368, 16 files, as of 2026-08-22**) | `apps/web/src/**/*.test.ts` | Node only (`environment: "node"`, no jsdom) | `npm test --workspace futurefin-web` |

**Whole-workspace total: 498 on 2026-08-22** (`cargo test --workspace`), which is engine + the 57 API
lib unit tests + integration. Ask the runner for totals; a `grep` of attributes is an approximation
(loops generate tests on the frontend, and an attribute is not always an executed test).

Plus API lib unit tests run by `cargo test --workspace` (no Postgres; count with
`grep -rn '#\[tokio::test\]\|#\[test\]' apps/api/src | wc -l` rather than trusting the 57 this
line used to freeze — that count predates Fases 2–4 of the same MCP-audit train, already merged
to `main` and not re-audited here): notably `apps/api/src/handlers/backup_user/schema.rs`
`mod tests` (**14**; 2 added in v1.6.0 for `.ffbackup` v5 and 2 in v1.8.0 for v6
migration/round-trip). **Fase 5 (issue #86) added 1**: `apps/api/src/handlers/person_view.rs`
gains `LedgerView::as_str()` (the inverse of `resolve`, used to echo `view` at the response root —
see `context_fields.rs` above) and its test `as_str_round_trips_through_resolve`, taking that
file from 4 to 5.

> **Correction (2026-08-22) — this skill used to claim `handlers/transactions/` carried unit tests
> for "CSV presets, fingerprint/ordinal, rule precedence". It did not, and the false claim cost
> money.** `csv_presets.rs` and `import.rs` had **zero** `#[test]`, so `parse_spanish_decimal` could
> do `replace('.', "")` without looking at what the dot separated: `2100.00` became `210000` and the
> transaction went in **a hundred times larger**, silently. No test caught it because no fixture
> amount contains a dot — that branch never executed in the whole suite. What exists **today**
> (count it, don't copy it: `grep -c '#[test]' apps/api/src/handlers/transactions/*.rs`):
> `csv_presets.rs` **1**, `schema.rs` **6** (concept normalization, the SQL fold mirroring the Rust
> fold, LIKE-needle escaping, canonical amount across scales, fingerprint determinism, rule-pattern
> derivation), everything else **0**. A row describing tests that do not exist is worse than a
> missing row: it stops anyone from writing them.

### Backend integration — full invocation (verbatim)

```bash
# One-time: dedicated test DB on port 5433 (avoids clashing with dev on 5432)
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine

# Run everything (engine + integration + api unit):
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace
```

If `TEST_DATABASE_URL` is unset, tests default to that exact URL
(`apps/api/tests/common/mod.rs::test_database_url`). If no Postgres is listening there, every
integration test panics at "connect to TEST_DATABASE_URL" — that is the "hangs/fails without
DB" symptom. Single test: append `-- <test_fn_name>` or `--test <file_stem>`.

**3.0.0 leaves this untouched.** The self-contained production image changed nothing here: the
test database is still the standalone `ff-test-db` container on **port 5433**, still reached over
TCP, still one `ff_test_<uuid>` schema per test. Do not point the suite at the embedded Postgres
of a running `futurefin` container — it holds real data and exposes no TCP port.

Useful new option: `TEST_DATABASE_URL` also accepts libpq's **Unix-socket** form,
`postgres:///futurefin?host=/path/to/socket&user=futurefin` (sqlx 0.8 parses it; the 3.0.0
entrypoint uses exactly this shape to point the API at the embedded server, and the full suite
was run against a socket URL to confirm). Handy when you already have a local Postgres on a
socket and would rather not publish a port; the 5433 TCP default stays the documented path.

### `isolated_pool()` mechanics (read before touching the harness)

`apps/api/tests/common/mod.rs::isolated_pool()`:
1. Creates schema `ff_test_<uuid-simple>` in the test DB.
2. Opens a pool (max 5 conns) with `after_connect` hook: `SET search_path TO "<schema>", public`
   on every connection — so all queries in the test hit only that schema.
3. Runs `sqlx::migrate!("./migrations")` inside it (**44** migration files as of 2026-08-27, 42 on
   2026-08-22 — count with `ls apps/api/migrations/*.sql | wc -l`).
4. Returns `(PgPool, schema_name)`. **Schemas leak intentionally** — no teardown, so a failed
   test leaves its state inspectable.

Cleanup when they pile up (note: the `make clean-test-schemas` / `scripts/clean-test-schemas.sh`
mentioned in the `common/mod.rs` doc comment do NOT exist as of 2026-07-02 — clean manually):

```bash
# Nuke everything: wipe the container
docker rm -f ff-test-db   # then re-run the docker run one-liner above

# Or drop schemas surgically:
docker exec ff-test-db psql -U futurefin -d futurefin_test -tAc \
  "SELECT 'DROP SCHEMA ' || quote_ident(nspname) || ' CASCADE;' FROM pg_namespace WHERE nspname LIKE 'ff_test_%'" \
  | docker exec -i ff-test-db psql -U futurefin -d futurefin_test
```

### Harness knobs: `TestConfig` and `spawn_with` (added 2026-08-27)

`TestApp::spawn()` is unchanged and is still what almost every test wants. For the reverse-proxy
axes there is `TestApp::spawn_with(TestConfig { .. })`, whose `Default` **is** the historical
`spawn()` (no prefix, no trusted peers, no SSO, no SPA):

| Field | Simulates |
|---|---|
| `trusted_header_auth: bool` | `FUTUREFIN_TRUSTED_PROXY_AUTH` |
| `trusted_peers_any: bool` | `PeerPolicy::Any` instead of `Disabled`. **`Any` is the only workable policy in tests**: `oneshot` carries no `ConnectInfo`, so the peer is `None` and an IP list would never match |
| `base_path: String` | `FUTUREFIN_BASE_PATH` |
| `with_spa_index: Option<String>` | Mounts `spa::serve_index` as the fallback with that HTML (no `ServeDir`, no disk) |
| `ha_idp: Option<Arc<FakeHaIdp>>` (4.3.1) | `FUTUREFIN_HA_SSO_URL` being set: `Some(_)` populates `AppState.ha_sso`, so `/v1/auth/ha/*` works with no real Home Assistant |
| `ha_sso_url: Option<String>` (4.3.1) | The HA origin the test sees; **defaults to `https://ha.test`** and is only read when `ha_idp` is `Some(_)` |
| `web_static_root: Option<PathBuf>` (4.4.0, issue #85) | Mounts the **real** static fallback via `spa::mount_static_spa` — the same function `main.rs` calls, `ServeDir` included. **Not the same axis as `with_spa_index`, and the difference is load-bearing**: `ServeDir` does not call its fallback for methods other than GET/HEAD, so a missing route answers a `POST` with a bare 405 and a `GET` with the SPA shell (200 `text/html`) — neither of which a router built without `ServeDir` can reproduce. Wins over `with_spa_index` if both are set. Pair it with `TempWebRoot::with_index(html)`, a temp directory holding an `index.html` that deletes itself on `Drop`. |
| `mcp_disabled: bool` (4.4.0) | `FUTUREFIN_MCP_ENABLED=0`, without hand-building a router. Default (`false`) is `spawn()`'s historical MCP-on. |
| `public_url: Option<String>` (4.4.0) | A pre-validated `FUTUREFIN_PUBLIC_URL`; since 4.4.0 it may carry a subpath (`https://host/futurefin`), not only a bare origin. |

**Any test that asserts what happens when a route does NOT exist must set `web_static_root`, or
it is proving a claim about a router nobody publishes.** This is exactly the bug Fase 4 of the
MCP work (issue #85) fixed: the old kill-switch test built a router with no SPA fallback at all,
so it confirmed a 404 that the shipped image never produced — in the real image, `POST /mcp`
with `FUTUREFIN_MCP_ENABLED=0` returned a silent 405 and `GET /.well-known/oauth-authorization-server`
returned the SPA shell as 200 `text/html`. Both `mcp_http.rs` and `oauth_flow.rs` now build their
kill-switch tests with `TestConfig { mcp_disabled: true, web_static_root: Some(tmp.path), .. }`
and assert the `ServeDir` is really serving something *before* asserting the 404 — otherwise a
future refactor that forgets to mount anything would pass for the wrong reason.

Two harness facts worth knowing before you write a proxy-ish test:
- **The router is wrapped in `frame::with_frame_policy`, exactly like `main.rs`.** Without it the
  `X-Frame-Options`/CSP policy would be invisible to tests, because `TestApp`'s router carries none
  of the binary's outer layers.
- Header-carrying requests go through `get_with_headers` / `post_with_headers` (the latter added
  with `sso_login.rs`); the cookie helpers are unchanged.
- **No test sets an environment variable.** Config arrives through `AppState::with_trusted_proxy`
  and `AppState::with_ha_idp`, which is what keeps the suites parallel-safe.

### Outbound integrations: fake behind a trait, **never** an HTTP mock (house style, 4.3.1)

The HA identity provider is the first time this binary calls **out** over the network, and the seam
it introduced is the pattern every future outbound integration must copy:

1. **A narrow trait for the outside world.** `ha_idp::HaIdp` has exactly three methods
   (`exchange_code`, `identity`, `revoke`) and lives beside the pure helpers it needs. The real
   client (`ha_idp/client.rs`: reqwest/rustls + tokio-tungstenite) is one implementation behind
   `Arc<dyn HaIdp>` in `AppState`.
2. **The test double is a hand-written fake in `common/mod.rs`, not `wiremock`.** Two structural
   reasons, both worth stating because "just use a mock server" is the reflex: the harness is
   `tower::ServiceExt::oneshot`, which **opens no sockets**, so there is no real HTTP stack to point
   a fake server at; and half the dialogue with HA is a **WebSocket** (`auth/current_user` — HA has
   no REST equivalent), which an HTTP mock cannot cover at all.
3. **The fake logs its calls in order.** `FakeHaIdp::calls() -> Vec<FakeCall>` is what turns "the
   order is a contract" into an assertion: `[Exchange { code, client_id }, Identity, Revoke]`, with
   the revocation **before** any DB write. A stub that only returns canned values could not prove
   that, and the property it proves (FutureFin keeps no HA credential) is the one the security
   story rests on. Scripted constructors: `happy`, `without_refresh`, `exchange_fails`,
   `identity_fails`, plus `set_identity` to make the same `AppState` speak for a second person.
4. **`revoke` is infallible by signature** — the trait forbids a failure there from sinking a login
   already proved. So "revocation failed" is modelled as what actually happens: the call occurs,
   changes nothing, the user gets in.
5. **The real client carries zero unit tests, on purpose.** Its pure parts (URL building, cookie
   codec, `next` sanitizing) were pushed up into `ha_idp/mod.rs` and unit-tested there (11); what
   remains is I/O against a real HA, verified by a live smoke, not by a double that would only
   assert our own assumptions back at us.

### Integration test files (recount, don't trust a frozen number here)

Recount before trusting: `ls apps/api/tests/*.rs | wc -l` and
`grep -rc '#\[tokio::test\]\|#\[test\]' apps/api/tests/*.rs | awk -F: '{s+=$2} END {print s}'`.
The **44** files / **449** attributes this section used to freeze (2026-08-27) is stale on two
independent fronts and neither correction is exhaustive here: **Fase 5** (issue #86) adds exactly
one file, `context_fields.rs` (11 tests — see its row above), which is fully documented in this
change; separately, Fases 2 and 3 of the same MCP-audit train (issues #83/#92, already merged to
`main` before this branch) added 13 more files this skill was never updated for:
`allocation_resolution.rs`, `budget_patch_guards.rs`, `liabilities_repayment_model.rs`,
`liability_derived_principal.rs`, `liability_derived_principal_parity.rs`,
`mcp_audit_and_scope.rs`, `mcp_confirm_and_impact.rs`, `projection_liability_interest.rs`,
`projection_number_semantics.rs`, `query_param_validation.rs`, `summary_net_return.rs`,
`transactions_rules_hardening.rs`, `write_safety_phase3.rs`. That drift is **out of scope for
this change** (Fase 5 doc work only) and is called out here so nobody mistakes the table above
for a complete inventory; a full re-sync of those rows is separate work.
On any disagreement between a
row's description here and `.claude/tests.md` (refreshed every release train), tests.md wins —
fix this table in the same change.

| File | Tests | Covers |
|---|---|---|
| `smoke.rs` | 5 | health/ready, 401 unauth, register→login→me roundtrip, first-user bootstrap → owner |
| `liabilities_purge.rs` | 5 | expired liabilities hidden from GET/summary but **persist in DB** (reads never mutate) |
| `body_limits.rs` | 3 | 1 MiB global body cap → 413; `/backup/user-import` accepts up to 16 MiB; **4.4.0 (issue #85)**: `oversized_mcp_body_returns_413` — `/mcp` is a `route_service`, so `DefaultBodyLimit` never reaches it (rmcp reads the body itself, default 4 MiB); the documented "1 MiB global" invariant was false there until `with_max_request_body_bytes` fixed it explicitly. Test body is 2 MiB — above the global, below rmcp's old default |
| `installation_patch.rs` | 5 | unknown `fire_number_mode` rejected; legacy `annual_expense_adjusted` alias accepted; valid mode change |
| `unique_violation.rs` | 2 | duplicate username / duplicate category name → 409 via central `From<sqlx::Error>` |
| `projection_marker.rs` | 1 | regression capture: stable marker + starting NW across the perf refactor (the template for capture-first) |
| `fire_parity.rs` | 1 (×7 fixture cases) | server `jubilacion_target_net_worth` matches `fire-parity.json` ± 1 € |
| `projection_cache.rs` | 5 | cache hit faster than miss + identical body; invalidation on mutation; logout drops only `view=mine` entries; `density=hybrid` decimation (months 0–12 monthly, then 24,36,48…); monthly/hybrid cached as separate keys |
| `history_snapshots.rs` | 20 | snapshot capture (copied terms) / same-day upsert / exclude shared+expired / backfill CRUD roundtrip with `year` filter + cascade / 400 validations (future, `duplicate_item_id`, terms-on-asset) / 409 date taken / 404 cross-user / 403 viewer on every mutation / GET never mutates / `snapshot_mutations_do_not_touch_projection_cache` (cache stays HIT — history is NOT a projection input) |
| `history_series.rs` | 7 | `GET /v1/history/series`: empty→200, exact linear interpolation between two asset snapshots, join to live values (deleted asset→0 at k=0), amortization curve above the chord with exact endpoints, household sums two users + `?view=mine` filters, markers carry date/kind/total, single today snapshot. Numbers predicted before running |
| `backup_user_roundtrip.rs` | 13 | `.ffbackup` v4/v5/v6: roundtrip with identical history series, item re-link to fresh asset UUIDs, null `ledger_index` keeps `item_key`, v3 still imports (0 snapshots), out-of-range index → 400 + rollback, import invalidates projection cache (pre-existing bug fix), preview reports snapshot/item counts, viewer 403; plus v5 (v1.6.0) transactions/imports/rules round-trip with index re-link and preserved `fingerprint_ordinal`; plus v6 (v1.8.0) `recurring_transaction_rules` round-trip with `recurring_rule_index` re-link and preserved `last_materialized_month` |
| `account_and_members.rs` (4.0.0) | — | The two levers the docs promised and the code lacked. `POST /v1/auth/password`: rotates the hash, keeps the CALLING session alive and kills the rest, revokes `ffp_` tokens and OAuth grants in the same transaction, wrong `current_password` → **400 `current_password_invalid`** (not 401, on purpose), length policy. `/v1/installation/members`: GET open to any member (viewer included), PATCH/DELETE owner-only, `last_owner` guard on both, DELETE cuts all four credentials at once and **keeps the person's data**, 404 on a non-member |
| `openapi_contract.rs` (4.0.0) | 4 | **No Postgres**: gates on the generated `openapi.json` — every templated path declares its parameters, the two `ImportPreviewResponse` structs no longer collide on one component name, authentication is declared and applied to every private operation (the spec previously had no `securityScheme` at all and showed 81 session-required operations as public), and no dangling `$ref`. There was **no** test on the spec before, which is why none of that broke anything |
| `transactions_import.rs` | 15 | CSV import: MyInvestor/N26 header autodetection, preview flags `already_imported` (omitted by default), confirm inserts with ordinals, same-file re-confirm → 0 new, `force` appends a fresh ordinal, internal-transfer heuristic, learned rule pre-assigns on next preview, non-EUR rejected on confirm, viewer 403, preview↔confirm sha mismatch → 400; **accent folding** (post-2.0.0): savings hint + learned-rule matching are diacritic-insensitive (`savings_hint_accent_insensitive_*`, `learned_rule_matches_accent_insensitive*`) |
| `fixtures_shape.rs` | 3 | **sin base de datos**: los tres CSV de `tests/fixtures/` siguen ejercitando lo que `transactions_import.rs` asume (escala `-26.000000000` → 4 dp, par opuesto, partner «Cuenta de Ahorro», tokens de transferencia, hint de ahorro, ≥3 filas que colapsan en UN patrón de regla) y **ninguno contiene una cadena con forma de IBAN**. Añadido en agosto de 2026 al rehacer los fixtures (`futurefin-data-hygiene`) |
| `error_codes_parity.rs` | 2 | **sin base de datos**: extrae del fuente todo `snake_code:` que la API puede devolver, lo compara con `tests/fixtures/error-codes.json` (187 códigos) y exige que ninguno repita el nombre de una clase HTTP. Regenerar: `UPDATE_ERROR_CODES=1 cargo test -p futurefin-api --test error_codes_parity`. Su pareja en el front es `errorMessages.test.ts`, que falla si un código se queda sin frase en español |
| `transactions_crud.rs` | 15 | manual create/batch, `savings` requires NULL category, income/expense scope validation, **PATCH edits op_date/amount/concept on imported rows with the fingerprint anchored to the CSV** (`patch_imported_fields_editable_fingerprint_anchored`, ex `patch_imported_op_date_is_immutable`; no more `immutable_field`) while manuals recompute the fingerprint and free the ordinal (`patch_manual_op_date_recomputes_and_allows_reuse`), deleting linked asset/liability SET NULLs the link keeping the row, category delete remaps transactions, viewer 403 |
| `transactions_summary.rs` | 15 | exact Decimal per-category actual/budget/avg, **weighted average** (denominator = `months_with_data`, not the window width → short history no longer dilutes to 0), `avg_window` 3/6/12/`ytd`/`all` + legacy `avg_months` alias, invalid `avg_window` → 400, partial month flagged, savings excluded from expense, «Sin categoría» bucket. **No** derived-debt line anymore: `totals.expense_budget` = Σ expense-category budget |
| `transactions_projection_cache.rs` | 6 | mode-conditioned cache contract (`fire_settings.savings_source`): `mode_a_mutations_do_not_touch_projection_cache` (mode `budget`: cache stays HIT across import/create/edit/delete/rule writes + recurring endpoints — transactions not engine inputs), `mode_b_each_mutation_invalidates_projection_cache` (mode `transactions_avg`: every mutation invalidates), `mode_c_mutation_invalidates_projection_cache` (mode `budget_income_real_expense`: parity with B), `flipping_savings_source_invalidates_projection_cache` (switching mode invalidates) |
| `savings_source.rs` | 22 | `savings_source` toggle observed through the projection (`monthly_delta_assumption`, `?months=240` to bypass the cache). Serde/PATCH: default `budget`, roundtrip of `transactions_avg` and `budget_income_real_expense`, unknown value → 422 listing all 3 variants. **Mode B**: weighted avg excludes `savings` + partial month, `months_with_data==0` → budget fallback, hybrid liability subtraction (real linked / nominal / ended not subtracted), `expense_eff ≥ 0` clamp, `annual_expense` target uses `expense_eff`, household/mine scoping, loan-end step-up, `GET /v1/assets` follows the mode. **Real months**: `pseudo_empty_month_excluded_from_avg` (real month 2000 + recurring-only month 3000 → months=1, avg=2000), `real_month_counts_recurring_too` (a real month counts its recurring → avg 5000), `mode_b_all_pseudo_empty_falls_back_to_budget` (full backfill → 0 real months → budget). **Mode C**: `mode_c_income_budget_expense_real` (budget income 5000 − real expense 800 = 4200), `annual_expense` target uses `expense_eff`, `current_income` target uses budget income, `months==0` → fallback. **v2.2.0**: `assets_cap_targets_follow_savings_source_mode` (asset caps `months_expense`/`income_multiple` are 18.000/10.000 in mode A and 6.000/8.000 in mode B — fails against the pre-fix code) and `projection_series_reports_effective_savings_source` (the two new `/v1/projection/series` fields: budget/0, fallback budget/0, transactions_avg/1). Numbers predicted before running |
| `summary_runway.rs` | 10 | `financial_health` runway + expense base (v2.2.0; SWR threshold v2.3.0). The first two are the **capture-first regression** written *before* the v2.2.0 change and still green after both: `runway_pre_change_baseline_liquid_over_expense` (mode A, no return/inflation → `runway_months == liquid_assets_total / expense_total` exactly, `expense_derived` = active liability payments) and `runway_zero_expense_is_null` (no expense → field not serialized). v2.2.0 behavior, each with its number predicted in the doc comment: return extends (12.000 @ 5 % vs 1.200/month → >10 and <11), inflation shortens (3 % → <10 and >9), `runway_indefinite_when_withdrawal_within_swr` (renamed from `..._when_returns_cover_expense`; 1M @ 7 % vs 1.000/month → `runway_months` null + `runway_is_indefinite` true), `mode_b_runway_uses_effective_expense_base` (identities `expense_total = expense_reg + expense_der` and `net = income − expense_total` restored; runway 16.000/1.600 = 10 where mode A would be 16.000/8.800), `mode_b_zero_months_falls_back_to_budget_runway` (fallback block identical to mode A). The three added in v2.3.0 pin the SWR threshold end-to-end: `runway_indefinite_at_exact_swr_threshold` (taxes off, exact boundary 240.000 € / 700 €/month / SWR 3,5 % → 840.000 = 840.000), `runway_gross_up_raises_threshold` (270.000 € / 700 €/month; the flag flips when `taxes_enabled` turns on — proof the handler feeds the *grossed* annual expense), `runway_swr_zero_never_indefinite` (`swr_pct = 0` → flag false and `runway_months` carries the 1200 floor) |
| `summary_savings_source.rs` | 13 | `GET /v1/summary` `financial_health` following `savings_source`: mode A = budget, mode B uses the avg with hybrid subtraction, `months==0` → fallback, household/mine scoping, `mode_b_summary_pseudo_empty_month_excluded` (recurring-only months don't count in the `months_with_data` of the `*_basis` fields — they replaced the `savings_source_months_with_data` scalar in 3.9.0), `mode_c_income_not_overwritten` (mode C keeps budget income in `income_monthly_equivalent`) |
| `transactions_recurring.rs` | 15 | recurring rules (v1.8.0): `recurrence` create makes rule + linked origin instance, idempotent `materialize` (2nd call → 0), never a future `op_date` (current month only once its day arrived), `day_of_month` clamped to month end, deleting an instance is not recreated on re-materialize (cursor), rule `DELETE` keeps instances (SET NULL), viewer 403 on materialize/delete, out-of-range `recurrence.day_of_month` → 400; **create-time backfill** (post-2.0.0): `create_with_past_date_only_fills_months_that_have_real_data` (a past date fills, in the same commit, **only the months that have real data** — the old name, `create_with_past_date_backfills_instances`, described the indiscriminate backfill from before 3.9.0's convergence), `recurrence_op_date_within_bound_created`, and the 10-year bound `recurrence_op_date_too_old_*` → 422 `recurrence_too_old` |
| `history_cashflow.rs` | 6 | `GET /v1/history/cashflow`: exact monthly aggregates (Decimal-string, household + mine), fine series passes through snapshots, `/v1/history/series` byte-identical with and without transactions (tier-1 regression), `daily` with window >6m → 400, `fine` absent without links |
| `oauth_flow.rs` | 35 | Embedded OAuth 2.1 (v3.1.0), the largest suite by test count: `.well-known` metadata is JSON and not the SPA fallback, issuer follows `X-Forwarded-*` and a malformed `Host` → 400, the `/mcp` **401** advertises `resource_metadata` while the **403** carries no `WWW-Authenticate` (anti-loop), DCR happy paths + 8 rejected `redirect_uris` + unknown metadata ignored, `authorization_code`+PKCE end-to-end to `/mcp` (`expires_in: 3600`, `no-store`, `state` echo + `iss`), code/refresh **reuse-detection** revoking the whole grant with `revoked_reason` `code_reuse`/`refresh_token_reuse`, refresh rotation, unknown `client_id` → **401 `invalid_client`**, consent fatal-vs-redirectable split (fatal never returns `redirect_to`), `plain` PKCE and foreign `resource` redirectable, deny/pending/no-session gating, re-consent reuses **one** grant row, panel + RFC 7009 revocation cutting `/mcp` instantly, cross-user isolation, kill-switch (protocol 404 but `/v1/oauth/connections` still 200, rewritten in 4.4.0 with the real static fallback mounted — see the harness-knobs section above), the `GET /oauth/authorize` route-shadowing guard (now also guards against a **401**, the signature of `/mcp`'s router applying `Router::layer` instead of `route_layer` and dragging its Bearer auth onto every unknown route — caught once during Fase 4), `.ffbackup` export unaffected, and no crossing between the `ffp_`/`ffo_` schemes. Real PKCE via `OsRng`+SHA-256; expiries forced with SQL (no clock mock). **4.4.0 (issue #85)**: `expired_oauth_credentials_are_collected_on_the_next_token_request` (no `DELETE` on the OAuth credential tables existed before this — lazy GC now runs inside `POST /oauth/token`, never a GET, pruning expired codes/access tokens but sparing a refresh that is consumed-but-not-yet-expired, which reuse-detection still needs alive), `discovery_metadata_is_never_cached_and_varies_on_the_forwarded_headers` (the two metadata endpoints now carry `Cache-Control: no-store` + `Vary: X-Forwarded-Proto, X-Forwarded-Host` — the one OAuth response that lacked it), `public_url_with_a_subpath_prefixes_every_advertised_url` (`FUTUREFIN_PUBLIC_URL` now accepts a subpath, validated with the same `prefix::normalize_prefix` as `FUTUREFIN_BASE_PATH`; fixes issuer, `resource`, all four endpoints, and the `WWW-Authenticate` challenge). **Note**: the whole `apps/api/src/oauth/` module has zero in-source unit tests — coverage is 100 % here |
| `api_tokens.rs` | 8 | API tokens (v3.0.0) + the `/mcp` Bearer gate: 201 exposes the `ffp_` secret exactly once with a coherent `token_prefix`, listing never returns secret or hash, revoked/expired/malformed/foreign-prefix/random all collapse to the same 401, pending user → 403 on create, a viewer's token authenticates, cross-user isolation (list + foreign DELETE → 404), 400 validations and the 10-active-token limit (`token_limit_reached`) released by revoking |
| `mcp_http.rs` | count with `grep -c '#\[tokio::test\]\|#\[test\]' apps/api/tests/mcp_http.rs` (the 18 this row used to freeze predates Fases 2–4 of the same MCP-audit train, already merged to `main` and not re-audited here) | MCP end-to-end over `/mcp` (stateless 2026-07-28 + SEP-2243 headers): initialize, `tools/list` freezing the **full 68-tool catalog** (52 before Fase 6/issue #87) (`tools_list_returns_exactly_the_v1_catalog` — every new tool must be added there consciously) and asserting annotations on every tool (hints derived from the name prefix), **byte-for-byte parity** `get_summary` vs `GET /v1/summary` + the issue-#2 read tools vs their GETs, `get_projection` fixed-hybrid + opt-in `asset_series` + shares the handler's cache, validation error → `is_error: true` with the HTTP wire JSON, `view: "mine"` scoping, `list_transactions` SQL pagination, `get_settings` user block, and `mcp_enabled=false` → 404 (router built by hand). **Fase 5 (issue #86) added 4**: `list_tools_echo_the_applied_view_and_keep_content_parity` (the 7 tools that wrap or paginate their GET echo the applied `view` and their envelope content matches `GET ?view=…` for both scopes, while the underlying GET stays a bare array), `list_snapshots_paginates_and_declares_item_suppression`, `list_transaction_imports_paginates_and_echoes_the_view`, `tool_descriptions_stay_within_the_context_budget` (merge gate, `PER_TOOL_MAX = 600` / `TOTAL_BUDGET = 24_000` — see § 4) |
| `mcp_write.rs` | count with `grep -c '#\[tokio::test\]\|#\[test\]' apps/api/tests/mcp_write.rs` (the 17 this row used to freeze predates Fases 2–4, already merged to `main` and not re-audited here) | MCP write tools: viewer → `forbidden`, the live `mcp_write_enabled` toggle (cuts the next write without restart, reads survive), MCP-created rows indistinguishable via HTTP, the **FULL/COND/NONE cache contract through the MCP path** (`warm`/`assert_invalidated` helpers), preview/confirm on every destructive tool (preview is SUCCESS and does not execute), `update_fire_settings` owner-gate + field-by-field merge, reconcile tools (3.5.0), and the post-3.5.0 CRUD-parity test (`update_asset` full body incl. `clear_purchase_price`, `update_liability` editing the SAME row via `patch_liability_core`, both FULL, shared 400s, toggle cutting both). **Fase 5 (issue #86) added 1**: `liability_type_tag_is_writable_and_reaches_the_summary_breakdown` (`create_liability`/`update_liability` gain a tri-state `type_tag` — omit keeps, empty string clears, no `clear_type_tag` needed — and the value reaches `summary.liabilities_by_type_tag`) |
| `mcp_simulate.rs` | 6 | `simulate_projection`: baseline ≡ `get_projection`, the discriminating override pair (real expense moves the target, neutral adjustments don't), `one_off_expense` by date ≡ by month_index, return override sinks final NW, bounds re-validated, and **cache neutrality** (simulating never creates nor touches cache entries) |
| `transactions_reconcile.rs` | 19 | Transfer reconciliation (3.5.0): deterministic auto-pass (±5-day window with exact 5/6 boundary, cross-import pair, greedy with multiple candidates, fixed-point idempotence, distinct owners never), unreconcile persists the anti-resurrection rejection, PATCH of `amount`/`op_date` breaks the pair WITHOUT rejection, deleting a leg/batch unreconciles the survivor, manual pairing without window + its 400s, viewer 403, owner-guard 404 |
| `base_path.rs` (2026-08-27) | 4 | Per-request public prefix in the SPA shell, seen through the whole router. **The master invariant goes first**: with no proxy headers the HTML is served **byte-identical** to the file (`root_without_proxy_headers_is_the_shell_verbatim`) — compose mode does not change one character. Then `X-Forwarded-Prefix` rewrites every absolute `src`/`href` and injects `window.__FF_BASE__`; `X-Ingress-Path` **wins** over `X-Forwarded-Prefix`; an invalid ingress header is ignored and the next valid source is used. Needs `TestConfig { with_spa_index: Some(html) }` — the harness mounts `spa::serve_index` as fallback, never a `ServeDir` |
| `frame_options.rs` (2026-08-27) | 4 | Both halves of the conditional anti-clickjacking (D17): default → `X-Frame-Options: DENY`; **`X-Ingress-Path` from an untrusted peer → still `DENY`** (the header alone must not disarm it); trusted peer + header → `Content-Security-Policy: frame-ancestors 'self'` **and no `X-Frame-Options`** (a surviving `DENY` wins over the CSP and the HA panel goes blank); trusted peer without the header → `DENY`. `TestConfig { trusted_peers_any: true }` |
| `session_cookie_path.rs` (2026-08-27) | 3 | `Path` of `ff_session`: `Path=/` with no proxy headers (unchanged for compose), scoped to the prefix under ingress (HA add-ons share an origin, so `Path=/` would leak the cookie to other add-ons' ingress paths), and **logout removes the cookie on that same `Path`** — a browser only matches a removal when name AND path agree |
| `sso_login.rs` (2026-08-27) | 12 | `POST /v1/auth/sso` (D18). In priority order: **the door is closed by default** (perfect headers, no config → 401), untrusted peer → 401, non-UUID identity → 400; the external identity is stable (same `X-Remote-User-Id` → same user, no duplicate rows); provisioning goes through the same gates as password registration (first = owner + installation, second = pending); diacritics fold into valid usernames and a taken username gets a suffix; an SSO account **cannot** log in with a password nor set one (`sso_account_no_password`), and password accounts are untouched by the new column |
| `ha_idp_login.rs` (4.3.1) | 18 | «Entrar con Home Assistant» (`GET /v1/auth/ha/start` + `/callback`, D19), in the order its own header declares. **The door is closed by default**: routes always mounted, but with no `FUTUREFIN_HA_SSO_URL` the start is 401 `ha_sso_disabled`, sets no cookie and provisions nobody. **Parity with header-SSO** is the cornerstone: `header_sso_and_ha_login_resolve_to_the_same_user` — hyphenated UUID through `POST /v1/auth/sso`, the same id as 32 bare hex through the HA flow, **one** `users` row (I16). **The `state` is the whole security boundary**: four ways of not having it (foreign state, no state, no cookie, garbage cookie) all end in `ha_state_mismatch` with **zero users and zero calls to the provider**, and the cookie is single-use. **Call order** pinned on `FakeHaIdp::calls()`: `[Exchange, Identity, Revoke]` with a byte-identical `client_id`, no `Revoke` when HA returned no refresh token. Plus the redirect shape (three exact params, `no-store`, `HttpOnly`/`Lax`/`Max-Age=600`/scoped `Path`, and under `base_path` the cookie is scoped while the `client_id` stays the bare origin), provider failures → `ha_exchange_failed` / `ha_identity_failed`, `?error=access_denied` never touching the provider, the `next` anti-open-redirect battery (prefixed exactly once), username collision → `maria`/`maria-2`, and the shell announcing `__FF_HA_LOGIN__`. Needs `TestConfig { ha_idp: Some(FakeHaIdp::happy(..)) }` |
| `migration_guard.rs` (2026-08-27) | 2 | Downgrade refusal. Uses **only** `common::isolated_pool()` (no `TestApp`): applies the real migrations to a fresh schema, then inserts a fake `_sqlx_migrations` row from "the future" — exactly what an older binary sees after an upgrade — and asserts `run_migrations` returns `MigrationError::Downgrade` with the operator banner. Tests the contract (don't lose sqlx's `VersionMissing`, translate it into something actionable), not the implementation |
| `budget_liability_quotas.rs` | 10 | Liability quotas inside `GET /v1/budget` (renamed from `budget_derived.rs` in 3.7.0, when the quota became an ordinary `entries` row): entry shape (`source`, `liability_id`, `label`, expense category) and coexistence with the manual entry of the same category; totals (`expense_regular` = sum of expense entries, no `expense_derived`); quota excluded from `expense_retirement_*`; **quota excluded from the engine expense base** (`monthly_delta_assumption` — hand-predicted double-count regression); active-liability predicate (NULL end date derives, expired doesn't, `>=` boundary, no payment plan → nothing), weekly ×52/12, household/mine scoping, quota without `expense_category_id` still counts |
| `context_fields.rs` (new, Fase 5 — issue #86) | 11 | The context-field contract (provenance/window/absence-reason), pinned over **HTTP, not MCP** — the core sets these fields, so the HTTP handler is the right level to test, and the MCP tools inherit them for free. Every test follows the two-case norm of § 1: `every_view_aware_response_echoes_the_view_it_applied` (`summary`/`budget`/`projection/series`/`allocation-rules/resolution` all carry `view` at the root, for both `household` and `mine`), `budget_and_summary_declare_whether_their_totals_are_plan_or_actual` (`financial_health.basis` moves between `plan`/`actual`/`mixed`; `budget.totals.basis` is always `"plan"`), `upcoming_totals_publish_the_horizon_they_are_summing` (`upcoming_flows_count` + `upcoming_last_due_date_ymd`), `untagged_liabilities_group_under_null_not_a_spanish_literal` (`liabilities_by_type_tag[].type_tag` is `Option<String>` now — `null`, not the old `"(sin etiqueta)"` string), `history_series_default_window_is_bounded_and_says_it_truncated` (omitted `window_months` = 120, `window_truncated` says so), `history_chart_values_are_published_with_two_decimals`, `markers_declare_capture_versus_backfill` (`source: "capture"` or `"backfill"`), `cashflow_says_why_the_fine_curve_is_missing` (`fine_absent_reason` across its four values), `snapshots_distinguish_suppressed_detail_from_an_empty_snapshot` (`item_count` + `items_included`), `import_batches_point_at_their_possible_duplicates` (`possible_duplicate_of`, matched on `original_filename` + `account_asset_id` **within the loaded page only**), `projection_publishes_the_dated_planning_flows_that_move_the_curve` (`events` + `events_truncated`, capped at 100) |

### Frontend Vitest files (no congeles el total aquí — cuéntalo con `npm test --workspace futurefin-web 2>&1 | grep Tests`; **368 en 16 ficheros a 2026-08-22**)

Config: `apps/web/vitest.config.ts` — `environment: "node"`, `include: ["src/**/*.test.{ts,tsx}"]`,
`globals: false` (import `describe/it/expect` from `vitest` explicitly).

| File | Tests | Covers |
|---|---|---|
| `apps/web/src/lib/format.test.ts` | 38 | es-ES Intl formatting, null/NaN/empty edges, Decimal string preservation; `formatMonthsRough` in years+months from 24 and `formatRunwayValue` («Infinito» when the runway is indefinite; «+100 años» for the `months ≥ 1200` floor) (v2.3.0) |
| `apps/web/src/lib/fire.normalize.test.ts` | 21 | `savings_source` normalizers/gating, incl. `savingsAvgParenthetical` («promedio de N meses», singular, `undefined` in mode A / after fallback) (v2.2.0) |
| `apps/web/src/lib/dates.test.ts` | 32 | civil calendar (leap years, day clamping, age around birthday), TZ fallback, payment intervals, negative `addMonthsCivil` deltas (v1.5.0) |
| `apps/web/src/api/client.test.ts` | 10 | fetch mocks: credentials, body serialization, 4xx propagation, 204 handling |
| `apps/web/src/lib/fire.test.ts` | 8 | FIRE parity vs the shared fixture (1 sanity + **7** fixture cases generated in a loop, so the file only shows 2 `it(` call sites) |
| `apps/web/src/lib/history-merge.test.ts` | 12 | `mergeProjectionWithHistory`: identity-by-reference (null/empty/anchor-mismatch → byte-identical render), drops `month_index ≥ 0`, asset-series union by id, future offset |
| `apps/web/src/lib/projection-chart.test.ts` | 10 | `deflationFactorAt` (0 / ±12 / inflation 0) + tick-builders with `startMonth=-24` and the `startMonth=0` regression (identical to prior behavior) |
| `apps/web/src/lib/snapshot-tracker.test.ts` | 8 | `liquidCoverageComplete` (empty→false, full coverage→true, stale after `pruneEditLog`→false, new asset within the window) |
| `apps/web/src/lib/navigation.test.ts` | 5 | Settings sub-tabs (v3.0.0/3.1.0): the `mcp` sub-tab has its own slug + label, `access` was renamed to «Usuarios» **keeping its historical slug** (saved links stay alive), unknown slug → `null` (App falls back to the default sub-tab), any `/ajustes/*` resolves to the `settings` tab, and every slug is unique and round-trips through `settingsSubTabPath` |
| `apps/web/src/lib/oauth.test.ts` | 8 | OAuth consent-screen helpers (v3.1.0): `parseAuthorizeParams` (full query URL-decoded, `null` if any of the 5 required params is missing, absent optionals not invented, and `code_challenge_method=plain` **does** parse — the client/server validation split is frozen in a test), `redirectHostLabel`, `authorizeErrorMessage` |

## 3. CI reality

`.github/workflows/ci.yml` runs on push to `main` and on PRs targeting `main` — since 4.0.1 there
is a single live branch, so that is every path into the tree. Verified against the file on
2026-07-02; the `docker-stack` job re-read on 2026-08-16 (v3.0.0); **the whole section rewritten on
2026-08-22 (4.0.0), when the gap this section used to describe was closed**, and the branch model
updated on 2026-08-24 (4.0.1).

> **4.0.0 — the integration suite, ESLint and Vitest now RUN in CI.** Until then they did not, and
> this skill listed them as a "local obligation" — i.e. they depended on nobody forgetting. A PR
> could go green with the entire handler layer broken (329 of the 447 tests never ran). With the
> repository public and outside contributors that does not hold, so all three entered as **blocking**
> gates. Any document still claiming they are absent from CI is stale — including older copies of
> this very section.

**CI DOES run** (six jobs):
- `rust`: `./scripts/audit-releases.sh --version` (the CHANGELOG covers `Cargo.toml`'s version — the
  hole 2.2.0 slipped through), `cargo build -p futurefin-api --locked` + `cargo test -p
  futurefin-engine --locked`. **clippy and rustfmt are installed but their steps are commented out on
  purpose**: the repo has never passed either (50 unique clippy warnings across 20 files; `cargo fmt
  --check` flags 1.175 blocks across 72 files), and turning them on today would leave CI red from the
  first push — a permanently red CI teaches people to ignore CI. Clean up first, in a separate
  commit, with `cargo fmt --all` alone in its own.
- `integration` (**4.0.0**): `cargo test --workspace --locked` against a **Postgres 16.4-alpine
  service**, `TEST_DATABASE_URL` on `127.0.0.1:5432` (locally it stays 5433, to avoid the dev
  Postgres). One job covers engine + API lib unit tests + integration. `timeout-minutes: 45`. Its
  healthcheck is `pg_isready -h 127.0.0.1` **deliberately**: without the host, during `initdb` the
  image runs a temporary server on the Unix socket only and `pg_isready` returns OK before the
  database exists.
- `docker-stack` also runs **actionlint** over every workflow file (added 2026-08-24 — before
  that, nothing validated the workflows themselves).
- `secrets-scan`: `./scripts/scan-sensitive.sh` — blocking. No IBAN, card, private key or
  provider token in tracked files. Added August 2026 after real bank exports were found in
  `apps/api/tests/fixtures/`; see [`futurefin-data-hygiene`](../futurefin-data-hygiene/SKILL.md).
- `web`: **`npm ci`** (not `npm install` — that silently rewrote the lockfile, so CI potentially
  tested a different dependency tree than the Dockerfile publishes), `npm run typecheck:web`,
  **`npm run lint:web`**, **`npm test` (Vitest)**, `npm run build:web` — build last, because it is the
  slow step and there is no point paying for it after a failure.
- `docker-stack`: since 3.0.0 this is no longer a boot smoke test — it is the **only automated
  evidence that upgrading does not lose data**, and the job comment says so ("no debilitar").
  Its steps, in order:

  | Step | What it proves |
  |---|---|
  | shellcheck (entrypoint + scripts) | `apps/api/docker-entrypoint.sh`, `scripts/*.sh` and this skill-family's `scripts/diagnostics/*.sh` are shellcheck-clean at `-S warning` |
  | Build image | `docker build -f apps/api/Dockerfile` succeeds |
  | Image sanity | both bundled majors run (`postgresql/16` **and** `/15` `postgres --version`), the label `com.futurefin.postgres.majors` equals `15,16`, and running **without a volume ABORTS** with `no persistent volume` (the anti-data-loss guard, asserted as a failure — a container that started would fail the job) |
  | Fresh install → `/v1/ready` + seed | virgin volume boots, logs `initializing fresh PostgreSQL 16`, `/v1/ready` answers within 90×2 s, then register + login + create a category named `Ácido Ñandú` (deliberate Ñ/accents) through the API |
  | Recreate (watchtower-style) keeps data | `up -d --force-recreate` and the same login + the accented category still there |
  | Clean shutdown | `stop -t 60`, then greps `shutdown signal received`, `database pool closed`, `database system is shut down`, `clean shutdown complete`, and asserts container `ExitCode == 0` |
  | V2 stack up + seed (2.3.0 **real**) | the frozen 2.x topology (two containers, image `maxlainz/futurefin:2.3.0`) boots and gets real seeded data |
  | Current image over untouched V2 compose | watchtower case. Up to 3.9.0 this entered external-compat mode; since 4.0.0 the container **must refuse to start**: the job waits for it to be `exited`, asserts `ExitCode != 0`, and greps the logs for `ya no habla con bases de datos externas` and `3.9.0` |
  | Migrate to V3 compose reusing the volume | the real upgrade: greps `adopting ownership of PGDATA` (uid 70 → 999) and `reindexing database after adoption`; same credentials log in; `Ácido Ñandú` intact; **duplicate username must be rejected (409/422)** — the detector for a unique index silently corrupted by the musl→glibc collation change; and a `pre-migration-*.sql.gz` exists in the `ffdata` volume |
  | Leftover `DATABASE_URL` + empty volume (scenario 3) | replaced the automigration scenario in 4.0.0: `docker run` with an external `DATABASE_URL` and two empty volumes must exit non-zero, log `ya no habla con bases de datos externas` **and** `docs/actualizar.md`, and leave the volume **still empty** (`test -z "$(ls -A /d)"`) — no half-initialized cluster |
  | pg_upgrade 15→16 | a PG15 volume seeded with a marker row is handed to the 3.x image: logs `pg_upgrade needed: PostgreSQL 15 -> 16` and `pg_upgrade 15 -> 16 completed`, the marker row survives, `SHOW server_version` starts with 16, `pgdata_old_15/` exists, and a `pre-pgupgrade-15-to-16-*.sql.gz` backup was written |

  Frozen inputs live in `.github/testdata/docker-compose.{v2,v2-app-v3}.yml` (`automigrate.yml`
  was deleted in 4.0.0 with the mode it tested).
  **`docker-compose.v2.yml` must NOT be updated when the production compose evolves** — its
  entire value is being the exact 2.x topology (two services, image pinned to 2.3.0). Treat it
  as a fixture, like `fire-parity.json`.

**CI still does NOT run** — verified absent from `ci.yml` on 2026-08-22:
- **clippy / rustfmt** — present but commented out, on purpose (see the `rust` job above).
- **Any browser E2E / component render.** Vitest runs in `environment: "node"`; nothing drives real
  UI. Layout, both themes and every visual regression stay a manual check.
- **The aborting startup guards** of the container (`pre-migration backup FAILED`, missing role,
  interrupted `pg_upgrade` swap) except the "no volume" one.

**Your pre-merge local list** (now a fast feedback loop, not the only net — the repo's "local CI
first" norm: find out in 3 minutes instead of 15):

```bash
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace                 # engine + integration
npm test --workspace futurefin-web       # Vitest
npm run lint:web                         # eslint
npm run typecheck:web                    # (CI also runs this, run anyway — it's fast)
```

Before tagging a release, additionally run the full local Docker-stack test (`docs/desarrollo.md`
§ "Construir la imagen en local"). Release gates live in
`.claude/skills/futurefin-change-control/SKILL.md`.

**If your change touches `apps/api/docker-entrypoint.sh`, `apps/api/Dockerfile`,
`docker-compose.yml` or the compose fixtures, the `docker-stack` job IS your test suite** —
read its output rather than trusting a green checkmark, and never weaken a step to make it pass
(each assertion above corresponds to a way a user could lose their database). Adding a new
startup path (a new guard, a new migration mode) means adding a step there in the same PR.

## 4. Golden / certified inventory

### `apps/api/tests/fixtures/fire-parity.json` — the canonical cross-language fixture

The FIRE target math is **deliberately duplicated**: the client (`apps/web/src/lib/fire.ts`)
computes a live form preview without a round-trip; the server (`/v1/projection/series`) is the
source of truth. One JSON pins both:

- Backend consumer: `apps/api/tests/fire_parity.rs` — for each case, PATCHes
  `fire_settings` on the installation, seeds an asset + budget entries reproducing `monthly`,
  calls `GET /v1/projection/series`, asserts `jubilacion_target_net_worth` ≈
  `expected_target_nw` ± 1 € (`null` must match `null`).
- Frontend consumer: `apps/web/src/lib/fire.test.ts` — loads the same file via
  `readFileSync` (relative path `../../../api/tests/fixtures/fire-parity.json`), computes
  `grossUpNetAnnualFire(computeFireAnnualNeedNetEur(...)) / (swr/100)`, same ± 1 € tolerance.

Formula pinned (from the fixture's `_formula`): `target_nw = gross_up(annual_need_net,
brackets, taxes_enabled) / (swr_pct / 100)`, where `annual_need_net` depends on
`fire_number_mode` (`manual` / `annual_expense` / `current_income`). 6 cases as of 2026-07-02,
covering all three modes, taxes on/off, multi-bracket gross-up, and the null-target case.

**Discipline:**
- If you change `tax_brackets` defaults, the gross-up formula, or the target contract on
  EITHER side → regenerate every `expected_target_nw` from an independent reference
  (`python3` hand calc or the Rust engine), then **both suites must pass**. One suite failing
  = drift; find which side moved.
- Every case carries a `_calc_note` documenting how its expected value was derived (e.g.
  `"500000 / 0.035 (sin taxes)"`). **Never commit a case without one** — it is the audit trail.
- **Adding a case**: append to `cases[]` with `name`, `fire_settings`, `monthly`
  (`income`/`income_retirement`/`expense_retirement`, all decimal strings),
  `expected_target_nw` (number or `null`), `_calc_note`. Re-run both suites.
- Historical motivation: the client once passed `expense_regular_monthly_equivalent` where
  the server used `expense_retirement_monthly_equivalent` → 2–3× preview divergence, found
  during the v1.3.0 refactor. The fixture exists so that class of drift fails a test.

### `apps/api/tests/fixtures/mcp-catalog.json` + `tool_descriptions_stay_within_the_context_budget` — the MCP context-budget gate

Fase 5 (issue #86, 4.4.0) turned "the catalog fits in context" from a subjective read into two
mechanical gates:

- **The input contract is frozen**: `mcp_http.rs::tools_list_freezes_the_input_contract_of_every_tool`
  pins each tool's `inputSchema` keys, its `required` list, and a hash of its `description` in
  `apps/api/tests/fixtures/mcp-catalog.json`. Regenerate with
  `UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http -- tools_list_freezes_the_input_contract`
  (same `UPDATE_*=1` pattern as `error_codes_parity.rs`) — never hand-edit the JSON.
- **The size is a hard merge gate**: `mcp_http.rs::tool_descriptions_stay_within_the_context_budget`
  asserts `PER_TOOL_MAX = 600` chars per tool description and `TOTAL_BUDGET = 24_000` chars across
  all 52. **When it fails, the fix is never to raise the constant** — the failure message says so
  explicitly. The fix is to move the prose that pushed a description over budget to a **provenance
  field** in the response (the pattern this Fase introduced: `basis`, `source`, `*_absent_reason`,
  `window_truncated` — a field that tells the model *where a number came from* the moment it looks
  at it, instead of a paragraph repeated in every tool's description that might touch that number),
  or to the server's `instructions` (`mcp/server.rs::get_info`) if the fact is true once for the
  whole catalog rather than per-tool. This is exactly how Fase 5 itself got from 37,214 to 21,319
  description characters: 30 warnings came out of individual descriptions and went to one field or
  one `instructions` block each, instead of thirty near-duplicate sentences.
- **Measuring the budget**: see `futurefin-diagnostics-and-tooling` § "MCP catalog context cost"
  for the read-the-fixture one-liner and the live `tools/list` recipe that also weighs
  `inputSchema` (found in the Fase-5 audit to be ~2.7× the descriptions — the next lever if this
  budget is ever tightened further).

### `projection_marker.rs` — the regression-capture exemplar

Deterministic setup (100 k asset at 15 % TAE, +100 €/month net savings) with hand-derivable
expectations: the compound-outpaces-savings marker at `month_index = 1`, 25 points for a
24-month horizon, NW at month 12 in a justified range. Copy this pattern whenever a refactor
must preserve outputs: seed deterministic state → assert exact/current values → refactor →
values must not move.

### History series: server-computed, NOT duplicated on the client (no parity fixture — yet)

Unlike the FIRE target (deliberately duplicated Rust↔TS, held by `fire-parity.json`), the
historical net-worth interpolation lives **only** in `crates/engine/src/history.rs` and the
`GET /v1/history/series` handler. The server returns the series **ready to paint**; the client
(`lib/history-merge.ts`) merely splices those points onto the projection's month-0 vertex and
does **no** interpolation of its own. Consequences for testing:
- The interpolation math is proven by engine unit tests (`history.rs`, exact `Decimal`) plus the
  `history_series.rs` integration tests (predict-then-measure numbers). There is **no**
  cross-language fixture because there is no second implementation to keep in sync.
- **Rule**: if a client-side interpolation preview is ever added (e.g. to redraw the past while a
  snapshot save is in flight), it becomes a duplicated computation → a parity fixture of the
  `fire-parity.json` kind (one canonical JSON both sides consume, ±1 € tolerance) becomes
  **mandatory**, and the D8/§2.5 drift discipline applies to it.

## 5. How to add tests

### Backend integration test

Create `apps/api/tests/my_feature.rs`. Helper names verified against
`apps/api/tests/common/mod.rs` on 2026-07-02:

```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn my_endpoint_does_x() {
    let app = TestApp::spawn().await;                       // fresh schema + router
    let owner = app.register_and_login_owner("alice").await; // first user → owner (bootstrap)

    // arrange via the API, not raw SQL
    let cat = app.create_category(&owner, "asset", "Cash").await; // scope: asset|income|expense

    // act
    let resp = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "EUR", "current_value": "1000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;

    // assert
    assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
    let body = resp.json();
    assert_eq!(body["name"], "EUR");
    let v: f64 = body["current_value"].as_str().unwrap().parse().unwrap();
    assert!((v - 1000.0).abs() < 0.01);                     // NOT assert_eq on the string
}
```

Available on `TestApp` (all in `common/mod.rs`): `spawn()` → `{router, pool, schema, state}`;
`register_and_login_owner(name)` → `LoggedInOwner {username, cookie, user_id}`;
`create_category(&owner, scope, name)` → id string; `count_rows(table)` → i64 (direct DB
check — how `liabilities_purge.rs` proves rows persist); `get`, `get_with_cookie`,
`post_json`, `post_json_with_cookie`, `patch_json_with_cookie`, `delete_with_cookie`, and
(v3.1.0) `post_form`, `post_form_with_basic_auth`, `get_with_headers`, `mcp_initialize` → all
return `ResponseParts {status, headers, body}` with `.json()` and `.session_cookie()`. Since
v3.1.0 `request()` also injects `Host: futurefin.test` when absent — `oneshot` doesn't synthesize
one and the OAuth endpoints derive their issuer from it. Full signatures and rationale:
[`.claude/tests.md`](../../tests.md) §Test infrastructure.
`app.state` exposes `AppState` internals (e.g. `projection_cache`) — see `projection_cache.rs`
for asserting cache keys directly.

**Never sleep to wait for a mutation's effects (3.8.0).** Cache invalidation is now **awaited
inside the handler**, so when a mutation responds the cache state is final: assert it straight
away. The old guidance here said to sleep ~100 ms, and that was actively harmful — under the
`current_thread` runtime every `#[tokio::test]` uses, a `spawn`ed task only runs when the test
yields, and `sleep` was the only place it did. The sleep was not a safety margin; it was the
window that let a *previous* pending invalidation delete the entry right before the assert. Four
different tests were failing intermittently because of it. Use the shared helpers on `TestApp`:
`warm_household`, `cache_contains`, `assert_invalidated`, `household_key`, `installation_id`.

The **one** remaining background task that touches the cache is the post-login warm-up (D7: login
must not wait for the recompute). Any assertion about the cache's **contents or size** must call
`app.settle_login_warmup(iid)` first — it waits for the warm-up to land and clears the cache, so
the test starts from a state nothing can repopulate. It is a bounded wait **for an event**, not a
guessed margin: it returns as soon as the entries appear.

### Engine unit test

Add to `mod tests` in `crates/engine/src/projection.rs`. Use the existing builders
`mk_asset`, `rule_fixed`, `rule_percent`, `rule_remainder`, `base_input` — do not
hand-construct `ProjectionInput`. Assert exact `Decimal` values (pure math, no tolerance
needed) and derive them by hand in a comment first (predict-then-measure). Run:
`cargo test -p futurefin-engine -- <name>`.

### Frontend test

Colocate beside the module: `lib/foo.ts` ↔ `lib/foo.test.ts` (Vitest `include` picks up
`src/**/*.test.{ts,tsx}`). Import `describe/it/expect` from `vitest` (`globals: false`).
Stub network with `vi.stubGlobal("fetch", ...)`-style mocks as in `api/client.test.ts`.

**What NOT to test on the frontend**: component rendering. There is no jsdom/happy-dom
configured — `environment: "node"` only. The config comment says: if component render tests
are ever added, switch to `happy-dom` or `jsdom` in `apps/web/vitest.config.ts`. Until then,
test pure functions only; extract logic out of components to make it testable.

## 6. Coverage gaps — be honest (as of 2026-07-02; container coverage restated 2026-08-16; the CI gap **closed** 2026-08-22)

- **No E2E browser tests.** Nothing drives the real SPA; auth-flow + UI regressions are
  caught only manually. The `docker-stack` job now drives a lot through the **API** (register,
  login, create/read an accented category, duplicate-username rejection) across fresh installs,
  2.x upgrades, the external-DB refusal paths and pg_upgrade — so it proves the server boots *and
  keeps your data*, but it still never loads the UI.
- **Container failure paths are covered only for the happy-ish cases CI exercises.** The guards
  that abort a boot (`pre-migration backup FAILED`, `cannot connect as role …`, an interrupted
  pg_upgrade swap resume) have no automated test; the no-volume guard and the two external-DB
  refusals (§ the docker-stack table) are the exceptions. Treat them as reasoned-but-unproven and read
  `futurefin-debugging-playbook` trap 12 before touching them.
- ~~**Integration tests not in CI.**~~ **CLOSED in 4.0.0** (2026-08-22): the `integration` job runs
  `cargo test --workspace` against a Postgres service, and the `web` job runs ESLint and Vitest. This
  was for a long time the biggest gap in the repo — a PR could go green with every `apps/api/tests/`
  test broken. The row stays, struck through, because "we already ran the tests in CI" is exactly the
  kind of belief that is wrong for months without anybody checking: if you ever wonder again, the
  answer is in `ci.yml`, not here.
- **Unit-test coverage of a module is not implied by the module existing.** Until 2026-08-22 this
  skill asserted unit tests for the CSV presets that did not exist, and a money bug (`2100.00` →
  `210000`) lived in exactly that hole. Before you write "covered by unit tests", `grep -c '#[test]'`
  the file.
- **No property-based tests** on the engine (e.g. invariants like "cascade never allocates
  more than the surplus", "NW series is deterministic under input permutation"). Labeled a
  candidate direction — see `.claude/skills/futurefin-research-frontier/SKILL.md`.
- **No load/performance tests.** The projection-cache tests assert relative hit/miss speed
  only; there is no throughput or memory baseline.

## When NOT to use this skill

- Getting the app or a dev/test environment running from scratch (Docker, `.env`, split-dev):
  `.claude/skills/futurefin-build-and-env/SKILL.md`.
- Measuring live behavior (curl recipes, `scripts/smoke-projection-cache.sh`) and interpreting
  the numbers: `.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md`.
- Deciding whether/how a change may proceed (migrations, releases, gates):
  `.claude/skills/futurefin-change-control/SKILL.md`.
- Understanding the FIRE/retirement math being tested:
  `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- The hunch→accepted-result research discipline (evidence bar, predict-then-run):
  `.claude/skills/futurefin-research-methodology/SKILL.md`.
- Deploy/upgrade/backup smoke tests in production: `.claude/skills/futurefin-run-and-operate/SKILL.md`.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 (`apps/api/Cargo.toml`); `.claude/tests.md` was corrected
the same day (CI claim, migration count, missing `projection_cache.rs` row), and both files were
updated together on 2026-08-16 for **v3.0.0** (`docker-stack` job contents, socket form of
`TEST_DATABASE_URL`, container coverage gaps) and again on **2026-08-17 for v3.1.0** (all counts,
plus the three inventory rows the MCP/OAuth releases had left out). **Rewritten again on 2026-08-22
for 4.0.0**: § 3 (CI now runs integration + ESLint + Vitest), all counts, the two new suite rows
(`account_and_members.rs`, `openapi_contract.rs`) and the retraction of the CSV-preset unit-test
claim. **Extended 2026-08-27 on branch `feat/home-assistant-addon`**: the `TestConfig`/`spawn_with`
harness knobs and the five new suites (`base_path.rs`, `frame_options.rs`,
`session_cookie_path.rs`, `sso_login.rs`, `migration_guard.rs`), verified by reading those files and
`apps/api/tests/common/mod.rs`. **Extended again 2026-08-27 for v4.3.1** (branch
`feat/ha-idp-login`): the outbound-integration house style (fake behind a trait, §Outbound
integrations), the `ha_idp` / `ha_sso_url` knobs of `TestConfig`, the `ha_idp_login.rs` row and the
file/attribute counts — read from `apps/api/tests/{ha_idp_login.rs,common/mod.rs}` and
`apps/api/src/ha_idp/`. **Extended 2026-08-28 for the 4.4.0 Fase 5 work** (issue #86, branch
`feat/mcp-fase-5-contexto`, uncommitted at verification time, `Cargo.toml` still 4.3.1): the new
§ 1 evidence-standard row for context fields, the new `context_fields.rs` suite row (11 tests, all
read from the file), the `mcp_http.rs` (+4) and `mcp_write.rs` (+1) additions, the 1 new
`handlers/person_view.rs` unit test, and the `mcp-catalog.json` / context-budget-gate write-up in
§ 4 — every fact in that pass was read from the diff against `main` and cross-checked against the
running source, not copied from the CHANGELOG's own `[4.4.0]` Fase-5 entry: that entry's "antes
había 12" (descriptions over 600 chars pre-Fase-5) does not match a direct count of `main`'s
source — the real count is 26; see the MCP-catalog subsection of § 4. **This pass did NOT re-sync the rest of
the table**: Fases 2/3 of the same MCP-audit train (issues #83/#92) had already added 13 more
`apps/api/tests/*.rs` files to `main` with no matching rows here, and that drift is called out
but left unresolved (§ "Integration test files") — treat every count in this skill outside the
Fase-5 additions as unverified until recounted. Re-verify volatile facts with:

- Test file inventory: `ls apps/api/tests/` and `ls apps/web/src/lib/*.test.ts apps/web/src/api/*.test.ts`
- Workspace total: `cargo test --workspace 2>&1 | grep "test result"` (**498 on 2026-08-22** — stale, do not trust without recounting; several files were added since)
- **Fase 5 additions (2026-08-28)**: `grep -c '#\[tokio::test\]' apps/api/tests/context_fields.rs`
  (**11**); `git diff main -- apps/api/tests/mcp_http.rs apps/api/tests/mcp_write.rs | grep '^+.*async fn'`
  for the exact new test names; `git diff main -- apps/api/src/handlers/person_view.rs | grep 'fn as_str'`;
  the pre/post description-budget numbers: `grep -n 'PER_TOOL_MAX\|TOTAL_BUDGET' apps/api/tests/mcp_http.rs`
  and the reproducible descriptions-count script in `futurefin-diagnostics-and-tooling` § "MCP
  catalog context cost"
- **HA-IdP seam (4.3.1)**: `grep -n "struct FakeHaIdp\|enum FakeCall\|ha_idp:\|ha_sso_url:" apps/api/tests/common/mod.rs`;
  `grep -c '#\[tokio::test\]' apps/api/tests/ha_idp_login.rs` (**18**);
  `grep -c '#\[test\]' apps/api/src/ha_idp/mod.rs` (**11**) vs
  `grep -c '#\[test\]' apps/api/src/ha_idp/client.rs` (**0**, deliberate);
  no HTTP-mock crate crept in: `grep -rn "wiremock\|mockito\|httpmock" apps/api/Cargo.toml` (empty)
- Engine test count: `cargo test -p futurefin-engine 2>&1 | grep "test result"` (**67 on 2026-08-22** = projection 32 + history 22 + runway 13; it was 61 = 27+21+13 on 2026-08-19)
- Integration attributes: `grep -rc "#\[tokio::test\]\|#\[test\]" apps/api/tests/*.rs | awk -F: '{s+=$2} END {print s}'` (**449 across 44 files on 2026-08-27**; 375 across 33 on 2026-08-22). Lib unit tests: `grep -rn '#\[tokio::test\]\|#\[test\]' apps/api/src | wc -l` (**84 on 2026-08-27**; 72 after 4.3.0, 57 on 2026-08-22)
- Frontend Vitest total — always ask the runner, never count `it(`: `npm test --workspace futurefin-web 2>&1 | grep "Tests "` (**368 in 16 files on 2026-08-22**; `chart-gestures.test.ts` and `fire.test.ts` generate tests in loops, so the static `it(` count is lower)
- Migration count: `ls apps/api/migrations/*.sql | wc -l` (**44 on 2026-08-27**; 42 on 2026-08-22; 40 on 2026-08-19)
- **Reverse-proxy / add-on suites (added 2026-08-27, branch `feat/home-assistant-addon`)**:
  `ls apps/api/tests/{base_path,frame_options,session_cookie_path,sso_login,migration_guard}.rs`;
  harness knobs: `grep -n "struct TestConfig" -A16 apps/api/tests/common/mod.rs` and
  `grep -n "fn spawn_with\|with_frame_policy\|post_with_headers" apps/api/tests/common/mod.rs`;
  the pure-unit half lives in the binary: `cargo test -p futurefin-api --lib prefix::` and
  `cargo test -p futurefin-api --lib spa::` / `sso::` (slug folding, `Cow::Borrowed` shell)
- CI coverage claims: read `.github/workflows/ci.yml` (jobs: `secrets-scan`, `rust`, `web`,
  `integration`, `docker-stack` — `main-guard` was retired with the two-branch model in 4.0.1). `grep -n TEST_DATABASE_URL .github/workflows/ci.yml`
  and `grep -n 'npm test\|lint:web' .github/workflows/ci.yml` must **print something** since 4.0.0 —
  if they ever go silent again, someone removed a gate
- `docker-stack` step list (the § 3 table, one row per step):
  `grep -n "      - name:" .github/workflows/ci.yml`
- Its no-data-loss assertions verbatim:
  `grep -n "no persistent volume\|initializing fresh PostgreSQL 16\|Ácido Ñandú\|adopting ownership\|reindexing database after adoption\|ya no habla con bases de datos externas\|pg_upgrade needed\|pgdata_old_15\|pre-migration-\|pre-pgupgrade-\|clean shutdown complete\|ExitCode" .github/workflows/ci.yml`
- Frozen compose fixtures still frozen (v2 pinned to the 2.x two-service topology):
  `ls .github/testdata/` and `grep -n "image:\|services:" .github/testdata/docker-compose.v2.yml`
- Shellcheck gate over the entrypoint and every shipped script:
  `grep -n "shellcheck" .github/workflows/ci.yml`
- Test DB is still TCP on 5433 **locally**, untouched by the embedded-Postgres image:
  `grep -n "5433" apps/api/tests/common/mod.rs`. In CI it is **5432** (`grep -rn "5433"
  .github/workflows/ci.yml` now prints **one comment line** — the YAML explains that 5433 is the
  *local* documented port and CI uses 5432 because the job's Postgres service has no dev Postgres
  to clash with. This line said "prints nothing" until 2026-08-29; the fact is unchanged, the
  command's promised output was not)
- TestApp helper names: `grep -n "pub async fn\|pub fn" apps/api/tests/common/mod.rs`
- Vitest env: `grep -n environment apps/web/vitest.config.ts` (still `"node"`?)
- Fixture case count: `grep -c '"name"' apps/api/tests/fixtures/fire-parity.json` (7 cases on 2026-08-14; v2.2.0 did **not** touch the fixture)
- Cleanup script existence: `ls scripts/` (clean-test-schemas.sh did NOT exist as of 2026-07-02)
- Default TEST_DATABASE_URL: `grep -n "5433" apps/api/tests/common/mod.rs`
