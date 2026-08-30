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

Facts below verified against the repo on **2026-07-02, v1.4.3** (`apps/api/Cargo.toml`),
re-verified **2026-08-16 for v3.0.0** (self-contained Docker image), again **2026-08-22 for
4.0.0** (the public-release audit) and again **2026-08-28 for 4.4.0** (MCP Fase 4, issue #85),
which is where §7's errata rows come from. This skill
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
| `.claude/api-routes.md` | Full route map, auth pattern, view-scoping pattern, error mapping, projection response shape/cache/density, **the MCP tool catalog (§MCP: per-tool semantics, cache class, preview/confirm)** | AI sessions |
| `.claude/data-model.md` | Tables, columns, invariants, `fire_settings` JSONB shape, `.ffbackup` schema notes | AI sessions |
| `.claude/engine.md` | Engine public API, `ProjectionInput`/`Output`, simulation loop, inflation model, handler↔engine boundary notes | AI sessions |
| `.claude/auth-and-membership.md` | Auth flow, roles table, cookie attrs, pending users, key auth functions | AI sessions |
| `.claude/env-and-config.md` | Every env var + default (API binary **and**, since 3.0.0, the container entrypoint's `FUTUREFIN_*` / `POSTGRES_*` vars), `.env` loading order, Vite config, and the compose-file matrix — which since 3.0.0 is `docker-compose.yml` (production, **one service** with PostgreSQL inside the image), `docker-compose.local.yml` (`pull_policy: never` for a locally built image) and `docker-compose.dev.yml` (standalone dev Postgres on 127.0.0.1:5432, replacing the deleted `docker-compose.split-dev.yml`) | AI sessions |
| `.claude/adding-handler.md` | The canonical new-handler recipe (handler → mod.rs → routes → openapi → migration → test) | AI sessions |
| `.claude/frontend-structure.md` | `apps/web/src/` layout, import rules, "where to add new code" table, prefetch/perf notes | AI sessions |
| `.claude/design-system.md` | Tokens (`--ff-*`, `--proj-*`), palette rules, theme, icon set, rules for new UI | AI sessions |
| `.claude/tests.md` | How to run/write backend integration tests + frontend Vitest, TestApp helpers, shared fixtures, CI status | AI sessions |
| `CHANGELOG.md` | User-facing AND forensic history per release (Keep a Changelog 1.1.0 + SemVer). The project's memory of *why* | Users + future sessions |
| `README.md` | Self-hoster quick start (Docker), update/rollback, env-var table (prod subset), image tag scheme | Self-hosters |
| `SECURITY.md` | Supported versions, private disclosure channel, and the **honest** security posture of a self-hosted install: no TLS, no login rate limiting, open registration, open DCR, `?view=mine` is not an authorization boundary, what `.ffbackup` encryption does and does not protect. Every claim must be verified in code — the file says so itself. **The out-of-scope list is a contract**: adding a behavior there means "reporting this gets a link, not a fix" | Self-hosters + security reporters |
| `docs/*.md` (`instalacion`, `actualizar`, `configuracion`, `backups`, `desarrollo`, `mcp`) | The user-facing manual the public README links to. Same accuracy bar as `.claude/*.md`; different audience (people, not sessions) | Self-hosters |
| `.claude/skills/*/SKILL.md` | Encoded judgment + runbooks per topic (this library) | AI sessions |

`CLAUDE.md` deliberately *summarizes* what the `.claude/*.md` docs detail. When you update a
reference doc, check whether the CLAUDE.md summary paragraph for that area still matches.

## 2. Change-type → docs checklist

Run through this at the end of every change. "Doc" columns are cumulative (update all that apply).

| You changed… | Update |
|---|---|
| Route added/removed/renamed, auth requirement, query param | `.claude/api-routes.md` (+ `openapi.rs` schemas in code) **and the MCP parity evaluation** (futurefin-mcp-parity §1: tool, recorded omission, or n/a) |
| MCP tool added/changed/omitted | `.claude/api-routes.md` §MCP (catalog) + CLAUDE.md counters (module map + MCP paragraph) + `.claude/tests.md` suite rows + futurefin-mcp-parity §3 register |
| Response/request field, serialization format (Decimal-string vs f64 boundary) | `.claude/api-routes.md`; if FIRE/projection shape: `.claude/engine.md` handler-notes section |
| Table/column/constraint, new migration | `.claude/data-model.md` (+ CHANGELOG "Migración / compatibilidad" note if data-affecting — template §4.3) |
| Engine input/output struct, simulation-loop step, inflation semantics | `.claude/engine.md` (+ `futurefin-fire-domain-reference` skill if FIRE math) |
| Env var added/renamed/default changed | `.claude/env-and-config.md` **and** the README env table if it's a prod/self-hoster var **and** `.env.example` |
| Auth flow, role, cookie attr, session TTL | `.claude/auth-and-membership.md` **and `SECURITY.md`** if it changes what a self-hoster is exposed to (a new credential, a new revocation path, a new open endpoint) |
| A metric's base, window, denominator, exclusions, or its name | `apps/web/src/lib/helpTexts.ts` + `futurefin-metric-definitions` (§2 gate: text updated / entry added-removed / reasoned n/a — never silent). **Also when you did not think you were touching a metric**: changing which rows enter an aggregate, or renaming a tab a help text cites, moves the meaning of a metric by ricochet — that is how six entries drifted by 4.0.0 |
| Visual/UI: token, palette, component convention, icon | `.claude/design-system.md` (+ `frontend-structure.md` if a new component/file) |
| Frontend module layout, new view/lib/component file | `.claude/frontend-structure.md` |
| Test infra: TestApp helper, fixture, CI workflow, test command | `.claude/tests.md` |
| Handler-authoring pattern itself (new mandatory step) | `.claude/adding-handler.md` |
| Dev/build/deploy command, git workflow | `CLAUDE.md` |
| Container behavior: `Dockerfile`, `apps/api/docker-entrypoint.sh`, any `docker-compose*.yml` (embedded PG, automatic backups, shutdown, upgrade paths) | `.claude/env-and-config.md` (entrypoint vars + compose matrix) **and** `README.md` (self-hoster steps: quick start, "Actualizar", "Actualizar desde 2.x") **and** `CLAUDE.md` if a command changed **and** `CHANGELOG.md` |
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
  "33 migrations" — that frozen number sat wrong in tests.md until 2026-07-02). If you must
  state a count, date-stamp it: "31 migrations as of 2026-07-02".

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

### 3.2b Worked figures are INVENTED but arithmetically coherent

Forensic entries carry before/after tables — that stays. What may never appear is a real
installation's data. **Never** write "sobre una instalación **real**" or "datos reales de una
instalación": write **«sobre una instalación de ejemplo»** and make up the numbers.

Made-up does not mean sloppy: if the table says `540,00 ÷ 6` and `540,00 ÷ 3`, the cells must read
90 and 180. An example that does not add up is worse than none — the reader stops trusting the
whole entry. INCIDENT (August 2026): entries for 3.9.0 and the issue-#5 fix reasoned over the
owner's live installation, publishing his rent, monthly income and savings rate in a repo about to
go public. See [`futurefin-data-hygiene`](../futurefin-data-hygiene/SKILL.md) §4.

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
4. PR contra `main` con CI verde (no hay `dev` desde 4.0.1: una sola rama viva).
5. **El merge del bump publica solo** (auto-tag on merge, 4.0.6): `publish-image.yml` espera la
   CI del commit, crea `vX.Y.Z` y publica `:X.Y.Z`, `:X.Y`, `:X`, `:latest` (móviles por rango).

SemVer signal: breaking API change ⇒ at least minor bump + explicit "breaking" entry (owner-endorsed
rule; v1.2.0 is the precedent).

## 6. Skill-library maintenance

- Every skill in `.claude/skills/` ends with a **Provenance and maintenance** section: one-line
  read-only commands that re-verify its volatile facts. When you touch a skill's area, run its
  provenance lines; any mismatch means the skill must be updated **in the same PR** as the change.
  Rule re-checked **2026-08-22: all 18 skills comply** —
  `for f in .claude/skills/*/SKILL.md; do grep -qiE '^## ([0-9]+\. )?(Provenance|Procedencia)' "$f" || echo "MISSING: $f"; done`
  prints nothing. **Ojo con el comando viejo**: era `grep -qi '^## .*Provenance'`, y daba un falso
  positivo con `futurefin-data-hygiene`, cuya sección se llama «## 7. Procedencia y mantenimiento» —
  una comprobación que marca como incumplidor a quien sí cumple enseña a ignorarla. A new skill
  without that section is incomplete.
- Skills date-stamp volatile facts ("as of 2026-07-02, v1.4.3"). When you re-verify, refresh the
  date even if nothing changed.
- One home per fact across the library: if your edit would duplicate a sibling skill's topic,
  summarize in ≤2 sentences and cross-reference the sibling instead.
- New recurring runbook with no home → propose a new skill dir under `.claude/skills/<name>/SKILL.md`
  with trigger-rich frontmatter, a "When NOT to use" section, and provenance — same shape as this file.

## 7. STANDING ERRATA — known doc drift

**No known drift as of 2026-07-02.** The eight errata found during the skill-library authoring
(stale "no CI yet" + "33 migrations" in tests.md; `projection_target_age` remnants in
data-model.md, engine.md and api-routes.md; the stale `mac_*` `horizon_basis` doc comment in
`projection.rs`; the dangling `docs/spec/AUTH_MODEL.md` reference; README's removed
`GET /v1/backup/export.zip`; and the split-dev DB command missing the override in CLAUDE.md and
README) were **all fixed in the docs-of-record on 2026-07-02** in the same change that made
CLAUDE.md the single entry point.

**Re-swept 2026-08-16 for v3.0.0 — still no known drift.** The self-contained-image release
rewrote the deployment story (single container, no `futurefin-database` service, no
`docker-compose.split-dev.yml`, `POSTGRES_PASSWORD` no longer required, healthcheck on
`/v1/ready`), which is exactly the kind of change that historically leaves stragglers. A sweep of
`grep -rn 'split-dev\|futurefin-database\|POSTGRES_PASSWORD' --include='*.md' .` on 2026-08-16
found **every** remaining hit to be legitimate: correct 2.x history (CHANGELOG; "the
`--remove-orphans` retires the old `futurefin-database` container"), an explicit "this no longer
exists / is deleted" note, the unrelated test-DB `docker run -e POSTGRES_PASSWORD=futurefin_test`,
or **"split-dev" as the name of the still-current workflow** (`cargo run` + Vite) — only the
`docker-compose.split-dev.yml` *file* is gone, replaced by the standalone `docker-compose.dev.yml`.
Nothing to record.

**Swept 2026-08-22 for 4.0.0 (public release + pre-publication audit) — the largest drift harvest so
far, all fixed in the same change.** Every item below was a document asserting something the code
had stopped doing:

- **`.claude/tests.md` and `futurefin-validation-and-qa` both said the integration suite, ESLint and
  Vitest do NOT run in CI.** They do since 4.0.0 (`ci.yml` job `integration` with a Postgres service;
  job `web` runs `lint:web` and Vitest). The docs had been *right for months* and became wrong on the
  day the gap was closed — the failure mode of every "known limitation" paragraph.
- **The same two files claimed unit tests for `handlers/transactions/` (CSV presets, fingerprint,
  rule precedence) that had never existed.** `csv_presets.rs` and `import.rs` carried zero `#[test]`,
  and a money bug lived in exactly that hole (`2100.00` imported as `210000`). A row describing tests
  that do not exist is worse than a missing row: it stops anyone from writing them.
- **Frozen counters, again**: engine 61 → 67, integration 27 files → 33, migrations 40 → 42, Vitest
  321/13 → 368/16, `.ffbackup` schema 8 → 9, MCP catalog 50 → 52. Third release train in a row.
- **`futurefin-mcp-parity` said inflation was "read-only via MCP"** — `update_fire_settings` has been
  persisting `annual_inflation_assumption_percent` for a while. The row made a covered axis look like
  an open gap.
- **UI names moved and the docs did not**: `Ajustes → MCP` is `Ajustes → Integraciones`,
  `Ajustes → Proyección` is `Ajustes → Plan`, and the theme toggle lives in `Ajustes → General`, not
  `Ajustes → Datos y sistema`. Hits in `frontend-structure.md`, `design-system.md`,
  `auth-and-membership.md`, `data-model.md`, `config-and-flags`, `architecture-contract`,
  `debugging-playbook`, `change-control` and `CLAUDE.md`. **The fix that sticks**: cite
  `SETTINGS_SUBTAB_LABEL` (`apps/web/src/lib/navigation.ts`) as the source, not the label you
  remember.
- **`frontend-structure.md` documented a «Conciliar ahora» button** that was removed when the
  periodic server sweep landed, and **`design-system.md` listed `--proj-jub` as a live token** with
  zero consumers.
- **`SECURITY.md` described `.ffbackup` behavior "if you change your password later"** for a
  password-change endpoint that did not exist, and `auth-and-membership.md` promised that revoking a
  membership cuts access with no way to revoke one. Both promises are now true — but they were
  written *before* the code, which is the one direction this library must never take: a doc may lag
  the code; it may not lead it.

**Lesson added to §3.1's "prefer commands over hard counts"**: the same applies to *negative* claims.
"X does not run in CI", "there are no unit tests for Y", "Z is read-only" all age exactly like a
frozen number, and unlike a number nobody ever re-counts them. Write the command that proves the
claim next to the claim.

**Swept 2026-08-17 for v3.1.0 (embedded OAuth 2.1) — drift found and fixed, none left.**
Unlike the two sweeps above, this one *did* turn something up, and it is a pattern worth naming:
**the 3.0.0/MCP release bumped code but not the frozen counters in the skills**. Corrected in the
same change: `futurefin-change-control`'s header (34 migrations / 20 integration-test files → 36 / 23)
and its provenance version stamp (3.0.0 → 3.1.0); `futurefin-validation-and-qa`'s suite table
(159 tests / 20 files → 206 / 23; Vitest 293 / 11 files → 309 / 12; migration count 34 → 36; three
inventory rows missing since the MCP release — `api_tokens.rs`, `mcp_http.rs` — plus the new
`oauth_flow.rs`); this file's own provenance (34 → 36, 2.3.0 → 3.1.0); `.claude/tests.md`'s Vitest
total and migration stamp. Two content-level drifts also fixed: `.claude/api-routes.md` and
`.claude/auth-and-membership.md` both said the `/mcp` failure adds a bare `WWW-Authenticate: Bearer`
on 401 *and* 403 — true at 3.0.0, wrong at 3.1.0 (only the 401, and it now carries
`realm` + `resource_metadata`). **Lesson**: §3.1's "prefer commands over hard counts" is not a style
preference — every count frozen without a date-stamp in this library has now been wrong at least once.

**Swept 2026-08-28 for MCP Fase 4 (issue #85/#88) — two source-comment errata found, left
unfixed because this pass was documentation-only (`apps/` was explicitly out of scope for the
change that produced it).** Both are Rust doc-comments, not `.claude/*.md`/CHANGELOG prose, which
is exactly the case this table exists for: a disagreement you find but cannot fix in the same
change.

The rule stands: when you find a doc/code disagreement, verify against code (the code is ground
truth), then fix the doc in the same change. If you cannot, add a row here with "verified <date>"
— never leave a known-wrong fact unrecorded:

**Barrido 2026-08-28 para MCP Fase 6 (issue #87) — siete erratas nuevas, todas del lado del CÓDIGO
o de un issue abierto, y por eso registradas en vez de arregladas**: la pasada que las encontró era
documentación-only, con `apps/` y `crates/` explícitamente fuera de alcance. Las que dependen de un
issue abierto llevan su número: **arreglar la doc antes que el código dejaría la doc describiendo un
bug que sigue vivo**, que es peor que la errata.

**Retiradas el 2026-08-30 (auditoría de coherencia)**: las antiguas filas 1 y 2 de esta tabla —
el hueco `SinkPolicy` de `patch_allocation_rule_core` y el «default 15, max 60» de
`suggest_transfer_matches.window_days` — **ya están arregladas en el código**: la core de patch
recibe `SinkPolicy` y la tool pasa `Forbidden` (guarda `sink_creation_not_allowed` en
`allocation_rules.rs`, Fase 6), y schema+core del `window_days` dicen ambos 1–365 default 30
(`DEFAULT_SUGGEST_WINDOW_DAYS`/`MAX_SUGGEST_WINDOW_DAYS`, Fase 7). Se borran las filas según la
norma de abajo — una errata registrada es deuda, no un archivo.

| # | Doc & location | It says | Reality (verified in repo) |
|---|---|---|---|
| 3 | `apps/web/src/lib/errorMessages.ts`, `idempotency_key_batch_unsupported` | «La clave de idempotencia solo vale para dar de alta un movimiento suelto, **no para un lote**» | **Dejó de ser cierto con la Fase 6**: el lote SÍ acepta clave, en la **raíz** del body. Lo que se rechaza es la clave **por ítem**. El mensaje inglés del servidor ya se corrigió («put idempotency_key at the root of the batch body»); la traducción de la SPA no. Verificado 2026-08-28 |
| 4 | `apps/web/src/lib/errorMessages.ts`, `idempotency_key_invalid` | «entre 1 y 200 caracteres» | Hay **dos** cotas desde la Fase 6: **200** en el alta individual y **180** en el lote (`MAX_BATCH_KEY_CHARS`, el margen es para el sufijo derivado `#b{i}`). Verificado 2026-08-28 |
| 5 | `apps/api/tests/query_param_validation.rs`, `VIEW_ROUTES` | La tabla enumera las rutas que aceptan `?view=` | **Le faltan dos de la Fase 6**: `/v1/changes` y `/v1/allocation-rules/goals` aceptan `view` y no están en la tabla (el diff solo añadió `aggregate` y `duplicates`). Es un hueco de cobertura del test que existe precisamente para cazar el enum que cae al default en silencio. Verificado 2026-08-28 |
| 7 | `apps/api/tests/mcp_http.rs`, doc-comments de siete tests («las 52 tools», «51 de las 52») — **no `server.rs`, que no tiene ninguna** | 7 menciones a un catálogo de 52 | Son **68** desde la Fase 6. Las siete están en `apps/api/tests/mcp_http.rs` (líneas ~2219, 2399, 2471, 2551, 2562, 2592, 2608) y describen mediciones **fechadas** de fases anteriores, así que varias son históricamente correctas — pero al menos dos (`tools_list_freezes_…` y el tope por descripción) hablan del estado **actual**. Verificado 2026-08-28 |

| 9 | `apps/api/src/handlers/projection.rs`, comentario junto a los ejes de `liability_overrides` | Dice que esos ejes no mueven el objetivo FIRE «ni las bases de los caps» | La segunda mitad es falsa: `debt_service` incluye ahora la amortización extra y el techo de un cap `months_expense` es `N × (expense + debt_service)`, así que **sí se mueve** en un what-if que amortiza. Efecto de segundo orden, alcanzable solo desde `simulate_projection` y **sin test que lo cubra**; documentado como tal en `.claude/engine.md` §AllocationRule y en `futurefin-fire-domain-reference` §5. Verificado 2026-08-28 |
| 10 | `apps/api/src/handlers/projection.rs`, doc-comment de `deflator_at_month_index` | «Un único helper con **dos** callers» | Son **cuatro**: `deflate_points_to_today` (→ `milestones_real`), `points[].net_worth_real`, `final_net_worth_real` de `simulate_projection` (+ su delta) y `deflate_amount_core`. La afirmación de unicidad sigue siendo cierta y es la que importa; lo que caducó es el recuento. Verificado 2026-08-28 |
| 11 | `apps/api/tests/mcp_http.rs`, doc-comment de `tool_descriptions_stay_within_the_context_budget` | «con **cinco** por encima de 1.200» (medición pre-Fase 5) | Son **seis** en el fixture de aquel commit. Medición histórica congelada en prosa, del tipo que §3.1 desaconseja: es reproducible (`git show 51b7675:apps/api/tests/fixtures/mcp-catalog.json`) y aun así se escribió a mano. Verificado 2026-08-28 |


**Barrido 2026-08-29 para MCP Fase 7 (issue #88) — ocho erratas nuevas, TODAS del lado del CÓDIGO**
(la pasada volvía a ser documentación-only). Seis son contadores que la Fase 3 escribió en prosa y
que la Fase 6 movió sin que nadie los tocara; las otras dos son peores porque **desarman su propia
comprobación**:

| # | Doc & location | It says | Reality (verified 2026-08-29) |
|---|---|---|---|
| 12 | `apps/api/src/mcp/server.rs`, doc-comment de `DeleteWithTokenParams` (≈L1720/L1724) | Prescribe contar las tools de dos fases con `grep -c 'confirm_token.as_deref' apps/api/src/mcp/server.rs` y dice que da **8** | Ese comando da **10**: el propio comentario contiene la cadena dos veces —una de ellas es la línea que prescribe el grep—, así que **el patrón se cuenta a sí mismo**. Contador auto-referencial. Comandos que sí dan 8: `grep -c '= two_phase('` o `grep -c 'p\.confirm_token\.as_deref()'`. Ironía documentada: ese mismo comentario advierte de que «enumerarla a mano en prosa ya se quedó corta una vez». `futurefin-mcp-parity` §5 ya usa el comando bueno |
| 13 | `apps/api/src/confirm_token.rs` (≈L35) | «`grep -c 'confirm_token.as_deref()' apps/api/src/mcp/server.rs` da las **7**» | El **comando** es correcto (con paréntesis no se autocuenta: da **8**); el **número** es de antes de la Fase 6. Cambiar solo el 7 por un 8 basta |
| 14 | `apps/api/src/confirm_token.rs` (≈L32) | «no se exige en las **14** tools con preview» | **17** (`grep -c 'p.confirm.unwrap_or(false)'`) |
| 15 | `apps/api/src/confirm_token.rs` (≈L40) | «`apps/api/src/mcp/` no contiene SQL salvo el `SELECT` del kill-switch en `auth.rs`» | `grep -c 'sqlx::query' apps/api/src/mcp/auth.rs` → **4** desde la Fase 3: el SELECT del toggle **más** el INSERT, el UPDATE y la poda de `mcp_write_audit`. El invariante que sigue siendo absoluto es el de `server.rs` (**0**), y así lo formula ya `futurefin-mcp-parity` §4 paso 1 |
| 16 | `apps/api/src/mcp/auth.rs` (≈L233) | «este gate es el único punto por el que pasan las **31** escrituras» | **40** (`grep -c 'read_only_hint = false' apps/api/src/mcp/server.rs`) |
| 17 | `apps/api/src/mcp/server.rs` (≈L3566) | «Forma común de los **14** previews» | **17** |
| 18 | `apps/api/src/mcp/server.rs` (≈L401, doc de `NoParams`) | «para que su `inputSchema` publique `additionalProperties: false` como el de las otras **48**» | Son **67** las otras (68 tools). El 48 es de un catálogo de 49; ni siquiera casa con el 52 de la Fase 2 |
| 19 | `apps/api/tests/mcp_write.rs` (≈L2524) | Comentario: «Las **7** que además exigen el token» | Su propio `assert_eq!(…, 8)` dos líneas más abajo. **El comentario contradice al assert que acompaña**, que es la peor variante: el test está bien y el humano que lo lea se lleva el número malo |

Dos observaciones de esta pasada que **no** son erratas de un número sino asimetrías de diseño,
anotadas por si el owner quiere issue:

- **`confirm_token.rs` ≈L216 vs L219**: el doc dice que la poda barre «los caducados **y
  consumidos**», pero el `DELETE` filtra solo `expires_at < now()`. No es un bug (un token consumido
  se va cuando caduca, ≤10 min después), pero la frase promete una segunda condición que no existe.
- **Owner-only comprobado en dos sitios distintos**: `update_fire_settings` lo comprueba **en la
  tool** (`server.rs`), `update_installation_settings` **en la core** (`installation.rs`), y el
  comentario de esta última dice explícitamente que la core es el sitio correcto «así una superficie
  nueva no puede dejárselo». `patch_fire_settings_core` no comprueba owner, así que por HTTP lo pone
  el handler: **dos comprobaciones paralelas del mismo permiso**, que es el patrón que D14 llama
  *dual-branch drift*.

**Retirada en esta pasada**: la antigua fila 8 (el `instructions` del servidor enumeraba siete tools
con `confirm_token` y omitía `delete_allocation_rule`) — **ya está arreglada en el código**: el
bloque ESCRITURA de `instructions` lista las ocho. Se borra la fila, según la norma de abajo.

Tres **incoherencias preexistentes** con issue abierto que esta documentación describe tal cual son,
sin propagar la promesa rota:

| Issue | Dónde muerde | Qué documenta esta pasada |
|---|---|---|
| **#95** | `PATCH /v1/assets`: el doc-comment de `purchase_price` (que viaja a OpenAPI) promete que `null` borra el precio de compra, y esa rama es **inalcanzable por HTTP** — serde colapsa `null` presente y clave ausente en `None`, así que sale 400 `patch_empty` | `.claude/api-routes.md` §Assets lo dice explícitamente al presentar la plusvalía latente, nombra la vía viva (el flag `clear_purchase_price` de la tool MCP) y el test que fija el 400 actual. **La doc NO repite la promesa rota** |
| **#96** | El motor emite `cap_ceiling: null` para todas las reglas cuando no hay sobrante, así que un consumidor no distingue «sin tope» de «no lo calculé» | `.claude/api-routes.md` §Allocation rules explica por qué `resolve_cap_ceiling_eur` existe **fuera** del motor y por qué el test que cruza las dos definiciones solo puede hacerlo en el camino donde el motor sí publica el techo |
| **#97** | `update_allocation_rule` devuelve `{id, antes, despues}` (claves en español) y pide `rule_id` donde el resto del catálogo usa `id` | No se documenta como si estuviera arreglado. La afirmación de `futurefin-mcp-parity` §4 de que `resumen`→`summary` fue «la última clave en español del wire» **era falsa** y el issue lo recoge; esta pasada no la repite en ningún sitio nuevo |

Las dos erratas que vivían aquí antes (`TOKEN_GATED_TOOLS` inexistente en `confirm_token.rs`, y el
wire descrito como `{error, message}` en dos doc-comments de `mcp/server.rs`) se **arreglaron** el
2026-08-28 en la Fase 4 del issue #85, junto con el literal de fallback de `to_tool_outcome`, que
construía un error sin `code`. Una errata registrada aquí es deuda, no un archivo: cuando el arreglo
cabe en el mismo cambio, se arregla y la fila se borra.

## Provenance and maintenance

Verified 2026-07-02 against v1.4.3 by reading: `CLAUDE.md`, all 9 `.claude/*.md`, `CHANGELOG.md`,
`README.md`, `.github/workflows/ci.yml`, `apps/api/src/routes/mod.rs`,
`apps/api/src/handlers/projection.rs`, `apps/api/migrations/`. Re-verified **2026-08-16 for
v3.0.0** (§1 env-and-config row, §2 container row, §7 sweep) against `README.md`, `CLAUDE.md`,
`.claude/env-and-config.md`, `docker-compose*.yml` and `apps/api/docker-entrypoint.sh`.
Re-verify with:

- Current version: `grep -n '^version' apps/api/Cargo.toml` (and top entry of `CHANGELOG.md`).
  **4.0.0 on 2026-08-22, with its `## [4.0.0] - 2026-08-22` CHANGELOG section already written**
  (release ritual §5; gates in futurefin-change-control §4)
- Migration count: `ls apps/api/migrations | wc -l` (**42 on 2026-08-22**; 36 on 2026-08-17)
- Doc inventory: `ls .claude/*.md | wc -l` (**9**) and `ls .claude/skills | wc -l` (**18**)
- Compose matrix matches the §1 env-and-config row: `ls docker-compose*.yml` (yml / local / dev —
  **no `split-dev`**) and `awk '/^services:/{f=1;next} /^volumes:/{f=0} f && /^  [a-z]/' docker-compose.yml` (one service)
- §7 sweep reproducible: `grep -rn 'split-dev\|futurefin-database\|POSTGRES_PASSWORD' --include='*.md' .`
  — every hit must be 2.x history, an explicit "no longer exists" note, or the test-DB `docker run`
- Old errata stay fixed? (all must return empty/clean): `grep -n "no CI yet" .claude/tests.md`;
  `grep -rn projection_target_age .claude/*.md apps/api/src crates` (**two** historical mentions are
  expected, not one: data-model.md's "eliminada" note and engine.md's horizon-derivation note — cero
  en `apps/api/src` y `crates`, que es lo que el check realmente vigila); `grep -n AUTH_MODEL .claude/auth-and-membership.md`;
  `grep -n export.zip README.md`; `grep -rn mac_target_age apps/api/src .claude/*.md`
- CI scope claim — **since 4.0.0 both must PRINT something** (they used to have to print nothing;
  see §7's lesson on negative claims):
  `grep -n TEST_DATABASE_URL .github/workflows/ci.yml` and
  `grep -n 'npm test\|lint:web' .github/workflows/ci.yml`
- UI names cited in docs come from the code, not from memory:
  `grep -n -A 9 'SETTINGS_SUBTAB_LABEL' apps/web/src/lib/navigation.ts`
- Exemplar quotes intact: `grep -n "fix definitivo del solape" CHANGELOG.md` and
  `grep -n "Fix de deflactación del chart" CHANGELOG.md`
- Release steps unchanged: `grep -n "El merge del bump ES la publicación" CLAUDE.md` (**la cita vieja, «Merge completo», ya no existe**: el ritual se reescribió para el auto-tag-on-merge de 4.0.6, así que el grep anterior salía vacío y se leía como «la sección desapareció»)
- Skill inventory for cross-refs: `ls .claude/skills/`
