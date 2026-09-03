---
name: futurefin-failure-archaeology
description: >
  The historical chronicle of FutureFin: every major investigation, dead end, rejected approach
  and removal, as symptom → root cause → evidence → status. Load this skill BEFORE proposing to
  (re)introduce any of: age-based retirement trigger / target age, per-asset contribution config,
  deflated ("real") engine simulation, migration auto-repair, GET handlers that delete/purge rows,
  binary-search tax gross-up, warm-up-cache-after-mutation, OAuth login, public pension API,
  ZIP/CSV export, Caddy/TLS overlay, Decimal-string serialization for large projection arrays, or
  any undo of the 3.0.0 self-contained image (Postgres back as its own compose service, a
  `postgres:*` runtime base, a `VOLUME` in the Dockerfile, SIGTERM to the postmaster, a `/dev/tcp`
  healthcheck fallback), of the 4.0.0 removal of the external-database mode (making the
  container honour `DATABASE_URL` again, re-adding the one-shot automigration), a `density`
  parameter on the `get_projection` MCP tool, or renaming a field to disambiguate it from a
  same-named field elsewhere instead of declaring its basis/mode.
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
v1.4.3, 31 migration files**; §1 row 19 and §2.11 were added on **2026-08-16 for v3.0.0**
(34 migration files, self-contained Docker image). Evidence columns cite commit hashes, versions
and current file paths so you can re-verify with `git show <hash>` and `Read <path>`.

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
| 1 | `projection_target_age` (age-based retirement trigger) | Caused visual gap: contributions stopped years before the Jubilación marker; FIRE crossover was the sole trigger for nine versions | 542ecfa, v1.0.6; migration `20260516120000_drop_projection_target_age.sql`. **READMITIDO CON ALCANCE en 5.0.0 — lee la scope note de §2.2 antes de tocarlo**: la edad vuelve como trigger de DOS estrategias, no como campo suelto |
| 2 | Per-asset contribution config (`monthly_contribution_fixed`, weights, caps on `assets`) | Overlapped badly with reality: fixed sums > surplus, weights >100 %, no explicit priority order | cc23186, v1.1.0 / v1.0.13; `20260519120100_drop_asset_contribution_columns.sql` |
| 3 | Inflation model v1 "real pure" (deflate returns, simulate in today-€) | Half-real/half-nominal mix produced incoherent output (assets drained *before* retirement with inflation on) | v1.0.12 introduced, v1.2.0 (3396725) replaced. **Reafirmado en 4.4.0 (Fase 6, issue #87)**: servir `points[].net_worth_real` y `GET /v1/projection/deflate` **no** reabre esto — el motor sigue simulando 100 % en nominal y el deflactado es capa de presentación. La forma testable de esa frase: `net_worth_real == net_worth / (1+i)^(month_index/12)`, o sea cero información que el motor no haya producido ya (`apps/api/tests/projection_deflation.rs`) |
| 4 | Flat FIRE target (fixed scalar) | Toggling inflation barely moved retirement age; target must grow with inflation | v1.2.0, `20260520120000_inflation_always_on.sql` |
| 5 | `projection_includes_inflation` boolean toggle | Redundant: `annual_inflation_assumption_percent = 0` already means "off" | v1.2.0 (API-breaking, dropped from `PATCH /v1/installation`) |
| 6 | GET-side purges (`purge_expired_liabilities` DELETE inside 6 GET handlers) | Reads must not mutate; broke HTTP semantics and caching; data now filtered, kept for audit | 0bba819, v1.3.0; guard: `apps/api/tests/liabilities_purge.rs` |
| 7 | Migration auto-repair loop (`IDEMPOTENT_MIGRATION_REPAIR_VERSIONS`, 12 checksum-repair rounds) | Masked real drift; now fails loud, fixed manually via `DELETE FROM _sqlx_migrations WHERE version = X` | 0bba819, v1.3.0; `apps/api/src/db.rs` |
| 8 | 90-iteration binary-search gross-up | After-tax(gross) is piecewise-linear per bracket → closed form, identical ±0.01 € | 0bba819, v1.3.0; reference kept as test `gross_up_binary_reference` in `handlers/projection.rs` |
| 9 | Hand-written `match view { Household / Mine }` SQL branches | Live bug: inverted bind order between branches in `budget.rs`; helpers enforce placeholder order | 0bba819, v1.3.0; `apps/api/src/handlers/person_view.rs` |
| 10 | Projection-cache warm-up after mutation | Race: two concurrent warm-ups could leave the cache stale; warm-up runs after login only, mutations only invalidate | b65acf6, v1.4.0 (CHANGELOG §Warm-up post-login) |
| 11 | Chart deflation by array index | Wrong with `?density=hybrid` (non-equidistant points); must use `month_index` | 669307d, v1.4.2 |
| 12 | OAuth **login** (FutureFin as *client* of an external IdP), `fire.rs`/`persons.rs` handler suite, engine `fire.rs` | Legacy pre-1.0 scope cut; username+password (Argon2id) is the auth model. **Distinct from v3.1.0's OAuth**: there FutureFin is the *authorization server* delegating MCP access after password login — that is adopted (architecture-contract D15), the login-with-IdP idea stays rejected — **except the narrow HA-only readmission of v4.3.1** («Entrar con Home Assistant», see §2.10 second scope note + architecture-contract D19: HA as identity source only, token revoked at once, add-on-only; generic IdP login remains rejected) | d123105 (2026-05-03), `20260506120000_installation_drop_fire_settings.sql` |
| 13 | Public pension API (`users.pension_*` columns) | Superseded by "persists after retirement" income toggle (v1.0.3) | 4a8e2af, ee24867; `20260515120000_drop_users_pension_columns.sql`. **READMITIDA CON ALCANCE en 5.0.0**: la pensión vuelve al usuario, pero como bloque del perfil de jubilación **con FECHA** — y la fecha cambia el objetivo, que es justo lo que la columna muerta nunca hizo. Scope note en §2.10 |
| 14 | ZIP/CSV export (`GET /v1/backup/export.zip`) | Replaced by encrypted per-user `.ffbackup` (AES-256-GCM, Argon2id-derived key) | 660a8ec, v1.0.9; routes in `apps/api/src/routes/mod.rs` |
| 15 | Caddy TLS overlay + compose-watch dev flow | Deploy simplified to a single `docker-compose.yml`; only `POSTGRES_PASSWORD` required | 5cc0914, 71a877d, v1.0.1 |
| 16 | `households`/`persons` as product concepts | Renamed/collapsed into the `installation` singleton; `persons` later dropped with legacy FIRE | migrations `20260203…households.sql` → `20260207…installation_remove_household.sql`; d123105 |
| 17 | Docker healthcheck `CMD` exec form | `curl` not on exec PATH → always unhealthy; use `CMD-SHELL` (+ `/dev/tcp` fallback) | d0bb259, v1.0.2 |
| 18 | `fire_number_expense_adjustment_pct`, `bump_contributed_series_with_purchase_basis` | Zombie code with no consumer / obsolete binary-compat patch | 0bba819, v1.3.0 |
| 19 | Postgres as a separate compose service (`futurefin-database`, `postgres:16.4-alpine`) | Two moving parts, an externally-managed `POSTGRES_PASSWORD` and no snapshot before migrations, in an app whose stated axis is "upgrades that never lose data". Replaced in 3.0.0 by PostgreSQL **embedded in the image** (one container, socket-only). Five traps found doing it — read §2.11 before touching the image | 5ca91f4, v3.0.0; `apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh`, `docker-compose.yml` (one service), CI job `docker-stack` |
| 20 | **External-database mode** in the container (`DATABASE_URL` → `exec_api_external`, the `DEPRECATED` banner, the one-shot `automigrate_prepare`/`automigrate_restore`, `FUTUREFIN_DB_MODE=external`, `FUTUREFIN_EXTERNAL_WAIT_SECS`) | It was the one supported topology with **none** of the guarantees 3.0.0 was built to give: no pre-migration backup, no `pg_upgrade`, no ordered postmaster shutdown, no volume guard. Deprecated in 3.0.0 and announced there (README §«Actualizar desde 2.x» + env table, `.env.example`, and the start-up banner itself: «se eliminara en 4.0.0»); removed on schedule in 4.0.0. `DATABASE_URL` itself is untouched and still required in split-dev — what is gone is the *container* ever honouring it. Read §2.12 before proposing anything that talks to a database outside the image | v4.0.0; `apps/api/docker-entrypoint.sh` (`refuse_external_database` is all that remains), `.github/testdata/docker-compose.automigrate.yml` deleted, CI `docker-stack` scenarios 2b and 3 |
| 21 | Binding the Streamable HTTP `Mcp-Session-Id` to the Bearer credential | Not a removal — a **deliberate non-addition**, reasoned in full in the code so it doesn't get re-proposed as an obvious hardening. Today it buys nothing: the Bearer middleware runs before the MCP protocol on *every* request, identity is re-resolved live (D14), and a stolen session id without a valid token never gets past 401. It also has a named trigger to revisit — see §2.17 | v4.4.0; `apps/api/src/mcp/mod.rs` module doc-comment |
| 22 | Exposing `density` as a `get_projection` MCP-tool parameter | Would multiply the payload ~5× (`density=monthly`) and still would not say WHY the curve fell, only WHERE — the fix for "why" is naming the events that moved it, not serving more points | Fase 5, issue #86, v4.4.0; `apps/api/src/handlers/projection.rs` (`ProjectionEvent` doc-comment) — see §2.18 |
| 23 | Renaming the four homonymous fields shared by `get_budget.totals` and `get_summary.financial_health` | Same names describe genuinely different bases (plan vs plan-or-actual, by `savings_source` mode) on purpose; renaming is breaking over six fields the SPA and both MCP surfaces read by name, and it would not have made either number more legible — what was missing was declaring the base, not renaming the field | Fase 5, issue #86, v4.4.0; `apps/api/src/handlers/budget.rs` (`BudgetTotalsResponse::basis` doc-comment) — see §2.19 |

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
  covered; since 4.0.0 they also run in CI (job `integration`).

### 2.2 projection_target_age removal — FIRE is the sole retirement trigger (v1.0.6)
- **Symptom**: "contributed capital" line on the projection chart stopped growing years before
  the Jubilación milestone marker — a visual gap users read as a bug.
- **Root cause**: two competing retirement triggers (manual target age vs FIRE crossover) could
  disagree; the engine entered retirement (stopping contributions) at the age trigger while the
  chart marker showed the FIRE crossover.
- **Fix**: 542ecfa — column dropped entirely; FIRE crossover is the only trigger. Horizon became
  a fixed 90-year lifespan from ONE resolved birth date: the session user's `users.birth_date`,
  else the first `persons` row ordered `is_primary DESC, sort_index ASC` — NOT the oldest member
  (clamped 5–70 years, 30-year fallback without any birth date).
- **Alternative rejected**: keeping both triggers and reconciling — inherently ambiguous.
- **Status**: settled **hasta 5.0.0**, cuando la edad se readmite deliberadamente. Ver la scope note.
  (The docs that still described the old field — data-model.md, engine.md, api-routes.md — were
  fixed on 2026-07-02; `apps/api/src/handlers/projection.rs` is ground truth.)
- **Scope note (añadida 2026-09-03, 5.0.0 · issue #207, decisiones D2/D17 del owner)** — la edad
  vuelve, y hay que decir EXACTAMENTE qué vuelve y qué sigue muerto:
  - **Qué vuelve**: `target_retirement_age` en `users.retirement_profile`, y es el **trigger de dos
    estrategias** (`retire_at_age`, `coast`) — más el fin opcional de la fase parcial. No es un
    ajuste independiente que conviva con el cruce: es la elección de estrategia del usuario.
  - **Qué sigue muerto, y es lo que v1.0.6 mató de verdad**: **la coexistencia ambigua de dos
    disparadores**. El invariante es **un solo trigger por simulación**, y lo hace cumplir el
    handler pasando `PhasePlan::crossing_is_reading_only = true`: con una estrategia por edad el
    cruce se sigue evaluando, se sigue publicando (`liquid_crossing_month_index`) y **no jubila**.
  - **Por qué un flag y no `fire_target: None`** (la vía que WP1b anticipaba): las estrategias por
    edad SIGUEN necesitando el objetivo — el chart lo pinta y el rojo de infra-financiado se mide
    contra él. Quitar el objetivo habría desactivado el trigger tirando también la lectura, que es
    el error simétrico del de 2026-05.
  - **El bug visual de entonces no puede volver por construcción**: aquel «hueco» era el marcador
    (cruce) discrepando de dónde paraban las aportaciones (edad). Hoy el marcador ES
    `retirement_month_index`, el mes efectivo, y el invariante testable —el mes en que el ingreso
    cambia a jubilación == `jubilacion_month_index` == el marcador == el primer mes de `Retired`—
    se comprueba **sobre la serie, no sobre el enum**
    (`crates/engine/tests/golden_pins.rs::the_phase_readings_agree_with_the_series_they_describe`).
  - **Salvaguarda de datos**: sin `users.birth_date` las estrategias por edad **degradan a `asap`**
    con `warnings: ["birth_date_missing"]`. Nunca un 500, nunca una edad inventada.
  - Si vas a proponer un trigger por edad **fuera** de una estrategia (un ajuste global, un segundo
    disparador que conviva con el cruce), estás proponiendo lo que v1.0.6 mató: vuelve a leer esta
    entrada entera.

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
- **Scope note (añadida 2026-09-03, 5.0.0 · issue #207, decisiones D11/D12 del owner)** — hay una
  **segunda** excepción sancionada, y llega con más salvaguardas que la primera:
  - **Dónde**: el crate nuevo `crates/engine-stochastic`, y solo ahí. Implementa `MoneyOps` sobre un
    newtype `F64Money` y con él **instancia el bucle de `futurefin-engine`** — no lo reimplementa.
    Un segundo bucle en coma flotante era la alternativa obvia y es exactamente la familia de fallos
    que esta casa tiene fichada: dos bucles divergen en silencio al primer cambio de modelo.
  - **La regla, y es dura: de ese crate NO sale un euro.** Lo que publica son magnitudes
    ESTADÍSTICAS (probabilidad de éxito, percentiles de una banda, probabilidad de agotamiento por
    edad), donde un error relativo de 1e-15 no cambia ninguna decisión. Todo importe en euros —
    patrimonio, objetivo FIRE, aportación necesaria— sale del camino `Decimal`.
  - **El freezer NO se ha tocado.** `crates_engine_src_has_no_f64_outside_comments`
    (`crates/engine/src/lib.rs`) sigue sin una sola excepción; lo que hizo posible el crate aparte
    es la **regla del huérfano**: el trait es público, así que otro crate puede implementarlo sobre
    su propio tipo sin que `crates/engine` conozca la coma flotante. `crates/engine` sigue además
    **sin RNG**: `rand_chacha` vive en el crate estocástico.
  - **La puerta que lo sostiene**: `crates/engine-stochastic/tests/degeneration.rs` compara los dos
    caminos sobre **todos** los casos de la batería —`net_worth` y `liquid_worth` mes a mes en todo
    el horizonte, y exactas las decisiones DISCRETAS (mes de jubilación, cruce, agotamiento,
    transiciones de fase)— con una cota de contrato de **1 € por mes** (máximo medido: 1,47e-7 € en
    P9 a 840 meses). Ningún caso se excluye; la única cota relativa es para los casos sintéticos por
    encima de `2^53 €`, donde el propio espaciado de los `f64` ya supera el euro, **y esos casos van
    marcados en la tabla que el test imprime**.
  - **Cada política del tipo va DECLARADA** en el doc-comment de `F64Money`: qué devuelve `total_cmp`
    con `NaN`, cuándo se rinden los `checked_*` (⟺ el resultado no es finito), cuánta precisión
    pierde `from_decimal`, y la única igualdad con tolerancia del núcleo (`gains_equal`, 1e-12) —
    aparte a propósito, porque `PartialEq` para `F64Money` sigue siendo la igualdad exacta.
  - Lo que **sigue prohibido**: `f64` en `crates/engine`, en `crates/domain`, en la BD, o en
    cualquier KPI en euros. Si te encuentras queriendo pasar un `f64` de vuelta al camino exacto,
    para y lee `futurefin-proof-and-analysis-toolkit` Recipe 5.

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
- **Scope note (added 2026-08-17, v3.1.0)**: the removed `auth/oauth.rs` was OAuth as a **login**
  mechanism — FutureFin acting as OAuth *client* of an external IdP so users could sign in with
  it. That rejection stands: Argon2id username+password remains the only way a person
  authenticates. v3.1.0's embedded OAuth 2.1 (`apps/api/src/oauth/`) is the OPPOSITE role —
  FutureFin as *authorization server* issuing delegated MCP credentials to client apps
  (claude.ai) after a normal password login + consent (read-only at birth; desde los issues #2/#3
  también escriben, gobernadas por el rol vivo + `installation.mcp_write_enabled`). Adopting it
  does not reopen this entry.
- **Scope note de la PENSIÓN (añadida 2026-09-03, 5.0.0 · issue #207, decisión D3 del owner)** —
  la fila 13 de §1 y el `4a8e2af` de esta entrada mataron `users.pension_*`, unas columnas **sin
  handler** que nadie podía escribir ni leer y cuyo trabajo hacía mejor el toggle
  `persists_after_retirement`. 5.0.0 devuelve la pensión al usuario, y la diferencia es la que
  justifica la readmisión: **el bloque `pension` del perfil tiene FECHA**
  (`starts_at_age` → `PensionSchedule { start_index, monthly_today, indexed, fraction_while_partial }`),
  y esa fecha **cambia el objetivo**: mientras la pensión no existe hay que financiar el gasto
  ENTERO (base `bridge_to_pension`, `futurefin-fire-domain-reference` §4b), y desde ella solo el
  hueco que no cubra. Eso es precisamente lo que una columna plana no podía expresar y lo que el
  toggle `persists_after_retirement` —que **sigue vivo y sin cambios** para rentas y pensiones sin
  calendario— restaba desde el cruce aunque llegara veinte años después. No se resucita ninguna
  columna: es JSONB en el perfil, con handler, cotas y tools MCP. Lo que sigue rechazado es
  **derivar** la pensión de cotizaciones (falsa precisión: `financial-contracts.md` §3.11).

- **Second scope note (added 2026-08-27, v4.3.1)**: 4.3.1 ships «Entrar con Home Assistant»
  (`/v1/auth/ha/start|callback`, `apps/api/src/ha_idp/`), which **is** FutureFin acting as OAuth
  client of an external IdP — the shape this entry rejected. It is reopened **deliberately and
  narrowly**, with contract entry **D19** as the normative record. Driver: MCP/OAuth parity for
  add-on users — their SSO accounts have no password (by design, 4.3.0) and therefore could not
  authorize the OAuth consent screen at the direct origin. Scope of the readmission: HA is an
  **identity source only** (roles, membership and the owner bootstrap stay FutureFin-side); the
  HA refresh token is revoked immediately after `auth/current_user` (zero HA credentials
  retained); the identity converges on the same `users.external_user_id` as the ingress
  header-SSO (parity test in `apps/api/tests/ha_idp_login.rs`); the feature only activates in
  add-on mode (`FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON=1`, panic otherwise). **Generic
  OAuth-login with arbitrary IdPs stays rejected**: Argon2id username+password remains the only
  standalone login. This is the chronology of three OAuth roles: client-for-login (rejected
  2026-05, readmitted narrowly for HA 2026-08), authorization-server (adopted v3.1.0),
  identity-provider-client for HA (adopted v4.3.1).

### 2.11 Embedding PostgreSQL in the image — five traps found on the way (v3.0.0)

- **Context**: 2.x shipped two containers (`futurefin` + `futurefin-database`). 3.0.0 collapsed
  them into one self-contained image (PostgreSQL 16 active, PostgreSQL 15 binaries bundled only
  for auto-`pg_upgrade`), reusing the **existing `pgdata` volume unchanged** — the whole point was
  that upgrading from 2.x loses nothing. Getting there surfaced five ways to lose data or hang the
  container, four of which are silent. Each is now pinned by a rule in the code and a scenario in
  the CI `docker-stack` job. **Do not undo any of them.**
- **Trap 1 — never base the runtime on `postgres:*` (and never declare `VOLUME`).** The official
  Postgres images declare `VOLUME /var/lib/postgresql/data`. A user running `docker run` **without**
  `-v` therefore gets an **anonymous** volume they never see — and watchtower discards it when it
  recreates the container: complete, silent data loss with no error anywhere. Fix: runtime is
  `debian:bookworm-slim` and the PG binaries are `COPY --from=` build stages; the Dockerfile
  declares **no `VOLUME` of its own**, so the entrypoint's `mountpoint` check can distinguish "real
  volume" from "nothing", and **aborts** without one (`no persistent volume is mounted at
  $PGDATA …`, opt-out `FUTUREFIN_ALLOW_EPHEMERAL_DB=1`). Declaring `VOLUME` would pre-mount an
  anonymous volume and blind that guard. CI: the "Image sanity" step asserts a volume-less
  `docker run` **fails** and logs `no persistent volume`.
- **Trap 2 — the PGDATA uid differs between Alpine and Debian.** `postgres:*-alpine` (what 2.x
  used) runs postgres as **uid 70**; the Debian-flavoured binaries here use **uid 999**. A 2.x
  volume mounted as-is is therefore unreadable by the new postmaster. Fix: `adopt_cluster` does a
  one-time `chown -R postgres:postgres "$PGDATA"` + `chmod 0700` when the owner differs, logging
  `adopting ownership of PGDATA (uid 70 -> 999)`.
- **Trap 3 — musl→glibc changes text collation, and btree indexes go silently corrupt.** The 2.x
  cluster's text indexes were built under musl's collation order; read back under glibc, index
  scans miss rows that are physically present. **Nothing errors.** A unique index looks like it
  enforces nothing: the classic observable is that **registering an already-taken username
  succeeds instead of returning 409**. Fix: `maybe_adoption_reindex` runs
  `REINDEX DATABASE` + `ALTER DATABASE … REFRESH COLLATION VERSION` **once per cluster**, keyed on
  the cluster's *system identifier* (`REINDEXED_SYSID` in the entrypoint state file), so it never
  repeats and never runs on a cluster this image created itself. CI encodes exactly that detector:
  after V2→V3 it re-registers `citest` and **requires 409/422** — with a broken unique index the
  call would return 200 (or 500).
- **Trap 4 — stop the postmaster with SIGINT, never SIGTERM.** For PostgreSQL, SIGTERM is *smart*
  shutdown: wait for every client to disconnect, indefinitely. SIGINT is *fast* shutdown: roll
  back, checkpoint, exit. Sending SIGTERM from the supervisor made the container hang until Docker
  SIGKILLed it — a killed checkpoint, i.e. recovery work (or worse) on next boot. This is the same
  reason the official image sets `STOPSIGNAL SIGINT`. Fix: `on_term` stops the **API first**
  (SIGTERM → axum graceful shutdown → `pool.close()`), then `stop_pid "$PG_PID" INT …` with
  **SIGQUIT** (immediate) as the escalation, never SIGKILL; compose sets `stop_grace_period: 60s`
  and self-hosters on watchtower must set `WATCHTOWER_TIMEOUT=60s`. CI: the "Clean shutdown" step
  greps the log for `shutdown signal received` → `database pool closed` →
  `database system is shut down` → `clean shutdown complete` and asserts **exit code 0**.
- **Trap 5 — the healthcheck's `</dev/tcp` fallback was REMOVED; `CMD-SHELL` STAYS.** The 2.x
  healthcheck was `curl -sf …/v1/health || bash -c '</dev/tcp/localhost/8080'`. With the database
  now inside the container, that fallback answers "healthy" from the mere TCP listener while
  `/v1/ready` is returning **503 with the database down** — it masks the one failure the probe
  exists to catch. The probe is now `/v1/ready` (which round-trips the pool) with **no fallback**.
  What did **not** change is the `CMD-SHELL` form: incident v1.0.2 (§1 row 17) — the exec form does
  not resolve `curl` through PATH and the container is permanently unhealthy — **is still live**.
  Removing the fallback is not permission to go back to `CMD`.
- **Status**: shipped in 3.0.0 (5ca91f4). **Evidence**: CI job `docker-stack`
  (`.github/workflows/ci.yml`) — image sanity + no-volume guard, fresh install, watchtower-style
  recreate, clean shutdown, **V2→V3 with real seeded data** (adoption `chown` + collation REINDEX +
  automatic pre-migration backup + the duplicate-register detector), the image over an untouched 2.x
  compose (external-DB compat until 3.9.0; a **hard refusal** since 4.0.0 — §2.12), a leftover
  `DATABASE_URL` with an empty volume (refusal + volume asserted still empty), and
  **pg_upgrade 15→16** with a row census. Frozen 2.x topologies live in `.github/testdata/`.
  Normative statement of the resulting contract: futurefin-architecture-contract **D13** and **W8**.

### 2.12 Retiring the external-database mode without it reading as data loss (v4.0.0)

- **Situation**: 3.0.0 moved PostgreSQL inside the image but kept talking to an external one when
  `DATABASE_URL` was set — a compat mode for 2.x installs, deprecated from day one and announced
  as "removed in 4.0.0" in the README, in `.env.example` and in the banner it printed on every
  start. (After the `docs/` split the same notice lived in `docs/configuracion.md` and
  `docs/actualizar.md`; the README had stopped repeating it by 3.9.0.)
- **Why it had to go**: it was the only supported topology with **none** of the safety nets that
  justify the self-contained image (§2.11): no automatic pre-migration dump, no `pg_upgrade`, no
  ordered postmaster shutdown, no volume guard. Every one of those is a promise the project makes
  and could not keep in that mode.
- **The trap in removing it**: the naive removal is "ignore `DATABASE_URL` and start". For someone
  whose data lives in that external database, that starts an **empty** installation — indistinguishable
  from total data loss to the person looking at the screen, even though nothing was lost. A
  deprecated feature must not be retired into a silent empty state.
- **What was done instead** (`apps/api/docker-entrypoint.sh`): the leftover variable is triaged.
  **Cluster already in the volume** → warn and ignore it (their data is already inside; they just
  have one stale line in their compose). **No cluster** → `refuse_external_database`: print what
  happened, state that the data is untouched, give the three steps (start 3.9.0 once with the same
  URL and volume, drop `DATABASE_URL`, come back to 4.0.0), link `docs/actualizar.md`, and **exit 1
  before initializing anything**. `FUTUREFIN_DB_MODE=external` is still *recognized*, purely to die
  with an explanation instead of `invalid FUTUREFIN_DB_MODE`.
- **Lesson**: when you delete a deprecated path, decide explicitly what happens to the inputs that
  used to select it. Ignoring them is only safe when ignoring them is harmless; otherwise refusing
  loudly beats proceeding quietly.
- **Status**: shipped in 4.0.0. **Evidence**: CI `docker-stack` — the current image over an
  untouched 2.x compose must exit non-zero and log `ya no habla con bases de datos externas` +
  `3.9.0`; and a `docker run` with an external `DATABASE_URL` over empty volumes must exit non-zero,
  log the same message plus `docs/actualizar.md`, and leave the volume **verifiably empty**.
  Operator-facing form: `docs/actualizar.md` §«Vengo de 2.x o tengo una base de datos externa».

### 2.13 El kill-switch se diagnosticaba como avería (MCP Fase 4, v4.4.0)
- **Symptom**: with `FUTUREFIN_MCP_ENABLED=0`, `POST /mcp` returned **405 with an empty body** and
  `GET /.well-known/oauth-authorization-server` returned **200 `text/html`** (the SPA shell). The
  claude.ai connector could not parse either as JSON and reported "connection failed" with no
  usable cause.
- **Root cause**: the switch **unmounted** `/mcp` and the OAuth protocol routes outright. The
  published image's final fallback is a `ServeDir` with a fallback to `index.html`, and
  **`ServeDir` does not invoke its fallback for methods other than GET/HEAD** — so an unmounted
  `POST /mcp` fell through to axum's bare 405, while an unmounted GET fell through to the SPA
  shell. **A security control that, once activated, diagnosed as an outage.** Generalized: **a
  test that asserted an absence while mounting a router the published image does not have** — the
  test suite that shipped this behaviour built the router *without* the SPA static fallback, so it
  confirmed a clean 404 production never actually returned.
- **Fix**: 4.4.0 — routes are **always mounted** (`mcp_router`, `oauth_protocol_router(enabled)`
  in `apps/api/src/mcp/mod.rs` / `apps/api/src/oauth/mod.rs`); with the switch off, every one of
  them answers **404 JSON `{error, code: "mcp_disabled", message}`**, any method. Same doctrine as
  `POST /v1/auth/sso` answering `sso_disabled` instead of disappearing (**D18**: the shape of the
  router does not depend on the environment). Both regression tests
  (`mcp_http.rs::mcp_disabled_answers_json_even_with_the_spa_mounted`,
  `oauth_flow.rs::oauth_protocol_disabled_with_mcp_but_connections_panel_survives`) now mount the
  router through `spa::mount_static_spa`, the exact function `main.rs` calls — and the harness
  axis behind it, `TestConfig::web_static_root`, is now the one every future test must use
  whenever it asserts something about a route that does NOT exist; a test that skips it describes
  a lab binary the published image does not match.
- **Status**: fixed 4.4.0. Operator-facing detail: `futurefin-run-and-operate` §3.8.

### 2.14 El invariante de 1 MiB era falso justo donde importaba (MCP Fase 4, v4.4.0)
- **Discovery**: the documented invariant — "every request body is capped at 1 MiB, except the
  explicitly larger backup-import route" — was false for `/mcp`.
- **Root cause**: `DefaultBodyLimit` enforces its cap **through axum's extractors**; `/mcp` is
  mounted as a `route_service` (rmcp's own Tower service), which reads the request body itself and
  never goes through an extractor — so it fell back to rmcp's own undocumented default of **4
  MiB**. Generalized lesson: **an invariant enforced by a layer only holds for the routes that
  layer's actual mechanism reaches** — sitting behind the same `.layer()` call is not enough if
  the route bypasses the thing the layer instruments.
- **Fix**: 4.4.0 — `mcp::MCP_MAX_REQUEST_BODY_BYTES = 1 MiB` set explicitly via
  `StreamableHttpServerConfig::with_max_request_body_bytes`. Regression test
  `body_limits.rs::oversized_mcp_body_returns_413` sends a 2 MiB body — above the documented
  global cap, below the SDK's undocumented default — exactly the gap that was silently open.
- **Status**: fixed 4.4.0.

### 2.15 Una lista de CORS, dos privilegios (MCP Fase 4, v4.4.0)
- **Symptom/discovery**: until 4.3.1 a single `CORS_ORIGINS`-derived layer with
  `allow_credentials(true)` wrapped the **entire** router. Adding an origin so a browser-based MCP
  client (the MCP Inspector) could reach `/mcp` also granted that origin **cookie-authenticated**
  cross-origin access to `/v1/backup/user-export`, `/v1/api-tokens` and `/v1/installation` —
  routes whose credential is the `ff_session` cookie, not a Bearer header.
- **Root cause**: `/mcp`'s credential is always a Bearer header, so it never needed
  `allow_credentials`; the HTTP API's credential is the cookie, so it does. One shared CORS layer
  cannot express two different privilege levels, and the safer default was never chosen because
  CORS was configured once, at the very top of the router.
- **Fix**: 4.4.0 — two layers: `routes::app_router`'s own `api_cors_layer` (`allow_credentials(true)`,
  unchanged surface) and a separate `mcp::mcp_cors_layer` (no `allow_credentials`, MCP-specific
  header allowlist, applied only inside `/mcp`'s own sub-router). Regression test
  `mcp_http.rs::mcp_preflight_is_complete_and_grants_no_cookie_access`.
- **Status**: fixed 4.4.0. Ordering trap this fix depends on: `mcp` must be `merge`d **after**
  `.layer(api_cors_layer(...))` is applied to the rest of the router — merging it earlier would
  make `/mcp` inherit `allow_credentials(true)` again. See §2.16 for the companion trap inside
  `mcp`'s own sub-router.

### 2.16 `Router::layer` vs `route_layer`: a merge drags the fallback along (MCP Fase 4, v4.4.0)
- **Symptom (caught before shipping)**: with the MCP Bearer-auth/CORS layer applied via
  `.layer(...)` inside `mcp`'s own sub-router instead of `route_layer`, every unknown route on the
  **whole merged application** — including `/oauth/authorize`, the SPA-served OAuth consent
  screen — started returning **401** from the MCP auth middleware.
- **Root cause**: axum's `Router::layer` wraps not only the router's registered routes but also
  its **fallback**; `Router::merge` then drags that already-wrapped fallback onto the destination
  router. The merged application's fallback effectively becomes the MCP sub-router's fallback, so
  any route the destination does not otherwise handle passes through the MCP layer first.
  `route_layer` wraps only the registered routes and leaves the fallback untouched.
- **Caught by**: the **pre-existing** test `oauth_flow.rs::get_oauth_authorize_is_not_handled_by_the_api`
  — written for an unrelated reason, it happened to cross this exact path and failed loudly before
  merge.
- **Status**: fixed 4.4.0 (`mcp`'s auth and CORS layers use `route_layer`, never `layer`). **This
  is a trap that will be reintroduced by anyone adding a new layer to a router meant to be merged
  into another: always use `route_layer` inside that sub-router, never `layer`, unless you
  specifically intend to also own the parent's fallback.**

### 2.17 Streamable HTTP sessions not bound to the credential — deliberate, with a trigger to revisit (MCP Fase 4, v4.4.0)
- **Not a bug, a considered non-addition**: `LocalSessionManager`'s `Mcp-Session-Id` is not
  cryptographically tied to the Bearer credential that opened it, and the decision to leave it
  that way is reasoned in full in the module doc-comment of `apps/api/src/mcp/mod.rs` so it does
  not get re-proposed as an obvious hardening later.
- **Why it is safe today**: the Bearer middleware runs before the MCP protocol on **every**
  request, identity is re-resolved live per request (**D14**), and every tool executes as the user
  of the presented token — never "the user of the session". A stolen `Mcp-Session-Id` without a
  valid token never gets past 401. And the server issues **no** notifications or server-initiated
  requests today, so no session carries data the authenticated request itself did not already ask
  for — that absence is the *entire* reason the missing binding is currently safe, not a
  side detail.
- **Named trigger to revisit**: the first capability that emits something toward the client on the
  server's own initiative — notifications, `progress` updates, a resumable SSE stream carrying
  data. At that point either build a `SessionManager` that ties session → credential, or drop to
  `legacy_session_mode: false` (stateless only, which SEP-2567 is retiring sessions toward anyway).
  **Do not add the binding pre-emptively "just in case" before that trigger fires** — it is
  complexity with no defender until then.
- **Status**: deliberately not implemented, v4.4.0.

### 2.18 `density` rejected as a `get_projection` parameter — a finer grid answers WHERE, never WHY (MCP Fase 5, issue #86, v4.4.0)
- **Symptom prompting the idea**: `get_projection` (MCP) forces `density=hybrid`, so past month 24
  the response carries roughly one point per year. A large net-worth drop between two consecutive
  points has nothing in the payload explaining it — the obvious-looking fix was to let the tool ask
  for `density=monthly` too.
- **Why it was rejected**: `density=monthly` multiplies the payload size for the same horizon
  (reproduce with the point-count harness in `futurefin-diagnostics-and-tooling` — don't freeze the
  number, the horizon it's measured against changes) and still only answers WHERE the curve moved,
  never WHY. The actual cause of a step is almost always a dated "Próximo" (planning flow) entering
  the model that month — information no amount of numeric resolution can carry.
- **Fix chosen instead**: `events`/`events_truncated` on `GET /v1/projection/series`
  (`ProjectionEvent`, capped at `PROJECTION_EVENTS_MAX = 100`) — the dated Próximos landing inside
  the horizon, each ~90 bytes (month, label, amount, direction), answering the question a finer grid
  could only gesture at. Undated Próximos are deliberately excluded: they spread evenly over
  `PLANNING_UNDATED_SPREAD_DAYS` (90 civil days), so by construction they produce a ramp, not a
  step — listing one as an "event" would misdescribe it.
- **Status**: rejected 4.4.0; `events` shipped instead. Grounded in the `ProjectionEvent`
  doc-comment, `apps/api/src/handlers/projection.rs`.

### 2.19 Renaming `get_budget.totals`/`get_summary.financial_health`'s homonyms — declare the base, don't rename the field (MCP Fase 5, issue #86, v4.4.0)
- **Symptom prompting the idea**: `income_monthly_equivalent`, `expense_regular_monthly_equivalent`,
  `expense_total_monthly_equivalent` and `net_monthly_equivalent` appear, spelled identically, in
  both `GET /v1/budget` (`totals`) and `GET /v1/summary` (`financial_health`) — and in
  `savings_source` modes B/C the two genuinely disagree (budget's plan figure vs the real
  12-month-average `financial_health` reports). Two different numbers under the same name in the
  same catalog is exactly the class of bug the 3.9.0 "three savings figures" incident already was.
- **Why it was rejected**: a rename (e.g. `_plan`/`_actual` suffixes) is breaking over six fields —
  the SPA reads them by name, and so does every MCP tool surfacing either response — and it would
  not have made either number more legible on its own: a caller still could not tell, from the name
  alone, whether *this particular* response's figures are plan or actual.
- **Fix chosen instead**: a `basis` field on both. `BudgetTotalsResponse::basis` is the constant
  `BUDGET_TOTALS_BASIS = "plan"` (never anything else, by construction — budget totals ARE the
  plan); `financial_health.basis` ∈ `plan`\|`actual`\|`mixed`, derived from the two
  `savings_*_basis` fields. Reading rule now documented at the source: if `financial_health.basis
  != "plan"`, the four homonymous fields are not directly comparable pair-for-pair.
- **Status**: rejected 4.4.0; `basis` fields shipped instead. Grounded in
  `BudgetTotalsResponse::basis`'s doc-comment, `apps/api/src/handlers/budget.rs`.

### 2.20 El invariante del sumidero era una **pre-condición repartida**, y tapaba dos bugs vivos (MCP Fase 6, issue #87, v4.4.0)
- **Cómo salió**: la Fase 6 abría `create_allocation_rule` / `delete_allocation_rule` por MCP, y el
  registro de paridad ya avisaba en su fila §3.2 #2: «**careful**: the sink invariant lives spread
  across the handler». Encapsularlo no era una limpieza opcional: era el requisito para que una
  superficie nueva no pudiera romperlo.
- **Lo que había**: I1 (**como mucho** un `remainder` sin cap y, si existe, **siempre el último** —
  cero sumideros es un estado válido, y `assert_sink_invariant` lo dice explícitamente) se comprobaba
  como **pre-condición**, en cada camino de escritura, por separado. Un camino nuevo que se
  olvidara de mirar simplemente no miraba, y nada fallaba.
- **Los dos bugs que aparecieron al encapsularlo** — ninguno era hipotético:
  1. El **`PATCH`** podía convertir una regla en sumidero **sin recolocarla**. La cascada quedaba
     con el `remainder` **en medio**: todo lo que hubiera por debajo dejaba de recibir un euro, en
     silencio y para todo el horizonte. El síntoma es exactamente el que nadie reporta, porque el
     dinero sigue yendo a algún sitio.
  2. La guardia `sink_must_be_last` del **`reorder`** derivaba el scope de **la vista** en vez del
     owner. En vista de hogar eso significaba comparar contra `owner_user_id IS NULL`, que no casa
     con ninguna fila real: **no comprobaba nada**. Es el primo del incidente §2.6 (binds
     invertidos entre ramas household/mine) — misma familia: una guardia que existe, corre y no
     mira lo que cree mirar.
- **Arreglo**: I1 pasa a **post-condición** verificada dentro de la transacción, con un **único
  punto de commit** en el módulo (`commit_with_sink_invariant`, el único `tx.commit()` de
  `allocation_rules.rs`, con cuatro llamantes: create, patch, delete, reorder). Un camino nuevo que
  abra transacción y se olvide no corrompe nada: se cae al hacer `drop`. **Matiz honesto**: la
  garantía cubre a quien abre transacción; un `execute(&state.pool)` suelto sí escribiría, y además
  sería invisible para la guardia, que cuenta `tx.commit()`. Hoy el módulo está limpio.
  Son **dos redes distintas**, no las confundas:
  - el **punto único** lo fija un `#[test]` **sin BD, dentro del propio módulo**
    (`apps/api/src/handlers/allocation_rules.rs`,
    `sink_guard_tests::el_modulo_tiene_un_unico_punto_de_commit`): hace `include_str!` de su propio
    fichero, compone la aguja en runtime (`format!("tx.{}()", "commit")`) **para no contarse a sí
    mismo** e ignora las líneas de comentario;
  - el **comportamiento** lo fijan los 6 tests de integración de
    `apps/api/tests/allocation_sink_invariant.rs`.
- **Lección transferible**: una pre-condición repartida por N caminos es N oportunidades de
  olvidarla y cero de detectarlo; una post-condición con un solo punto de commit es una. Y cuando
  encapsulas una invariante que llevaba tiempo «funcionando», cuenta cuántos bugs caen: si caen
  cero, probablemente no la has encapsulado.

### 2.21 `delete_category` con `remap_to` ignoraba el remap **en silencio** justo donde dolía (MCP Fase 6, issue #87, v4.4.0)
- **Síntoma**: borrar una categoría de gasto pasando `remap_to` funcionaba… salvo cuando el
  contador de referencias daba 0. En ese caso el remap **no se aplicaba**, y la referencia que sí
  existía —`liabilities.expense_category_id`— se degradaba a `NULL` por la FK.
- **Causa raíz**: la FK de esa columna es `ON DELETE SET NULL`, así que la referencia **no entraba
  en el recuento** de las que bloquean el borrado. Como el remap solo se ejecutaba en el camino
  «hay referencias contadas», la única referencia que quedaba era precisamente la que el `remap_to`
  venía a salvar: la atribución de las cuotas de pasivo, el enlace que empareja el recibo real con
  su partida de presupuesto.
- **Por qué es peor que un error**: el usuario pide explícitamente «muévelo a esta otra categoría»,
  recibe 200, y el dato queda **peor** que si no hubiera pedido nada. Un 400 habría sido correcto;
  el silencio no.
- **Arreglo + regresión**: el remap se aplica siempre que se pide, incluidas las referencias
  `SET NULL` no contadas; el preview desglosa por tabla y separa las bloqueantes de las que solo se
  degradan (`liabilities_expense_attribution`, `categorization_rules_degraded`). Cubierto en
  `apps/api/tests/categories_crud.rs` (8 tests).
- **Lección transferible**: «no hay referencias» y «no hay referencias **que bloqueen**» no son lo
  mismo, y una FK `SET NULL` es exactamente la diferencia. Si un contador decide si un remap se
  ejecuta, comprueba primero que el contador cuenta todo lo que el remap iba a arreglar.

### 2.22 `suggest_transfer_matches`: ver un par candidato exigía **escribir** (MCP Fase 6, issue #87, v4.4.0)
- **Síntoma de partida**: la conciliación de transferencias (3.5.0) tenía pase automático y par
  manual, pero **ninguna forma de mirar** los candidatos. Desde el chat, la única manera de saber
  qué pares habría era ejecutar el pase y ver el resultado.
- **Regla que se aplicó, y por qué es dura**: **GET aparte, nunca un `?dry_run` sobre el POST**. Un
  GET que muta ya costó caro una vez (§3 fila 4, `purge_expired_liabilities`), y un `dry_run` sobre
  el verbo que escribe es la misma puerta con otra etiqueta: un día alguien invierte el default.
- **Y cierra una omisión deliberada sin reabrirla.** El registro de paridad excluía
  `reconcile_pair` manual como *LLM footgun* — su fila habla de elegir dos UUID a dedo y de que un
  par equivocado saca a las dos patas de todos los agregados de flujo y mueve el ahorro de los
  modos B/C —, con un *revisit trigger* que era, literalmente, que existiera una tool de
  sugerencias. Existe. Lo que se
  implementó **no es `reconcile_pair`**: `confirm_transfer_match` acepta **solo un `match_id`
  emitido por el servidor** (24 hex del SHA-256 de `instalación|owner|ids ordenados`,
  deliberadamente **no** un UUID). Un par arbitrario **no es expresable en el esquema** — no hay
  barrera que saltarse, es que el input no existe. Regresión:
  `apps/api/tests/transactions_transfer_matches.rs`.
- **Lección transferible**: cuando la razón de omitir algo es «un modelo lo va a equivocar», la
  salida buena no es una advertencia en la descripción — es un esquema en el que el error no se
  puede escribir.

### 2.23 La «g efectiva iterada» — el punto fijo escalar es la bisección con otro nombre (issue #178, 4.12.0)

- **Qué se propuso**: para el gross-up con `g` heterogénea por activo (base agregada
  `Σ g_i·venta_i` sobre tramos progresivos), iterar `G_{t+1} = gross_up(N, g_eff(G_t))` con la
  «g efectiva» de la venta candidata, esperando convergencia en 1-2 pasos.
- **Por qué se RECHAZÓ (spike de #178, medido)**: convergencia lineal con razón ≈ 0,11 — **9
  iteraciones para 1e-6 €**, ~19 para 1e-12; puede OSCILAR cuando el candidato cruza una
  frontera de activo (`g_eff` salta a trozos); el caso de capacidad agotada deja el mapa plano;
  y no es reproducible a mano (el fixture cruzado exige derivación independiente). Es la MISMA
  familia que el «binary-search tax gross-up» ya retirado — un punto fijo escalar sobre una
  función lineal a trozos, cuando la función es **invertible exacta por un paseo sobre sus
  quiebros** (`gross_up_mixed_monthly`, `crates/engine/src/tax.rs`: pendiente `1 − r·g_j` por
  trozo, termina en ≤ n+|tramos| pasos, sin tolerancias).
- **Lección**: si la función es lineal a trozos, no la busques — recórrela. Cada solver iterado
  de fiscalidad que ha entrado en este repo ha salido con un incidente (la bisección TS publicó
  un objetivo ~20 % bajo por saturación).

### 2.24 `x > 0` no es una guarda de división — la `g` denormal que el propio motor fabricaba (issue #208, 5.0.0 WP1a)

- **Síntoma**: `GET /v1/projection/series` devolvía un **400 `task_panic` opaco y permanente** para
  un hogar concreto. Función pura, entrada que la API acepta, y el pool blocking convirtiendo el
  pánico en un error ininteligible.
- **Causa raíz**: `tax::gross_up_mixed_monthly` remataba cada tramo con `x.min((ceiling − base)/g)`,
  guardado solo por `if g > Decimal::ZERO`. Una `g` **positiva pero denormal** desborda `Decimal`
  (~7,9e28) al dividir: medido, `g = 1e-20` pasa y `g = 1e-27` panica (`200000/1e-27 = 2e32`).
- **Y la fabricaba el propio motor**: una cuenta al 0 % alimentada por la cascada lleva la base
  pegada al valor (cada euro asignado sube ambos), el drenaje conserva `b/v` (teorema de #178) y un
  retorno del 0 % nunca reabre el hueco. Tras un gasto puntual grande, `g = 1 − b/v ≈ 1e-27`.
- **Fix**: `checked_div` en las dos divisiones con divisor fabricado por el motor; desbordar ⇒ el
  tope es efectivamente infinito y `x` se conserva (la capacidad real ya la limita el propio tramo).
  Cero cambio donde no desbordaba — el golden de 4.15.0 no movió un hash.
- **Lección transferible**: **`x > 0` no es una guarda de división.** Lo que hay que preguntarse no
  es si el divisor es positivo, sino si el COCIENTE cabe. Y el caso no era de laboratorio: cuenta
  corriente al 0 % + sumidero + un Próximo grande es un hogar normal.
- **Status**: cerrado en WP1a con reproductor convertido en regresión
  (`golden_pins.rs::mixed_drawdown_must_not_panic_on_a_denormal_gain_ratio`) y caso golden nuevo
  `P13_cash8k_denormal_g`.

### 2.25 El mismo fallo con la otra operación: `basis · values` sin `checked_mul` (issue #209, 5.0.0 WP2)

- **Síntoma**: idéntico —`Multiplication overflowed` → 400 `task_panic`—, pero por `*`, no por `/`.
- **Causa raíz**: la base de coste se actualiza con `basis[i] · values[i] / v_pre` (#120). Con un
  activo cerca del techo de la columna (`NUMERIC(18,4)`) y rentabilidad alta, el crecimiento empuja
  `values` por encima de ~7,9e14 y el **producto intermedio** sale del rango de `Decimal`.
- **Por qué no se arregló con #208**: el arreglo natural (`basis · (values/v_pre)`) **no es
  bit-idéntico**, y WP1a era bit-identidad estricta. Se aplazó con la decisión declarada.
- **Fix**: `checked_mul` y, **solo si no cabe**, la forma reordenada. El orden natural multiplica
  antes de dividir porque drenar el activo entero deja la base en **0 EXACTO**, y ese orden es el
  que 4.15.0 pineó: ninguna entrada que hoy funciona cambia un dígito.
- **Lección transferible**: cuando un arreglo cambia dígitos, **la salida honesta es aplicarlo solo
  en la rama que hoy panica**, no reescribir el camino común y regenerar el pin. Y un desbordamiento
  arreglado en una operación deja hermanos vivos en las otras: audita `*`, `/` y `+` juntos.
- **Status**: cerrado en WP2 con caso golden aditivo `P14_techo_numeric`.

### 2.26 Un filo de navaja que solo la puerta de `f64` podía ver: `cap_exhausted` (5.0.0 WP5.5)

- **Síntoma**: la puerta de degeneración del crate estocástico marcó **8.138 € de diferencia** entre
  el camino `Decimal` y el `f64` en el caso P15, muy por encima de la cota de 1 €/mes — con los dos
  goldens `Decimal` **intactos**, o sea sin ninguna regresión que un test existente pudiera ver.
- **Causa raíz**: el llamante del paseo mixto deducía «¿se vendió el techo entero?» comparando
  `w.gross_monthly >= gross_cap`. Es exacto en `Decimal` y **un filo de navaja en aritmética
  aproximada**: `(a·12)/12` puede caer un ulp por debajo de `a`. Y de esa rama cuelga el reparto
  de magnitudes: recorte **informativo** vs descubierto que **RESTA patrimonio**.
- **Fix**: el paseo **publica el booleano** (`MixedGrossDrawdown::cap_exhausted`, `remaining ≤ 0` al
  terminar) en vez de que el llamante lo re-derive de una comparación de flotantes.
- **Lección transferible, y es la general de este tren**: **publica el booleano que el algoritmo ya
  sabe, en vez de re-derivarlo de una comparación numérica aguas abajo.** Una comparación
  reconstruida es una segunda definición del mismo hecho, y las dos definiciones divergen en cuanto
  cambia el tipo, la escala o el redondeo.
- **Segunda lección**: el bug era **preexistente y ninguna suite lo veía**. Lo cazó una puerta nueva
  que compara dos implementaciones del mismo modelo — el valor de un gate de paridad no es solo
  proteger lo nuevo, es **iluminar lo viejo**.
- **Status**: cerrado en WP5.5; la puerta es
  `crates/engine-stochastic/tests/degeneration.rs::every_case_degenerates_from_decimal_to_floating_point`.

## 3. Designs that were tried and replaced

| Old design | Specific failure mode | Replacement (current) |
|---|---|---|
| **Inflation model v1 "real pure"** (v1.0.12): deflate each asset's return (`r_real = (1+r_nom)/(1+inf) − 1`), simulate everything in today-€ | Predecessor mixed deflation/inflation inconsistently; v1 itself, combined with a FLAT target, made inflation toggling nearly a no-op on retirement age, and pre-v1 produced asset drain before retirement | **v2 nominal + moving target** (v1.2.0, current): everything simulated in nominal €; ONLY the FIRE target and the loop's EXPENSE grow with inflation (#139), via `fire_target_at_month_index` (the engine evaluates month k against the target at index k−1 — see futurefin-fire-domain-reference §4). **La fórmula que aquí vivía —`target(m) = base × (1+inf/100)^(m/12)`— es la PRE-#170**: `FireTarget.base_amount` se retiró en 4.10.0 y el objetivo se evalúa mes a mes sobre la necesidad real (`gross_up(need(i)·12)/SWR + debt_term(i)`). Lo que la fila afirma —que el modelo es nominal y solo el objetivo se mueve— sigue siendo cierto; lo que caducó es la fórmula citada. Deflation is a display-layer concern (`milestones_real`, chart toggle) |
| **Per-asset contributions** (`monthly_contribution_fixed` + `contribution_remainder_weight` + per-asset cap, v1.0.11) | Sum of fixed contributions could exceed surplus; weights confusing when >100 %; no explicit priority; cap-overflow redistribution needed ad-hoc fallback rules (v1.0.11's "highest-return liquid asset" patch was a symptom) | **Allocation cascade** (v1.1.0): ordered rules `fixed`/`percent`/`remainder` with optional caps (`amount`, `months_expense`, `income_multiple`); exactly one uncapped `remainder` sink, always last (server-enforced: `remainder_required`, `uncapped_remainder_exists`, `sink_must_be_last`). Clean column drop, NO data migration — owner signed off losing config; backup schema_version bumped to 3 |
| **Migration auto-repair** (12-round checksum repair loop) | Masked genuine drift between shipped migration files and applied checksums; a silently "repaired" DB can diverge from what migrations say | Fail-loud `sqlx::migrate!().run()` (`apps/api/src/db.rs`); manual `DELETE FROM _sqlx_migrations WHERE version = X` only when the change is genuinely idempotent |
| **GET-side purges** (`purge_expired_liabilities` called from 6 GET handlers) | GETs issued DELETEs: violates HTTP semantics, breaks caching (v1.4.0's cache would have been impossible), destroys audit data | `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)` filter in liabilities/summary/budget/assets/projection reads; rows persist |
| **Binary-search gross-up** (90 iterations to invert after-tax) | 90× slower than needed and convergence-threshold noise; obscured that the function is piecewise-linear | Closed-form per-bracket inversion `gross_up_net_annual_fire` (`handlers/projection.rs`); old binary search preserved as the test oracle `gross_up_binary_reference` |

## 4. "If you are tempted to X, read Y first"

| Temptation | Read first |
|---|---|
| Add a retirement age / target-age setting | §2.2 |
| Añadir un trigger de jubilación por EDAD **fuera de una estrategia** (un ajuste global, un segundo disparador que conviva con el cruce) | §2.2 + su scope note de 5.0.0 — la edad se readmitió como trigger de `retire_at_age`/`coast` y de nada más; el invariante es **un solo trigger por simulación** (`crossing_is_reading_only`), y lo que v1.0.6 mató fue justo la coexistencia ambigua |
| Meter `f64` en `crates/engine` (o «añadir una excepción» al freezer para un método numérico) | §2.9 + su scope note de 5.0.0 — la coma flotante vive en `crates/engine-stochastic`, sobre el MISMO bucle vía `MoneyOps`, y de ahí no sale un euro. Si de verdad hace falta, la conversación es de diseño: `futurefin-architecture-contract` + `futurefin-proof-and-analysis-toolkit` Recipe 5 |
| **Duplicar** el bucle en coma flotante «porque el genérico se resiste» | §2.26 y `futurefin-proof-and-analysis-toolkit` Recipe 7 — dos bucles divergen en silencio al primer cambio de modelo; era la salida de emergencia declarada del plan y no hizo falta |
| Relajar un fixture golden (o regenerarlo) para que pase tu cambio | §2.25 — regenerar `pins-4.15.json` es declarar que la aritmética cambió, y eso **exige entrada de CHANGELOG con el delta medido**. Si el arreglo mueve dígitos, aplícalo solo en la rama que hoy falla. Regenerar `pins-5.0-outputs.json` porque la canonicalización CRECIÓ sí es legítimo, y tiene su propio control (`the_5_0_canonicalization_grew_without_moving_the_old_fields`) |
| Guardar una división con `if x > 0` | §2.24 — la pregunta no es si el divisor es positivo, sino si el COCIENTE cabe |
| Re-derivar aguas abajo un hecho que el algoritmo ya conoce (comparando dos números) | §2.26 — publica el booleano; dos definiciones del mismo hecho divergen en cuanto cambia el tipo o la escala |
| Put contribution config back on assets, or "simplify" the cascade | §3 row 2; engine cascade tests in `crates/engine/src/projection.rs` |
| Deflate returns / simulate in real terms inside the engine | §3 row 1 y §1 fila 3 — deflactado solo de presentación; servirlo (4.4.0) no es simularlo |
| «Simplificar» la invariante del sumidero, o comprobarla en cada camino de escritura | §2.20 — post-condición con un solo punto de commit, no pre-condición repartida |
| Decidir si aplicas un remap por un contador de referencias | §2.21 — una FK `SET NULL` no entra en ese contador |
| Añadir `?dry_run` a un POST para «solo mirar», o exponer un `reconcile_pair` con dos UUID | §2.22 y §3 fila 4 |
| Make a GET delete/clean anything | §3 row 4; `apps/api/tests/liabilities_purge.rs` |
| Auto-fix a migration checksum mismatch in code | §3 row 3; CLAUDE.md Migrations |
| Warm the projection cache after a mutation | §2.7 |
| Compute anything from a projection point's array position | §2.8 |
| Send projection arrays as Decimal strings, or use f64 anywhere else | §2.9 |
| Duplicate the FIRE target formula, or edit FIRE math on one side only | §2.4, §2.5; regenerate `fire-parity.json` expectations, run both suites |
| Hand-write household/mine SQL branches | §2.6; use `LedgerView` helpers |
| Apply `display: flex`/`inline-flex` to a `<td>` | §2.3 |
| Drop a column in a migration | §2.1 — grep handlers for the column first; §3 row 2 for the data-loss sign-off precedent |
| Re-add OAuth **as app login** (external IdP), pensions API, persons, ZIP export, Caddy | §2.10 + its two scope notes (OAuth-as-AS adopted v3.1.0/D15; HA-as-IdP readmitted narrowly v4.3.1/D19 — anything beyond HA is still a re-add), table rows 12–16 |
| Base the runtime image on `postgres:*`, or add a `VOLUME` to the Dockerfile | §2.11 trap 1 — anonymous volumes + watchtower = silent total data loss |
| "Simplify" the entrypoint's `mv`-aside of an old cluster into an `rm -rf` | §2.11; futurefin-architecture-contract W8 (the entrypoint never deletes a cluster) |
| Stop the embedded postmaster with SIGTERM, or drop `stop_grace_period` | §2.11 trap 4 — SIGTERM is *smart* shutdown and hangs |
| Add a `</dev/tcp` fallback to the healthcheck, or switch it back to exec-form `CMD` | §2.11 trap 5 + §1 row 17 — the two rules point in opposite directions on purpose |
| Split PostgreSQL back out into its own compose service | §1 row 19, §2.11 |
| Make the container talk to an external database again (honour `DATABASE_URL`, re-add automigration) | §1 row 20, §2.12 — removed in 4.0.0 after a full deprecation cycle |
| Retire a deprecated path by silently ignoring the input that selected it | §2.12 — refuse loudly when ignoring is not harmless |
| Trust an old migration file as evidence of current schema | §2.10 last bullet |
| Unmount routes based on a runtime toggle (kill-switch, feature flag) | §2.13, §1 row 21's sibling doctrine — **D18**: keep the router shape environment-independent; return a disabled response (`mcp_disabled`, `sso_disabled`) instead of removing the route |
| Assume a `DefaultBodyLimit`/`.layer()` cap reaches every route behind it | §2.14 — a `route_service` (or anything reading the body itself) bypasses extractor-based layers; verify per-route, don't assume |
| Configure CORS once for a whole router that mixes cookie-auth and Bearer-auth routes | §2.15 — one `allow_credentials` cannot express two privilege levels; split the layer |
| Add a `.layer(...)` to a sub-router you are about to `merge` into another | §2.16 — `layer` wraps the fallback too and `merge` drags it along; use `route_layer` inside a sub-router meant to be merged |
| Bind an MCP `Mcp-Session-Id` to its credential "just in case" | §2.17 — no defender exists until the server emits something on its own initiative; wait for that trigger |
| Add `density` (or any resolution knob) as an MCP-tool parameter to explain a curve jump | §2.18 — a finer grid says WHERE, never WHY; add a named-event field instead |
| Rename a field to disambiguate it from a same-named field elsewhere | §2.19 — declaring the field's `basis`/mode costs less than a rename and fixes legibility better |

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
and direct code inspection. §1 row 19 and §2.11 (embedded PostgreSQL) added 2026-08-16 for
**v3.0.0**, by reading `apps/api/Dockerfile`, `apps/api/docker-entrypoint.sh`,
`docker-compose.yml`, `.github/workflows/ci.yml` and `.github/testdata/`.

**§1 row 21 and §2.13–§2.17 added 2026-08-28 for v4.4.0** (MCP Fase 4, issue #85), by reading
`apps/api/src/mcp/mod.rs`, `apps/api/src/oauth/mod.rs`, `apps/api/src/main.rs` and
`apps/api/tests/{mcp_http.rs,oauth_flow.rs,body_limits.rs}`. Re-verify:
- §2.13 (kill-switch, always mounted, JSON 404): `grep -n 'mcp_disabled\|MCP_DISABLED_MESSAGE' apps/api/src/mcp/mod.rs apps/api/src/oauth/mod.rs`;
  `grep -n 'web_static_root\|mount_static_spa' apps/api/tests/common/mod.rs`.
- §2.14 (explicit 1 MiB cap on `/mcp`): `grep -n 'MCP_MAX_REQUEST_BODY_BYTES\|with_max_request_body_bytes' apps/api/src/mcp/mod.rs`.
- §2.15 (two CORS layers): `grep -n 'mcp_cors_layer\|api_cors_layer' apps/api/src/mcp/mod.rs apps/api/src/routes/mod.rs`.
- §2.16 (`route_layer`, not `layer`, inside the merged `mcp` sub-router):
  `grep -n 'route_layer' apps/api/src/mcp/mod.rs`.
- §2.17 (session not bound to credential, reasoned in the module doc-comment):
  `grep -n 'Mcp-Session-Id\|LocalSessionManager' apps/api/src/mcp/mod.rs`.
- Counters unmoved by this phase (transport-only, no tool/route surface change):
  `grep -c '#\[tool(' apps/api/src/mcp/server.rs` (**71** el 2026-09-03 por la tarde — y **70 esa misma mañana**, con la rama 5.0.0 viva; 68 desde la Fase 6 del tren 4.4.0, y 52 cuando se escribió esta línea. Tercera vez que este contador se queda corto en la biblioteca: no lo congeles, recuéntalo).

**§1 filas 1 y 13 (scope notes), §2.2, §2.9, §2.10 (pensión), §2.24–§2.26 y las seis filas nuevas
de §4 se añadieron el 2026-09-03 para 5.0.0** (rama `release/5.0.0`, issue #207), leyendo
`crates/engine/src/{phases,target,withdrawal,solve,money,sim_core}.rs`,
`crates/engine-stochastic/{src/lib.rs,tests/degeneration.rs}`, `crates/engine/tests/golden_pins.rs`
y los issues #208/#209. **Dos readmisiones declaradas** (edad como trigger de estrategia, pensión
por usuario con fecha) y **una excepción `f64` nueva y acotada**; ninguna borra la entrada que
readmite — las tres se leen con su scope note al lado. Re-verificar:
- Trigger único, no dos: `grep -n "crossing_is_reading_only" crates/engine/src/sim_core.rs apps/api/src/handlers/projection.rs` (3 y 3 hits el 2026-09-03) y el invariante sobre la SERIE `grep -n "fn the_phase_readings_agree_with_the_series_they_describe" crates/engine/tests/golden_pins.rs`
- El freezer sigue sin excepciones: `grep -n "fn crates_engine_src_has_no_f64_outside_comments" crates/engine/src/lib.rs` y `grep -rn "f6"$'4' crates/engine/src/ | grep -v "^crates/engine/src/lib.rs"` (solo comentarios; el token va partido a propósito, igual que en el propio freezer, para no cazarse a sí mismo)
- La coma flotante vive en un crate aparte y sin RNG en el motor: `grep -c "impl MoneyOps for F64Money" crates/engine-stochastic/src/lib.rs` (1) y `grep -c "rand" crates/engine/Cargo.toml` (0)
- La puerta que la sostiene: `grep -n "const EUR_TOLERANCE\|fn every_case_degenerates_from_decimal_to_floating_point" crates/engine-stochastic/tests/degeneration.rs` (2 hits)
- Los tres bugs de §2.24–§2.26, cerrados en el código: `grep -n "issue \*\*#209\*\*\|#209" crates/engine/src/sim_core.rs | head -3`, `grep -n "checked_div" crates/engine/src/tax.rs | head -3` y `grep -n "pub cap_exhausted" crates/engine/src/tax.rs`
- La pensión con fecha es JSONB del perfil, no una columna resucitada: `grep -n "PensionSchedule" crates/engine/src/phases.rs | head -3` y `grep -rn "users.pension_" apps/api/src` (**debe salir vacío**: las columnas siguen muertas)

**§1 rows 22–23 and §2.18–§2.19 added 2026-08-28 for v4.4.0** (MCP Fase 5, issue #86), by reading
`apps/api/src/handlers/projection.rs` (`ProjectionEvent` doc-comment) and
`apps/api/src/handlers/budget.rs` (`BudgetTotalsResponse::basis` doc-comment) — both rejections are
argued in the source, not just in the PR description. Re-verify:
- §2.18 (`density` rejected, `events` shipped): `grep -n "PROJECTION_EVENTS_MAX\|struct ProjectionEvent" -B12 apps/api/src/handlers/projection.rs` (doc-comment states the ~5× multiplier and "sigue sin decir POR QUÉ").
- §2.19 (rename rejected, `basis` shipped): `grep -n "BUDGET_TOTALS_BASIS\|Los nombres no se renombran" -A3 apps/api/src/handlers/budget.rs`; the summary-side twin: `grep -n "financial_health.basis\|fn.*basis" apps/api/src/handlers/summary.rs`.
- Counter unmoved by **that** phase (context/pagination/view-echo only, no tool added/removed): `grep -c '#\[tool(' apps/api/src/mcp/server.rs`. Sí lo movió la **Fase 6** (issue #87): 52 → **68**, 16 altas y cero bajas.

Re-verify volatile facts before relying on them:

- Commit hashes and dates: `git log --oneline` (50 commits as of 2026-07-02; new work lands on `dev`).
- Version: `grep '^version' apps/api/Cargo.toml` + top of `CHANGELOG.md` (3.0.0 on 2026-08-16).
- Migration count and drop-migrations: `ls apps/api/migrations/ | wc -l` (34 as of 2026-08-16;
  31 as of 2026-07-02); `ls apps/api/migrations/ | grep -i drop`.
- §2.11 trap 1 (no `postgres:*` base, no `VOLUME`): `grep -n '^FROM' apps/api/Dockerfile` and
  `grep -n '^VOLUME' apps/api/Dockerfile` (the latter must be empty); guard text:
  `grep -n 'no persistent volume' apps/api/docker-entrypoint.sh`.
- §2.11 trap 2 (uid adoption): `grep -n 'adopting ownership of PGDATA' apps/api/docker-entrypoint.sh`.
- §2.11 trap 3 (collation REINDEX, once per system identifier):
  `grep -n 'REINDEX DATABASE\|REFRESH COLLATION VERSION\|REINDEXED_SYSID' apps/api/docker-entrypoint.sh`;
  the CI detector: `grep -n 'duplicate register' .github/workflows/ci.yml`.
- §2.11 trap 4 (SIGINT, not SIGTERM): `grep -n 'stop_pid "\$PG_PID" INT' apps/api/docker-entrypoint.sh`
  (two sites) and `grep -n 'stop_grace_period' docker-compose.yml`.
- §2.11 trap 5 (no `/dev/tcp`, `CMD-SHELL` kept): `grep -n 'dev/tcp' docker-compose.yml apps/api/Dockerfile`
  → only the two "do NOT add it back" comments; `grep -n 'CMD-SHELL' docker-compose.yml`.
- §2.11 evidence: `grep -n '^      - name:' .github/workflows/ci.yml` (job `docker-stack`
  scenarios) and `ls .github/testdata/` (frozen 2.x compose topologies — `automigrate.yml` gone
  since 4.0.0).
- §2.12 (external mode retired, added 2026-08-22 for v4.0.0): the mode is really gone —
  `grep -n 'exec_api_external\|automigrate_\|EXTERNAL_WAIT' apps/api/docker-entrypoint.sh` must hit
  **only** the two comment lines of the «Base de datos externa: retirada en 4.0.0» block — no
  definition, no call site; what replaced it: `grep -n 'refuse_external_database\|ya no existe\|se ignora' apps/api/docker-entrypoint.sh`;
  its CI guards: `grep -n 'ya no habla con bases de datos externas' .github/workflows/ci.yml` (two hits).
  The 3.9.0 behaviour it points people back to: `git show v3.9.0:apps/api/docker-entrypoint.sh`.
- FIRE helper is still the single source: `grep -rn 'fire_target_at_month_index' crates/ apps/api/src/`.
- Scope helpers still used: `grep -n 'scope_where' apps/api/src/handlers/person_view.rs`.
- No auto-repair regression: `grep -n 'repair' apps/api/src/db.rs` (expect no hits).
- Cache invalidation call sites: `grep -rln 'refresh_projection_after_mutation' apps/api/src/handlers/`.
- Chart deflation by month_index: `grep -n 'deflator' apps/web/src/views/ProjectionNetWorthChart.tsx`.
- Parity fixture pair: `ls apps/api/tests/fixtures/fire-parity.json apps/web/src/lib/fire.test.ts`.
- f64 boundary: `grep -n 'serialize_decimal_as_f64' apps/api/src/handlers/projection.rs`.
- RetirementView field: `grep -c 'expense_retirement_monthly_equivalent\|fireExpenseM' apps/web/src/views/RetirementView.tsx` (expect ≥4; **6** hoy). El grep de solo el nombre de campo da **1**: la vista se refactorizó para leerlo una vez en la local `fireExpenseM` y reutilizar esa. El dato sigue vivo, el patrón viejo ya no lo demostraba.
- Doc drift record: the stale docs found while authoring this library (projection_target_age
  remnants, "no CI yet", "33 migrations"…) were fixed on 2026-07-02; the standing-errata table
  lives in futurefin-docs-and-writing §7. Since 4.0.0 CI DOES run `apps/api/tests/` (job
  `integration` — `grep -n TEST_DATABASE_URL .github/workflows/ci.yml` must print; W1 in
  architecture-contract records the closure).
- When a new incident is settled (root cause found, design rejected or removed), append it here:
  one table row in §1 plus a ≤15-line entry in §2 if it carries a lesson, citing commit + version.
