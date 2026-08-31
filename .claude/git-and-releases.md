# Git y releases — rama única, publicación automática y rutina de dependencias

> **Dueño de**: el modelo de rama única y su historia, el ritual de release completo (auto-tag on
> merge, `publish-image.yml`, bump del add-on) y la política de la rutina de dependencias con sus
> artefactos. **NO es dueño de**: las puertas de merge y la clasificación de cambios
> ([`futurefin-change-control`](skills/futurefin-change-control/SKILL.md)), la mitad de escritura
> del ritual — CHANGELOG y bump ([`futurefin-docs-and-writing`](skills/futurefin-docs-and-writing/SKILL.md) §5) —
> ni el runbook operativo de la rutina ([`routines/dependabot-autonomo.md`](routines/dependabot-autonomo.md)).

El flujo diario (rama corta → PR → CI verde → merge) vive resumido en `CLAUDE.md` §Git workflow;
aquí está el detalle y el porqué.

## Una sola rama viva: `main`

`main` es la rama por defecto, la que se publica y la única de larga vida. El trabajo se hace en
ramas cortas que salen de `main` y vuelven por Pull Request. Los releases son **tags** sobre
`main`, no una rama aparte.

Hasta la 4.0.1 hubo un `dev` de larga vida que se volcaba en `main` en cada release. Se retiró:
mantenerlo costaba ~244 líneas de maquinaria (`release-to-main.sh`, el job `main-guard`, y la
documentación que explicaba por qué las dos ramas no eran espejo) cuya única función era gestionar
una complejidad autoinfligida — y, sobre todo, impedía exigir que CI estuviera en verde antes de
mergear, porque el script empujaba a `main` directamente. **`main` no es un espejo de nada: es el
sitio.**

`main` está protegida: PR obligatorio, CI en verde, sin force-push ni borrado. No se empuja
directamente — la protección lo rechaza, y ese es el objetivo.

## Releases

> **Una versión, una imagen.** Un número de versión existe si y solo si hay una imagen publicada
> que lo lleva. **Si un cambio no altera la imagen, no cambia la versión**: documentación, CI,
> scripts de release y utillaje de test entran en `main` sin bump y viajan dentro de la siguiente
> versión que sí lo necesite. INCIDENTE (agosto 2026): se bumpó tres veces seguidas por cambios de
> docs y CI, dejando 4.0.1, 4.0.2 y 4.0.3 en el CHANGELOG **sin ninguna imagen detrás**; hubo que
> colapsarlas en una sola 4.0.1. `./scripts/audit-releases.sh` lista las secciones sin tag.

1. En una rama: bumpar `apps/api/Cargo.toml` (sincronizar `Cargo.lock` con `cargo update -p futurefin-api`), añadir la sección `## [X.Y.Z]` a `CHANGELOG.md` y su resumen equivalente (`## X.Y.Z`, sin corchetes ni fecha) a `addon/futurefin/CHANGELOG.md` — en prosa para quien usa el add-on de Home Assistant, no una copia técnica; puede ser una línea si la versión no cambia nada visible desde el add-on. **Las dos secciones deben existir antes de taguear**: `publish-image.yml` redacta las notas del Release desde la primera, la tienda de HA renderiza la segunda (se quedó clavada en 4.3.1 durante cuatro releases porque nada la exigía), y el job `rust` comprueba ambas con `./scripts/audit-releases.sh --version`.
2. PR → CI verde → merge a `main`.
3. **El merge del bump ES la publicación** (auto-tag on merge, desde 4.0.6): `publish-image.yml`
   corre en cada push a `main`; si `Cargo.toml` lleva una versión sin tag, ese mismo run espera a
   que la CI del commit esté verde, comprueba el orden estricto, **crea el tag y construye**. Un
   merge sin bump es un no-op verde de segundos. Consecuencia: mergear un bump publica — no hay
   paso manual, y un bump mergeado ya no puede quedarse sin imagen. Vías manuales que siguen
   existiendo (fallback y reconstrucciones):
   - **Desde local**: `git tag vX.Y.Z && git push origin vX.Y.Z` estando en `main`.
   - **Desde GitHub**: `publish-image.yml` por `workflow_dispatch` con la casilla **«Crear el tag
     sobre main»**. Es **idempotente**: si el auto-tag del merge ya creó el tag, el dispatch
     termina verde sin hacer nada (así la rutina de dependencias puede seguir lanzándolo sin
     chocar). El tag se crea dentro del mismo workflow y la ejecución sigue construyendo; **no
     vale un workflow aparte**, porque un tag empujado con `GITHUB_TOKEN` no dispara
     `on: push: tags` (los `workflow_dispatch` sí crean runs — es la excepción documentada).
4. `publish-image.yml` construye la imagen multi-arch (~2 h) a GHCR y Docker Hub, y al terminar
   **crea él solo el GitHub Release** con las notas del CHANGELOG. **El orden es estricto**:
   `vX.Y.Z` no construye hasta que el tag inmediatamente anterior tenga su Release (= su imagen);
   si la publicación anterior falló, las siguientes abortan en vez de publicar por encima del
   agujero. Encadenar releases sin esperar los builds es seguro por eso. Matiz del auto-tag: en
   modo merge el tag se crea **después** de ver la CI verde (no antes, como el dispatch) — un bump
   mergeado con CI rota no deja tag huérfano, deja un run rojo.
5. **Último paso del mismo run: el add-on de Home Assistant apunta a la versión recién publicada.**
   Con la imagen ya verificada en el registry y el Release creado, `publish-image.yml` sube el
   `version:` de `addon/futurefin/config.yaml` en `main` por la **contents API** (los checkouts van
   con `persist-credentials: false`: no hay credencial para un `git push`). El Supervisor usa ese
   número como tag de imagen, así que sin este paso la tienda se queda clavada. **Requisito
   (2026-08-30)**: el commit va autenticado como la GitHub App propia **`futurefin-release-bot`**
   (secrets `ADDON_BUMP_APP_ID` + `ADDON_BUMP_APP_PRIVATE_KEY`; token emitido en el paso previo con
   `actions/create-github-app-token`), que es *bypass actor* del ruleset «Proteger main». **No puede
   ser el `GITHUB_TOKEN`**: la app integrada de GitHub Actions no es admisible como bypass actor en
   un repo personal (422 «must be part of the ruleset source or owner organization»), y sin bypass
   el push muere con 409 «Changes must be made through a pull request» — el fallo real del run de
   4.4.0, que obligó al PR manual #103. Si el paso falla, la imagen y el Release ya están fuera y el
   add-on se queda **una versión por detrás**: se arregla con un PR normal que suba el `version:`.
   El commit lleva `[skip ci]` y no reentra (un push de una App SÍ dispara workflows, a diferencia
   del `GITHUB_TOKEN`; el `[skip ci]` es lo que lo corta). Comprueba la sincronía con
   `./scripts/audit-releases.sh --addon`. Runbook completo del bot (rotación de la clave, fallos):
   skill [`futurefin-run-and-operate`](skills/futurefin-run-and-operate/SKILL.md) §canal add-on.

Tags publicados: `:X.Y.Z`, `:X.Y`, `:X`, `:latest`. Requiere los secrets `DOCKERHUB_USERNAME` +
`DOCKERHUB_TOKEN`.

> **El tag es la publicación.** Nunca taguees una versión histórica sin publicar: el workflow
> incluye `type=raw,value=latest`, así que reconstruir una versión vieja **sobrescribe `:latest`**
> con código antiguo.

## Dependencias — automatizado (rutina cloud)

Los PRs de Dependabot los procesa una **rutina cloud** («Dependabot autónomo», trigger de
claude.ai): se dispara **por webhook** cuando Dependabot abre un PR, con un **barrido los
martes ~06:30** que caza huérfanos si un evento se perdió. **Su prompt operativo vive en este
repo** — [`routines/dependabot-autonomo.md`](routines/dependabot-autonomo.md) —
y el trigger solo contiene un puntero que le manda leerlo del clon: editar la rutina es un PR
normal (revisable y con historia); el trigger no se toca salvo para el cron, el entorno o el
propio puntero. Si mueves o renombras ese fichero, actualiza el puntero del trigger a la vez.
Política:

- **Parche/minor dentro de rango**: se mergea con los 5 checks en verde.
- **Major o 0.x-minor**: pasa una barra de evidencia — notas del salto leídas del cuerpo del
  PR, cada rotura anunciada buscada con `grep` en el repo (salida pegada como evidencia en un
  comentario del PR), checks sobre el SHA actual. Sin notas legibles no se mergea.
- Cada fix que **llega a la imagen** produce su propio release patch (norma «una versión, una
  imagen»); lo que no llega (vitest, eslint, `@types/*`, acciones) se mergea sin bump.
- **Desde el auto-tag on merge (4.0.6) el dispatch de la rutina es redundante pero inofensivo**:
  mergear el bump ya taguea y publica solo. Si la rutina sigue lanzando `publish-image.yml` con
  «crear el tag», el que llegue segundo (dispatch o auto-tag) termina verde sin hacer nada — se
  puede retirar ese paso de la rutina cuando se revise, sin prisa.
- Los issues-informe que la rutina abre **se cierran solos** cuando todo lo que reportaban
  queda resuelto.
- **Método de merge**: los PRs de Dependabot se mergean con **merge commit** (título = el del
  PR), no con squash — un squash deja a `dependabot[bot]` como autor del commit visible en la
  portada de `main`, y el owner no lo quiere. Los PRs propios de la rutina (releases, misión
  toolchain) van con squash: su autor ya es el owner.
- El **webhook es el disparo principal pero no está garantizado** (en el estreno no disparó
  con el evento de merge); el barrido del martes es la red que siempre corre.

Dos artefactos suyos que NO hay que «limpiar» a mano:

- **`ops/routine-lock`**: rama efímera que la rutina usa como candado anti-carrera (varios
  webhooks en ráfaga → solo una sesión procesa). La credencial de la rutina no puede borrar
  refs (403), así que «liberar» es dejar un commit `lock: LIBERADO` en la punta; el workflow
  `routine-lock-janitor.yml` borra la rama al verlo (y por `workflow_dispatch` borra también
  un candado caducado). Verla viva durante una pasada es normal; con punta `LIBERADO`
  desaparece sola en segundos. Si lleva >2 h sin liberar es un candado caducado y la propia
  rutina lo roba. Borrarla a mano en mitad de una pasada deja dos sesiones escribiendo a la vez.
- **El issue con label `dependabot-mirror`**: espejo de las alertas abiertas, regenerado por
  `dependabot-alerts-mirror.yml` (la rutina no puede leer la API de alertas desde su sandbox
  y lee este issue en su lugar). Su ESTADO es parte del dato y lo gestiona el workflow — no
  tocarlo a mano: abierto ⟺ hay alertas; con 0 alertas queda **cerrado** con
  `SIN_ALERTAS: true` (cerrado+fresco = cero; ausente o `GENERADO` >36h = espejo roto).
  Necesita el secret `DEPENDABOT_ALERTS_TOKEN` (el `GITHUB_TOKEN` de Actions no
  puede leer alertas; TODO: sustituir el actual por un PAT fine-grained de solo lectura).
