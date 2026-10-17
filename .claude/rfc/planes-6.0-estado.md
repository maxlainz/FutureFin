# Estado del trabajo 6.0.0 «Planes» — cómo retomarlo

**Léelo primero si retomas el trabajo en otra sesión.** Fotografía viva del avance: qué está mergeado,
qué está a medias, dónde vive cada cosa y cuál es el siguiente paso. Se actualiza con **push directo a
`next/6.0`** (es contabilidad del trabajo, no código) cada vez que cambia algo relevante.

- Diseño (decisiones D-1…): [`planes-6.0.md`](planes-6.0.md)
- Plan de la fase F2, PR a PR: [`planes-6.0-f2.md`](planes-6.0-f2.md)
- Prototipo visual (D-25/D-26): [`planes-6.0-prototipo/`](planes-6.0-prototipo/), artboards `.dc.html` del
  canvas del owner (https://claude.ai/artifact/Ju6HKFPfszH3GXfANN1uE5, privado). `Main.dc.html` es la
  pestaña Planes con el selector por pestañas y la lista cronológica.

## Ramas

- **`next/6.0`**: rama de integración (excepción documentada en `.claude/git-and-releases.md`). Todo
  entra por PR desde `feat/6.0-…` con CI verde, merge commit (no squash). `main` se mergea en ella con
  regularidad. Entra en `main` con un único PR que lleva el bump a 6.0.0.
- **`wip/6.0-…`**: instantáneas de trabajo a medias (sin CI, sin garantías) para no perderlo si la sesión
  se corta. Se borran cuando su PR se mergea.

## Avance (actualizado 2026-10-17 ~14:40 UTC) — total ≈ 29 %

| Fase | Peso | Estado |
|---|---|---|
| F1 motor | 25 % | **Hecha.** F1a #263 (activos/pasivos con mes de inicio), F1b #264 (cambios de ingreso/gasto + indexación), F1c #265 (ventas explícitas, déficit por el sumidero, fallo en todo el horizonte; 4 bugs de la revisión adversarial arreglados) |
| F2 API/MCP | 35 % | F2-1 (tablas + CRUD de planes) y F2-2 (retirada de la jubilación) en curso; F2-3…F2-7 pendientes |
| F3 finanzas compartidas | 10 % | F3a (interruptor + permisos) en curso; F3b (simulación única del hogar, planes de hogar, cuotas de propiedad D-28) pendiente |
| F4 UI | 20 % | Prototipo hecho; implementación pendiente (arranca cuando F2-1 fije el contrato) |
| F5 docs + release | 10 % | RFC y plan escritos; reescritura de docs/skills, CHANGELOG y bump pendientes |

## Trabajo a medias

| Pieza | Instantánea | Qué falta |
|---|---|---|
| F2-1 tablas + CRUD | `wip/6.0-f2-1-plans-crud` | Terminar la suite completa de tests de la API, abrir PR `feat/6.0-f2-1-plans-crud` → `next/6.0` |
| F2-2 retirar jubilación | `wip/6.0-f2-2-remove-retirement` | Ver el plan F2 §Retirada; pines de `projection_pins.rs` sin cambios; web en verde |
| F3a finanzas compartidas | `wip/6.0-f3a-shared-finances` | Interruptor `shared_finances` en installation, `require_row_owner` condicional, tests en ambos modos |

Para retomar una instantánea: `git checkout -b feat/6.0-<pieza> origin/wip/6.0-<pieza>` (el commit es una
«stash commit»: `git checkout origin/wip/… -- .` sobre `origin/next/6.0` si prefieres partir limpio),
revisar el diff contra `origin/next/6.0`, completar, pasar las puertas y abrir el PR.

## Decisiones tomadas después del RFC (también en el plan F2)

- **D-31**: las partidas del presupuesto con fecha de fin futura se borran sin importarlas (D-18).
- **D-32**: los «Próximos» importados entran como importe fijo nominal (`keeps_purchasing_power = false`).
- **D-33**: ninguna página espera a Monte Carlo (en 5.0 el Resumen tarda 4-10 s por esto); sin parche
  5.0.x por decisión del owner.
- Decisiones técnicas de F1 (cerradas por la sesión, documentadas en `engine.md`): un importe fijo es
  nominal desde su inicio; un mes con ingreso o gasto negativo se lleva a 0 con aviso; un sumidero
  ilíquido nunca se vende; en `SinkOnly` no hay comprobación de SWR; `ManualAmount` solo en ventas
  puntuales o totales (la API lo validará en F2); las ventas puntuales tributan como eventos separados
  (simplificación declarada).

## Entorno de la sesión (por si sirve)

- El disco del contenedor es limitado: compilar con `CARGO_INCREMENTAL=0` y sin debuginfo, y borrar
  `target/` cuando sobre.
- Postgres de test sin Docker: `initdb` de PostgreSQL 16 en `/var/tmp/ffpg` como usuario `postgres`,
  puerto 5433, `TEST_DATABASE_URL=postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test`.
- Los commits van con la identidad del owner (CI rechaza las identidades de asistente) y sin
  atribución.

## Siguiente paso

1. Mergear F2-1, F2-2 y F3a en `next/6.0` (en ese orden si chocan).
2. Lanzar F2-3 (semántica 6.0 sin plan) y F4 (UI de Planes contra la API de F2-1) en paralelo.
3. F2-4 (plan activo + D-33), F2-5 (importación + D-18 + backup v14), F2-6 (MCP), F3b, F2-7, F5.
