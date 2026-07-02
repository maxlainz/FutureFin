---
name: futurefin-failure-archaeology
description: >
  The historical chronicle of FutureFin: every major investigation, dead end, rejected approach
  and removal, as symptom → root cause → evidence → status. Load this skill BEFORE proposing to
  (re)introduce any of: age-based retirement trigger / target age, per-asset contribution config,
  deflated ("real") engine simulation, migration auto-repair, GET handlers that delete/purge rows,
  binary-search tax gross-up, warm-up-cache-after-mutation, OAuth login, public pension API,
  ZIP/CSV export, Caddy/TLS overlay, or Decimal-string serialization for large projection arrays.
  Also load it when you hit a symptom that "smells historical": backup export 500s, projection
  numbers that look plausible but shift with inflation toggles, chart deflation wrong only at some
  densities, overlapping table action buttons, FIRE preview diverging from server target, inverted
  SQL binds between household/mine branches. Do NOT use for triaging a live bug step-by-step
  (futurefin-debugging-playbook), for forward-looking improvement ideas
  (futurefin-research-frontier), or for the current invariants themselves
  (futurefin-architecture-contract owns them; this skill owns how they were earned).
---

# FutureFin failure archaeology

Purpose: no future session should re-fight a settled battle or re-introduce a rejected design.
Everything below is mined from `git log` (50 commits, 2026-05-02 → 2026-06-24), `CHANGELOG.md`
(forensic-grade; read it when in doubt), and cross-checked against the code as of **2026-07-02,
v1.4.3, 31 migration files**. Evidence columns cite commit hashes, versions and current file paths
so you can re-verify with `git show <hash>` and `Read <path>`.

Vocabulary used below (defined once):
- **FIRE target / gross-up**: net worth needed to retire = gross annual need / SWR (safe
  withdrawal rate). "Gross-up" converts net annual need to gross, accounting for capital-gains tax
  brackets. **Nominal** = current euros; **real** = deflated to today's purchasing power.
- **Cascade**: ordered allocation rules distributing the monthly surplus ("sobrante") to assets.
- **Installation**: the singleton row all data belongs to. **Scope/view**: `?view=mine` filters
  rows by `owner_user_id`; default `household` is the full installation.

## 1. Settled battles — do not reopen

| # | Rejected / removed | Why | Documented |
|---|---|---|---|
| 1 | `projection_target_age` (age-based retirement trigger) | Caused visual gap: contributions stopped years before the Jubilación marker; FIRE crossover is the sole trigger | 542ecfa, v1.0.6; migration `20260516120000_drop_projection_target_age.sql` |
| 2 | Per-asset contribution config (`monthly_contribution_fixed`, weights, caps on `assets`) | Overlapped badly with reality: fixed sums > surplus, weights >100 %, no explicit priority order | cc23186, v1.1.0 / v1.0.13; `20260519120100_drop_asset_contribution_columns.sql` |
| 3 | Inflation model v1 "real pure" (deflate returns, simulate in today-€) | Half-real/half-nominal mix produced incoherent output (assets drained *before* retirement with inflation on) | v1.0.12 introduced, v1.2.0 (3396725) replaced |
| 4 | Flat FIRE target (fixed scalar) | Toggling inflation barely moved retirement age; target must grow with inflation | v1.2.0, `20260520120000_inflation_always_on.sql` |
| 5 | `projection_includes_inflation` boolean toggle | Redundant: `annual_inflation_assumption_percent = 0` already means "off" | v1.2.0 (API-breaking, dropped from `PATCH /v1/installation`) |
| 6 | GET-side purges (`purge_expired_liabilities` DELETE inside 6 GET handlers) | Reads must not mutate; broke HTTP semantics and caching; data now filtered, kept for audit | 0bba819, v1.3.0; guard: `apps/api/tests/liabilities_purge.rs` |
| 7 | Migration auto-repair loop (`IDEMPOTENT_MIGRATION_REPAIR_VERSIONS`, 12 checksum-repair rounds) | Masked real drift; now fails loud, fixed manually via `DELETE FROM _sqlx_migrations WHERE version = X` | 0bba819, v1.3.0; `apps/api/src/db.rs` |
| 8 | 90-iteration binary-search gross-up | After-tax(gross) is piecewise-linear per bracket → closed form, identical ±0.01 € | 0bba819, v1.3.0; reference kept as test `gross_up_binary_reference` in `handlers/projection.rs` |
| 9 | Hand-written `match view { Household / Mine }` SQL branches | Live bug: inverted bind order between branches in `budget.rs`; helpers enforce placeholder order | 0bba819, v1.3.0; `apps/api/src/handlers/person_view.rs` |
| 10 | Projection-cache warm-up after mutation | Race: two concurrent warm-ups could leave the cache stale; warm-up runs after login only, mutations only invalidate | b65acf6, v1.4.0 (CHANGELOG §Warm-up post-login) |
| 11 | Chart deflation by array index | Wrong with `?density=hybrid` (non-equidistant points); must use `month_index` | 669307d, v1.4.2 |
| 12 | OAuth login, `fire.rs`/`persons.rs` handler suite, engine `fire.rs` | Legacy pre-1.0 scope cut; username+password (Argon2id) is the auth model | d123105 (2026-05-03), `20260506120000_installation_drop_fire_settings.sql` |
| 13 | Public pension API (`users.pension_*` columns) | Superseded by "persists after retirement" income toggle (v1.0.3) | 4a8e2af, ee24867; `20260515120000_drop_users_pension_columns.sql` |
| 14 | ZIP/CSV export (`GET /v1/backup/export.zip`) | Replaced by encrypted per-user `.ffbackup` (AES-256-GCM, Argon2id-derived key) | 660a8ec, v1.0.9; routes in `apps/api/src/routes/mod.rs` |
| 15 | Caddy TLS overlay + compose-watch dev flow | Deploy simplified to a single `docker-compose.yml`; only `POSTGRES_PASSWORD` required | 5cc0914, 71a877d, v1.0.1 |
| 16 | `households`/`persons` as product concepts | Renamed/collapsed into the `installation` singleton; `persons` later dropped with legacy FIRE | migrations `20260203…households.sql` → `20260207…installation_remove_household.sql`; d123105 |
| 17 | Docker healthcheck `CMD` exec form | `curl` not on exec PATH → always unhealthy; use `CMD-SHELL` (+ `/dev/tcp` fallback) | d0bb259, v1.0.2 |
| 18 | `fire_number_expense_adjustment_pct`, `bump_contributed_series_with_purchase_basis` | Zombie code with no consumer / obsolete binary-compat patch | 0bba819, v1.3.0 |

## 2. Detailed entries

### 2.1 Backup export 500 — queries drift from schema (v1.0.10)
- **Symptom**: `POST` backup export returned 500 after upgrading.
- **Root cause**: export SQL still selected `b.label` and `b.frequency` from `budget_entries`,
  columns dropped by migration `20260505180000_budget_entries_monthly_only.sql` (budget became
  monthly-only). Raw SQL strings are not checked at compile time against live schema.
- **Fix**: bd8440d — export/import omit both fields; `BackupBudgetEntry` schema updated.
- **Status**: settled. **Guard**: none automatic for raw SQL — when a migration drops a column,
  grep every handler for the column name (`grep -rn '<column>' apps/api/src/handlers/`).
  Integration tests (`apps/api/tests/`) now exist and would catch this class if the endpoint is
  covered; they run only locally with `TEST_DATABASE_URL` (NOT in CI — `.claude/tests.md` is
  stale on this point).

### 2.2 projection_target_age removal — FIRE is the sole retirement trigger (v1.0.6)
- **Symptom**: "contributed capital" line on the projection chart stopped growing years before
  the Jubilación milestone marker — a visual gap users read as a bug.
- **Root cause**: two competing retirement triggers (manual target age vs FIRE crossover) could
  disagree; the engine entered retirement (stopping contributions) at the age trigger while the
  chart marker showed the FIRE crossover.
- **Fix**: 542ecfa — column dropped entirely; FIRE crossover is the only trigger. Horizon became
  a fixed 90-year lifespan from the oldest household member's birth date (clamped 5–70 years,
  30-year fallback without birth date).
- **Alternative rejected**: keeping both triggers and reconciling — inherently ambiguous.
- **Status**: settled. **Warning**: `.claude/data-model.md` and parts of `.claude/engine.md`
  still describe the old field — verify against `apps/api/src/handlers/projection.rs` instead.

### 2.3 The table-CSS saga — three wrong fixes before the root cause (v1.0.18 → v1.0.20)
- **Symptom**: edit/delete action buttons visually overlapped the previous column's content
  (Importe mensual in Ingresos was fully hidden).
- **Investigation**: v1.0.18 changed `display: flex` → `inline-flex` + padding + background
  (insufficient). v1.0.19 added `position: sticky; right: 0` + `::before` shadow (still wrong).
- **Root cause (v1.0.20)**: `.budget-row-actions { display: inline-flex }` was applied
  **directly to the `<td>`**, overriding `display: table-cell` and ejecting the cell from the
  table layout model — the browser rendered it outside its column.
- **Fix**: wrap the buttons in an inner `<div className="budget-row-actions">`; the `<td>` keeps
  only `.asset-actions-cell` with default display. v1.0.18/19 hacks reverted. Applied to 6 tables.
- **Lesson (owner-endorsed)**: find the root cause before patching symptoms. Two "fixes" that
  each seemed plausible shipped broken because nobody asked *why* the cell escaped its column.
- **Status**: settled. Never set a non-table `display` on `<td>`/`<tr>` elements.

### 2.4 FIRE off-by-one between engine and handler (fixed v1.3.0)
- **Symptom**: FIRE crossover month from the engine could differ by one month from the
  `fire_target_series` the handler built for the chart.
- **Root cause**: the moving-target formula was duplicated — engine used `years=(k-1)/12`, the
  handler used `years=month_index/12` — so the two curves disagreed at boundaries.
- **Fix**: single public helper `fire_target_at_month_index` in
  `crates/engine/src/projection.rs` (doc comment: "única fuente de verdad"); the engine calls it
  with `k-1`, the handler with the point's `month_index`. Both consume the same function.
- **Status**: settled. **Guard**: engine unit tests around the helper
  (`cargo test -p futurefin-engine`); never re-inline the formula `base × (1+inf/100)^(years)`.

### 2.5 RetirementView FIRE preview 2–3× off (found during v1.3.0 App.tsx split)
- **Symptom**: FIRE target preview in the Jubilación form could diverge 2–3× from the server's
  target when the user had expenses marked `ends_at_retirement = true`.
- **Root cause**: `RetirementView` passed `expense_regular_monthly_equivalent` into the FIRE
  calculation while the server used `expense_retirement_monthly_equivalent`. Silent — both
  numbers looked plausible.
- **Fix**: corrected in all 4 call sites; verified today in
  `apps/web/src/views/RetirementView.tsx` (uses `expense_retirement_monthly_equivalent`).
- **Status**: settled. **Guard**: shared fixture `apps/api/tests/fixtures/fire-parity.json`,
  consumed by BOTH `apps/api/tests/fire_parity.rs` and `apps/web/src/lib/fire.test.ts` — the
  FIRE math is deliberately duplicated client/server, and this fixture is the tripwire. If tax
  brackets or gross-up change, regenerate expected values; both suites must pass.

### 2.6 Inverted binds between Household/Mine branches (live bug fixed v1.3.0)
- **Symptom**: subtle wrong data under one view; found in `budget.rs` — the derived-from-
  liabilities query had placeholder order differing between the `Household` and `Mine` branches.
- **Root cause**: 6 handlers each hand-wrote `match view { Household => "WHERE installation_id
  = $1", Mine => "… AND owner_user_id = $2" }` plus separate bind chains; nothing kept the two
  branches in sync.
- **Fix**: `LedgerView::scope_where(alias)`, `next_arg_index()`, `bind_scope_as`,
  `bind_scope_scalar` in `apps/api/src/handlers/person_view.rs` (~500 LOC removed).
- **Status**: settled; using the helpers is a CLAUDE.md non-negotiable. Never hand-write the two
  branches again — the helper eliminates the entire bug class, not one instance.

### 2.7 Warm-up-after-mutation rejected — cache invalidates only (v1.4.0)
- **Context**: in-memory projection cache in `AppState` (`apps/api/src/state.rs`): sliding
  60-min TTL, keyed (installation, view, owner, density).
- **Rejected design**: recompute-and-store (warm-up) right after each mutation.
- **Failure mode**: two concurrent mutations → two concurrent warm-ups; the one computed from
  older data can finish last and overwrite the newer result → cache permanently stale until TTL.
- **Settled design**: mutations call `refresh_projection_after_mutation` which only **deletes**
  entries (8 handler files call it — assets, liabilities, budget, planning, allocation_rules,
  installation, auth, projection); next GET recomputes once. Warm-up
  (`warm_up_household_projection`) runs ONLY after login (`handlers/auth.rs`), where concurrency
  is per-user and harmless. **Guard**: `apps/api/tests/projection_cache.rs` +
  `scripts/smoke-projection-cache.sh`. Do not "optimize" by re-adding post-mutation warm-up.

### 2.8 Hybrid-density deflation bug — decimated series break index math (v1.4.2)
- **Symptom**: with the "Inflation Adjusted" toggle on, the chart under-deflated from month 12
  onward — but only briefly, until the full `monthly` series arrived (two-phase loading), making
  it look like flicker rather than a math bug. Invisible at `monthly` density.
- **Root cause**: `ProjectionNetWorthChart` deflated each point by its **array index** instead
  of its `month_index`. `?density=hybrid` serves ~82 non-equidistant points (months 0–12 monthly,
  then annual), so array index ≠ elapsed months.
- **Fix**: 669307d — deflator takes `p.month_index` (see `apps/web/src/views/
  ProjectionNetWorthChart.tsx`, `deflator(monthIndex)`). Same release added backend
  `milestones_real` (milestones crossed on the deflated series, computed by
  `deflate_points_to_today` at full monthly resolution so hybrid decimation loses no precision).
  Retirement crossing is inflation-invariant, so it needs no real variant.
- **Status**: settled. **Rule**: any math over projection points must use `month_index`, never
  the array position; anything needing month-precision crossings computes on the FULL series
  server-side, not the decimated one.

### 2.9 The f64 wire decision — deliberate, bounded exception (v1.4.0)
- **Context**: "Money is Decimal-as-string everywhere" is a non-negotiable. v1.4.0 made ONE
  scoped exception: large projection arrays (`points[].net_worth`, `points[].contributed_capital`,
  `fire_target_series`, `asset_series[].values`) serialize as `f64`
  (`serialize_decimal_as_f64` in `handlers/projection.rs`).
- **Why**: ~30 KB smaller JSON and ~5,000 fewer `parseDisplayDecimal` calls per load; f64 keeps
  ~15 significant digits — error <1 € over a 70-year series, and these values are chart-only.
- **Boundary**: scalars and KPIs (`starting_net_worth`, `jubilacion_target_net_worth`,
  milestones) stay Decimal-as-string. Engine internals remain pure `Decimal` — the cast happens
  only at serialization.
- **Status**: settled. Do not "fix the inconsistency" in either direction: neither convert the
  arrays back to strings, nor extend f64 to scalars/KPIs or to any engine/DB code.

### 2.10 Legacy purges: OAuth, FIRE v0, persons, public pension (May 2026, pre-1.0 → 1.0.x)
- d123105 removed in one sweep: `auth/oauth.rs` (OAuth login), `handlers/fire.rs` (317 LOC) +
  engine `fire.rs` (203 LOC) (a first-generation FIRE API superseded by `fire_settings` JSONB on
  `installation`), `handlers/persons.rs` (391 LOC), and installation `fire_settings` v0 columns
  (`20260506120000_installation_drop_fire_settings.sql`).
- ee24867 then rebuilt FIRE/pension as installation-level settings; 4a8e2af removed the public
  pension API and `users.pension_*` columns after v1.0.3 replaced pensions with the
  `persists_after_retirement` toggle on income budget entries (simpler and more general).
- Early-migration archaeology: `20260203…households.sql` / `20260204…persons.sql` created a
  households/persons model; `20260207…installation_remove_household.sql` renamed it into the
  installation singleton. These tables' names in old migrations are NOT evidence the concepts
  exist — always check the final schema, not migration N of 31.
- **Status**: settled. If you want multi-tenant/households or pension modeling, that's a
  research-frontier topic, not a restoration job.

## 3. Designs that were tried and replaced

| Old design | Specific failure mode | Replacement (current) |
|---|---|---|
| **Inflation model v1 "real pure"** (v1.0.12): deflate each asset's return (`r_real = (1+r_nom)/(1+inf) − 1`), simulate everything in today-€ | Predecessor mixed deflation/inflation inconsistently; v1 itself, combined with a FLAT target, made inflation toggling nearly a no-op on retirement age, and pre-v1 produced asset drain before retirement | **v2 nominal + moving target** (v1.2.0, current): everything simulated in nominal €; ONLY the FIRE target grows: `target(month k) = base × (1+inf/100)^((k−1)/12)` via `fire_target_at_month_index`. Deflation is a display-layer concern (`milestones_real`, chart toggle) |
| **Per-asset contributions** (`monthly_contribution_fixed` + `contribution_remainder_weight` + per-asset cap, v1.0.11) | Sum of fixed contributions could exceed surplus; weights confusing when >100 %; no explicit priority; cap-overflow redistribution needed ad-hoc fallback rules (v1.0.11's "highest-return liquid asset" patch was a symptom) | **Allocation cascade** (v1.1.0): ordered rules `fixed`/`percent`/`remainder` with optional caps (`amount`, `months_expense`, `income_multiple`); exactly one uncapped `remainder` sink, always last (server-enforced: `remainder_required`, `uncapped_remainder_exists`, `sink_must_be_last`). Clean column drop, NO data migration — owner signed off losing config; backup schema_version bumped to 3 |
| **Migration auto-repair** (12-round checksum repair loop) | Masked genuine drift between shipped migration files and applied checksums; a silently "repaired" DB can diverge from what migrations say | Fail-loud `sqlx::migrate!().run()` (`apps/api/src/db.rs`); manual `DELETE FROM _sqlx_migrations WHERE version = X` only when the change is genuinely idempotent |
| **GET-side purges** (`purge_expired_liabilities` called from 6 GET handlers) | GETs issued DELETEs: violates HTTP semantics, breaks caching (v1.4.0's cache would have been impossible), destroys audit data | `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)` filter in liabilities/summary/budget/assets/projection reads; rows persist |
| **Binary-search gross-up** (90 iterations to invert after-tax) | 90× slower than needed and convergence-threshold noise; obscured that the function is piecewise-linear | Closed-form per-bracket inversion `gross_up_net_annual_fire` (`handlers/projection.rs`); old binary search preserved as the test oracle `gross_up_binary_reference` |

## 4. "If you are tempted to X, read Y first"

| Temptation | Read first |
|---|---|
| Add a retirement age / target-age setting | §2.2 (and note `.claude/data-model.md` is stale here) |
| Put contribution config back on assets, or "simplify" the cascade | §3 row 2; engine cascade tests in `crates/engine/src/projection.rs` |
| Deflate returns / simulate in real terms inside the engine | §3 row 1 — display-layer deflation only |
| Make a GET delete/clean anything | §3 row 4; `apps/api/tests/liabilities_purge.rs` |
| Auto-fix a migration checksum mismatch in code | §3 row 3; CLAUDE.md Migrations |
| Warm the projection cache after a mutation | §2.7 |
| Compute anything from a projection point's array position | §2.8 |
| Send projection arrays as Decimal strings, or use f64 anywhere else | §2.9 |
| Duplicate the FIRE target formula, or edit FIRE math on one side only | §2.4, §2.5; regenerate `fire-parity.json` expectations, run both suites |
| Hand-write household/mine SQL branches | §2.6; use `LedgerView` helpers |
| Apply `display: flex`/`inline-flex` to a `<td>` | §2.3 |
| Drop a column in a migration | §2.1 — grep handlers for the column first; §3 row 2 for the data-loss sign-off precedent |
| Re-add OAuth, pensions API, persons, ZIP export, Caddy | §2.10, table rows 12–16 |
| Trust an old migration file as evidence of current schema | §2.10 last bullet |

## 5. When NOT to use this skill

- **Triaging a live bug right now** (reproduce → isolate → fix): use
  `.claude/skills/futurefin-debugging-playbook/SKILL.md` — it owns symptom→triage tables. Come
  back here only to check whether your suspect design was already tried and rejected.
- **Forward-looking ideas** (stochastic returns, tax-aware withdrawal, variable SWR — all
  currently unimplemented): `.claude/skills/futurefin-research-frontier/SKILL.md`.
- **Current invariants and why the architecture is shaped this way**:
  `.claude/skills/futurefin-architecture-contract/SKILL.md` (this skill owns the incidents that
  *produced* the invariants, not their normative statement).
- **How to classify/gate a change, migration and release discipline**:
  `.claude/skills/futurefin-change-control/SKILL.md`. Nothing in this chronicle authorizes
  bypassing its gates.
- **The FIRE math itself** (SWR, gross-up mechanics, cascade semantics as implemented):
  `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.

## 6. Provenance and maintenance

Compiled 2026-07-02 at v1.4.3 from `git log` (all 50 commits), `CHANGELOG.md` (complete read),
and direct code inspection. Re-verify volatile facts before relying on them:

- Commit hashes and dates: `git log --oneline` (50 commits as of 2026-07-02; new work lands on `dev`).
- Version: `grep '^version' apps/api/Cargo.toml` (1.4.3) + top of `CHANGELOG.md`.
- Migration count and drop-migrations: `ls apps/api/migrations/ | wc -l` (31 as of 2026-07-02);
  `ls apps/api/migrations/ | grep -i drop`.
- FIRE helper is still the single source: `grep -rn 'fire_target_at_month_index' crates/ apps/api/src/`.
- Scope helpers still used: `grep -n 'scope_where' apps/api/src/handlers/person_view.rs`.
- No auto-repair regression: `grep -n 'repair' apps/api/src/db.rs` (expect no hits).
- Cache invalidation call sites: `grep -rln 'refresh_projection_after_mutation' apps/api/src/handlers/`.
- Chart deflation by month_index: `grep -n 'deflator' apps/web/src/views/ProjectionNetWorthChart.tsx`.
- Parity fixture pair: `ls apps/api/tests/fixtures/fire-parity.json apps/web/src/lib/fire.test.ts`.
- f64 boundary: `grep -n 'serialize_decimal_as_f64' apps/api/src/handlers/projection.rs`.
- RetirementView field: `grep -c 'expense_retirement_monthly_equivalent' apps/web/src/views/RetirementView.tsx` (expect ≥4).
- Known stale docs (do not propagate): `.claude/data-model.md` / `.claude/engine.md` still mention
  `projection_target_age`; `.claude/tests.md` claims "no CI" (CI exists at `.github/workflows/ci.yml`
  but does NOT run `apps/api/tests/` Postgres integration tests) and says "33 migrations" (count them).
- When a new incident is settled (root cause found, design rejected or removed), append it here:
  one table row in §1 plus a ≤15-line entry in §2 if it carries a lesson, citing commit + version.
