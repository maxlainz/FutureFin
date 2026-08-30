# Contribuir a FutureFin

FutureFin es una aplicación autoalojada de finanzas del hogar y planificación FIRE: API en
Rust/Axum (`apps/api`), motor de proyección puro (`crates/engine`), SPA en React 19 (`apps/web`) y
PostgreSQL, que en producción va **dentro** de la propia imagen Docker.

Lo que se rompe aquí no suele hacer ruido: un error en la proyección devuelve un número
verosímil pero equivocado. Por eso las puertas de la sección 3 no son burocracia — son la única
forma de distinguir «funciona» de «da el número correcto».

## 0. Antes de escribir código

**Mira el tablero de issues primero.**

```bash
gh issue list --state open
```

- Si tu cambio responde a un issue abierto, dilo en él antes de empezar: evita dos personas
  arreglando lo mismo.
- Si no existe, abre uno. Para algo grande —un modo nuevo de proyección, una fuente de datos
  nueva, un cambio de la API— abre el issue **antes** que el pull request: es más barato descartar
  una idea en un párrafo que en un diff.
- El commit que cierra un issue lleva `Closes #N` en el cuerpo, y la entrada del CHANGELOG lo
  referencia como `(issue #N)`. Esto es una norma nueva y tiene motivo: los issues #5 y #6 se
  arreglaron por completo y se quedaron abiertos meses porque ningún commit los mencionó.
- Si tu cambio solo toca **parte** de un issue, escríbelo en el issue en vez de dejarlo
  silenciosamente desactualizado.

**Idioma.** La interfaz y toda la documentación pública del repositorio están en español. El
código, los identificadores, los campos de la API, el SQL y los nombres de fichero están en
inglés. No mezcles: un handler se llama `require_installation_member`, y el texto que ve el
usuario dice «Sin datos.».

## 1. Levantar el entorno

Lo normal es `split-dev`: la API de Rust en el puerto **8081** y el servidor de desarrollo de Vite
en el **8080**, que hace de proxy hacia ella. La imagen con PostgreSQL embebido no se usa en
desarrollo.

| Herramienta | Versión | Notas |
|---|---|---|
| Rust | stable | Lo fija `rust-toolchain.toml`; rustup lo coge solo. |
| Node.js | 24 recomendado (20+ funciona) | CI usa 24 y la imagen se construye con 24. |
| npm | 10+ | Hace falta el soporte de *workspaces*. |
| Docker + Compose v2 | reciente | Para el PostgreSQL de desarrollo y para construir la imagen. |

PostgreSQL **no se instala en la máquina**: en desarrollo corre como su propio contenedor.

```bash
cp .env.example .env
```

Desde la 3.0.0 todas las líneas del ejemplo vienen comentadas (producción no necesita ninguna
variable). Para `split-dev`, descomenta las tres del bloque de desarrollo:

```env
PORT=8081
DATABASE_URL=postgres://futurefin:futurefin@127.0.0.1:5432/futurefin
RUST_LOG=futurefin_api=info,tower_http=info
```

> No dejes esa `DATABASE_URL` descomentada si en la misma máquina levantas el compose de
> producción: la imagen interpreta cualquier `DATABASE_URL` como «quiero una base de datos
> externa» y entra en el modo deprecado. Ten ficheros `.env` separados.

Levanta la base de datos de desarrollo —es un compose **autónomo**, no un override— y las dos
mitades de la aplicación:

```bash
docker compose -f docker-compose.dev.yml up -d

# Terminal 1 — API en :8081 (aplica las migraciones al arrancar)
cd apps/api && cargo run

# Terminal 2 — interfaz en :8080, desde la raíz del repositorio
npm install
npm run dev:web
```

Abre `http://127.0.0.1:8080` y regístrate: **el primer usuario se convierte en propietario** de la
instalación, igual que en producción.

Las migraciones de `apps/api/migrations/` se empotran en el binario al compilar y se aplican en
cada arranque, así que **cambiar de rama y arrancar la API muta tu base de datos de desarrollo**.
Cómo salir de ahí, el modo solo-API, construir la imagen en local y la tabla de problemas
frecuentes: [docs/desarrollo.md](docs/desarrollo.md).

## 2. Cómo se organizan las ramas

Una sola rama viva: **`main`**. Es la rama por defecto, la que se publica y la única de larga vida.

Saca una rama corta de `main`, trabaja ahí y dirige el pull request **a `main`**:

```bash
git checkout main && git pull --ff-only
git checkout -b fix/lo-que-sea
# … trabajo, commits …
git push -u origin fix/lo-que-sea && gh pr create --fill
```

`main` está protegida: el pull request es obligatorio y CI tiene que estar en verde para poder
mergear. No hay forma de empujar directamente, y es a propósito.

Los releases son **tags** sobre `main` (`vX.Y.Z`), no una rama aparte; los publica quien mantiene
el repositorio.

Antes de retomar el trabajo: `git pull --ff-only`.

> **Nota de administración del repositorio** (solo afecta a quien lo mantiene, no a quien
> contribuye): tras publicar una imagen, `publish-image.yml` escribe en `main` el `version:` de
> `addon/futurefin/config.yaml`, que es lo que hace que la tienda de add-ons de Home Assistant vea
> la versión nueva. Para que ese commit pase la protección de rama, la app **«GitHub Actions» tiene
> que estar como *bypass actor* del ruleset «Proteger main»**. Es un ajuste manual de GitHub que
> **no vive en git**: si alguien lo quita, el paso falla con un 403 y el add-on se queda una
> versión por detrás (la imagen y el Release ya están fuera; se arregla con un PR normal que suba
> ese `version:`). `./scripts/audit-releases.sh --addon` comprueba que el add-on y
> `apps/api/Cargo.toml` declaran la misma versión.

## 3. Las puertas — ejecútalas antes de proponer el cambio

**Córrelas en local aunque CI las repita.** Desde 4.0.0 CI ejecuta las mismas suites, pero
enterarte en tres minutos en tu máquina es mejor que en quince en un runner — y hay cosas que
ningún job puede ver (la interfaz de verdad, en tema claro **y** oscuro).

**La puerta entera con un solo comando** (los mismos gates, en orden, falla al primero):

```bash
./scripts/test-all.sh              # necesita la base de test del paso 2 en el 5433
SKIP_DB=1 ./scripts/test-all.sh    # solo los gates sin base de datos
```

Paso a paso, para iterar sobre una puerta concreta:

```bash
# 1. Build de Rust + tests del engine (no necesitan base de datos)
cargo build -p futurefin-api --locked
cargo test -p futurefin-engine

# 2. Tests de integración con PostgreSQL (CI también los corre, en su propio servicio).
#    Base de datos de test dedicada, en el 5433 para no chocar con la de desarrollo:
docker run -d --name ff-test-db \
  -e POSTGRES_USER=futurefin -e POSTGRES_PASSWORD=futurefin_test \
  -e POSTGRES_DB=futurefin_test -p 5433:5432 postgres:16.4-alpine

TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
  cargo test --workspace

# 3. Frontend (CI corre estos cuatro)
npm run typecheck:web
npm run lint:web
npm run build:web
npm test --workspace futurefin-web
```

Cada test de integración crea su propio esquema `ff_test_<uuid>`, le aplica todas las migraciones
y corre contra el router de verdad.

Lo que corre CI (`.github/workflows/ci.yml`), cinco jobs: `secrets-scan` (escáner de datos
sensibles), `rust` (build de la API + tests del engine), `web` (`npm ci`, typecheck, **lint**,
**Vitest**, build), `integration` (**`cargo test --workspace` contra un servicio PostgreSQL
16.4-alpine**) y `docker-stack` (shellcheck, build de la imagen y los caminos críticos del
contenedor, incluida la preservación de datos al actualizar). Aparte va `codeql.yml`, que analiza
el código propio (`rust`, `javascript-typescript`, `actions`) y publica en la pestaña Security.

Hasta la 4.0.0 los tests de integración, `lint:web` y Vitest **no** corrían en CI: eran una
obligación local, es decir, dependían de que a nadie se le olvidara, y una PR podía salir verde con
toda la capa de handlers rota. Con el repositorio público eso no se sostiene, así que los tres
entraron como puertas bloqueantes. Lo que sigue **sin** cubrir ningún job: clippy y rustfmt
(instalados pero desactivados a conciencia — el repo nunca ha pasado por ellos y activarlos hoy
dejaría CI en rojo desde el primer push), y cualquier verificación visual.

### Puertas adicionales según lo que toques

| Si tocas… | Además de lo anterior |
|---|---|
| Matemática del motor (`crates/engine`, objetivo FIRE, cascada, inflación) | Los tests de integración de proyección; si cambia el gross-up fiscal o la fórmula del objetivo, regenera `apps/api/tests/fixtures/fire-parity.json` y deja **las dos** suites en verde; entrada de CHANGELOG con un ejemplo numérico de antes y después |
| Contrato de la API (rutas, campos, códigos de estado, tools MCP) | Anotaciones `#[utoipa::path]` al día (el OpenAPI se genera); tests de integración de la forma nueva; nota explícita si el cambio es *breaking* |
| Una migración nueva | Nunca edites una migración ya publicada: se detecta por checksum y falla el arranque. Si el cambio pierde datos, hace falta el visto bueno explícito de quien mantiene el repositorio |
| Cualquier cosa que se renderice | Verifica el tema **claro y oscuro**: no hay tests de render. Solo tokens `var(--ff-*)`, nunca hex a pelo. Iconos solo en `apps/web/src/components/icons.tsx` |
| La base, la ventana o el nombre de una cifra visible | El texto del catálogo de descripciones (`apps/web/src/lib/helpTexts.ts`) va en el mismo cambio, y el CHANGELOG dice la base de antes y la de después |
| `Dockerfile`, `docker-entrypoint.sh` o cualquier `docker-compose*.yml` | `shellcheck -S warning apps/api/docker-entrypoint.sh scripts/*.sh` limpio y el job `docker-stack` en verde |

### Qué cuenta como prueba

- **Un fix** lleva un test que falla con el código viejo y pasa con el nuevo. Uno por
  comportamiento, sin paquetes de aserciones «ya que estamos».
- **Un refactor que no debe cambiar la salida** lleva una captura de regresión: escribe primero el
  test con el valor de antes, ejecútalo contra el código viejo, y luego refactoriza hasta que pase.
- **Un cambio de modelo** (matemática del motor, fórmula FIRE) se predice antes de medirse: escribe
  el número que esperas —a mano, o con `python3`— y después compruébalo. No al revés.
- Los importes se serializan como cadenas (`"1000.0000"`, nunca `"1000"`). Compáralos parseando a
  `f64` con tolerancia, no con `assert_eq!` sobre la cadena.
- «El gráfico se ve igual» no es una prueba.

## 4. Documentación: no es un paso opcional

El repositorio tiene documentos de record y cada área tiene el suyo. Un cambio que añade una ruta
pero no toca `.claude/api-routes.md` está incompleto.

| Cambiaste… | Actualiza |
|---|---|
| Rutas, handlers, campos de request/response | `.claude/api-routes.md` |
| Tablas, columnas, migraciones | `.claude/data-model.md` |
| API pública del motor, bucle de simulación | `.claude/engine.md` |
| Variables de entorno | `.claude/env-and-config.md`, `.env.example` y `docs/configuracion.md` |
| Autenticación, roles, cookie, sesiones | `.claude/auth-and-membership.md` |
| Tokens, componentes, convenciones visuales | `.claude/design-system.md` |
| Estructura de `apps/web/src/` | `.claude/frontend-structure.md` |
| Infraestructura de tests | `.claude/tests.md` |
| Comandos, flujo de git, arquitectura | `CLAUDE.md` |
| Cualquier cosa que un usuario o quien autoaloja pueda notar | `CHANGELOG.md`, bajo `## [Unreleased]` |

El CHANGELOG de este proyecto es **forense**: para un fix, la entrada tiene que dejar reconstruir
síntoma → causa raíz → arreglo → por qué no vuelve, sin ir al historial de git. «Arreglado el
solape de la tabla» está por debajo del listón. Si hubo intentos fallidos previos, nómbralos.

## 5. Convención de commits

[Conventional Commits](https://www.conventionalcommits.org/): `tipo(scope): asunto`.

- **Tipos en uso**, por frecuencia: `feat`, `docs`, `fix`, `chore`, `ci`, `test`, `refactor`,
  `release`.
- **El scope es libre** y describe el área, no la capa: `web`, `api`, `engine`, `projection`,
  `mcp`, `transactions`, `docker`, `skills`, `release`… Se admiten varios separados por coma
  (`fix(transactions,projection): …`). Se puede omitir.
- **El asunto va en español**, en minúscula, sin punto final, y dice **qué cambia para quien usa la
  app**, no qué fichero tocaste. Compara:

  ```
  feat(recurring): las instancias convergen a los meses con datos reales
  fix(reconcile): el barrido periódico invalida la cache de proyección
  ```

  con «actualiza recurring.rs». El primero se puede leer dentro de un año.
- **El cuerpo** explica el porqué y lleva `Closes #N` si cierra un issue.
- Un cambio *breaking* se marca en el asunto o en el cuerpo, y además en el CHANGELOG.
- **Los merges también.** Un commit de merge lleva asunto propio: «Merge branch 'x'» no dice nada
  y obliga a ir al historial para saber qué entró. Mergeando un PR desde GitHub el asunto sale del
  título del pull request, así que ponle uno que se lea dentro de un año.

## 6. Pull requests

Rellena la plantilla ([`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md)): es
la lista de puertas de la sección 3 en forma de casillas. Pega la salida real de los comandos que
ejecutaste; «lo he probado» no es evidencia.

Un pull request que cambia números —proyección, FIRE, presupuesto, promedios— se revisa por las
cifras, no por el diff. Enséñalas.

## 7. Qué no se acepta

- **Datos reales de una persona, en ningún sitio.** Ni en un fixture, ni en un comentario, ni en
  el CHANGELOG, ni en una captura, ni en un issue. Da igual que sean tuyos: el repositorio es
  público y el historial de git es para siempre. Un fixture se **fabrica**, no se anonimiza —
  anonimizar un extracto real es borrar sobre datos que siguen ahí. Las reglas completas, cómo se
  construye un fixture que sigue valiendo como prueba y qué hacer si algo ya se ha colado están en
  [`.claude/skills/futurefin-data-hygiene/SKILL.md`](.claude/skills/futurefin-data-hygiene/SKILL.md).
  Hay un gate automático, `./scripts/scan-sensitive.sh`, que corre en CI y es bloqueante; es una
  red, no un sustituto del criterio: no detecta un alquiler ni el nombre de un comercio.
- **`f64` para dinero.** En el dominio, en el motor y en la base de datos, los importes son
  `rust_decimal::Decimal`, y la API los serializa como cadenas. El frontend los recibe y los envía
  como cadenas, nunca como números en coma flotante.
- **Colores a pelo en CSS o componentes.** Solo tokens `var(--ff-*)` de
  `apps/web/src/styles/theme.css`.
- **Handlers `GET` que mutan.** Un `GET` no borra ni actualiza filas. Los pasivos vencidos se
  **filtran** en la consulta; no se purgan.
- **Editar una migración ya publicada.** El checksum no cuadra y la instalación de alguien no
  arranca. Migración nueva siempre.
- **Reintroducir algo ya descartado** sin releer por qué se descartó. El historial de callejones
  sin salida está en
  [`.claude/skills/futurefin-failure-archaeology/SKILL.md`](.claude/skills/futurefin-failure-archaeology/SKILL.md).

## 8. Licencia

FutureFin se distribuye bajo [AGPL-3.0](LICENSE). Al enviar una contribución aceptas que se
publique bajo esa misma licencia.
