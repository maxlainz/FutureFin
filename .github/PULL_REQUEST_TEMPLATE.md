<!--
Las casillas de abajo son las puertas de CONTRIBUTING.md §3. Marca lo que aplique y borra
las secciones que no. Pega la salida real de los comandos: «lo he probado» no es evidencia.
-->

## Qué cambia y por qué

<!-- El problema primero, la solución después. Si es un fix: síntoma → causa raíz → arreglo. -->

Closes #

## Evidencia

<!--
Qué demuestra que esto hace lo que dice:
- fix → el test que falla con el código viejo y pasa con el nuevo (nómbralo).
- refactor sin cambio de salida → la captura de regresión, con el valor de antes.
- cambio de modelo o de cifras → el número esperado, calculado ANTES de ejecutar, y el obtenido.
- cambio visual → dilo explícitamente: verificado en claro y en oscuro.
-->

## Puertas ejecutadas en local

CI también las corre desde 4.0.0, pero se ejecutan en local primero: el bucle es más corto y CI
no es un depurador. Lo que CI NO cubre: la verificación visual claro/oscuro y los drills de
Docker-stack previos a un release.

- [ ] `cargo build -p futurefin-api --locked`
- [ ] `cargo test -p futurefin-engine`
- [ ] `TEST_DATABASE_URL=… cargo test --workspace` (integración contra PostgreSQL)
- [ ] `npm run typecheck:web`
- [ ] `npm run lint:web`
- [ ] `npm run build:web`
- [ ] `npm test --workspace futurefin-web`

## Puertas según lo que toca

- [ ] **Matemática del motor**: tests de proyección en verde; si cambió el gross-up fiscal o la
      fórmula del objetivo FIRE, `apps/api/tests/fixtures/fire-parity.json` regenerado y las dos
      suites (Rust y Vitest) verdes; CHANGELOG con ejemplo numérico de antes y después.
- [ ] **Contrato de la API**: anotaciones `#[utoipa::path]` al día, `.claude/api-routes.md`
      actualizado, y la evaluación de paridad MCP resuelta (tool añadida, tool actualizada,
      omisión deliberada registrada, o no aplica).
- [ ] **Migración nueva**: fichero nuevo, ninguna migración ya publicada editada; si hay pérdida
      de datos, con el visto bueno explícito de quien mantiene el repositorio; nota de
      «Migración / compatibilidad» en el CHANGELOG.
- [ ] **Cambio visual**: verificado en tema claro **y** oscuro; solo tokens `var(--ff-*)`, sin hex
      a pelo; iconos solo en `apps/web/src/components/icons.tsx`.
- [ ] **Cifra o KPI visible** (base, ventana, denominador o nombre): el catálogo de descripciones
      `apps/web/src/lib/helpTexts.ts` va en este mismo cambio, y el CHANGELOG dice la base de
      antes y la de después.
- [ ] **Contenedor** (`Dockerfile`, `docker-entrypoint.sh`, `docker-compose*.yml`):
      `shellcheck -S warning apps/api/docker-entrypoint.sh scripts/*.sh` limpio y el job
      `docker-stack` en verde.

## Documentación

- [ ] El documento de record del área tocada está actualizado (`.claude/api-routes.md`,
      `data-model.md`, `engine.md`, `env-and-config.md`, `auth-and-membership.md`,
      `design-system.md`, `frontend-structure.md`, `tests.md`, `CLAUDE.md`, `docs/`).
- [ ] Entrada en `CHANGELOG.md` bajo `## [Unreleased]`, en estilo forense: dice **por qué**, no
      solo qué. Los cambios *breaking* van marcados de forma explícita.

## Higiene de datos

- [ ] Ni el diff, ni los tests, ni el CHANGELOG, ni las capturas llevan datos reales de nadie:
      IBAN, nombres, importes de una instalación real, comercios, direcciones.
- [ ] `./scripts/scan-sensitive.sh` sale limpio.
