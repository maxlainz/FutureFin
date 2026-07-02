---
name: futurefin-docs-and-writing
description: >
  Load this skill whenever you WRITE or UPDATE FutureFin's documents of record: CHANGELOG.md,
  CLAUDE.md, README.md, any .claude/*.md reference doc, or a .claude/skills/*/SKILL.md. Triggers:
  "add a CHANGELOG entry", "document this change", "update the docs", "release notes", "bump the
  version", "which doc do I update after adding a route / column / env var / test / UI token?",
  "the docs say X but the code says Y" (doc drift), "write a migration note", "keep the skill
  fresh". Also load it at the END of any task that changed routes, schema, env vars, engine
  behavior, tests or visuals — those changes are not done until the matching doc is updated.
  Do NOT use for: deciding whether a change is allowed or how to release safely
  (futurefin-change-control), debugging (futurefin-debugging-playbook), historical root causes
  (futurefin-failure-archaeology), test authoring (futurefin-validation-and-qa), FIRE math content
  (futurefin-fire-domain-reference).
---

# FutureFin — Docs of Record & House Writing Style

Facts below verified against the repo on **2026-07-02, v1.4.3** (`apps/api/Cargo.toml`). This skill
tells you (a) which file owns which facts, (b) what to update when you change something, (c) how
entries must be written, and (d) which documented facts are currently WRONG (standing errata).

Core rule, from CLAUDE.md and non-negotiable: **whenever an area changes, update its `.claude/*.md`
doc in the same change**. A PR that adds a route but doesn't touch `api-routes.md` is incomplete.

## When NOT to use this skill

- Deciding whether/how a change may ship (gates, migration discipline, release safety) →
  `.claude/skills/futurefin-change-control/SKILL.md`. This skill only covers the *writing* half of
  the release ritual.
- Investigating why something broke → `.claude/skills/futurefin-debugging-playbook/SKILL.md`;
  settled past incidents → `.claude/skills/futurefin-failure-archaeology/SKILL.md`.
- Writing or running tests → `.claude/skills/futurefin-validation-and-qa/SKILL.md` (this skill only
  tells you to update `tests.md` when test infra changes).
- The FIRE/retirement math itself → `.claude/skills/futurefin-fire-domain-reference/SKILL.md`.
- Env-var semantics / adding a config axis → `.claude/skills/futurefin-config-and-flags/SKILL.md`
  (this skill covers where to *document* it).

## 1. Doc-of-record map

One home per fact. If two docs state the same fact, one of them will rot (see errata below — every
current drift is a duplicated or orphaned fact).

| File | Owns (authoritative for) | Audience |
|---|---|---|
| `CLAUDE.md` | Commands (dev, test, build, deploy), architecture summary, UI conventions, git workflow/release steps, index of `.claude/*.md` | AI sessions |
| `.claude/api-routes.md` | Full route map, auth pattern, view-scoping pattern, error mapping, projection response shape/cache/density | AI sessions |
| `.claude/data-model.md` | Tables, columns, invariants, `fire_settings` JSONB shape, `.ffbackup` schema notes | AI sessions |
| `.claude/engine.md` | Engine public API, `ProjectionInput`/`Output`, simulation loop, inflation model, handler↔engine boundary notes | AI sessions |
| `.claude/auth-and-membership.md` | Auth flow, roles table, cookie attrs, pending users, key auth functions | AI sessions |
| `.claude/env-and-config.md` | Every env var + default, `.env` loading order, Vite config, compose-file matrix | AI sessions |
| `.claude/adding-handler.md` | The canonical new-handler recipe (handler → mod.rs → routes → openapi → migration → test) | AI sessions |
| `.claude/frontend-structure.md` | `apps/web/src/` layout, import rules, "where to add new code" table, prefetch/perf notes | AI sessions |
| `.claude/design-system.md` | Tokens (`--ff-*`, `--proj-*`), palette rules, theme, icon set, rules for new UI | AI sessions |
| `.claude/tests.md` | How to run/write backend integration tests + frontend Vitest, TestApp helpers, shared fixtures, CI status | AI sessions |
| `CHANGELOG.md` | User-facing AND forensic history per release (Keep a Changelog 1.1.0 + SemVer). The project's memory of *why* | Users + future sessions |
| `README.md` | Self-hoster quick start (Docker), update/rollback, env-var table (prod subset), image tag scheme | Self-hosters |
| `.claude/skills/*/SKILL.md` | Encoded judgment + runbooks per topic (this library) | AI sessions |

`CLAUDE.md` deliberately *summarizes* what the `.claude/*.md` docs detail. When you update a
reference doc, check whether the CLAUDE.md summary paragraph for that area still matches.

## 2. Change-type → docs checklist

Run through this at the end of every change. "Doc" columns are cumulative (update all that apply).

| You changed… | Update |
|---|---|
| Route added/removed/renamed, auth requirement, query param | `.claude/api-routes.md` (+ `openapi.rs` schemas in code) |
| Response/request field, serialization format (Decimal-string vs f64 boundary) | `.claude/api-routes.md`; if FIRE/projection shape: `.claude/engine.md` handler-notes section |
| Table/column/constraint, new migration | `.claude/data-model.md` (+ CHANGELOG "Migración / compatibilidad" note if data-affecting — template §4.3) |
| Engine input/output struct, simulation-loop step, inflation semantics | `.claude/engine.md` (+ `futurefin-fire-domain-reference` skill if FIRE math) |
| Env var added/renamed/default changed | `.claude/env-and-config.md` **and** the README env table if it's a prod/self-hoster var **and** `.env.example` |
| Auth flow, role, cookie attr, session TTL | `.claude/auth-and-membership.md` |
| Visual/UI: token, palette, component convention, icon | `.claude/design-system.md` (+ `frontend-structure.md` if a new component/file) |
| Frontend module layout, new view/lib/component file | `.claude/frontend-structure.md` |
| Test infra: TestApp helper, fixture, CI workflow, test command | `.claude/tests.md` |
| Handler-authoring pattern itself (new mandatory step) | `.claude/adding-handler.md` |
| Dev/build/deploy command, git workflow | `CLAUDE.md` |
| Anything a user or self-hoster can observe | `CHANGELOG.md` entry (under `## [Unreleased]` until release) |
| Docker quick start, backup story, supported tags | `README.md` |
| Any fact stated in a `.claude/skills/*/SKILL.md` | that skill, same PR (see §6) |

## 3. House style (extracted from the docs themselves — imitate, don't invent)

### 3.1 Language convention (the honest observed pattern)

- **Code identifiers, API fields, SQL, commands: always English.** UI copy: **Spanish (es-ES)**,
  minimal ("Sin datos.", tab names Resumen/Jubilación/Presupuesto/Próximos/Ajustes).
- **CHANGELOG**: early releases (1.0.0–1.0.7) are English; from ~1.0.8 onward the narrative is
  Spanish with English technical terms embedded untranslated ("sliding TTL", "warm-up post-login",
  "cache hit", "drop limpio"). Match the *current* pattern: Spanish narrative, English tech nouns.
- **`.claude/*.md`**: mixed ES/EN, often mid-file or mid-sentence. Don't "fix" the language of an
  existing doc; write new sections in whichever language the surrounding section uses.
- **Skills** (`.claude/skills/*`): English, keeping Spanish project vocabulary ("sobrante",
  "Jubilación") as-is — it *is* the vocabulary.
- Prefer commands over hard counts in docs ("`ls apps/api/migrations | wc -l`" ages better than
  "33 migrations" — that exact number is one of the current errata). If you must state a count,
  date-stamp it: "31 migrations as of 2026-07-02".

### 3.2 CHANGELOG entries are forensic, not just descriptive

The bar: a future session must be able to reconstruct **symptom → root cause → fix → lesson** from
the entry alone, without git archaeology. "Fixed table overlap" is below the bar. Two exemplary
entries to imitate (verbatim from `CHANGELOG.md`):

> **[1.0.20] — Tablas — fix definitivo del solape en celdas de acciones**: La causa raíz no era ni
> `display: flex` vs `inline-flex` ni la falta de sticky: era que `.budget-row-actions` (con
> `display: inline-flex`) se aplicaba **directamente al `<td>`**, sobreescribiendo el
> `display: table-cell` natural y sacando la celda del modelo de tabla. […] Solución: los botones
> se envuelven ahora en un `<div className="budget-row-actions">` interno […]. Se revierten los
> hacks de v1.0.18–v1.0.19 (sticky, ::before sombra, hover-bg).

(Note it names the two *wrong* prior fixes and reverts them — failed attempts are part of the record.)

> **[1.4.2] — Fix de deflactación del chart**: `ProjectionNetWorthChart` deflactaba cada punto
> usando su índice de array en vez de su `month_index` real. Con densidad `hybrid` (los puntos no
> son equidistantes) esto subestimaba los años transcurridos y deflactaba de menos a partir del mes
> 12 […]. Ahora usa `month_index`, lo que además alinea la curva con los `milestones_real` del
> backend. Para densidad `monthly` el resultado es idéntico (sin regresión).

(Note: mechanism, the condition under which it manifested, the fix, and the no-regression claim.)

### 3.3 Structure and formatting rules

- **Keep a Changelog 1.1.0 + SemVer** (stated at the top of CHANGELOG.md). Newest release first,
  `## [X.Y.Z] — YYYY-MM-DD`, with a `## [Unreleased]` bucket at the top.
- Section headings inside a release: two accepted patterns, both live in the file —
  (a) canonical categories `### Added / Changed / Fixed / Removed / API / Migración / compatibilidad`
  (used through v1.1.x), (b) thematic area headings `### <Área> — <Tema>` (e.g.
  "### Proyección — Milestones ajustados a inflación"; the pattern since v1.2.0). Use (b) for
  feature-rich releases, (a) for small patch releases.
- **Bold-lead-then-detail bullets** everywhere: `- **Short bold claim**: detail sentences.` This is
  the dominant style in CHANGELOG and all `.claude/*.md` docs.
- Mark breaking changes explicitly in the heading or bold lead: "(breaking)", "**API breaking**",
  "**Engine breaking**" (see v1.2.0).
- Migration filenames are always cited inline in backticks (`20260520120000_inflation_always_on.sql`).
- Reference docs cross-link with relative links (`[design-system.md](design-system.md)`), start with
  a one-line scope statement, and prefer tables over prose.
- Monetary/format conventions in doc examples follow the UI rules: `1.234 €`, `3,5 %`.

## 4. Templates (copy, fill, delete unused parts)

### 4.1 CHANGELOG entry skeleton

```markdown
## [X.Y.Z] — YYYY-MM-DD

### <Área> — <Tema en una frase>          <!-- or ### Added / ### Changed / ### Fixed -->

- **<Qué cambia, en negrita, desde el punto de vista del usuario>**: <detalle. Para fixes, formato
  forense obligatorio — síntoma observable → causa raíz (mecanismo exacto, con nombres de
  clase/campo/función en backticks) → fix → por qué no regresiona / lección>. <Si hubo intentos
  fallidos previos, nómbralos y di que se revierten.>
- **Backend**: <campos nuevos del response, helpers, invalidaciones — nombres exactos>.
- **API breaking**: <solo si aplica: qué requests/responses cambian y cómo migra un cliente>.

### Migración / compatibilidad            <!-- only if a migration ships; see 4.3 -->
- <resumen del skeleton 4.3>
```

### 4.2 `.claude` reference-doc section skeleton

```markdown
## <Topic>

<One sentence: what this section is authoritative for, and the code path that implements it.>

| <Thing> | <Key attribute> | Notes |
|---|---|---|
| `exact_identifier` | value | constraint / default / gotcha |

- **<Bold invariant or rule>**: <why it exists; what breaks if violated. Link the incident in
  CHANGELOG (vX.Y.Z) if there is one.>

```rust-or-bash
<copy-pasteable canonical usage — the pattern handlers/views must follow>
```
```

### 4.3 Migration-note skeleton (for any data-affecting change; goes in CHANGELOG + PR description)

```markdown
### Migración / compatibilidad
- **Migración `YYYYMMDDHHMMSS_description.sql`**: <DDL en una frase: qué tabla/columna crea/dropea>.
- **Datos**: <"sin pérdida de datos" | "DROP limpio SIN migración de datos: se pierde <qué>;
  el usuario debe <reconfigurar cómo>. Firmado por el owner en <PR/fecha>">. <!-- data loss
  requires explicit owner sign-off — futurefin-change-control -->
- **Backups `.ffbackup`**: <sin cambio | `schema_version` sube a N; vN−1 se migra vía
  `migrate_to_current` descartando/transformando <campos>>.
- **Primer arranque tras actualizar**: <qué verá el usuario; pasos manuales si los hay>.
- **Rollback**: <qué pasa si se vuelve a la imagen anterior con la migración ya aplicada>.
```

Model to imitate: the v1.1.0 "Migración / compatibilidad" section (allocation_rules drop).

## 5. Release documentation ritual

The full release gate (what to test before tagging) is owned by
`.claude/skills/futurefin-change-control/SKILL.md`. The **writing** half, from CLAUDE.md "Git
workflow", in order:

1. On `dev`: move `## [Unreleased]` content into a new `## [X.Y.Z] — YYYY-MM-DD` section
   (today's date), leaving an empty `[Unreleased]` bucket.
2. Bump `version` in `apps/api/Cargo.toml` **and** sync `Cargo.lock` (a `cargo build` regenerates
   it; committing Cargo.toml without the lock is a classic miss).
3. Verify every entry in the new section meets the forensic bar (§3.2) and that every migration in
   the release has a §4.3 note.
4. Full merge `dev` → `main` (`git checkout main && git merge dev`) — `main` is a complete mirror,
   never partial file copies.
5. Tag `vX.Y.Z` from `main` → `publish-image.yml` publishes `:X.Y.Z`, `:X.Y`, `:X`, `:latest`.
6. Back to `dev`; keep it up to date with `main`.

SemVer signal: breaking API change ⇒ at least minor bump + explicit "breaking" entry (owner-endorsed
rule; v1.2.0 is the precedent).

## 6. Skill-library maintenance

- Every skill in `.claude/skills/` ends with a **Provenance and maintenance** section: one-line
  read-only commands that re-verify its volatile facts. When you touch a skill's area, run its
  provenance lines; any mismatch means the skill must be updated **in the same PR** as the change.
- Skills date-stamp volatile facts ("as of 2026-07-02, v1.4.3"). When you re-verify, refresh the
  date even if nothing changed.
- One home per fact across the library: if your edit would duplicate a sibling skill's topic,
  summarize in ≤2 sentences and cross-reference the sibling instead.
- New recurring runbook with no home → propose a new skill dir under `.claude/skills/<name>/SKILL.md`
  with trigger-rich frontmatter, a "When NOT to use" section, and provenance — same shape as this file.

## 7. STANDING ERRATA — known doc drift, verified 2026-07-02

These are facts the docs of record currently get **wrong**. Do not propagate them. Fix each one the
next time you touch its file (fixing them is in-scope doc maintenance, not a behavior change).
Until fixed, trust this table and the code over the named doc.

| # | Doc & location | It says | Reality (verified in repo) |
|---|---|---|---|
| 1 | `.claude/tests.md` §CI | "There is no CI yet" | `.github/workflows/ci.yml` exists and runs on push/PR to `main`/`dev`: `cargo build -p futurefin-api --locked`, `cargo test -p futurefin-engine --locked`, npm typecheck + build, and a full Docker-stack build + `/v1/health` smoke test. **Still true**: the Postgres integration tests (`apps/api/tests/`) and frontend Vitest are NOT run in CI — run them locally with `TEST_DATABASE_URL`. |
| 2 | `.claude/tests.md` §Integration tests | "applies all 33 migrations" | 31 files in `apps/api/migrations/` as of 2026-07-02. When fixing, replace the number with the command `ls apps/api/migrations \| wc -l` or date-stamp it. (Related: tests.md says "146 tests", CHANGELOG v1.3.0 says 156 — counts disagree; prefer commands over counts.) |
| 3 | `.claude/data-model.md` §installation + §Key invariants | `installation` has `projection_target_age (smallint nullable)`; invariant "bounded 65–105"; `budget_entries.persists_after_retirement` "continues after `projection_target_age`" | Column dropped by `20260516120000_drop_projection_target_age.sql` (CHANGELOG v1.0.6); zero grep hits in `apps/api/src` + `crates`. FIRE crossover is the **sole** retirement trigger; "after retirement" means after the FIRE-crossover month. |
| 4 | `.claude/engine.md` §Notes for the API handler | "Horizon derivation: `projection_target_age` → …; fallback 240 months"; "handler computes `retirement_start_month` from `projection_target_age` + birth date"; `RetirementInput { target_age, … }` | Horizon = years until age **90** from ONE resolved birth date — the session user's `users.birth_date`, else the first `persons` row by `is_primary DESC, sort_index ASC` (NOT the oldest member) — clamped **5–70** years; **30-year** fallback with no birth date (`projection_horizon_months` in `apps/api/src/handlers/projection.rs`). No age-based retirement input exists. |
| 5 | `.claude/api-routes.md` §Installation + §Projection | `PATCH /v1/installation` "updates … target_age …"; `horizon_basis` values `mac_target_age \| mac_fallback_no_demographics \| months_override` | No `target_age` anywhere. Actual `horizon_basis` strings emitted: `lifespan_90`, `fallback_no_demographics`, `months_override` (the doc comment on `horizon_basis` inside `projection.rs` is stale too). |
| 6 | `.claude/auth-and-membership.md` line 3 | "Full spec: `docs/spec/AUTH_MODEL.md`" | `docs/` does not exist in the repo. The doc itself + the code are the spec; delete the dangling reference when touching the file. |
| 7 | `README.md` §Backups | "El API expone `GET /v1/backup/export.zip` (solo owner)" | Route removed. Current backup surface (routes/mod.rs): `POST /v1/backup/user-export`, `POST /v1/backup/user-import/preview`, `POST /v1/backup/user-import` — per-user encrypted `.ffbackup`, any role can export (see api-routes.md). |
| 8 | `CLAUDE.md` §Development (split-dev), also mirrored in `README.md` | Dev DB start command: `docker compose up -d futurefin-database` | That command starts the DB but exposes NO host port (`docker-compose.yml` maps no `ports:` on `futurefin-database`), so the host-side `cargo run` cannot connect to `127.0.0.1:5432`. The working command adds the split-dev override: `docker compose -f docker-compose.yml -f docker-compose.split-dev.yml up -d futurefin-database` (verified 2026-07-02; see futurefin-build-and-env §2/T4). |

Reporting a NEW drift: when you find one, verify against code, then add it to this table (with
"verified <date>") if you cannot fix the doc in the same change — never leave a known-wrong fact
unrecorded.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 by reading: `CLAUDE.md`, all 9 `.claude/*.md`, `CHANGELOG.md`,
`README.md`, `.github/workflows/ci.yml`, `apps/api/src/routes/mod.rs`,
`apps/api/src/handlers/projection.rs`, `apps/api/migrations/`. Re-verify with:

- Current version: `grep -n '^version' apps/api/Cargo.toml` (and top entry of `CHANGELOG.md`)
- Migration count (errata #2): `ls apps/api/migrations | wc -l`
- Errata #1 still applies?: `grep -n "no CI yet" .claude/tests.md` (empty ⇒ fixed, remove row)
- Errata #3/#4/#5 still apply?: `grep -rn projection_target_age .claude/ apps/api/src crates`
  (hits only under `.claude/` ⇒ docs still stale; hits in code would invalidate the errata)
- Errata #6 still applies?: `grep -n AUTH_MODEL .claude/auth-and-membership.md; ls docs 2>&1`
- Errata #7 still applies?: `grep -n export.zip README.md; grep -n backup apps/api/src/routes/mod.rs`
- Errata #8 still applies?: `grep -n "up -d futurefin-database" CLAUDE.md README.md; grep -n "ports" docker-compose.yml docker-compose.split-dev.yml`
- CI scope claim: `cat .github/workflows/ci.yml` (integration tests/Vitest still absent?)
- Doc inventory unchanged (9 reference docs): `ls .claude/*.md | wc -l`
- Exemplar quotes intact: `grep -n "fix definitivo del solape" CHANGELOG.md` and
  `grep -n "Fix de deflactación del chart" CHANGELOG.md`
- Release steps unchanged: `grep -n "Merge completo" CLAUDE.md`
- Skill inventory for cross-refs: `ls .claude/skills/`
