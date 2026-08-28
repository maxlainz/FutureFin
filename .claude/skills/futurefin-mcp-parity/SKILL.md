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
  "extract a *_core", "preview/confirm", "tool annotations", "68 tools". Do NOT use it for WHY
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

Volatile facts date-stamped **2026-08-28, Fases 0–6 del tren 4.4.0** (**68 tools**; recount con
los comandos de §5 antes de fiarte de cualquier número de aquí). **La Fase 6 (issue #87) es la
primera del tren que mueve los contadores**: 52 → **68 / 28 lectura / 40 escritura / 17 preview /
8 `confirm_token` / 18 `impact`**, con 16 altas y cero bajas. Las Fases 0–5 no movieron ninguno
(52/21/31/14/7) porque reescribían prosa, formas de respuesta y contratos de campo compartidos con
la HTTP API, no el inventario de tools. Previamente 2026-08-22, tren 4.0.0 (52);
2026-08-20, tren 3.8.0 (50); 2026-08-19, tren 3.6.0 (47).

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
| Session lifecycle **and password rotation** | MCP clients arrive already authenticated by Bearer; there is no cookie jar to fill. And `POST /v1/auth/password` (4.0.0) is the *human's* emergency brake: it revokes every other session, every `ffp_` token and every OAuth grant — i.e. it is designed to cut the agent off. An agent able to call it could rotate the owner's password out from under them, and would need the plaintext current password in its context to do it | `/v1/auth/register`, `login`, `logout`, **`/v1/auth/password`** |
| Membership boundaries | Approving a user grants access to the whole household ledger — the exact action a prompt-injected agent must never perform. `role_can_write` governs data, never membership. The 4.0.0 endpoints that *revoke* and *downgrade* are excluded for the mirror reason: revocation is the human's brake on a misbehaving credential (it kills sessions, `ffp_` tokens and OAuth grants), so it must work **against** the agent, not through it. The `GET` listing is excluded too — it is only useful as the input to those writes, and it names people | `/v1/installation/pending-users*`, `/setup`, **`GET/PATCH/DELETE /v1/installation/members`** |
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
| `GET /v1/installation/session-context` (**Infra probes**, clasificada 2026-08-28) | **never** | Era la ÚNICA pareja método-ruta sin clasificar del registro — daño real cero, pero §5 define exactamente eso como «the finding», así que se clasifica en vez de dejarse. Es el sondeo de arranque de la SPA (`{installation_initialized, access}`) para decidir qué pantalla pintar: no devuelve dato financiero alguno, y lo poco que dice (¿hay instalación?, ¿qué acceso tengo?) ya lo cubre `get_settings` con más contexto. Una tool aquí sería un round-trip para saber si merece la pena hacer el round-trip siguiente | Category-level change of posture only |
| `POST /v1/auth/password` (4.0.0) | **never** | §2.1 session lifecycle / credential brake. Rotating the password revokes every other session, every `ffp_` token and every OAuth grant — including, quite possibly, the token making the call. It also requires the plaintext current password, which is exactly what must not travel through a model's context. It is the lever a compromised agent must not have and the human must always have | Category-level change of posture only |
| `POST /v1/auth/sso` (2026-08-27) | **never** | §2.1 session lifecycle. It is a **browser-session mechanism**: it turns a trusted proxy's `X-Remote-User-*` headers into an `ff_session` cookie. MCP clients already hold their own first-class credentials (`ffp_` API tokens, `ffo_` OAuth access tokens), so a tool would buy nothing — and it would buy it by moving identity assertion into a channel where the peer check that makes the endpoint safe (`FUTUREFIN_TRUSTED_PROXY_IPS`, D18) does not mean the same thing. Also note the ingress does not reach `/mcp` at all: an add-on user talking MCP is on the direct port, i.e. exactly the peer this endpoint refuses | Category-level change of posture only |
| `GET /v1/auth/ha/start` + `GET /v1/auth/ha/callback` (4.3.1, 2026-08-27) | **never** | §2.1 session lifecycle, and the sharpest case in this table: the credential is not a value at all, it is a **browser round-trip** — a 302 to Home Assistant, a human approving there, a 302 back carrying a `code` that only matches a single-use `HttpOnly` cookie this server set. An MCP client cannot drive that: it has no browser, cannot receive the redirect, cannot hold the cookie, and the thing that comes out the far end is an `ff_session` cookie, **not a token** it could use on `/mcp`. A tool would have to either invent a headless HA login (a second, weaker auth path) or hand the model an HA access token — exactly what the flow revokes seconds after using it (D19: identity, never authorization; FutureFin retains no HA credential). Same rationale as the `POST /v1/auth/sso` row: identity plumbing for the browser, not an operation over data | Category-level change of posture only |
| `GET/PATCH/DELETE /v1/installation/members` (4.0.0) | **never** | §2.1 membership boundaries. `PATCH` can promote to `owner` and `DELETE` cuts all four credentials at once; both are the human's control over *who* is in the household, which `role_can_write` deliberately never governs. The `GET` follows them: its only real use is choosing a target for those writes, and a household roster is not financial data | Category-level change of posture only |
| `POST /v1/transactions/{id}/reconcile` (manual pair; `reconcile_pair_core` EXISTS) | **omit — SUPERADA 2026-08-28 (Fase 6, issue #87), la ruta sigue sin tool y ya no la necesita** | El motivo original sigue en pie: elegir dos UUID entre cientos; un par equivocado saca a las dos patas de todos los agregados de flujo y mueve el ahorro de los modos B/C. Su *revisit trigger* era literalmente «que exista una tool de sugerencias». Existe (`suggest_transfer_matches`), y lo que se implementó **no es `reconcile_pair`**: `confirm_transfer_match` acepta **solo un `match_id` emitido por el servidor** (24 hex del SHA-256 de `instalación\|owner\|ids ordenados`, deliberadamente **no** un UUID), y el core lo resuelve re-derivando el hash sobre los candidatos vivos. **Un par arbitrario no es expresable en el esquema.** Eso no es una barrera que se pueda saltar con un prompt: es hacer imposible el error que motivaba la omisión — el espacio de acciones alcanzables es exactamente el que el servidor propondría. La regla dura que acompaña: **GET aparte para las sugerencias, jamás un `?dry_run` sobre el POST** (un GET que muta ya costó caro, §2.1 de `futurefin-failure-archaeology` fila 6) | Que alguien proponga aceptar dos UUID «para casos que la sugerencia no ve»: eso ES reabrir la fila, no ampliarla |
| `/v1/transactions/import/preview\|confirm` | **omit** | §2.1 context-window abuse + untrusted third-party content | MCP gains an out-of-band attachment channel |
| `POST /v1/transactions/batch` (create) | **defer** | `create_transaction` loops fine; batch adds all-or-nothing tx semantics + shared fingerprint ordinals that complicate preview. **Sigue vigente en 3.8.0**: lo que se hizo tool-able fue el PATCH, no el POST | Real demand for >10-item batches from chat |
| `POST /v1/transactions/rules` con `apply_to_existing` (el eje de backfill del body HTTP) | **omit** | En el momento del preview la regla todavía no existe, así que no hay nada que simular; y un `create_*` capaz de reescribir cientos de filas haría mentir a sus propias annotations, que es lo que el cliente MCP usa para decidir si pide permiso al humano. Desde el chat: `create_categorization_rule` → `apply_categorization_rule`, con un único gate de confirmación (3.8.0) | Que el SPA necesite el round-trip único también desde MCP, cosa que hoy no pasa |
| `POST /v1/allocation-rules/reorder` | **omit** | Requires echoing the exact full id set; one missing id = 400; near-zero conversational value | UX rethink of the cascade |
| `PATCH /v1/auth/me` (`birth_date`) | **defer** | Engine input but set-once identity data; marginal | Bundled into a future profile tool |
| `GET /v1/history/snapshots/prefill` | **defer** | Only meaningful as companion of snapshot backfill | Gap #3 below is implemented |
| Full-body `PATCH /v1/installation` tool | **restricted** | `mcp_write_enabled` self-reference (§2.1); FIRE subset already covered by `update_fire_settings` (owner-only) | Only ever as explicit field-allowlist tool, never `PatchInstallationBody` passthrough |

### 3.2 Pertinent gaps — BACKLOG VACÍO (cero filas abiertas)

> **Estado: 0 huecos abiertos.** Las cuatro filas que esta sección tuvo se **cerraron** el
> 2026-08-28 (Fase 6, issue #87). Lo que sigue es el **archivo** de esas cuatro, no una lista
> de trabajo pendiente: cada fila dice con qué se cerró y qué hubo que decidir. Si buscas qué
> falta por hacer, la respuesta es «nada en esta tabla» — salta a *¿Y si el backlog está
> vacío?*, al final de la sección.

Se archiva en vez de borrarse porque una fila cerrada dice qué se decidió y por qué, y una tabla
vacía sin historia invita a reabrir lo mismo dentro de seis meses. **Antes de añadir una fila
nueva aquí, muévela arriba de este aviso**: el estado de la sección se lee en el encabezado, no
en la letra pequeña. Deriva que esto arregla (Fase 7, 2026-08-29): la sección se titulaba
«Pending pertinent gaps (open backlog, priority order)» con cuatro filas debajo, así que quien
escaneara la tabla —lo normal— contaba cuatro huecos abiertos que llevaban un día cerrados.

#### Archivo de filas cerradas (Fase 6, 2026-08-28)

| # | Tool(s) | Cerrada con | Lo que hubo que decidir, y que no se ve en el código |
|---|---|---|---|
| 1 ✅ | `create_snapshot` (backfill) + `update_snapshot` | `create_snapshot_core` / `update_snapshot_core` extraídas de `history.rs`; cache **NONE por contrato (D12)** | Era «el diferencial conversacional» del registro: grabar el pasado («en enero de 2023 tenía 40.000 € en el fondo») es lo que el chat hace mejor que un formulario. La ausencia de `impact` **no es un olvido**: publican `"affects_projection": false`, porque un bloque de ceros se leería como «no ha pasado nada» y no como «esto no mueve la proyección». `kind` inmutable en el update; omitir `items` conserva, mandarlos —incluso `[]`— **reemplaza la lista entera** |
| 2 ✅ | `create_allocation_rule` + `delete_allocation_rule` | cores extraídas; cache **FULL**, con `impact`; `delete` exige **`confirm_token`** | El aviso de esta fila («the sink invariant lives spread across the handler») era el trabajo real: encapsularlo destapó **dos bugs vivos** (`futurefin-failure-archaeology` §2.20). Decisión propia de la superficie: **`create_allocation_rule` NO puede crear el sumidero** (`SinkPolicy::Forbidden`, aplicado **dentro de la core**, no en el esquema) — crearlo donde no había redirige todo el sobrante de golpe y **no se deshace por el mismo canal**. El `confirm_token` del delete tiene su propio motivo: recrear la regla no restaura su prioridad, y mientras tanto el sobrante se ha ido por otro sitio. **Hueco conocido**: `patch_allocation_rule_core` no recibe `SinkPolicy`, así que `update_allocation_rule` + `clear_cap` sí crea el sumidero en dos llamadas — la `description` de esa tool afirma lo contrario (errata en `futurefin-docs-and-writing`) |
| 3 ✅ | `update_category` + `delete_category` (con `remap_to`) | cores extraídas de `categories.rs`; cache **NONE**; delete con preview/confirm **sin** `confirm_token` | `create_category` sin contraparte era un pozo sin fondo en un catálogo que comparte toda la instalación. El delete encaja perfecto en preview/confirm: el preview desglosa las referencias **por tabla** y separa las bloqueantes de las que solo se degradan. Cerrarla destapó un bug real: el `remap_to` se ignoraba en silencio cuando el contador daba 0 (§2.21 de la arqueología) |
| 4 ✅ | `update_installation_settings` (allowlist `calendar_tz`, `show_age_mode`, `base_currency`) | `patch_presentation_settings_core` (**tool-without-endpoint**, §3.3); owner-only comprobado **dentro de la core**; cache **FULL**, con `impact` | Allowlist estricta y **exhaustiva por construcción**: el `is_empty()` del patchset hace destructuring sin `..`, así que añadir un eje deja de compilar. **Jamás** `mcp_write_enabled` (§2.1, autorreferencia del kill-switch) **ni** `onboarding_completed` (estado de UI: que un agente lo ponga a `true` no cambia un dato del hogar, solo le quita a una persona una pantalla que quizá no había visto). Owner-only vive en la core y no en la tool **precisamente** para que una superficie nueva no se lo deje. FULL porque `calendar_tz` mueve el mes 0 de la proyección entera |

Closed since the register was created: las **cuatro filas de arriba** (Fase 6, 2026-08-28);
**`update_categorization_rule` + `delete_categorization_rule`
(4.0.0, auditoría MCP §10 — era la fila #4; `patch_rule_core`/`delete_rule_core` extraídas de `rules.rs`,
cache NONE, preview/confirm en el borrado con la huella actual vía `apply_categorization_rule_core`
en `dry_run`. Cerrarla dejó dos guardias nuevas en la core, `rule_patch_empty` y
`rule_patch_conflict`: el PATCH aceptaba cuerpo vacío y dejaba que el `clear_*` ganara en silencio
sobre el campo puesto)**; `update_asset` and `update_liability` (post-3.5.0; the
CRUD-symmetry incident in §2.2); and in the 3.8.0 issue-#4 train, three rows that were not even
in this table because the endpoints did not exist yet — `apply_categorization_rule` (backfill
retroactivo, cache COND), `update_transactions` (PATCH en lote, COND una sola vez) y
`get_allocation_resolution` (la cascada resuelta, read-only, cache NONE; cierra además el
_stretch_ pendiente del issue #2). When you close a row, move it to a one-line mention here —
history matters, the table stays short.

**Y si el backlog está vacío, ¿qué queda?** No «nada»: queda que el próximo hueco se descubra por el
rubro §2, no por esta tabla. La Fase 6 salió de una observación que ninguna fila recogía — **paridad
de rutas ≠ paridad de capacidades**: 89 de 90 parejas método-ruta estaban clasificadas y aun así la
app calculaba cosas que no son una ruta y que el chat no alcanzaba (el calendario que el motor tira,
el deflactado que el servidor ya hace para `milestones_real`, la huella de dedup que vive dentro de
un preview, el cap que YA es un objetivo). **Cuando esta tabla se vacíe otra vez, esa es la
pregunta que hay que hacerse**, no la de si falta un CRUD.

### 3.3 Reverse direction (tools without an endpoint)

**Ya no es una excepción única: son CUATRO, y eso cambia lo que hay que exigirle a la quinta.**
Cuando esta sección decía «`simulate_projection` is the only one», la lectura implícita era que
tool-sin-endpoint es un caso singular que se justifica por sí mismo. No lo es — es un patrón
aceptado, con cuatro formas distintas. Recuento reproducible (cores que **solo** llama `mcp/`):

```bash
for c in simulate_projection_core patch_fire_settings_core settings_user_core \
         patch_presentation_settings_core; do
  echo "$c -> $(grep -rln "$c" apps/api/src/ | tr '\n' ' ')"
done   # cada uno: su handlers/*.rs de definición + mcp/server.rs, nunca un handler HTTP
```

| Tool | Qué le falta en HTTP | Por qué se acepta |
|---|---|---|
| `simulate_projection` | **todo**: no hay recurso REST que representar | What-if puro y cache-neutral por construcción; la SPA previsualiza en cliente |
| `update_fire_settings` | **la core**: `patch_fire_settings_core` no la llama ningún handler (`installation.rs`, y el comentario del propio código lo dice) | El `PATCH /v1/installation` reemplaza el objeto `FireSettings` entero; desde el chat eso obliga a leer-modificar-escribir y a arriesgar pisar un campo que el modelo no miró. La tool es **campo a campo** (`FireSettingsPatch`), que es la única forma segura de tocar ajustes desde una conversación |
| `get_settings` | **media respuesta**: el bloque `user {id, username, birth_date}` sale de `settings_user_core`, que el endpoint HTTP no expone | La SPA ya tiene al usuario en su estado; un cliente MCP no tiene sesión de la que sacarlo, y sin `birth_date` no puede convertir `jubilacion_month_index` en una edad |
| `update_installation_settings` (Fase 6, 2026-08-28) | **la core**: `patch_presentation_settings_core` no la llama ningún handler | **Misma forma que `update_fire_settings`** — y esa es justo la exigencia de esta tabla cumplida: no trajo un argumento nuevo, nombró la forma que repetía. El `PATCH /v1/installation` reemplaza el objeto entero, que desde el chat obliga a leer-modificar-escribir; la tool es una **allowlist estricta** (`calendar_tz`, `show_age_mode`, `base_currency`) exhaustiva por construcción, con el owner-only comprobado **dentro** de la core |

Las cuatro comparten la propiedad que sí es exigible: **el consumidor MCP no tiene el contexto que
la SPA sí tiene** (estado de cliente, pantalla previa, sesión). Una quinta tool-sin-endpoint
necesita nombrar cuál de esas formas es, o traer un argumento nuevo a esta tabla — no basta con que
sea cómoda. **Dos de las cuatro son ya la misma forma** («la core existe, el PATCH HTTP reemplaza
el objeto entero, el chat necesita campo a campo»), así que esa vía está agotada como novedad: la
quinta que la invoque no está argumentando nada, solo está aplicando un precedente — legítimo, pero
dilo así.

### 3.4 View echo — object responses vs `list_*` envelopes (Fase 5, issue #86)

Every scope-aware response must echo which `view` it actually applied. The failure mode this
closes: in a single-user installation, `view: "mine"` and `view` omitted returned byte-identical
payloads, so there was no way to tell "mine happens to equal household" from "the parameter was
silently ignored." In a two-person household that ambiguity is exactly the question that decides
whether the number being quoted is the household's or the caller's.

**Object responses get it for free.** `SummaryResponse`, `BudgetSnapshotResponse`,
`ProjectionSeriesResponse` and `AllocationResolutionResponse` gained a `view` field on the
**core** (`HistorySeriesResponse`, `CashflowResponse`, `TransactionsSummaryResponse` and
`CategoryMonthlySeriesResponse` already had it) — `LedgerView::as_str()` in
`handlers/person_view.rs` is the single source now, replacing four separate copies of
`if view == Mine { "mine" } else { "household" }`. Because every tool calls the same core its
HTTP handler calls, every object-shaped tool inherits the field with **zero code in
`server.rs`**.

**Listings cannot do that.** Every `GET /v1/*` list endpoint returns a bare JSON array on
purpose — REST convention, and the SPA deserializes it as one. Wrapping that array to smuggle in
a `view` field would be a breaking HTTP change for a field the SPA doesn't even need (it already
knows its own `view` — it's the query param it sent). So for a scoped `list_*`, the echo has to
be put on by **the tool itself**, in an envelope: `{"view": "...", "<entity_key>": [...]}`.

This is a **design consequence, not a workaround**. It moves 7 tools out of the byte-for-byte
loop (`mcp_http.rs::new_read_tools_match_http_endpoints`) and into content parity
(`mcp_http.rs::list_tools_echo_the_applied_view_and_keep_content_parity`, which asserts
`envelope[key] == GET?view=…` for both views, AND that the bare `GET` still returns an array).
It's the same path `list_categorization_rules` already walked when it grew pagination in 4.0.0 —
an enveloped listing is not a new kind of problem, it's the second occurrence of one. See
`NOTA-VIEW-ENVELOPE` in `apps/api/src/mcp/server.rs` for the comment block this section mirrors.

The 7 (tool → envelope key): `list_assets` → `assets`, `list_liabilities` → `liabilities`,
`list_planning_flows` → `planning_flows`, `list_allocation_rules` → `allocation_rules`,
`list_transaction_months` → `months`, `list_transactions` → `transactions`,
`list_transaction_imports` → `imports`.

Still in the byte-for-byte loop — **11 tras la Fase 6** (eran 5 cuando se escribió esta sección;
recuéntalas, no las copies):

```bash
# los pares (tool, endpoint) del bucle, sin contarlos a mano
sed -n '/fn new_read_tools_match_http_endpoints/,/^}/p' apps/api/tests/mcp_http.rs \
  | grep -oE '"(list|get|find|suggest|aggregate|deflate)[a-z_]*"' | sort -u    # → 11
```

Las cinco originales (`list_categories`, `get_budget`, `list_recurring_rules`,
`get_history_cashflow`, `get_category_monthly_series`) más las **seis lecturas nuevas de la Fase 6**
(`aggregate_transactions`, `find_duplicate_transactions`, `suggest_transfer_matches`, `list_goals`,
`deflate_amount`, `get_liability_schedule`). Ninguna necesita eco por parte de la tool:
`list_categories` ni scopea ni pagina, y todas las demás son **objetos** cuya core ya lleva `view`
cuando es scope-aware — las once son `to_tool_result(core(...))` literal, así que la paridad byte a
byte ES el contrato que prometen, no una aspiración.

**La séptima lectura de la Fase 6, `list_recent_changes`, se queda fuera de los dos bucles**: su
campo `now` es el instante de la consulta, así que dos llamadas no pueden coincidir byte a byte.
Tiene test propio (`mcp_http.rs`, paridad `list_recent_changes` ↔ `GET /v1/changes` **ignorando
`now`**). Toda tool futura cuya respuesta lleve un reloj, un id aleatorio o un cursor hereda ese
patrón: tercera vía, no una excepción — y hay que nombrar el campo que se ignora.

No `view` field at all — **4 tras la Fase 6** (own-user por construcción): `list_snapshots`,
`list_categorization_rules`, `list_recurring_rules` y **`suggest_transfer_matches`** (nueva en la
Fase 6). Sus cores no aceptan `view` de entrada — el doc-comment de `list_recurring_rules_core` lo
dice explícito («siempre own-user… no inventarlo en la tool»). Añadir ahí un campo `view` no sería
un eco, sería afirmar un scope que la tool nunca tuvo. Caso vecino que **no** es este: `deflate_amount`
tampoco acepta `view`, pero no porque sea own-user sino porque la inflación es de la instalación —
no confundas «sin scope de usuario» con «own-user» al clasificar la quinta.

**Rule for future `list_*` tools**: if the new listing's core accepts `view`, the envelope is
**mandatory** and its parity row belongs in the content-parity test, never the byte-parity one.
If the core is own-user (no `view` param), do **not** invent a `view` field in the tool's output.

### 3.5 `prompts` — la segunda primitiva del protocolo (Fase 6, issue #87)

Hasta 4.4.0 este registro era **tool-shaped**: todo el rubro §2 decide si algo merece una *tool*.
La Fase 6 declara la capacidad `prompts` (`ServerCapabilities::builder().enable_tools().enable_prompts()`; **`resources` sigue sin declararse**), así que hace falta un criterio propio —
si no, la primera pregunta sobre un prompt se responderá con el rubro de las tools, que decide otra
cosa.

**Qué es un prompt aquí**: un guion **estático**. Cero SQL, cero identidad, cero lectura de la
instalación. `prompts/get` no toca la base de datos, así que **no hay nada que gatear** por rol ni
por `mcp_write_enabled` — y por eso tampoco abre fila de auditoría. Lo que aporta no es acceso: es
el **orden** en que se encadenan tools que ya existen, más las salvedades que un modelo con prisa se
salta (el modo de ahorro decide si las transacciones mueven el motor; los agregados de flujo
excluyen las conciliadas; `null` no es cero).

**Cuándo un prompt es pertinente** (los tres de hoy —`revision_mensual`,
`auditoria_categorizacion`, `amortizar_o_invertir`— cumplen los tres puntos):

1. **Es un flujo multi-tool**, no una tool con otro nombre. Si se resuelve con una llamada, la
   respuesta correcta es mejorar esa tool o su descripción.
2. **Sus salvedades son transversales** y hoy viven repetidas (o ausentes) en varias descripciones.
   Un prompt las versiona con el código en vez de reinventarlas en cada conversación.
3. **No necesita argumentos.** Los tres se publican **sin ninguno a propósito**: interpolar texto de
   cliente dentro de un guion que el modelo lee como instrucciones es una vía de inyección gratuita.
   Un prompt que "necesita" un parámetro casi siempre quería ser una tool.

**Coste y límite, medidos y no supuestos (2026-08-28)**: cuestan una tabla de constantes y dos
métodos sin I/O — pero **el conector remoto de claude.ai NO los muestra hoy** (sus docs dicen que en
MCP remoto prompts y resources «are not yet supported»). Claude Code y los clientes genéricos sí
(`/mcp__<servidor>__<prompt>`). Se publican igual porque el coste es ese y el día que el conector
los soporte ya están — pero **no cuentan como capacidad entregada al cliente principal**, y
cualquier texto público debe decirlo (regla de claims, `futurefin-research-frontier`).

**No cuentan en los contadores de §5.** Los ocho números son de *tools*. Si algún día hay tantos
prompts como para que su prosa pese, añade su propia línea al presupuesto de contexto — hoy no la
tienen porque son tres.

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
   so no future caller can forget it. Zero SQL in **`apps/api/src/mcp/server.rs`** — ese es el invariante que importa, y es el que
   un `grep -c` sobre ese fichero comprueba en un segundo. `auth.rs` sí tiene SQL propio y
   creciente (el SELECT del toggle desde siempre; el INSERT/UPDATE/poda de `mcp_write_audit`
   desde 4.4.0), porque la autorización y su registro NO son lógica de dominio y no tienen
   core que compartir. Contar «1 hit en todo `mcp/`» era una aproximación que caducó en
   cuanto la auditoría aterrizó: el número subía sin que nada se hubiera roto.
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
   FIRST for any write → the core fn → compact `serde_json::json!({id, summary, …})` →
   `to_tool_result`. Reads return the core's serde struct unmodified (Decimal-as-string
   intact). **`summary`, in English** — until Fase 2 of issue #83 this key was named
   `resumen`, the last Spanish key on the MCP wire (repo norm: "UI copy en español, código e
   identificadores en inglés"). When the handler names the field in Spanish — today only
   `BatchPatchResponse.resumen`/`resumen_truncated`, the HTTP contract of
   `PATCH /v1/transactions/batch` — **translate it in the tool**, exactly as
   `apply_categorization_rule` already publishes `out.sample` as `summary`: the catalog must
   speak ONE language, because a client that learned `summary` from ten tools reads
   `result.summary` on the eleventh and gets `undefined` with no error at all.

   **Description has a hard budget** (Fase 5, issue #86): **≤600 characters per tool, ≤24,000 for
   the whole catalog**, enforced by
   `mcp_http.rs::tool_descriptions_stay_within_the_context_budget`. The incident that forced it:
   a client received `get_summary`'s 2,278-character description **truncated**, mid-warning,
   about an inconsistency between two other tools. When the guard fails, **do not raise the
   constant** — its own failure message says so. Move the overflow to a response-field caveat
   (the pattern this server already uses: `savings_income_basis`, `avg_basis`,
   `fire_target_absent_reason`, `skipped_reason`) or to `get_info`'s `instructions`, which every
   client reads once per session instead of once per tool. Reproduce the current numbers with:
   ```bash
   python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];l=[x['description_len'] for x in t];print(len(t),sum(l),max(l))"
   ```
   (today: `68 23874 596` — **126 characters from the ceiling**; `52 21319 596` right after Fase 5;
   before Fase 5: 37,214 total, one description at 3,821, 26 tools over 600 chars). **Read that
   margin before you start**: Fase 6's 16 tools took the raw total to 28,884 (+4,884 over budget)
   and bringing it back was real work, not a trim. The next tool has to buy its description out of
   somebody else's.
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

Reproducible counters (run from repo root; **expected values dated 2026-08-28, Fases 0–6 del tren
4.4.0**):

```bash
grep -c '#\[tool(' apps/api/src/mcp/server.rs                      # 68 — total tools
grep -c 'read_only_hint = true' apps/api/src/mcp/server.rs          # 28 — reads + simulate
grep -c 'read_only_hint = false' apps/api/src/mcp/server.rs         # 40 — writes
grep -c 'require_mcp_write(&self.state.pool' apps/api/src/mcp/server.rs  # 40 — MUST equal writes
grep -c 'p.confirm.unwrap_or(false)' apps/api/src/mcp/server.rs     # 17 — preview/confirm (Fase 6: +3)
grep -c '= two_phase(' apps/api/src/mcp/server.rs                    # 8 — las que exigen token de dos fases
#   (OJO 1: `grep -c 'confirm_token'` a secas da 47 — cuenta también el campo del schema y su prosa.
#    OJO 2: el comando de esta línea fue `grep -c 'confirm_token.as_deref'` hasta la Fase 7, y hoy
#    da **10**, no 8: el propio doc-comment de `DeleteWithTokenParams` en `server.rs` menciona esa
#    cadena dos veces —una de ellas al prescribir este mismo grep—, así que el patrón se cuenta a sí
#    mismo. Es la trampa de todo contador auto-referencial: el comando que un comentario recomienda
#    deja de funcionar en cuanto el comentario lo escribe literal. `= two_phase(` no la tiene (las
#    dos menciones en prosa son `[`two_phase`]`, con backticks). Alternativa igual de buena:
#    `grep -c 'p\.confirm_token\.as_deref()'` → 8. **`server.rs` sigue prescribiendo el grep viejo**
#    y no se puede arreglar desde documentación — reportado como deriva código↔contrato.)
grep -c 'settled(&self.state.pool' apps/api/src/mcp/server.rs       # 40 — == escrituras: toda escritura cierra su fila de auditoría
#   (el patrón lleva `&self.state.pool` a propósito: `grep -c 'settled('` da 41, contando la definición)
grep -c 'impact_since(&self.state' apps/api/src/mcp/server.rs       # 18 — escrituras que publican el bloque `impact`
#   (`grep -c 'impact_since('` da 19: cuenta también la definición)
grep -c 'sqlx::query' apps/api/src/mcp/server.rs                   # 0 — EL invariante real
grep -c 'sqlx::query' apps/api/src/mcp/auth.rs                     # 4 — gate + auditoría (4.4.0)
```

Los ocho números son **68/28/40/40/17/8/40/18** a 2026-08-28 (Fases 0–6 del tren 4.4.0), recontados
enteros ese día. Antes de la Fase 6: 52/21/31/31/14/7/31/15.

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
**Refreshed 2026-08-22 for the 4.0.0 train** (50→52 with `update_categorization_rule` +
`delete_categorization_rule`), and that refresh ran the §1 evaluation over the routes the train
added — recorded, never silent:

| New HTTP surface (4.0.0) | Parity outcome |
|---|---|
| `POST /v1/auth/password` | **deliberate omission**, §2.1 session lifecycle / credential brake → row in §3.1 |
| `GET/PATCH/DELETE /v1/installation/members` | **deliberate omission**, §2.1 membership boundaries → row in §3.1 |
| `SavingsAvgBasis.months_with_data` → `avg_months` | **tool updated** — the field travels in `simulate_projection` too; renaming on one side only would have recreated the very ambiguity the rename fixes |
| `simulate_projection`: inflation alias + `deny_unknown_fields` | **tool updated** (no HTTP counterpart — §3.3 tool-without-endpoint) |
| `delete_asset` preview: `allocation_rules_deleted`, `allocation_remainder_rules_deleted` | **tool updated** alongside `delete_asset_preview_core` |
| `density=hybrid` always emits the last month | **tool follows automatically** — `get_projection` forces `hybrid`, which is why the bug's default victim was the MCP consumer |
| `destructive_hint` on `materialize_recurring` / `unreconcile_transfer` | **annotations corrected** (§4 step 4) |

Evaluación posterior (2026-08-25, sin cambio de contadores — el catálogo sigue en **52**):

| New HTTP surface | Parity outcome |
|---|---|
| `GET /v1/summary` → `financial_health.net_return_nominal_annual_pct` + `net_return_real_annual_pct` | **Tool actualizada sin código nuevo**: `get_summary` comparte `summary_core` y el propio struct `FinancialHealthMetrics`, así que hereda los dos campos. Lo que SÍ hubo que tocar a mano es su `description` — son **porcentajes**, no fracciones como el vecino `savings_rate` (confundirlos multiplica por 100), y traen un aviso que ninguna otra tool puede dar: la proyección no cobra el interés de la deuda, así que este número es más conservador que `get_projection`. Cero tools nuevas: no hay recurso REST nuevo, ni escritura, ni nada que `get_summary` no cubra ya |

Evaluación del tren **4.2.0** (modelo de amortización por pasivo; 2026-08-25, catálogo **sin cambios en 52**):

| New HTTP surface (4.2.0) | Parity outcome |
|---|---|
| `repayment_model` en `LiabilityResponse` / `CreateLiabilityBody` / `PatchLiabilityBody` | **Dos tools actualizadas**: `create_liability` y `update_liability` ganan el parámetro (`Option<String>`, parseado con `handlers::liabilities::RepaymentModel::parse` → **400 `repayment_model_invalid`** en un literal desconocido, no el 422 de serde que da el camino HTTP: por MCP el parámetro llega como string suelto y el error debe ser uno nuestro). Sus `description` enumeran ahora los cuatro modelos y sus requisitos. Cero tools nuevas: es un campo de un recurso que ya está cubierto |
| Derivación del principal dependiente del modelo (Σ cuotas vs valor actual al TIN) | **Descripciones reescritas, código compartido**: las dos tools llaman a `create_liability_core` / `patch_liability_core`, así que la fórmula nueva llega sola. Lo que había que arreglar a mano es la promesa — la descripción de `create_liability` ya llevaba una corrección forense por prometer amortización francesa cuando sumaba cuotas (`.claude/api-routes.md` §MCP); ahora distingue `Σ cuotas` en `fixed_payments` del **valor actual** en `french`, y `update_liability` avisa de que cambiar el modelo o la TAE con el derive activo **re-deriva** el principal |
| Cinco códigos de error 400 nuevos (`repayment_model_invalid`, `apr_required_for_model`, `payment_plan_required_for_model`, `weekly_not_supported_for_model`, `derive_not_supported_for_model`) | **Heredados**, sin trabajo de tool: viajan por la misma `ApiError` de las cores. El fixture `error-codes.json` los lleva y `errorMessages.ts` los traduce (guard de paridad) |
| `simulate_projection` | **n/a**: la tool no acepta pasivos hipotéticos — simula sobre los del ledger. No hay parámetro que ampliar hasta que el what-if los admita, momento en el que esto pasaría a ser un gap de §3.2 |
| `get_summary` — el aviso sobre `net_return_*` | **Descripción matizada**: la frase «la proyección NO descuenta el interés de la deuda», exacta en 4.1.0 y en la fila de arriba, deja de ser cierta en general. Ahora dice que el KPI cuenta el interés de **todos** los pasivos vivos mientras la proyección solo devenga en los que llevan modelo con intereses y plan activo — más conservador solo si queda alguna deuda en cuota fija |

Evaluación de la rama **`feat/home-assistant-addon`** (add-on de HA; 2026-08-27, catálogo **sin
cambios en 52** — recontado ese día: 52/21/31/31):

| New HTTP surface | Parity outcome |
|---|---|
| `POST /v1/auth/sso` | **Omisión deliberada** → fila en §3.1. Es un mecanismo de sesión de navegador (cabeceras de un proxy de confianza → cookie `ff_session`); MCP ya tiene credenciales propias (`ffp_`, `ffo_`) y una tool no añadiría capacidad, solo movería una afirmación de identidad a un canal donde el chequeo de peer que la hace segura no significa lo mismo |
| `FUTUREFIN_BASE_PATH` / prefijo por request, cookie acotada al prefijo, `X-Frame-Options` condicional | **n/a**: no hay superficie nueva. Son propiedades del transporte y del shell HTML, invisibles para `/mcp` (que no viaja por el Ingress: el add-on solo lo expone por el puerto directo opcional) |
| Guarda de downgrade (`db.rs`) | **n/a**: no es una ruta. Falla el arranque; el servidor MCP no llega a montarse |

Evaluación de la rama **`feat/ha-idp-login`** (4.3.1, «Entrar con Home Assistant»; 2026-08-27,
catálogo **sin cambios en 52**):

| New HTTP surface (4.3.1) | Parity outcome |
|---|---|
| `GET /v1/auth/ha/start` + `GET /v1/auth/ha/callback` | **Omisión deliberada** → fila fechada en §3.1. Mecanismo de redirect de navegador que termina en una **cookie de sesión**, no en un token: un cliente MCP no tiene navegador que seguir el 302 a HA, ni forma de sostener la cookie `ff_ha_state` de un solo uso, ni sacaría de ahí una credencial usable en `/mcp` |
| `FUTUREFIN_HA_SSO_URL` / `FUTUREFIN_HA_ADDON`, `window.__FF_HA_LOGIN__`, cookie `ff_ha_state`, códigos `?ha_error=` | **n/a**: configuración de arranque, bandera del shell HTML y códigos que viajan por redirect. Cero superficie de datos; `/mcp` no los ve |
| `handlers/sso.rs::resolve_or_provision` pasa a `pub(crate)` con dos callers | **n/a**: refactor interno de visibilidad, ninguna ruta nueva ni campo nuevo en ninguna respuesta |

Evaluación de la rama **`feat/mcp-fase-1-numeros`** (Fase 1 de la revisión adversarial del MCP,
issue #82 — «números que mienten»; tren 4.4.0, sin publicar aún; catálogo **sin cambios en 52**,
recontado 52/21/31/31 el 2026-08-28). A diferencia de los trenes anteriores, este NO añade
superficie: reescribe el **contrato de salida** de handlers que ya existían, y como cada tool
comparte la core de su handler, la mayoría del trabajo llegó gratis — lo que costó fue la prosa:

| Surface tocada | Parity outcome |
|---|---|
| `points[].net_worth` nullable + `liabilities_snapshotted` `any`→`all` (`GET /v1/history/series`) | **Tool actualizada**: `get_history` hereda el campo sin código propio (comparte `history_series_core`); `description` reescrita — prometía «cuadra con `get_summary.net_worth`» y se desmentía a sí misma dos frases más abajo |
| `fine.net_worth` nullable + `liabilities_snapshotted` nuevo en la raíz (`GET /v1/history/cashflow`) | **Tool actualizada**: `get_history_cashflow` idem, vía `history_cashflow_core` — este caso era **peor** que el anterior, porque la respuesta ni publicaba el flag con el que sospechar de la cifra |
| `jubilacion_series_position` + `jubilacion_target_net_worth_nominal` (`GET /v1/projection/series`) | **Tool actualizada**: `get_projection` gana los dos campos vía `projection_series_cached`, cero código propio; descripción reescrita para dejar de decir que `jubilacion_month_index` sirve para indexar (nunca sirvió) |
| `debt_service_monthly`/`final_net_worth_real_delta` nullable + `*_absent_reason` | **Tool actualizada** en su propia core (`sim_kpis`, `simulate_projection_core`): `simulate_projection` no tiene ruta HTTP homóloga (tool-without-endpoint, §3.3), así que la evaluación recae sobre la tool misma, no sobre un GET que auditar |
| `debt_service` nullable + `debt_service_absent_reason` (`GET /v1/allocation-rules/resolution`) | **Tool actualizada**: `get_allocation_resolution` comparte `allocation_resolution_core`, misma razón (`expense_from_avg`) que `simulate_projection` |
| `actual_txn_count` + `has_actual_data`; `avg`/`delta_vs_budget`/`delta_vs_avg` nullable (`GET /v1/transactions/summary`) | **Tool actualizada**: `get_transactions_summary` comparte `transactions_summary_core` |
| `has_data` por punto + `first_month_with_data`; 2 códigos 400 nuevos donde antes 200 con serie vacía (`GET /v1/transactions/category-series`) | **Tool actualizada**: `get_category_monthly_series` comparte `category_monthly_series_core` |
| Guardia `<campo>_set_and_clear` (5 códigos) en `PATCH /v1/transactions/{id}` | **Tool actualizada**: `update_transaction` comparte `patch_transaction_core`, así que la guardia llega sin tocar `server.rs` salvo la `description` |
| 409 `rule_duplicate` alcanza ahora a las reglas sin `source` (`POST /v1/transactions/rules`) | **Tool actualizada**: `create_categorization_rule` comparte `create_categorization_rule_core` |
| `assigns_nothing`/`shadowed_transactions`/`note` en `ApplyRuleOutcome` (preview de `DELETE /v1/transactions/rules/{id}`) | **Tool actualizada, sin fila HTTP propia**: `apply_categorization_rule` comparte `apply_categorization_rule_core` en `dry_run` con el preview de `delete_categorization_rule` — las dos heredan el fix sin cambiar de firma |
| `UpdateFireSettingsParams`: alias `annual_inflation_percent` + `deny_unknown_fields` | **Tool actualizada, invisible al fixture de contrato**: `update_fire_settings` no tiene equivalente HTTP con ese nombre de campo (persiste vía `patch_fire_settings_core`, propia de MCP), y como el alias no añade una `property` nueva al JSON Schema, `mcp-catalog.json` no detecta el cambio — es el reverso exacto del incidente que 4.0.0 cerró en `simulate_projection` (simular con el nombre corto, guardar con el mismo nombre, la inflación se descartaba en silencio), y quedaba **sin arreglar en la dirección de escritura** sobre el eje que más mueve la proyección |
| SQLSTATE `22003` → 400 `amount_out_of_range` (`error.rs`) | **n/a**: mapeo de error transversal en el `impl From<sqlx::Error>` central; no cambia el schema de ninguna tool, todas heredan el código estable igual que cualquier otro 400 de dominio |
| Migración `20260828120000_categorization_rules_unique_agnostic.sql` | **n/a**: no es una ruta ni cambia un contrato observable por MCP; la superficie visible es el 409 de `create_categorization_rule`, ya en la fila de arriba |

**Nueve tools con `description`/payload actualizados** (`get_projection`, `simulate_projection`,
`get_allocation_resolution`, `get_history`, `get_history_cashflow`, `get_transactions_summary`,
`get_category_monthly_series`, `update_transaction`, `create_categorization_rule`) **+ una décima
tocada solo en su struct de params** (`update_fire_settings`, sin `description` reescrita — ver
fila de arriba). Cero tools nuevas, cero retiradas: el catálogo sigue en **52**. Ninguna fila
necesitó promoción HTTP→tool nueva ni omisión — todo lo demás ya estaba cubierto y lo único que
cambió fue lo que esa cobertura devuelve. Detalle campo a campo, con el porqué de cada nulabilidad:
`.claude/api-routes.md` §MCP y las secciones HTTP de cada endpoint (History series, History
cash-flow, Projection, Allocation rules, Transactions).

Evaluación de la rama **`feat/mcp-fase-2-esquema`** (Fase 2 de la revisión adversarial del MCP,
issue #83 — «el esquema no valida lo que la prosa promete»; catálogo **sin cambios en 52**,
recontado 52/21/31/31/11/1 el 2026-08-28). Como la Fase 1, no añade superficie: endurece el
**contrato de entrada** (`deny_unknown_fields` en las 52, `enum`/`pattern`/cotas en el JSON
Schema en vez de solo en la prosa) y unifica el **contrato de salida** de las escrituras. Las dos
filas que cierran los huecos que la auditoría de la propia fase encontró:

| Surface tocada | Parity outcome |
|---|---|
| `constraints` + `constraints_sha256_12` en `mcp-catalog.json` | **Gate, no tool**: el congelador (`tools_list_freezes_the_input_contract_of_every_tool`) fijaba `properties`, `required` y el hash de la `description` — es decir, era **ciego a todo lo que la Fase 2 acababa de construir**. Quitar un `enum` o un `deny_unknown_fields` no rompía un solo test. Ahora congela también las restricciones del `inputSchema` recorrido **recursivamente** (`properties`, `items`, `$defs`, combinadores, `additionalProperties` sub-schema): `additionalProperties`, `enum`, `pattern`, `type`, `required`, `format`, `const`, `$ref` y las cotas `minimum`/`maximum`/`minLength`/`maxLength`/`minItems`/`maxItems`/`multipleOf`/`uniqueItems`, **a cada nivel**. 296 nodos sobre las 52 tools **de entonces** (hoy son **427 sobre 68** — recuéntalo, no lo copies: `python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];print(len(t),sum(len(x.get('constraints') or []) for x in t))"`). El hash es estable por construcción (claves de objeto ordenadas, `enum`/`type`/`required` tratados como conjuntos y ordenados también), así que ni el orden de emisión de schemars ni una actualización de dependencia lo mueven |
| Clave `resumen` → **`summary`** en los 11 payloads de confirmación de escritura | **Once tools actualizadas, breaking**. `{id, resumen}` era la última clave en español del wire MCP (la norma del repo es «UI copy en español, código e identificadores en inglés»), y la misma fase ya había unificado los `effects` de los previews a `entity`/`side_effects`. Diez tools sintetizan la cadena en el propio MCP; `update_transactions` **traduce** los campos `resumen`/`resumen_truncated` de `BatchPatchResponse` (contrato HTTP de `PATCH /v1/transactions/batch`, que **no** cambia). Se traduce en vez de dejar una excepción porque el fallo de la excepción es silencioso: un cliente que aprendió `summary` en diez tools lee `result.summary` en la onceava y recibe `undefined` sin error. Cero tools nuevas, cero retiradas |

Evaluación de la rama **`feat/mcp-fase-3-escritura-segura`** (Fase 3, issue #84 — «escritura
segura»: auditoría, scope, confirmación en dos fases; catálogo **sin cambios en 52**, recontado
52/21/31/31 el 2026-08-28). A diferencia de las Fases 1/2, esta SÍ añade superficie nueva de
protocolo (`mcp_write_audit`, `api_tokens.scope`, `confirm_token`) pero **no añade ni retira
ninguna tool** — reescribe el andamiaje común de las escrituras y el contrato de entrada/salida de
22 de las 31:

| Surface tocada | Parity outcome |
|---|---|
| `mcp_write_audit` (auditoría append-only de toda escritura) | **n/a — no es superficie del catálogo**. Vive por completo dentro de `require_mcp_write`/`settled`, invisible en `inputSchema` y en la respuesta de cualquier tool; no hay nada que un cliente MCP pueda leer o negociar. Es el mismo tipo de "n/a" que el guard de downgrade de la rama del add-on: existe, protege, pero no es una tool ni cambia una |
| `api_tokens.scope` (`read_write` \| `read_only`) | **n/a para el catálogo MCP** (es un eje de `POST/GET /v1/api-tokens`, ya fuera del catálogo por la fila «Credential minting/revocation» de §2.1 — un token de solo lectura sigue sin tool propia por la misma razón que emitir uno de lectura-escritura nunca la tuvo). Sí es una puerta nueva DENTRO de `require_mcp_write`, documentada en `.claude/auth-and-membership.md` y `.claude/api-routes.md`, no en este registro |
| `mcp_confirm_tokens` + `confirm_token` en 7 tools | **7 tools actualizadas** (`delete_import`, `delete_asset`, `delete_liability`, `apply_categorization_rule`, `materialize_recurring`, `unreconcile_transfer`, `delete_snapshot`): ganan el parámetro `confirm_token` en su `inputSchema`. No es una tool nueva — es el mismo verbo con una puerta más, exactamente el patrón que el fixture de contrato (Fase 2) existe para detectar |
| `confirm`/preview en `materialize_recurring`, `reconcile_transfers`, `unreconcile_transfer` | **3 tools actualizadas, la reapertura de un hueco real**: las tres llevaban desde el issue #3 aceptando `{}`/un id suelto y ejecutando sin preview — dos de ellas irreversibles (`materialize_recurring` poda, `unreconcile_transfer` es de un solo sentido). `reconcile_transfers` pasa de `NoParams` a `ReconcileTransfersParams {confirm}`; `materialize_recurring` de `NoParams` a `MaterializeRecurringParams {confirm, confirm_token}`; `unreconcile_transfer` gana `confirm`+`confirm_token` sobre su `transaction_id` ya existente |
| `impact` en las 15 escrituras que invalidan FULL | **15 tools actualizadas**: `create_planning_flow`/`update_planning_flow`/`delete_planning_flow`, `create_asset`/`update_asset`/`update_asset_value`, `create_liability`/`update_liability`, `create_budget_entry`/`update_budget_entry`/`delete_budget_entry`, `update_allocation_rule`, `update_fire_settings`, `delete_asset`, `delete_liability` ganan el bloque `impact` en la respuesta (comparten `impact_since`/`impact_probe`, cero SQL propio — la misma core que `get_summary`). No toca ninguna tool de lectura ni las escrituras COND (transacciones): esas quedan deliberadamente sin `impact`, ver el bullet de `api-routes.md` §MCP |
| `idempotency_key` en `create_transaction` | **Tool actualizada**, comparte `create_transaction_core` con el POST HTTP — el parámetro y el 409 `idempotency_key_conflict` llegan sin código propio en `server.rs` salvo la `description` |
| `budget_entry_removed` en el preview de `delete_liability` | **Tool actualizada**, comparte `liability_delete_effects` con el HTTP DELETE — la cuota de presupuesto que desaparece y sus cuatro totales before/after se enseñan en el mismo `effects` que ya contaba `transactions_unlinked` |
| 422 `budget_entry_is_liability_derived` en `PATCH`/`DELETE /v1/budget/entries/{id}` | **Tool actualizada, sin código nuevo**: `update_budget_entry`/`delete_budget_entry` comparten `patch_budget_entry_core`/`delete_budget_entry_core`, así que el 422 llega heredado; documentado en `.claude/api-routes.md` §Budget |
| `heavy::run_projection_sim` (tercer semáforo) | **n/a**: no es superficie observable por MCP — envuelve la simulación por dentro, con el mismo contrato de entrada/salida de siempre (`simulate_projection`, `get_projection`). El único efecto visible sería un 503 `Unavailable` bajo saturación extrema, que ya existía como posibilidad antes de este semáforo (el pool de blocking podía agotarse igual) |

**22 tools con `inputSchema`/payload tocados**, sin solapamientos dobles: las 15 del `impact` +
las 7 del `confirm_token` (`delete_import`, `apply_categorization_rule`, `materialize_recurring`,
`unreconcile_transfer`, `delete_snapshot`, y `delete_asset`/`delete_liability` que YA estaban en el
grupo de `impact` — de ahí que no sumen 22 a secas) + `reconcile_transfers` (gana `confirm` pero no
`confirm_token`, y no invalida FULL así que no está en el grupo de `impact`) + `create_transaction`
(`idempotency_key`, ajeno a los otros dos ejes). El fixture `mcp-catalog.json` cambia exactamente
**22 entradas** — reprodúcelo con el diff de abajo. Cero tools nuevas, cero retiradas: el catálogo
sigue en **52**.

Evaluación de la rama **`feat/mcp-fase-4-transporte`** (Fase 4, issue #85 — «transporte, CORS y
kill-switch»; catálogo **sin cambios en 52**, recontado 52/21/31/31/14/7 el 2026-08-28):

> **Resultado: `n/a`, y se registra en vez de omitirse.** Es la tercera salida válida del contrato
> de §1 y la menos escrita de las tres, precisamente porque «no hay nada que hacer» es la conclusión
> que la gente no documenta. Ninguna tool cambió, ninguna ruta `/v1` cambió y el fixture
> `mcp-catalog.json` no se movió: lo que esta fase toca es el **transporte** de `/mcp` y el
> **protocolo OAuth**, dos capas por debajo de la superficie que este registro gobierna.

| Superficie tocada (4.4.0, Fase 4) | Parity outcome |
|---|---|
| Kill-switch: `/mcp` y las 7 rutas de protocolo pasan de desmontarse a responder 404 JSON `mcp_disabled` | **n/a**. No es una tool ni un campo: es la respuesta de un endpoint **apagado**. Un cliente MCP con el switch echado no ve un catálogo distinto, ve que no hay servidor — y ahora se lo dicen con un código estable en vez de con un 405 mudo. El código nuevo (`mcp_disabled`) vive en `error-codes.json` y `errorMessages.ts`, no en el fixture del catálogo |
| Capa CORS propia de `/mcp`, sin credenciales; preflight completo (`MCP-Protocol-Version`, `Last-Event-ID`, `WWW-Authenticate` expuesta) | **n/a**: cabeceras HTTP del transporte. Ninguna tool las declara ni las lee; `tools/list` es idéntico antes y después. Lo que cambia es **qué origen de navegador puede hablar con qué superficie**, que es política de despliegue (`futurefin-config-and-flags`), no contrato de catálogo |
| Validación de `Origin` (`with_allowed_origins`) y tope de body de 1 MiB en `/mcp` | **n/a**: dos rechazos del transporte (403 y 413) **antes** de que rmcp deserialice nada. Ninguna tool puede observarlos — si se disparan, no llegó a haber llamada a tool. Nota para quien añada una tool con payloads grandes: el tope real de `/mcp` es ahora 1 MiB, no los 4 MiB del SDK |
| `FUTUREFIN_PUBLIC_URL` acepta subpath; `no-store` + `Vary` en la metadata; GC de credenciales OAuth caducadas | **n/a**: protocolo OAuth, explícitamente fuera del catálogo por la fila «OAuth protocol+consent» de §3.1. Cambian **dónde** vive el authorization server y cuánto duran sus filas, no qué puede hacer un token una vez emitido |
| Sesión Streamable HTTP sin ligar a la credencial (decisión, no olvido) | **n/a hoy, con disparador nombrado**: la ligadura solo compraría algo si el servidor emitiera datos por iniciativa propia. La **primera capacidad server→cliente** (notificaciones, `progress`) es a la vez un cambio de catálogo y el momento de reabrir esto — cuando llegue, no será `n/a` por partida doble |
| `spa::mount_static_spa` extraída a la lib; tres ejes nuevos en `TestConfig` | **n/a**: andamiaje de tests y de arranque, sin ruta ni campo. Sí importa para §5: un test que afirme que una tool o una ruta **no** existe tiene que montar `web_static_root`, o está probando un router que no se publica |

Evaluación de la rama **`feat/mcp-fase-5-contexto`** (Fase 5, issue #86 — «coste de contexto y
ergonomía del catálogo»; catálogo **sin cambios en 52**, recontado 52/21/31/31/14/7 el
2026-08-28). Como las Fases 1 y 3, esta fase no añade superficie REST nueva — reescribe el
**coste de lectura** del catálogo (prosa) y **completa contratos de campo** que la Fase 1 ya había
puesto en marcha (nulabilidad → ahora también scope, ventana y motivo). Una sola pareja de tools
gana un parámetro nuevo de verdad:

| Superficie tocada (4.4.0, Fase 5) | Parity outcome |
|---|---|
| `type_tag` en `create_liability` / `update_liability` | **Dos tools actualizadas**. Tri-estado sin `clear_*` (omitir conserva, cadena vacía borra) — mismo patrón que `clear_expense_end_date`/`clear_cap`. Cierra `get_summary.liabilities_by_type_tag` como la dimensión que se leía y no se escribía: sin esto, un agente que veía la partida no tenía forma de corregirla salvo destruir y recrear el pasivo. Test: `mcp_write.rs::liability_type_tag_is_writable_and_reaches_the_summary_breakdown`. **Es la única fila de esta tabla que es realmente «tool actualizada» en el sentido de §4** — el resto llega heredado por las cores compartidas, igual que en la Fase 1 |
| Eco de `view` en 4 respuestas-objeto (`get_summary`, `get_budget`, `get_projection`, `get_allocation_resolution`) vía `LedgerView::as_str()` | **Heredado, cero código en `server.rs`**: la core ya pone el campo, la tool solo lo transporta. Ver §3.4 (nueva) — la parte que sí costó trabajo de tool es la fila siguiente |
| 7 listados con scope pasan a sobre `{"view", "<key>": [...]}` (`list_assets`, `list_liabilities`, `list_planning_flows`, `list_allocation_rules`, `list_transaction_months`, `list_transactions`, `list_transaction_imports`) | **7 tools actualizadas**, salen del bucle byte a byte y pasan a paridad de contenido — regla nueva documentada en §3.4, con el porqué y el precedente (`list_categorization_rules`, 4.0.0) |
| Paginación nueva en `list_snapshots` (`limit`/`offset`, `item_count`/`items_included`, orden `snapshot_date DESC, kind ASC, id ASC`) | **Tool actualizada**, sin sobre `view` (own-user, §3.4) — `list_snapshots_core` gana la firma ampliada; el mismo patrón `limit = None` ⇒ sin `LIMIT`/`OFFSET`/`COUNT` que ya usaba `list_transactions_query`, así que el camino HTTP sin `limit` no cambia. Test: `list_snapshots_paginates_and_declares_item_suppression` |
| Campos nuevos en respuestas-objeto ya cubiertas (`financial_health.basis`, `totals.basis`, `upcoming_flows_count`, `upcoming_last_due_date_ymd`, `window_months`/`window_truncated`/`first_snapshot_date_ymd`/`first_snapshot_month_index`, `markers[].source`, `fine_absent_reason`, `events`/`events_truncated`, `possible_duplicate_of`) | **Heredados por 6 tools** (`get_summary`, `get_budget`, `get_history`, `get_history_cashflow`, `get_projection`, `list_transaction_imports`) sin código propio — comparten core con el HTTP handler. Contrato probado del lado HTTP, no MCP, en `context_fields.rs` (11 tests) porque el campo lo pone la core |
| `summary.liabilities_by_type_tag[].type_tag`: `String` (literal `"(sin etiqueta)"`) → `Option<String>` (`null`) | **Breaking en lectura, heredado por `get_summary`**. Mismo criterio que `category_id` en `CategoryMonthlySeriesEntry` (Fase 1): un sentinela en español dentro de un campo de datos deja de ser un valor de negocio y pasa a ser ausencia real |
| `simulate_projection`: `net_monthly`→`net_recurring_monthly`, `net_monthly_delta`→`net_recurring_monthly_delta`; nuevos `monthly_cash_adjustment`, `net_cash_monthly`, `net_cash_monthly_delta`, `model_note` | **Tool actualizada en su propia core** (`SimKpis`/`simulate_projection_core`) — no hay ruta HTTP homóloga (§3.3, tool-without-endpoint), así que la evaluación recae enteramente sobre la tool. Breaking del único lado que existe: `net_monthly` no se podía «arreglar» sin mentir (sigue siendo `income − expense_total`, los ejes de caja no lo tocan), así que se movió el **nombre**, no la fórmula |
| `GET /v1/history/series` sin `window_months` = 120 meses (antes, todo el histórico); `GET /v1/history/cashflow` curva fina acotada a 36 meses con `fine_absent_reason` nuevo | **Heredado por `get_history`/`get_history_cashflow`**: mismas cores, `description` reescrita para avisar del default acotado. Peor caso medido: 53,6 KB→16,1 KB y 64 KB→20 KB |
| Recorte de `description` (37.214→21.319 caracteres; 26 tools por encima de 600→0) + guardia `tool_descriptions_stay_within_the_context_budget` | **n/a como fila de catálogo** (no es una tool, es un test transversal) **pero toca las 52 descripciones**: ver el paso nuevo en §4 (Tool fn) para el presupuesto y cómo respetarlo al escribir la próxima |

**Dos tools con parámetro nuevo de verdad** (`create_liability`, `update_liability`), **7 tools
con forma de respuesta nueva** (el sobre de vista), **1 tool con paginación nueva**
(`list_snapshots`), y **el resto heredado** vía cores compartidas o, en el caso de
`simulate_projection`, tocado en su propia core por no tener ruta HTTP. Cero tools nuevas, cero
retiradas: el catálogo sigue en **52**. Ninguna fila necesitó promoción HTTP→tool nueva ni omisión
deliberada nueva — el único hueco que esta fase cierra (`type_tag` escribible) ya estaba cubierto
por tools existentes, solo les faltaba el parámetro.

Evaluación de la rama **`feat/mcp-fase-6-capacidades`** (Fase 6, issue #87 — «capacidades nuevas del
catálogo»; **52 → 68**, recontado 68/28/40/40/17/8/40/18 el 2026-08-28). **Es la primera fase del
tren que mueve los contadores**, y la que menos se parece a las anteriores: las Fases 1–5
reescribían el contrato de una superficie que ya existía; ésta añade **capacidad**. La observación
que la genera no salía de ninguna fila del registro: **paridad de rutas ≠ paridad de capacidades**
— 89 de 90 parejas método-ruta estaban clasificadas y aun así la app calculaba cosas que no son una
ruta y que el chat no alcanzaba. **Cuatro de las cinco primeras entradas eran código que ya existía
y solo necesitaba superficie.**

| Superficie (4.4.0, Fase 6) | Parity outcome |
|---|---|
| `GET /v1/transactions/aggregate` | **Tool nueva** `aggregate_transactions` (lectura, acepta `view`, cache NONE). Cierra un modo de fallo, no una comodidad: «¿cuánto llevo gastado en X este año?» obligaba a bajar hasta 500 filas al contexto y sumarlas con un modelo que **no aplicará** `transfer_counterpart_id IS NULL`. El predicado va **dentro de la core** y lo excluido se publica (`reconciled_excluded_count`) para que la exclusión sea auditable. Paridad mes a mes con `/summary`, pineada |
| `GET /v1/transactions/duplicates` + `uncategorized` en los filtros del listado | **Tool nueva** `find_duplicate_transactions` + **`list_transactions` actualizada**. El filtro `uncategorized` no existía **ni en HTTP ni en MCP**: `category_id` solo hacía igualdad de UUID, así que «enséñame lo que falta por clasificar» exigía paginar el ledger entero detectando la ausencia de una clave. Excluyente con `category_id` (`category_filter_exclusive`) |
| `GET /v1/transactions/transfer-matches` | **Tool nueva** `suggest_transfer_matches` (**sin `view`**: own-user por construcción). Regla dura aplicada: **GET aparte, nunca `?dry_run` sobre el POST**. *(Deriva a corregir: el doc-comment del parámetro dice «default 15» y la core usa 30.)* |
| `POST /v1/transactions/transfer-matches/{match_id}` | **Tool nueva** `confirm_transfer_match` (COND, **sin preview/confirm a propósito**: el GET de sugerencias ES el preview). **Supera la omisión §3.1 de `reconcile_pair` sin reabrirla** — solo acepta un `match_id` emitido por el servidor, así que un par arbitrario no es expresable en el esquema. Fuera de la convención de nombres, así que su brazo en el test de annotations es **consciente** |
| `GET /v1/liabilities/{id}/schedule` | **Tool nueva** `get_liability_schedule` (acepta `view`). Cero matemática nueva: envuelve `liability_amortization_schedule`, que publica el `closing_principal` que el motor derivaba 840 veces por request **y tiraba**. Contraste cruzado con `simulate_projection` pineado (`the_what_if_debt_kpis_agree_with_the_liability_schedule`) |
| `GET /v1/projection/deflate` + `points[].net_worth_real` | **Tool nueva** `deflate_amount` (**sin `view`**: la inflación es de la instalación) + **`get_projection` actualizada** (hereda `net_worth_real` y `deflation_annual_inflation_percent` por la core). **NO reabre el motor «real puro»** rechazado en v1.2.0, y la afirmación es testable: `net_worth_real == net_worth / (1+i)^(month_index/12)`, o sea cero información que el motor no haya producido |
| `GET /v1/allocation-rules/goals` | **Tool nueva** `list_goals` (acepta `view`). **Sin tabla `goals`**: el cap YA es el objetivo, y una tabla nueva duplicaría el número — la lección de las contribuciones por activo. Único hueco: el techo se resuelve fuera del motor (`resolve_cap_ceiling_eur`) porque con sobrante ≤ 0 el motor emite `cap_ceiling: null` (**issue #96**) |
| `GET /v1/changes` | **Tool nueva** `list_recent_changes` (acepta `view`). **La mitad honesta de una auditoría**, y las tres carencias son campos de la respuesta (`covers_deletions: false`, `deletions_absent_reason`, `tables_missing_updated_at`) en vez de omisiones. Venderlo como «auditoría» sin esa nota sería mentir |
| `POST /v1/transactions/batch` (ya existía) | **Tool nueva** `create_batch`. **Reabre a conciencia la fila «defer» de §3.1**, cuyo *revisit trigger* era «demanda real de lotes >10 desde el chat»: un importador desatendido **es** esa demanda. Estaba bloqueada por la idempotencia de la Fase 3 — sin ella un lote reintentado es peor que N llamadas — y se resuelve con **una clave por LOTE** (el lote es una unidad atómica), sin tabla nueva: una fila por ítem con clave derivada y el hash del lote entero. COND, sin `impact` |
| `POST /v1/history/snapshots` + `PUT /v1/history/snapshots/{id}` (ya existían) | **Dos tools nuevas**, cierran §3.2 #1. Cache **NONE por contrato (D12)**; publican `"affects_projection": false` en lugar de `impact` |
| `POST /v1/allocation-rules` + `DELETE /v1/allocation-rules/{id}` (ya existían) | **Dos tools nuevas**, cierran §3.2 #2. FULL + `impact`; el delete exige `confirm_token`. `SinkPolicy::Forbidden` en la creación (ver §3.2 fila 2 y su hueco conocido) |
| `PATCH` + `DELETE /v1/categories/{id}` (ya existían) | **Dos tools nuevas**, cierran §3.2 #3. NONE; el delete con preview/confirm sin `confirm_token` |
| `PATCH /v1/installation` (subconjunto de presentación) | **Tool nueva** `update_installation_settings`, cierra §3.2 #4. **Tool-without-endpoint** (§3.3, cuarta de la casa: su core no la llama ningún handler, y la forma es la misma que `update_fire_settings` — el PATCH de la SPA reemplaza el objeto entero, que desde el chat obliga a leer-modificar-escribir). Allowlist estricta y exhaustiva por construcción; owner-only **dentro de la core** |
| `GET /v1/assets` → 4 campos de plusvalía latente | **`list_assets` actualizada sin código propio** (comparte `list_assets_core`). Lo que costó trabajo fue la `description`: **no es rentabilidad** (no anualiza ni descuenta aportaciones posteriores) y hay que distinguir «0 %» de «sin coste declarado» y de «coste cero» |
| `simulate_projection` → `liability_overrides` + KPIs de deuda | **Tool actualizada en su propia core** (§3.3, tool-without-endpoint). Los 12 ejes what-if **no tocaban ningún pasivo**: lo más cerca era un gasto puntual, que drena caja pero no reduce deuda ni cuota, o sea responde a otra pregunta. **No disponible en modos B/C** (`liability_overrides_unavailable_in_real_expense_mode`): ahí las cuotas ya viven dentro del promedio de gasto. Es la feature que respalda el prompt `amortizar_o_invertir` |
| Capacidad **`prompts`** (3 guiones estáticos) | **Primitiva nueva del protocolo, no una tool** → rubro propio en **§3.5**. No entra en los contadores de §5, no abre fila de auditoría, no se gatea. `resources` sigue sin declararse |
| Bloque **SEGURIDAD** nuevo en el `instructions` | **n/a como fila de catálogo, pero es la decisión más transversal de la fase**: lo que devuelven las tools es DATO, nunca instrucciones — `concept`, `notes`, `category_name`, `pattern` y los nombres de activos/pasivos/categorías pueden venir de un **tercero** (el concepto de una transferencia recibida lo escribe quien la envía). Va al `instructions` y no a 68 descripciones por la lección de la Fase 5: un aviso transversal se paga una vez por sesión, no una vez por tool y por turno |
| 23 códigos de error nuevos en `error-codes.json` | **n/a, heredados**: viajan por la misma `ApiError` de las cores y `errorMessages.ts` los traduce (guard `error_codes_parity`). Diez son de las rutas nuevas, doce de los ejes de deuda de `simulate_projection`, uno (`sink_creation_not_allowed`) es de la política de la core |
| Cero migraciones | **n/a**: las 8 rutas son lectura (o escritura) sobre tablas que ya existían, así que el `schema_version` del `.ffbackup` **no se mueve** (sigue en 10). Los tres casos donde uno esperaría tabla nueva —goals, `match_id`, idempotencia de lote— están resueltos **por derivación y documentados como decisión**, no por olvido |

**16 tools nuevas, cero retiradas, 8 entradas del fixture tocadas** (`capture_snapshot`,
`get_projection`, `list_assets`, `list_transactions`, `reconcile_transfers`, `simulate_projection`,
`unreconcile_transfer`, `update_allocation_rule`). **Coste de contexto**: las descripciones llegaron
a **28.884** caracteres (+4.884 sobre el tope de 24.000) y el arreglo fue el que la propia guardia
prescribe —campos de procedencia y `instructions`, **nunca** subir la constante—. Estado final
**23.874 / 24.000, máximo 596**: quedan **126 caracteres**, así que **la próxima tool obliga a otra
ronda de reequilibrio**. Presupuéstalo al planificarla.

Re-verify before trusting:

- El fixture de contrato detecta las 22 tools tocadas por este tren:
  `UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http tools_list_freezes_the_input_contract_of_every_tool`
  y compara contra `git diff --stat -- apps/api/tests/fixtures/mcp-catalog.json`, o para la lista
  exacta de nombres:
  ```bash
  python3 -c "
  import json,subprocess
  old=json.loads(subprocess.run(['git','show','HEAD:apps/api/tests/fixtures/mcp-catalog.json'],capture_output=True,text=True).stdout)
  new=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))
  oi={t['name']:t for t in old['tools']}; ni={t['name']:t for t in new['tools']}
  print(sorted(n for n in ni if oi.get(n)!=ni.get(n)))
  "
  ```
  (Compara el `git show HEAD:…` de tu checkout contra el fichero en disco: útil para ver qué
  cambió justo antes de commitear la Fase 3, y sirve igual de guardia de drift después — con el
  árbol limpio y sin cambios pendientes, la lista sale vacía.)
- `confirm_token` de dos fases: `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_confirm_and_impact`.
- Auditoría + scope: `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_audit_and_scope`.
- Idempotencia + preview del presupuesto + semáforo: `TEST_DATABASE_URL=… cargo test -p futurefin-api --test write_safety_phase3`.
- Tool counts + gate invariants: the §5 block (**68/28/40/40/17/8/40/18 el 2026-08-28, tras la
  Fase 6**; 52/21/31/31/11/1 on 2026-08-22, recontado 52/21/31/31 el 2026-08-27;
  50/21/29/29/10/1 on 2026-08-20). Ningún tren entre 4.0.0 y la Fase 5 los movió — **la Fase 6
  (issue #87) sí**, y es la única del tren 4.4.0 que lo hace.
- *(Fase 1, histórico.)* El fixture de contrato detectaba 9 de las 10 tools tocadas por **aquel**
  tren (`update_fire_settings` era la excepción — ver su fila arriba). **Ejecutado hoy, el comando
  devuelve el resultado de la Fase 6** (16 altas + 8 tocadas): para reproducir el de la Fase 1 hay
  que comparar contra el fixture de aquel commit, no contra `HEAD`.
  `UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http tools_list_freezes_the_input_contract_of_every_tool`
  y compara el diff contra `git diff -- apps/api/tests/fixtures/mcp-catalog.json` (debe tocar
  exactamente `create_categorization_rule`, `get_allocation_resolution`,
  `get_category_monthly_series`, `get_history`, `get_history_cashflow`, `get_projection`,
  `get_transactions_summary`, `simulate_projection`, `update_transaction`).
- `POST /v1/auth/sso` sigue sin tool (fila §3.1): `grep -n 'sso' apps/api/src/mcp/server.rs` (vacío)
  frente a `grep -n 'sso' apps/api/src/routes/mod.rs` (la ruta existe y se monta siempre).
- Las dos rutas de HA-IdP siguen sin tool (fila §3.1, 4.3.1):
  `grep -rn 'ha_idp\|ha_sso\|auth/ha' apps/api/src/mcp/` (vacío) frente a
  `grep -n 'ha/start\|ha/callback' apps/api/src/routes/mod.rs` (las dos, montadas siempre).
- Las restricciones del esquema siguen congeladas (Fase 2): el fixture trae `constraints` +
  `constraints_sha256_12` por tool, y el fallo nombra la tool, la ruta del schema (`$`, `$.kind`,
  `$defs.TaxBracketParam.pct`) y **qué restricción se perdió**. Comprobación de que el gate sigue
  vivo: quita un `#[schemars(extend("enum" = …))]` o un `#[serde(deny_unknown_fields)]` y el test
  debe decir «PERDIDA la restricción». Regenerar el fixture ante una línea así **no arregla nada,
  borra la prueba**.
- Ninguna clave en español en el wire MCP:
  `grep -n '"resumen"' apps/api/src/mcp/server.rs` (vacío; los `out.resumen` sin comillas son los
  campos del handler HTTP, que sí siguen en español y no son de este módulo).
- Frozen catalog still matches the code (never count quotes — run the test):
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_http tools_list_returns`
- Cores still own invalidation, MCP module still SQL-free:
  `grep -rn 'refresh_projection_after_mutation\|invalidate_projection_if_savings' apps/api/src/mcp/` (empty)
- `reconcile_pair` sigue sin tool propia (§3.1, fila **superada** por `confirm_transfer_match`):
  `grep -n 'reconcile_pair' apps/api/src/mcp/server.rs` (vacío), y el `match_id` no es un UUID —
  `grep -n 'MATCH_ID_STRING' apps/api/src/mcp/server.rs` debe dar `^[0-9a-f]{24}$`.
- §3.2 backlog **cerrado** desde la Fase 6 — la comprobación se invierte:
  `grep -c 'name = "create_snapshot"\|name = "update_snapshot"\|name = "create_allocation_rule"\|name = "delete_allocation_rule"\|name = "update_category"\|name = "delete_category"\|name = "update_installation_settings"' apps/api/src/mcp/server.rs`
  → **7** (mientras las filas estaban abiertas daba 0).
- Choke points still route here: `grep -n 'mcp-parity' CLAUDE.md .claude/adding-handler.md .claude/skills/futurefin-change-control/SKILL.md .claude/skills/futurefin-docs-and-writing/SKILL.md .claude/api-routes.md`
- Recipe step 4's derivation still holds: read the `is_write`/`expect_destructive`/`expect_idempotent`
  match arms in `mcp_http.rs::tools_list_exposes_annotations_on_every_tool`.
- Context budget (Fase 5) still respected — the test fails only if the total goes UP, never down:
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_http tool_descriptions_stay_within_the_context_budget`,
  and the exact fixture snapshot: `python3 -c "import json;t=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))['tools'];l=[x['description_len'] for x in t];print(len(t),sum(l),max(l))"`
  (`68 23874 596` today, **126 from the ceiling**; `52 21319 596` at the close of Fase 5;
  `37214`/`3821`/26-over-600 before Fase 5 — none of these lives anywhere but this skill and the
  CHANGELOG, so don't freeze any of them into the guard's constant).
- View echo on the 7 enveloped listings, and the content parity that replaces byte parity for them:
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_http list_tools_echo_the_applied_view_and_keep_content_parity`;
  the 5 still on byte parity: `… --test mcp_http new_read_tools_match_http_endpoints`.
- `type_tag` writable via MCP:
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test mcp_write liability_type_tag_is_writable_and_reaches_the_summary_breakdown`.
- The 11 Fase-5 context fields (`basis`, `upcoming_flows_count`, `window_truncated`,
  `markers[].source`, `fine_absent_reason`, `possible_duplicate_of`, `events`, …) have their own
  suite, run over the HTTP path — not MCP — because the core is what sets the field:
  `TEST_DATABASE_URL=… cargo test -p futurefin-api --test context_fields`.
- Tool counts after Fase 6: **68/28/40/40/17/8/40/18**, recounted 2026-08-28 with the §5 commands.
  Fases 0–5 held at 52/21/31/31/14/7 — Fase 6 is the one that moves them.
- Las 16 altas y las 8 entradas tocadas del fixture, sin fiarte de esta lista:
  ```bash
  python3 -c "
  import json,subprocess
  old=json.loads(subprocess.run(['git','show','HEAD:apps/api/tests/fixtures/mcp-catalog.json'],capture_output=True,text=True).stdout)
  new=json.load(open('apps/api/tests/fixtures/mcp-catalog.json'))
  o={t['name']:t for t in old['tools']}; n={t['name']:t for t in new['tools']}
  print('altas', sorted(set(n)-set(o))); print('bajas', sorted(set(o)-set(n)))
  print('tocadas', sorted(k for k in set(o)&set(n) if o[k]!=n[k]))
  "
  ```
- La capacidad `prompts` está declarada y `resources` NO:
  `grep -n 'enable_prompts\|enable_resources' apps/api/src/mcp/server.rs` (la primera aparece, la
  segunda no). Los tres nombres: `grep -n 'const PROMPTS' -A6 apps/api/src/mcp/server.rs`.
- El invariante del sumidero tiene **un solo punto de commit**, y lo comprueba un `#[test]` sin BD
  que lee su propio fichero: `cargo test -p futurefin-api --lib el_modulo_tiene_un_unico_punto_de_commit`.
