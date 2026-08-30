# CLAUDE.md

Guía para cualquier modelo (o humano) que trabaje en este repositorio. Es el **punto de entrada
único**: todo lo demás — referencia, runbooks, historia — se alcanza desde aquí.

FutureFin es una app autoalojada de finanzas del hogar + planificación FIRE: API Rust/Axum
(`apps/api`), motor de proyección pure-Decimal (`crates/engine`), SPA React 19 (`apps/web`),
PostgreSQL embebido en la imagen Docker (un contenedor en producción; en dev, un Postgres aparte).
UI en español; código e identificadores en inglés.

Tres capas, en orden de consulta: **este fichero** (reglas y enrutado) → **skills**
(`.claude/skills/`, runbooks por tarea con el juicio y la historia) → **docs de referencia**
(`.claude/*.md`, fichas por superficie con los contratos exactos).

## Enrutado por tarea (carga la skill ANTES de empezar)

| Tu tarea se parece a… | Carga |
|---|---|
| Any change you plan to merge (gates, migration/release rules, pre-merge checklist) | [`futurefin-change-control`](.claude/skills/futurefin-change-control/SKILL.md) |
| A symptom: wrong numbers, HTTP errors, unhealthy container, layout breakage | [`futurefin-debugging-playbook`](.claude/skills/futurefin-debugging-playbook/SKILL.md) |
| "Why is X designed this way?" / touching cache, auth, scoping, serialization | [`futurefin-architecture-contract`](.claude/skills/futurefin-architecture-contract/SKILL.md) |
| Understanding the FIRE/projection math (SWR, gross-up, cascade, inflación) | [`futurefin-fire-domain-reference`](.claude/skills/futurefin-fire-domain-reference/SKILL.md) |
| About to (re)introduce an old idea — check what was already tried and rejected | [`futurefin-failure-archaeology`](.claude/skills/futurefin-failure-archaeology/SKILL.md) |
| Añadir/cambiar rutas o handlers (¿tool MCP?), añadir/actualizar una tool, deriva de `/mcp` | [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md) |
| Tocar una métrica o un KPI: su base, su ventana, su nombre, o añadir/retirar uno | [`futurefin-metric-definitions`](.claude/skills/futurefin-metric-definitions/SKILL.md) |
| Añadir/cambiar un fixture, ilustrar un cambio con números, capturas, datos de demo | [`futurefin-data-hygiene`](.claude/skills/futurefin-data-hygiene/SKILL.md) |
| Env vars, compose files, query params, fire_settings axes; adding a config axis | [`futurefin-config-and-flags`](.claude/skills/futurefin-config-and-flags/SKILL.md) |
| Add-on de Home Assistant, ingress/subpath, `/data`, SSO por cabeceras de proxy | [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md) + [`futurefin-config-and-flags`](.claude/skills/futurefin-config-and-flags/SKILL.md); el porqué de las concesiones, en `futurefin-architecture-contract` D17/D18 |
| Setting up / building / dev-environment failures | [`futurefin-build-and-env`](.claude/skills/futurefin-build-and-env/SKILL.md) |
| Deploy, upgrade, rollback, backups, logs, production ops | [`futurefin-run-and-operate`](.claude/skills/futurefin-run-and-operate/SKILL.md) |
| Measuring: timings, cache hits, payload sizes, DB state (ships scripts) | [`futurefin-diagnostics-and-tooling`](.claude/skills/futurefin-diagnostics-and-tooling/SKILL.md) |
| Running or writing tests; what evidence a change needs; fire-parity fixture | [`futurefin-validation-and-qa`](.claude/skills/futurefin-validation-and-qa/SKILL.md) |
| Updating CHANGELOG/README/docs; doc drift; house style; templates | [`futurefin-docs-and-writing`](.claude/skills/futurefin-docs-and-writing/SKILL.md) |
| Improving projection realism/correctness (Monte Carlo, taxes, invariants…) | [`futurefin-projection-realism-campaign`](.claude/skills/futurefin-projection-realism-campaign/SKILL.md) |
| Numeric analysis: closed forms, index proofs, f64 safety, determinism audits | [`futurefin-proof-and-analysis-toolkit`](.claude/skills/futurefin-proof-and-analysis-toolkit/SKILL.md) |
| "What should we build next?" / public capability claims | [`futurefin-research-frontier`](.claude/skills/futurefin-research-frontier/SKILL.md) |
| Turning a hypothesis into an accepted change (evidence bar, predict-then-run) | [`futurefin-research-methodology`](.claude/skills/futurefin-research-methodology/SKILL.md) |

## Documentos de referencia (`.claude/`) — léelos antes de tocar su área

| Fichero | Dueño de |
|---|---|
| [`api-routes.md`](.claude/api-routes.md) | Rutas HTTP `/v1` + OAuth, auth por endpoint, view-scoping, error mapping, forma/cache de la proyección |
| [`mcp-catalog.md`](.claude/mcp-catalog.md) | Catálogo MCP por tool (cache class, preview/confirm, sobres), transporte de `/mcp`, contadores |
| [`backend-structure.md`](.claude/backend-structure.md) | Mapa de módulos de `apps/api/src` + receta «cómo añadir un handler» |
| [`data-model.md`](.claude/data-model.md) | Esquema de BD, invariantes por tabla, forma del JSONB `fire_settings`, notas `.ffbackup` |
| [`engine.md`](.claude/engine.md) | API pública del motor, loop de simulación, frontera handler↔engine |
| [`auth-and-membership.md`](.claude/auth-and-membership.md) | Flujo de auth, roles, cookie, usuarios pendientes |
| [`env-and-config.md`](.claude/env-and-config.md) | Toda env var + default (binario Y entrypoint), orden de carga de `.env`, matriz de compose, config de Vite |
| [`frontend-structure.md`](.claude/frontend-structure.md) | Layout de `apps/web/src`, dónde va el código nuevo |
| [`design-system.md`](.claude/design-system.md) | Tokens, paleta, tema, iconos, reglas para UI nueva (LEE ANTES de tocar estilos) |
| [`tests.md`](.claude/tests.md) | Cómo correr/escribir tests backend (Postgres) y Vitest, fixtures compartidos |

**Mantenlos al día**: un cambio de área sin su doc de record actualizado está incompleto. Cada
skill termina con comandos de re-verificación — si tu cambio los desactualiza, la skill va en el
mismo PR. El mapa normativo de dueños vive en `futurefin-docs-and-writing` §1.

## Contrato en vigor

Reglas siempre ciertas. Sin porqués (columna «Detalle»), sin versiones. Si una tarea parece exigir
violar una fila, para y re-planifica.

| Regla | Congelada por | Detalle en |
|---|---|---|
| El dinero es `Decimal` de punta a punta; el wire son strings decimales — jamás `f64` en `crates/` | `no_f64_in_domain_code` (crates/domain y crates/engine) | AC D4 (la única excepción: series de chart en `apps/api`) |
| Un GET jamás muta; los pasivos vencidos se filtran, no se borran | `history_get_never_mutates` + `expired_liability_is_hidden_from_listing_but_persists_in_db` | AC D5 |
| Mutaciones de transacciones invalidan la cache ⟺ `SavingsSource::uses_transactions()` (conjunto O clasificación) | `transactions_projection_cache.rs` | AC D12a |
| Los snapshots no son input del motor: sus mutaciones nunca invalidan | `snapshot_mutations_do_not_touch_projection_cache` | AC D12 |
| Toda escritura MCP pasa por `require_mcp_write` (rol → scope → toggle) y audita | `every_write_tool_in_the_source_calls_require_mcp_write` | AC D20 |
| Ninguna tool MCP duplica SQL/validación: llama a la core fn del handler | `tools_list_returns_exactly_the_v1_catalog` + `grep -c 'sqlx::query' apps/api/src/mcp/server.rs` = 0 | AC D14 · mcp-parity |
| `?view=mine` es filtro de presentación, no frontera de autorización; los dos brazos van por `scope_where`/`bind_scope_*` y el eco usa `as_str()` | `as_str_round_trips_through_resolve` | AC D2 |
| Enum de request desconocido = 422, nunca default silencioso | `installation_patch.rs` | AC (strict deserialization) |
| Jamás registres un método backend en `/oauth/authorize` (lo sirve el fallback SPA) | `get_oauth_authorize_is_not_handled_by_the_api` | AC D15 |
| `/mcp` fija su tope de body aparte: `DefaultBodyLimit` no alcanza a un `route_service` | `oversized_mcp_body_returns_413` | api-routes §CORS y topes |
| Un checksum de migración desalineado falla ruidoso; nada de auto-repair; jamás editar una migración publicada | `migration_guard.rs` (downgrade + no-op doble) | change-control §3 |
| Cero hex fuera de `theme.css`: solo `var(--ff-*)`/`var(--proj-*)` | `no-hex-outside-theme.test.ts` | design-system |
| Ningún dato real de ninguna persona entra al repo — fixtures fabricados, cifras inventadas coherentes | `fixtures_carry_no_iban_shaped_string` + job CI `secrets-scan` | data-hygiene |
| Una versión ⟺ una imagen publicada; un cambio que no altera la imagen no bumpea | `./scripts/audit-releases.sh` (job `rust`) | change-control §4 |
| El código es la verdad; una incoherencia contrato↔código que no puedas arreglar en el cambio se abre como issue con evidencia `path:line` | — (norma de sesión) | docs-and-writing §7 |

## Normas de sesión

- **Issues primero**: `gh issue list --state open` antes de empezar y antes de mergear. Si tu tarea
  solapa con uno abierto, pregunta al owner si lo abordamos, no lo decidas tú. El commit que cierra
  lleva `Closes #N`; la entrada del CHANGELOG lo referencia.
- **Incoherencias**: verifica tú contra el código antes de abrir nada; si cabe arreglarlo en el
  mismo cambio, arréglalo. Explica el coste, no solo el hecho. Con números, desconfía por defecto:
  prefiere el comando al contador, cuidado con defaults duplicados schema↔runtime, con greps de
  re-verificación vacíos y con comandos que se cuentan a sí mismos (las cuatro lecciones, con sus
  incidentes, en docs-and-writing §3).
- **Delegación**: por defecto, delega a subagentes — como mucho **Opus, nunca Fable** (Opus para
  dinero/invariantes/revisión adversarial; Sonnet para exploración y barridos con spec; Haiku para
  inventario mecánico; esfuerzo acorde a la etapa). Los subagentes no heredan tu contexto: dales
  tarea, rutas y qué skill cargar, y pide conclusión CON evidencia. La sesión principal re-verifica
  todo lo que toque dinero antes de commitear, y se queda con: git/merges/releases, migraciones,
  lo destructivo, y las ediciones pequeñas de ficheros ya abiertos.

## Comandos

```bash
# Dev split (API :8081 + Vite :8080; DB dev en 127.0.0.1:5432):
docker compose -f docker-compose.dev.yml up -d
cd apps/api && cargo run          # terminal 1 (automigra al arrancar)
npm install && npm run dev:web    # terminal 2 (proxy /v1 → API)

# Gates locales (CI también los corre; local primero — el bucle es más corto):
cargo build -p futurefin-api --locked && cargo test -p futurefin-engine
TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" cargo test --workspace
npm run typecheck:web && npm run lint:web && npm run build:web && npm test --workspace futurefin-web
# (DB de test una sola vez: docker run -d --name ff-test-db --shm-size=1g -e POSTGRES_USER=futurefin \
#   -e POSTGRES_PASSWORD=futurefin_test -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine)

# Imagen local + stack completo (drills de release): futurefin-build-and-env §4 / change-control §4.2
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .
docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d
curl -sf http://127.0.0.1:8080/v1/ready
```

El detalle (trampas de `.env`, modos, troubleshooting) es de `docs/desarrollo.md` (humanos) y
`futurefin-build-and-env` (fallos). No dupliques estos bloques: cítalos.

## Convenciones de UI

- Importes sin decimales, símbolo detrás (`1.234 €`): `formatCurrencyAmount`/`formatCurrencyNumber`
  — nunca `toString()` ni concatenación (las dos excepciones de chart, en design-system §formateo).
- Porcentajes con un decimal y ` %` (`3,5 %`): `formatPercentAmount`/`formatPercentDisplay`.
- Info adicional de `MetricCard` en `parenthetical`, no `suffix` (el slot se reserva siempre).
- Copy mínimo, en español (`Sin datos.`).
- Paleta: base zinc + un acento; verde/rojo solo en cifras delta; charts = única zona polícroma.
- Tema claro/oscuro/auto por `<html data-theme>`. **Verifica ambos temas antes de mergear nada visual.**
- Iconos solo en `components/icons.tsx`; charts pequeños con `MiniProjection`.

## Git y releases

- **Una sola rama viva: `main`** (protegida: PR + CI verde, sin force-push). Ramas cortas
  `fix/…`/`feat/…`/`docs/…` que vuelven por PR. `git pull --ff-only` antes de retomar trabajo.
- **Un release es un tag sobre `main`, y el merge del bump ES la publicación** (auto-tag on
  merge). Bump = `apps/api/Cargo.toml` + `Cargo.lock` sincronizado + sección `## [X.Y.Z]` en el
  CHANGELOG **antes** de mergear. Orden estricto entre versiones; jamás taguear una versión
  histórica (reconstruir sobrescribe `:latest`).
- El ritual completo, los drills locales obligatorios y las vías manuales de fallback:
  **`futurefin-change-control` §4** (dueño). El lado usuario (elegir tag, rollback):
  `docs/actualizar.md`.

## Dependencias

Los PRs de Dependabot los procesa una rutina cloud autónoma (webhook + barrido de los martes). Su
prompt operativo, su política de merge y sus dos artefactos que NO se tocan a mano
(`ops/routine-lock`, el issue `dependabot-mirror`) viven en
[`.claude/routines/dependabot-autonomo.md`](.claude/routines/dependabot-autonomo.md) y en
`futurefin-change-control` §4.1.
