# CLAUDE.md

This file provides guidance to any AI model (or human) working with code in this repository. It is the **single entry point**: everything else — reference docs, runbooks, history — is reachable from here.

## Start here — route your task

FutureFin is a self-hosted household finance + FIRE-planning app: Rust/Axum API (`apps/api`), pure-Decimal projection engine (`crates/engine`), React 19 SPA (`apps/web`), PostgreSQL — **embebido en la propia imagen Docker desde 3.0.0** (un solo contenedor en producción; en dev sigue siendo un Postgres aparte). Money is NEVER `f64` in domain code. UI copy en español; código e identificadores en inglés.

The repo carries three documentation layers. Consult them in this order:

1. **This file** — always-on norms, task routing, architecture summary, daily git flow. Everything context-specific lives one link away.
2. **Skills** (`.claude/skills/*/SKILL.md`) — task-shaped runbooks with verified commands, the project's history and its discipline. **Pick by task type** (table below).
3. **Reference docs** (`.claude/*.md`) — per-area fact sheets (routes, schema, engine, env…).

El manual de usuario (`docs/*.md`) es una cuarta superficie con la **misma barra de exactitud**: las secuencias de comandos canónicas (dev, build local, producción) viven allí — ver §Commands.

### Skill routing (load BEFORE starting the matching task)

| Your task looks like… | Load |
|---|---|
| Any change you plan to merge (gates, migration/release rules, pre-merge checklist) | [`futurefin-change-control`](.claude/skills/futurefin-change-control/SKILL.md) |
| A symptom: wrong numbers, HTTP errors, unhealthy container, layout breakage | [`futurefin-debugging-playbook`](.claude/skills/futurefin-debugging-playbook/SKILL.md) |
| "Why is X designed this way?" / touching cache, auth, scoping, serialization | [`futurefin-architecture-contract`](.claude/skills/futurefin-architecture-contract/SKILL.md) |
| Understanding the FIRE/projection math (SWR, gross-up, cascade, inflación) | [`futurefin-fire-domain-reference`](.claude/skills/futurefin-fire-domain-reference/SKILL.md) |
| About to (re)introduce an old idea — check what was already tried and rejected | [`futurefin-failure-archaeology`](.claude/skills/futurefin-failure-archaeology/SKILL.md) |
| Añadir/cambiar rutas o handlers (¿tool MCP?), añadir/actualizar una tool MCP, deriva del catálogo `/mcp` | [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md) |
| Tocar una métrica o un KPI: su base, su ventana, su nombre, o añadir/retirar uno | [`futurefin-metric-definitions`](.claude/skills/futurefin-metric-definitions/SKILL.md) |
| Añadir/cambiar un fixture, ilustrar un cambio con números, capturas, datos de demo | [`futurefin-data-hygiene`](.claude/skills/futurefin-data-hygiene/SKILL.md) |
| Env vars, compose files, query params, fire_settings axes; adding a config axis | [`futurefin-config-and-flags`](.claude/skills/futurefin-config-and-flags/SKILL.md) |
| Add-on de Home Assistant, ingress/subpath, `/data`, SSO por cabeceras de proxy | [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md) (canal, backups, upgrade) + [`futurefin-config-and-flags`](.claude/skills/futurefin-config-and-flags/SKILL.md) (opciones→env, `FUTUREFIN_BASE_PATH`/`TRUSTED_PROXY_*`); el **porqué** de las dos concesiones (iframe, identidad delegada) está en `futurefin-architecture-contract` D17/D18 |
| Setting up / building / dev-environment failures | [`futurefin-build-and-env`](.claude/skills/futurefin-build-and-env/SKILL.md) |
| Deploy, upgrade, rollback, backups, logs, production ops | [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md) |
| Measuring: timings, cache hits, payload sizes, DB state (ships scripts) | [`futurefin-diagnostics-and-tooling`](.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md) |
| Running or writing tests; what evidence a change needs; fire-parity fixture | [`futurefin-validation-and-qa`](.claude/skills/futurefin-validation-and-qa/SKILL.md) |
| Updating CHANGELOG/README/docs; doc drift; house style; templates | [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md) |
| Improving projection realism/correctness (Monte Carlo, taxes, invariants…) | [`futurefin-projection-realism-campaign`](.claude/skills/futurefin-projection-realism-campaign/SKILL.md) |
| Numeric analysis: closed forms, index proofs, f64 safety, determinism audits | [`futurefin-proof-and-analysis-toolkit`](.claude/skills/futurefin-proof-and-analysis-toolkit/SKILL.md) |
| "What should we build next?" / public capability claims | [`futurefin-research-frontier`](.claude/skills/futurefin-research-frontier/SKILL.md) |
| Turning a hypothesis into an accepted change (evidence bar, predict-then-run) | [`futurefin-research-methodology`](.claude/skills/futurefin-research-methodology/SKILL.md) |

## Norm — read the issues BEFORE starting a task

`gh issue list --state open` **antes de empezar**, siempre. Contrasta lo que te han pedido con lo
que ya hay abierto y, si algo solapa, **pregúntale al owner si lo abordamos también** en vez de
decidirlo tú: arreglar media cosa deja la otra media abierta describiendo un bug que ya no existe.

Al terminar, el commit que cierra un issue lleva `Closes #N` y la entrada del CHANGELOG lo
referencia como `(issue #N)` — convención que el repo ya usa. INCIDENTE: los issues #5 y #6 se
resolvieron enteros y se quedaron **abiertos** porque el commit no los mencionaba; nadie se enteró
hasta la auditoría previa a hacer público el repositorio.

## Norma — una incoherencia aparente se abre como issue, no se calla

Cuando encuentres algo que **el contrato promete y el código no cumple** (o al revés), y no puedas
arreglarlo en el mismo cambio, **abre un issue con la evidencia**. No lo dejes en un comentario del
PR, no lo apuntes «para luego», y sobre todo **no lo propagues** escribiendo la promesa rota como si
fuera cierta.

Vale también aunque el contrato sea explícito y esté firmado: que una decisión esté escrita no la
hace verdadera hoy. La revisión adversarial del MCP (agosto 2026) abrió cuatro issues así —#95, #96,
#97, #99— y **ninguno era regresión del trabajo en curso**: eran deuda preexistente que solo apareció
al construir encima.

**Énfasis especial en las incoherencias NUMÉRICAS.** Un número en la documentación se lee como
verificado, se copia sin comprobar y sobrevive a la realidad que describía. Los que más han mordido
aquí:

- **Contadores congelados** («52 tools», «19 de lectura», «11 previews», «31 escrituras»): quedan
  obsoletos al primer cambio y nadie los recuenta. **Prefiere siempre el comando al número** — es la
  norma de la casa y existe por esto.
- **Defaults y cotas duplicados** entre el esquema y el runtime. El caso real: el schema de una tool
  MCP —el texto que **lee el modelo**— anunciaba «default 15» cuando el real era 30, y topaba en 60
  donde la core acepta 365; el mismo parámetro funcionaba por HTTP y fallaba por MCP.
- **Greps de re-verificación que ya no encuentran nada.** Un `grep` vacío es deriva **silenciosa**:
  o el comando está mal escrito, o describe algo que se retiró, y en ninguno de los dos casos avisa.
- **Comandos que se cuentan a sí mismos**: si escribes el patrón dentro del comentario que lo
  explica, el `grep` lo cuenta. Pasó dos veces en la misma sesión, la segunda **arreglando la
  primera**.

### Antes de abrirlo

1. **Verifícalo tú** contra el código, con `path:line`. Un issue que resulta ser falso quema la
   señal de todos los demás.
2. **Si cabe arreglarlo en el mismo cambio, arréglalo** y no abras nada. La tabla de erratas de
   [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md) §7 es para lo
   que **no** se puede arreglar ahí: una errata registrada es deuda, no un archivo. Cuando el arreglo
   llega, la fila se borra.
3. **Explica el coste, no solo el hecho.** Los issues útiles de esta tanda no decían «esto está
   mal», decían qué se rompió por creerlo: uno ya había obligado a **duplicar** una función del
   motor; otro señalaba que el guard que debía cazarlo miraba solo la mitad del contrato.

## Norm — delegate to subagents whenever possible (Opus or smaller, never Fable)

**Default posture: delegate.** Any unit of work that can be handed to a subagent should be. **Pick the subagent's model by difficulty — at most Opus, never Fable** (the main session is the only place Fable runs; delegating on Fable duplicates its cost for work a smaller model does fine):

- **Opus** (`model: "opus"`): anything touching money math, projection/FIRE semantics, Decimal handling, index arithmetic, invariants, or adversarial review. This repo's failure modes are *silent wrong numbers*, and a cheaper model reads as confident on exactly those — when in doubt between tiers, Opus.
- **Sonnet** (`model: "sonnet"`): exploration/search across many files, per-area research, doc audits, mechanical multi-file edits with a clear spec.
- **Haiku** (`model: "haiku"`): trivial mechanical sweeps — grep-and-report, listing occurrences, format checks.

**Adjust reasoning effort to the task** as well (`effort` where the harness supports it, e.g. Workflow `agent()` calls): `low` for mechanical stages, default/`medium` for normal work, `high`+ only for the hardest verify/judge/math stages. Model tier and effort are independent knobs — a Sonnet sweep at low effort is often the right shape; an Opus verifier of money math deserves high effort.

**Delegate by default:**
- Exploration and search that spans many files or naming conventions (`Explore` / `general-purpose`).
- Per-area research feeding a change (routes, schema, engine, frontend structure).
- Independent work that can run concurrently — launch those subagents **in a single message** so they run in parallel.
- Reviews, audits and adversarial verification of a finding before you act on it.
- Anything whose raw output (file dumps, long logs, test noise) would flood the main context; the subagent returns the conclusion, not the dump.

**Keep in the main session** (do NOT delegate): git operations (commit, push, tag, merge to `main`), migrations and releases, anything destructive or irreversible, and small edits to a file you already have open — delegation there costs more than it saves.

**How to delegate well:**
- Subagents start with **fresh context**: they do not inherit yours. State the task, the files/paths involved, and **which skill they must load** from the routing table above.
- Ask for a conclusion plus the evidence for it (paths, line numbers, command output) — never a bare verdict.
- Never take a subagent's claim at face value on money math or invariants: the main session stays the owner and re-verifies with the gates in [`futurefin-change-control`](.claude/skills/futurefin-change-control/SKILL.md) before committing.

## Reference docs (`.claude/`)

Extended reference — read these before working on the relevant area:

| File | Contents |
|------|----------|
| [`.claude/api-routes.md`](.claude/api-routes.md) | Full route map with auth patterns |
| [`.claude/mcp-catalog.md`](.claude/mcp-catalog.md) | Catálogo MCP: semántica por tool, preview/confirm, sobres de listado, transporte de `/mcp` y sus contadores |
| [`.claude/data-model.md`](.claude/data-model.md) | DB schema, table invariants, FIRE JSONB shape |
| [`.claude/engine.md`](.claude/engine.md) | Projection engine public API and simulation loop |
| [`.claude/financial-contracts.md`](.claude/financial-contracts.md) | Contratos financieros canónicos: qué magnitud representa cada cifra, con qué unidad y convención, y las divergencias conocidas con la realidad española (deuda contabilizada, cada una con su issue) |
| [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md) | Auth flow, roles, cookie, pending users |
| [`.claude/env-and-config.md`](.claude/env-and-config.md) | All env vars, `.env` loading order, Vite config |
| [`.claude/backend-structure.md`](.claude/backend-structure.md) | apps/api/src module map + step-by-step pattern for adding a new API handler |
| [`.claude/frontend-structure.md`](.claude/frontend-structure.md) | SPA layout post-refactor (lib/, api/, components/, views/, auth/) and where to put what |
| [`.claude/design-system.md`](.claude/design-system.md) | V1 redesign — tokens, paleta, formato de cifras, reglas para añadir UI nueva (LEE ANTES de tocar estilos) |
| [`.claude/git-and-releases.md`](.claude/git-and-releases.md) | Rama única, ritual de release completo (auto-tag, bump del add-on) y rutina de dependencias |
| [`.claude/tests.md`](.claude/tests.md) | How to run + write backend integration tests (Postgres schemas) and frontend Vitest tests |

**Keep these files up to date** whenever the corresponding area changes (routes, schema, env vars, etc.). The same applies to the skills: each `SKILL.md` ends with a "Provenance and maintenance" section listing one-line re-verification commands — if your change makes one of those facts stale, update the skill in the same PR. If you find a doc/code disagreement you cannot fix in the same change, record it in the standing-errata table of [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md); the code is ground truth.

## Commands

Las secuencias canónicas viven en el manual (`docs/`) y en las referencias — no se duplican aquí:

- **Entorno de desarrollo (split-dev), build y verificación**: [`docs/desarrollo.md`](docs/desarrollo.md) — `./scripts/dev-db.sh`, `cargo run` + `npm run dev:web`, `./scripts/test-all.sh`, y §Construir la imagen en local (`./scripts/build-local-image.sh` + `docker-compose.local.yml`).
- **Tests — qué suite correr y cómo escribirla**: [`.claude/tests.md`](.claude/tests.md) (el Postgres de test `ff-test-db` en `:5433` y `TEST_DATABASE_URL` viven ahí).
- **Producción — deploy, logs, psql embebido, modo rescate**: [`docs/instalacion.md`](docs/instalacion.md) + [`docs/actualizar.md`](docs/actualizar.md); runbook completo en la skill [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md).
- Referencia rápida de comandos individuales (cargo/npm): skill [`futurefin-build-and-env`](.claude/skills/futurefin-build-and-env/SKILL.md) §5.

## Architecture

### Workspace layout
```
Cargo workspace: apps/api + crates/domain + crates/engine + crates/engine-stochastic
npm workspace:   apps/web (futurefin-web)
```

**crates/domain** — shared primitives: `UserId` (newtype over `Uuid`), re-exports `Decimal` and `Uuid`. No `f64` for monetary values anywhere in the domain.

**crates/engine** — pure projection math (`project_net_worth_series`), historical-snapshot interpolation (`history.rs`), liquidity runway (`runway.rs`), pasivos que devengan interés (`RepaymentModel`, 4.2.0) y, desde 5.0.0, el plan por fases (`phases.rs`), las reglas de retirada (`withdrawal.rs`) y los solves (`solve.rs`). No I/O, no DB, **sin RNG**; only `Decimal` arithmetic; has unit tests. API pública, bucle de simulación y semántica completa: [`.claude/engine.md`](.claude/engine.md).

**crates/engine-stochastic** (5.0.0) — Monte Carlo sobre el **mismo** bucle: instancia el núcleo genérico (`MoneyOps`) con `F64Money` y publica **solo salidas estadísticas** (bandas p10/p50/p90, probabilidad de éxito, agotamiento por edad). De aquí no sale un euro publicado, el `rand_chacha` vive solo aquí, y el freezer `no_f64` de `crates/engine` **no se toca**: la red que lo sostiene es la puerta de degeneración (`tests/degeneration.rs`, ≤ 1 € por mes en todos los casos de la batería). Porqué y condiciones: `futurefin-architecture-contract` D4.

**crates/engine-stochastic** — la evaluación estocástica (Monte Carlo) del MISMO bucle: no tiene
simulación propia, tiene el tipo `F64Money` que implementa `MoneyOps` e instancia el núcleo
genérico de `crates/engine` en coma flotante (5.0.0). El freezer `f64` del motor **no se toca**: la
coma flotante vive solo aquí, y de aquí **no sale un euro** — sus salidas son estadísticas
(probabilidad de éxito, percentiles), nunca un KPI monetario. Puerta de aceptación:
`tests/degeneration.rs` (los dos caminos, mes a mes, todo el horizonte).

**apps/api** — Axum HTTP server. Mapa de módulos y receta para añadir un handler: [`.claude/backend-structure.md`](.claude/backend-structure.md); contrato de cada ruta: [`.claude/api-routes.md`](.claude/api-routes.md).

**apps/web** — React 19 + TypeScript + Vite. `App.tsx` is the composition root (auth gate + global state + route → view dispatch). All views, components, helpers and types live in separate modules — see [`.claude/frontend-structure.md`](.claude/frontend-structure.md).

### Key design decisions

Resumen ejecutivo; el detalle y el porqué viven en el doc o la skill que cada bullet enlaza.

**Authentication**: cookie `ff_session` (UUID, `HttpOnly`, `SameSite=Lax`), sesión en DB con expiry; el primer usuario registrado se convierte en owner de la instalación. Detalle: [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md).

**Installation singleton**: one row in `installation` per deployment. All financial data belongs to it. Users who register but aren't in `installation_memberships` are "pending" — they see no data until the owner approves them.

**Money**: always `rust_decimal::Decimal`. API serializes amounts as decimal strings; the frontend receives and sends strings, never floats. Never use `f64` for financial values in domain code — hay **dos** excepciones sancionadas y las dos están acotadas en `futurefin-architecture-contract` D4: los arrays numéricos de las series de chart (**publicación**) y el crate `crates/engine-stochastic` (**cómputo**, solo salidas estadísticas).

**Dual-port dev**: Vite `:8080`, API `:8081`; `vite.config.ts` lee `FUTUREFIN_API_PORT` y `WEB_DEV_PORT` del `.env` raíz. La imagen Docker sirve todo en `:8080` (`WEB_STATIC_ROOT=/app/web`).

**Imagen autocontenida (3.0.0)**: en producción PostgreSQL 16 corre **dentro** del contenedor (socket Unix, sin TCP), supervisado por `apps/api/docker-entrypoint.sh` (PID 1). El entrypoint **jamás borra un cluster**, la imagen **no declara `VOLUME`** y aborta si no hay volumen montado en `PGDATA`. Runbook: skill `futurefin-run-and-operate`; porqué y trampas: `futurefin-architecture-contract` D13 + `futurefin-failure-archaeology`.

**View scoping**: `?view=mine` filtra por `owner_user_id = current_user` y **es el default desde 5.0.0** (`household` hay que pedirlo explícitamente; un literal desconocido es 400 `invalid_view`). Sigue siendo un filtro de cliente, **no** una frontera de autorización — la frontera de ESCRITURA sí es nueva: toda mutación del ledger exige ser el dueño de la fila (403 `not_row_owner`, D23) y `household` es un agregado de solo lectura. Los handlers usan los helpers de `handlers/person_view.rs` (`scope_where` + `bind_scope_as/scalar`), y toda respuesta cuyo contenido dependa del scope **ecoa** la vista aplicada en un campo `view` (4.4.0). Detalle e incidente: `futurefin-architecture-contract` D2 + [`.claude/backend-structure.md`](.claude/backend-structure.md).

**Reads never mutate**: los pasivos con `payment_end_date < today` se **filtran** en los GET, nunca se borran (la legacy `purge_expired_liabilities` se retiró en mayo 2026: los GET emitían `DELETE`s en silencio). Detalle: `futurefin-architecture-contract` D5.

**Histórico por snapshots**: snapshots manuales per-user; el servidor interpola la serie histórica entre ellos (`GET /v1/history/series`, cash-flow tier-2 en `/v1/history/cashflow`). **No son inputs del engine de proyección** → sus mutaciones jamás invalidan la cache (regresión: `snapshot_mutations_do_not_touch_projection_cache`). Detalle: [`.claude/api-routes.md`](.claude/api-routes.md) §History + [`.claude/data-model.md`](.claude/data-model.md).

**Transactions**: histórico de gasto per-user (`/v1/transactions`: imports CSV, reglas de categorización, recurrentes, conciliación de transferencias). **Contrato de cache condicionado al modo** (`fire_settings.savings_source`): con `budget` (A, default) las transacciones no son inputs del engine y sus mutaciones **nunca** invalidan la cache de proyección; con `transactions_avg` (B) o `budget_income_real_expense` (C) **sí lo son**, y toda mutación que cambie el conjunto **o su clasificación** invalida (regresión: `transactions_projection_cache.rs`). Detalle: [`.claude/api-routes.md`](.claude/api-routes.md) §Transactions + [`.claude/data-model.md`](.claude/data-model.md).

**Servidor MCP (3.0.0)**: endpoint `/mcp` (Streamable HTTP) en el mismo binario. Las tools reutilizan las core fns `*_core` de los handlers — nunca dupliques SQL, validación ni tipos de respuesta en una tool — y toda escritura pasa por las tres puertas de `require_mcp_write`. Catálogo, contadores, preview/confirm y presupuesto de contexto: [`.claude/mcp-catalog.md`](.claude/mcp-catalog.md) (léelo antes de tocar nada MCP). **La superficie MCP es derivada de la API HTTP y se mantiene en paridad**: todo cambio de rutas/handlers pasa la evaluación de [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md) antes de mergear.

**OAuth 2.1 embebido (3.1.0)**: el binario es también authorization + resource server de `/mcp` (lo que exige el conector de claude.ai web). **Jamás registres una ruta backend en `/oauth/authorize`** — la sirve el fallback SPA. Detalle (rutas de protocolo, kill-switch `FUTUREFIN_MCP_ENABLED`, issuer, «Entrar con Home Assistant»): [`.claude/api-routes.md`](.claude/api-routes.md) §OAuth + [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md).

**Subpath por request + add-on de Home Assistant**: el servidor monta sus rutas en la raíz pero el prefijo público es **por request** (`apps/api/src/prefix.rs`); el SSO de cabeceras y el relax del anti-clickjacking exigen **peer de confianza** (opt-in doble — con AUTH y sin IPS el binario no arranca). El add-on va empaquetado en este repo (`repository.yaml` + `addon/futurefin/`). Detalle: [`.claude/api-routes.md`](.claude/api-routes.md), [`.claude/auth-and-membership.md`](.claude/auth-and-membership.md) y skill `futurefin-run-and-operate`.

**OpenAPI**: generated via `utoipa`, served at `GET /openapi.json`. All handler structs annotated with `#[utoipa::path]`.

**CORS**: una lista (`CORS_ORIGINS`), **dos capas con privilegios distintos** — la del API con credenciales (cookie `ff_session`) y la de `/mcp` sin ellas (su credencial es el header `Authorization`). Detalle, incidente 4.3.1 y trampas de axum: [`.claude/api-routes.md`](.claude/api-routes.md) §CORS.

### Migrations
SQLx embed migrations in `apps/api/migrations/`. Run automatically on startup via `db::run_migrations`. Filenames: `YYYYMMDDHHMMSS_description.sql`. No auto-repair: a checksum mismatch fails loud and must be resolved by hand (e.g. `psql -c "DELETE FROM _sqlx_migrations WHERE version = X"` if the change is genuinely idempotent; en producción el `psql` es `docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin`). Antes de aplicar migraciones nuevas, el entrypoint escribe un backup automático `pre-migration-*.sql.gz` en el volumen `ffdata`.

## UI conventions

**Antes de tocar UI o estilos, lee [`.claude/design-system.md`](.claude/design-system.md)** — tokens y paleta (nunca hex hardcoded: `var(--ff-*)`), formato de cifras (`formatCurrencyAmount`/`formatPercentAmount`, nunca concatenación manual), iconografía y reglas para UI nueva viven ahí. Norma no negociable: **verifica claro y oscuro antes de mergear cualquier cambio visual**.

## Git workflow

**Una sola rama viva: `main`** — el trabajo sale de `main` en ramas cortas y vuelve por Pull Request con CI en verde; los releases son **tags** sobre `main`. `main` está protegida (PR obligatorio, sin force-push ni borrado): no se empuja directamente, y ese es el objetivo. Historia y porqué (la retirada de `dev` incluida): [`.claude/git-and-releases.md`](.claude/git-and-releases.md).

**Sin atribuciones.** Ningún commit ni PR lleva firma de herramienta: ni trailers de coautoría de un asistente, ni URLs de sesión, ni pies «Generated with». CI lo bloquea (job `attribution-scan`); el porqué y el incidente que obligó a reescribir el historial completo (2026-08-31): [`.claude/git-and-releases.md`](.claude/git-and-releases.md) §Sin atribuciones.

### Desarrollar

```bash
git checkout main && git pull --ff-only
git checkout -b fix/lo-que-sea       # o feat/…, docs/…, chore/…
# … trabajo, commits …
git push -u origin fix/lo-que-sea
gh pr create --fill                  # CI corre en el PR; sin verde no se mergea
```

Before resuming work: `git pull --ff-only`. After push: pull again.

### Releases y dependencias

> **Una versión, una imagen.** Un número de versión existe si y solo si hay una imagen publicada
> que lo lleva; **si un cambio no altera la imagen, no cambia la versión**.
> **El merge del bump ES la publicación** (auto-tag on merge): mergear el bump taguea, construye
> y publica solo — no hay paso manual. Y nunca taguees una versión histórica sin publicar:
> reconstruir una versión vieja **sobrescribe `:latest`**.

El ritual completo (pasos, vías manuales de fallback, orden estricto entre releases, bump del
add-on con la App `futurefin-release-bot`) y la **rutina cloud de Dependabot** — política de merge
y sus dos artefactos que no se tocan a mano (`ops/routine-lock`, issue `dependabot-mirror`) —
viven en [`.claude/git-and-releases.md`](.claude/git-and-releases.md).
