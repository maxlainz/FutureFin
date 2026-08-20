---
name: futurefin-mcp-parity
description: >
  Load this skill whenever a change touches the surface of FutureFin's API or its embedded MCP
  server: adding/removing/renaming a route or handler, changing a request/response field, adding
  an MCP tool, or auditing whether the MCP catalog is up to date. It owns the parity discipline
  (every HTTP-surface change must end in exactly one of: tool added/updated, deliberate omission
  recorded, or n/a), the decision rubric for WHEN a tool is pertinent vs excluded, the standing
  register of deliberate omissions and pending gaps, and the step-by-step recipe to add or
  update a tool. Triggers: "add an MCP tool", "añadir una tool", "does this endpoint need a
  tool?", "the MCP is out of date", "el MCP se ha quedado atrás", "tools/list", "frozen catalog",
  "tools_list_returns_exactly_the_v1_catalog", "update the tool catalog", "new handler — MCP?",
  "extract a *_core", "preview/confirm", "tool annotations", "50 tools". Do NOT use it for WHY
  tools share core fns / live-role auth (futurefin-architecture-contract D14/D15), the catalog's
  per-tool semantics (.claude/api-routes.md §MCP), MCP env vars and the write toggle
  (futurefin-config-and-flags), how to run the MCP test suites (futurefin-validation-and-qa /
  .claude/tests.md), or generic merge gates (futurefin-change-control — it routes here, this
  skill is the evaluation it routes to).
---

# FutureFin — MCP surface parity

The embedded MCP server (`apps/api/src/mcp/`) is a **derived surface**: every tool is a thin
Bearer-authenticated wrapper over the same `*_core` fn its HTTP handler uses. Derived surfaces
rot silently — an endpoint gains a field, a handler changes semantics, a new feature ships
HTTP-only — unless something forces the question at merge time. This skill is that something.

Volatile facts date-stamped **2026-08-20, tren 3.8.0** (50 tools; recount with the
commands in §5 before trusting any number here). Previously 2026-08-19, tren 3.6.0 (47).

## When NOT to use this skill

- WHY tools must call shared cores, live-role auth, hash-only credentials →
  `.claude/skills/futurefin-architecture-contract/SKILL.md` (D14, D15).
- What each existing tool does, its params, cache class, preview shape →
  `.claude/api-routes.md` §MCP (the catalog's single home).
- `FUTUREFIN_MCP_ENABLED`, `FUTUREFIN_PUBLIC_URL`, `mcp_write_enabled` semantics →
  `.claude/skills/futurefin-config-and-flags/SKILL.md`.
- Running/writing the MCP test suites, TestApp mechanics → `.claude/tests.md` and
  `.claude/skills/futurefin-validation-and-qa/SKILL.md`.
- Whether the overall change may ship at all (gates, breaking policy, release) →
  `.claude/skills/futurefin-change-control/SKILL.md`. Its "API contract" class routes to this
  skill; this skill never overrides its gates.
- The whole-handler authoring recipe (routes, openapi, migration, test) →
  `.claude/adding-handler.md` (its step 7 points here).

## 1. The parity contract

**Any change to the HTTP API surface must end, before merge, in exactly ONE of:**

1. **Tool added or updated** — following the recipe in §4.
2. **Deliberate omission recorded** — a row added/confirmed in §3 with a rationale that
   survives the rubric in §2.
3. **N/A** — the change demonstrably cannot affect the MCP surface (e.g. a refactor with no
   contract change; an area already classified as excluded in §3 with unchanged rationale).

"Surface change" means: route added/removed/renamed; request/response field added, removed,
retyped or revalidated; a handler's semantics changed (even keeping its shape — shared cores
propagate semantics to tools silently, which is correct, but tool *descriptions* may now lie);
a new query param or setting axis reachable from a handler.

Two directions, both mandatory:

- **HTTP → MCP**: new/changed endpoint ⇒ run the §2 rubric. The failure mode this kills is the
  quiet one: the feature ships, the SPA uses it, and the MCP client simply never learns it
  exists (nothing fails — `tools/list` is happily complete-looking at any size).
- **MCP → docs/tests**: new/changed tool ⇒ frozen catalog test, annotations expectations,
  `.claude/api-routes.md` §MCP, and the CLAUDE.md counters, same PR. The frozen-catalog test
  (`tools_list_returns_exactly_the_v1_catalog`) makes forgetting the test impossible; nothing
  mechanical protects the docs — that is why the recipe lists them explicitly.

**Who enforces it**: `futurefin-change-control` §1, class "API contract", carries this
evaluation as a gate, and `.claude/adding-handler.md` step 7 asks the question at authoring
time. If you arrive here from either, do the evaluation, record the outcome (PR description or
CHANGELOG entry: "MCP: tool X added" / "MCP: omisión deliberada, ver skill §3"), and go back.

## 2. Decision rubric — is a tool pertinent?

Default posture: **financial data and simulation belong in the catalog; credentials, membership
and infrastructure do not.** Work the exclusions first — they are cheaper to decide.

### 2.1 Excluded categories (do NOT build a tool; record the row in §3)

| Category | Why | Precedents |
|---|---|---|
| Credential minting/revocation | A tool that mints (`POST /v1/api-tokens`) would let a compromised agent grant itself persistence beyond the current token; revocation is the human's emergency brake and must work *against* the agent, not through it | `/v1/api-tokens` CRUD, `/v1/oauth/connections` |
| OAuth protocol + consent | Protocol scaffolding of `/mcp` itself; consent's entire value is that a human executes it in a browser with a session cookie | `/.well-known/*`, `/oauth/register`, `/oauth/token`, `/oauth/revoke`, `/v1/oauth/authorize*` |
| Session lifecycle | MCP clients arrive already authenticated by Bearer; there is no cookie jar to fill | `/v1/auth/register`, `login`, `logout` |
| Membership boundaries | Approving a user grants access to the whole household ledger — the exact action a prompt-injected agent must never perform. `role_can_write` governs data, never membership | `/v1/installation/pending-users*`, `/setup` |
| Encrypted blob transport | `.ffbackup` is password-encrypted, up to 16 MiB base64: an export floods the context window with ciphertext; an import both demands the primary password in a prompt AND mass-replaces the ledger | `/v1/backup/*` |
| Infra probes | The MCP client learns liveness from `tools/list` itself; OpenAPI is the REST self-description, `tools/list` is MCP's | `/health`, `/v1/ready`, `/openapi.json` |
| LLM footguns | An action whose worst-case is a *silent* data corruption an LLM is likely to trigger (e.g. pairing two hand-picked UUIDs where a mistake silently removes both from all flow aggregates) | `reconcile_pair` manual (§3) |
| Context-window abuse | Payloads that must traverse the model's context to work (CSV base64 up to 16 MiB) and are third-party untrusted content — an injection surface feeding a DB-writing agent | `/v1/transactions/import/*` (§3) |
| Kill-switch self-reference | `mcp_write_enabled` must NEVER be writable through MCP — a tool able to re-enable itself makes the switch decorative | part of why `PATCH /v1/installation` has no full-body tool |

### 2.2 Pertinence signals (build the tool)

- **CRUD symmetry**: if `create_X` and `delete_X` exist, `update_X` must too. The incident that
  proved it: until post-3.5.0 the catalog had `create_liability` + `delete_liability` but no
  update — "el TIN de mi hipoteca ha bajado" left the agent ONE path: destructive
  delete+recreate, which NULLs `linked_liability_id` on every linked transaction and drops the
  cuota's `expense_category_id`. A missing tool is not neutral; it *pushes toward the dangerous
  alternative*. Same logic, still open, for the create-without-cleanup asymmetries in §3.
- **Engine inputs should be at least readable, ideally writable**: anything that moves the
  projection (budget, planning, assets/liabilities, fire_settings, transactions in modes B/C,
  `annual_inflation_assumption_percent`) is what a conversational client most plausibly wants
  to inspect and adjust.
- **The conversational diferencial**: data whose natural entry point is a sentence, not a form
  ("en enero de 2023 tenía 40.000 € en el fondo" → snapshot backfill). If chat is *better* than
  the SPA at capturing it, the tool earns its slot.
- **Self-cleanup**: a tool that can create (categories, rules) should be able to correct its
  own mistakes, or the catalog drifts monotonically dirtier over a long conversation.
- **Feasibility marker**: does the handler already have a `*_core`? If yes the tool costs ~50
  lines (§4). If not, the core extraction is the real work — and per D14 it is mandatory, never
  optional: a tool with its own SQL is an automatic review block.

### 2.3 Genuinely contested → record as "dudoso" in §3 with both sides

If neither list decides it, write the argument down in §3 rather than deciding by default. A
"dudoso" with reasoning beats both a rushed tool and an invisible gap.

## 3. Standing decision register

This table is the skill's living core — **the only home of "why is there no tool for X"**.
Update it whenever the rubric runs. An entry here is a decision, not a prophecy: revisit when
the blocking reason changes (noted per row).

### 3.1 Deliberate omissions (decided, stable)

| Surface | Decision | Rationale (short) | Revisit when |
|---|---|---|---|
| auth/session, api-tokens, OAuth protocol+consent, membership/pending-users, backup, probes | **never** | §2.1 categories | Category-level change of posture only |
| `POST /v1/transactions/{id}/reconcile` (manual pair; `reconcile_pair_core` EXISTS) | **omit** | Hand-picking two UUIDs among hundreds; a wrong pair silently leaves all flow aggregates and moves modes B/C savings | A server-side *suggestions* tool ships (candidates with opposite amounts), reducing the LLM's choice to confirming a proposed pair |
| `/v1/transactions/import/preview|confirm` | **omit** | §2.1 context-window abuse + untrusted third-party content | MCP gains an out-of-band attachment channel |
| `POST /v1/transactions/batch` (create) | **defer** | `create_transaction` loops fine; batch adds all-or-nothing tx semantics + shared fingerprint ordinals that complicate preview. **Sigue vigente en 3.8.0**: lo que se hizo tool-able fue el PATCH, no el POST | Real demand for >10-item batches from chat |
| `POST /v1/transactions/rules` con `apply_to_existing` (el eje de backfill del body HTTP) | **omit** | En el momento del preview la regla todavía no existe, así que no hay nada que simular; y un `create_*` capaz de reescribir cientos de filas haría mentir a sus propias annotations, que es lo que el cliente MCP usa para decidir si pide permiso al humano. Desde el chat: `create_categorization_rule` → `apply_categorization_rule`, con un único gate de confirmación (3.8.0) | Que el SPA necesite el round-trip único también desde MCP, cosa que hoy no pasa |
| `POST /v1/allocation-rules/reorder` | **omit** | Requires echoing the exact full id set; one missing id = 400; near-zero conversational value | UX rethink of the cascade |
| `PATCH /v1/auth/me` (`birth_date`) | **defer** | Engine input but set-once identity data; marginal | Bundled into a future profile tool |
| `GET /v1/history/snapshots/prefill` | **defer** | Only meaningful as companion of snapshot backfill | Gap #3 below is implemented |
| Full-body `PATCH /v1/installation` tool | **restricted** | `mcp_write_enabled` self-reference (§2.1); FIRE subset already covered by `update_fire_settings` (owner-only) | Only ever as explicit field-allowlist tool, never `PatchInstallationBody` passthrough |

### 3.2 Pending pertinent gaps (open backlog, priority order)

| # | Missing tool(s) | Core status (the real cost) | Cache | Notes |
|---|---|---|---|---|
| 1 | `create_snapshot` (backfill) + `update_snapshot` | extract from `history.rs` backfill/PUT handlers (validations `normalize_kind`, `validate_snapshot_date`, 409 on (user, kind, date)) | NONE (D12) | The strongest conversational case: recording the past |
| 2 | `create_allocation_rule` + `delete_allocation_rule` | extract; **careful**: the sink invariant (exactly one uncapped remainder, always last) lives spread across the handler | FULL | Completes cascade control; `update_allocation_rule` exists |
| 3 | `update_category` + `delete_category` (with `remap_to`) | extract from `categories.rs` PATCH/DELETE | NONE | delete fits preview/confirm perfectly (preview shows `refs`, forces naming the remap target) |
| 4 | `update_categorization_rule` + `delete_categorization_rule` | extract from `rules.rs` | NONE | Self-cleanup for `create_categorization_rule`. **Subió de prioridad en 3.8.0**: ahora que una regla puede reescribir el histórico (`apply_categorization_rule`), no poder corregirla desde el chat empuja hacia borrar y recrear |
| 5 | `update_installation_settings` (allowlist: `annual_inflation_assumption_percent`, maybe `calendar_tz`, `show_age_mode`) | partial — only the FIRE slice has a core | FULL | Inflation is a direct engine input, today read-only via MCP; NEVER include `mcp_write_enabled` |

Closed since the register was created: `update_asset` and `update_liability` (post-3.5.0; the
CRUD-symmetry incident in §2.2); and in the 3.8.0 issue-#4 train, three rows that were not even
in this table because the endpoints did not exist yet — `apply_categorization_rule` (backfill
retroactivo, cache COND), `update_transactions` (PATCH en lote, COND una sola vez) y
`get_allocation_resolution` (la cascada resuelta, read-only, cache NONE; cierra además el
_stretch_ pendiente del issue #2). When you close a row, move it to a one-line mention here —
history matters, the table stays short.

### 3.3 Reverse direction (tools without an endpoint)

`simulate_projection` is the only one, and deliberately so: pure what-if, cache-neutral by
construction, no REST resource to represent (the SPA previews client-side). Any new
tool-without-endpoint needs the same two properties or an explicit new argument here.

## 4. Recipe — add or update a tool

Model commits: `82a43cb` (reconcile tools, 43→45), the post-3.5.0 `update_asset`/
`update_liability` commit (45→47, includes a core extraction) and the 3.8.0 issue-#4 train
(47→50: `apply_categorization_rule`, `update_transactions`, `get_allocation_resolution` — two
new cores, one new engine entry point and a verb outside the `create_/update_/delete_` naming
convention, which forced a conscious arm in the annotations test). Steps, in order:

1. **Core first.** The handler logic must live in a `*_core(state|pool, iid, user_id, …)`
   shared fn — auth/extractors stay in the HTTP handler, everything else (validation, SQL,
   response struct, **cache invalidation post-commit**) moves into the core. Classify the
   invalidation: FULL (`refresh_projection_after_mutation`), COND
   (`invalidate_projection_if_savings_uses_transactions`) or NONE — and put it INSIDE the core
   so no future caller can forget it. Zero SQL in `apps/api/src/mcp/` (sole exception:
   `require_mcp_write`'s toggle SELECT in `auth.rs`).
2. **Params struct** in `server.rs` (before the `#[tool_router]` impl): `#[derive(Debug,
   Deserialize, schemars::JsonSchema)]`, `///` doc comments in Spanish on every field (they ARE
   the schema descriptions the LLM reads), `#[serde(default)]` + `Option` for optional fields,
   `#[schemars(range(...))]` for bounds. Decimals travel as **strings**; UUIDs/dates as strings
   parsed with `parse_uuid_param`/`parse_date_param`/`parse_decimal_param`. A PATCH-style
   "omit vs null" tri-state cannot be expressed in the schema — model it as a `clear_*: bool`
   flag (precedents: `clear_expense_end_date`, `clear_cap`, `clear_purchase_price`).
3. **Tool fn** inside the `#[tool_router] impl FutureFinMcp`, placed next to its thematic
   neighbors. Canonical body: `identity(&ctx)?` → a `run()` closure mapping params to the
   handler's body struct (fail early via `to_tool_outcome`) → async block: `require_mcp_write`
   FIRST for any write → the core fn → compact `serde_json::json!({id, resumen, …})` →
   `to_tool_result`. Reads return the core's serde struct unmodified (Decimal-as-string
   intact).
4. **Annotations are mandatory and pattern-bound**: `title` in Spanish + `read_only_hint` +
   `destructive_hint` + `idempotent_hint` + `open_world_hint = false`. The annotations test
   *derives* expectations from the name (`update_*`/`delete_*` ⇒ destructive+idempotent;
   `create_*` ⇒ neither) — name the tool to match the convention or extend the test's match
   arms consciously.
5. **Destructive tools get preview/confirm**: without `confirm: true` return `{preview: true,
   confirm_required: true, action, effects}` as SUCCESS (a preview is information for the LLM,
   not an error), using read cores for the effects. Updates that edit fields in place are
   destructive-hinted but do NOT need preview/confirm (editing keeps the row).
6. **Sibling descriptions**: a new tool can change what an existing description should say
   (`update_asset_value` gained "para el resto de campos usa update_asset"; `list_transactions`
   gained the `transfer_counterpart_id` explanation when reconcile tools shipped). Sweep the
   related tools' descriptions.
7. **Tests** — the write-tool quartet in `apps/api/tests/mcp_write.rs` (shared core: row
   indistinguishable via HTTP; cache contract via `warm`/`assert_invalidated`; a shared domain
   error surfacing with the HTTP wire code; the `mcp_write_enabled` toggle cutting it live) +
   add the name to the **frozen catalog** in `mcp_http.rs::tools_list_returns_exactly_the_v1_
   catalog` (alphabetical). Read tools: byte-parity vs the GET instead of the quartet.
8. **Docs, same PR**: `.claude/api-routes.md` §MCP (the catalog), CLAUDE.md counters (two
   spots: module map + MCP paragraph), `.claude/tests.md` suite rows, this skill's §3 register
   (close/open rows), CHANGELOG entry. Run §5's counters to catch what you missed.

## 5. Keeping it honest — verification and drift audit

Reproducible counters (run from repo root; expected values dated 2026-08-20):

```bash
grep -c '#\[tool(' apps/api/src/mcp/server.rs                      # 50 — total tools
grep -c 'read_only_hint = true' apps/api/src/mcp/server.rs          # 21 — reads + simulate
grep -c 'read_only_hint = false' apps/api/src/mcp/server.rs         # 29 — writes
grep -c 'require_mcp_write(&self.state.pool' apps/api/src/mcp/server.rs  # 29 — MUST equal writes
grep -c 'p.confirm.unwrap_or(false)' apps/api/src/mcp/server.rs     # 10 — preview/confirm tools
grep -rn 'sqlx::query' apps/api/src/mcp/                            # exactly 1 hit (auth.rs toggle)
```

Invariant cross-checks: writes == `require_mcp_write` count (a write tool skipping the gate is
a security bug); reads + writes == total; the frozen-catalog vec length == total. The doc-side
counters (CLAUDE.md ×2, api-routes §MCP, this file) must all agree with the code counter — any
mismatch is the "3.0.0 bumped code but not the frozen counters" incident repeating
(futurefin-docs-and-writing §7 owns that lesson).

**Full parity audit** (run when the catalog smells stale, or ~once per release train): list the
HTTP surface and diff it against the catalog and this register —

```bash
grep -rn '\.route(' apps/api/src/routes/mod.rs apps/api/src/handlers/ | grep -v tests
grep -oP 'name = "\K[a-z_]+' apps/api/src/mcp/server.rs
```

Every route must land in one of: covered (name the tool), §3.1 omission row, §3.2 backlog row,
or — the finding — an unclassified gap, which means the parity contract was skipped: classify
it now and check what else that PR missed.

## Provenance and maintenance

Written 2026-08-19 (post-3.5.0 train, branch `dev`), sourced from: full tool inventory and
`git show 82a43cb` (the add-a-tool pattern), a route-by-route HTTP↔MCP coverage matrix over
`apps/api/src/routes/mod.rs` + `apps/api/src/mcp/server.rs`, and the norm inventory across
CLAUDE.md, `.claude/api-routes.md`, architecture-contract D14/D15 and change-control. The
`update_asset`/`update_liability` pair (45→47) shipped in the same change as this skill.
Refreshed 2026-08-20 for the 3.8.0 issue-#4 train (47→50).
Re-verify before trusting:

- Tool counts + gate invariants: the §5 block (50/21/29/29/10/1 on 2026-08-20).
- Frozen catalog still matches the code (never count quotes — run the test):
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_http tools_list_returns`
- Cores still own invalidation, MCP module still SQL-free:
  `grep -rn 'refresh_projection_after_mutation\|invalidate_projection_if_savings' apps/api/src/mcp/` (empty)
- `reconcile_pair_core` still tool-less (top §3.1 revisit-trigger):
  `grep -n 'reconcile_pair' apps/api/src/mcp/server.rs` (empty)
- §3.2 backlog rows still open: `grep -n 'create_snapshot\|create_allocation_rule\|update_category' apps/api/src/mcp/server.rs` (empty while open)
- Choke points still route here: `grep -n 'mcp-parity' CLAUDE.md .claude/adding-handler.md .claude/skills/futurefin-change-control/SKILL.md .claude/skills/futurefin-docs-and-writing/SKILL.md .claude/api-routes.md`
- Recipe step 4's derivation still holds: read the `is_write`/`expect_destructive`/`expect_idempotent`
  match arms in `mcp_http.rs::tools_list_exposes_annotations_on_every_tool`.
