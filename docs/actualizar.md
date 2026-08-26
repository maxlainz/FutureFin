# Actualizar y volver atrás

Cómo subir de versión, qué red de seguridad se activa sola, cómo volver a la versión anterior y
qué hacer si vienes de la topología de dos contenedores de la 2.x o de una base de datos
externa.

## Cómo se publican las versiones

Cada release publica la misma imagen en dos registries:

| Registry | Imagen |
|---|---|
| Docker Hub | `maxlainz/futurefin` |
| GHCR | `ghcr.io/maxlainz/futurefin` |

Y cuatro etiquetas por versión. Publicar la `v3.2.1` crea:

| Etiqueta | Qué sigue |
|---|---|
| `3.2.1` | Exactamente esa versión. Nunca cambia. |
| `3.2` | El último parche de la 3.2. |
| `3` | La última versión de la serie 3.x. |
| `latest` | La última versión, sin más. |

Las etiquetas de imagen **no llevan la `v`**: el tag de git `v3.2.1` produce la imagen `:3.2.1`.

### Cuál elegir

`FUTUREFIN_TAG=latest` es el valor por defecto y significa que cualquier `docker compose pull`
puede saltar de versión, incluidas las que traen migraciones nuevas de base de datos. Para una
instalación que te importe, **fija la versión completa** en tu `.env`:

```env
FUTUREFIN_TAG=3.9.0
```

`3.9` y `3` son términos medios: flotan dentro del parche o del minor. Quien siguiera `:2` no
recibió la 3.x automáticamente — esa es justo la vía conservadora.

> **Nota para versiones 4.0.1–4.0.6 en Docker Hub**: por un fallo de publicación (corregido en
> 4.0.6) esas versiones salieron a Docker Hub solo como `:latest`, así que fijarlas ahí falla
> con «manifest not found». En **GHCR están todas**: para fijar una de ellas usa
> `FUTUREFIN_IMAGE=ghcr.io/maxlainz/futurefin` junto al `FUTUREFIN_TAG`. Desde la siguiente
> versión, ambos registries vuelven a llevar todos los tags.

## Actualizar dentro de la 3.x

```bash
# 1. (Opcional pero barato) copia manual antes de tocar nada:
./scripts/backup-postgres.sh

# 2. Cambia FUTUREFIN_TAG en .env, o quédate en :latest y acepta el salto
docker compose pull && docker compose up -d

# 3. Verifica
curl -sf http://127.0.0.1:8080/v1/health    # el campo "version" debe ser el nuevo
docker compose logs futurefin | grep -E "pre-migration backup written|migrations applied|ERROR"
```

Las migraciones pendientes se aplican solas en el primer arranque de la imagen nueva, **después**
de que el contenedor haya escrito su backup automático.

Después de una actualización, la prueba que de verdad vale no es `/v1/health`: **entra, abre la
pestaña Jubilación y exporta un `.ffbackup`**. Una vez, un cambio de esquema dejó la exportación
rota mientras el health seguía en verde.

## El backup automático pre-migración

Es la red de seguridad que se activa sola, sin que nadie la pida.

**Cuándo salta**: cuando la versión de la app ha cambiado desde el último arranque, o cuando la
imagen trae migraciones que la base de datos todavía no ha aplicado. Una instalación recién creada
se salta el paso (no hay nada que perder aún).

**Qué escribe**: un `pg_dump` comprimido dentro del volumen `ffdata`, con el nombre
`pre-migration-<versión-origen>-to-<versión-destino>-<timestamp>.sql.gz`. Por ejemplo:

```
/var/lib/futurefin/backups/pre-migration-3.8.0-to-3.9.0-20260821T031500Z.sql.gz
```

**Si falla, el arranque se aborta.** El mensaje lo dice sin rodeos: `pre-migration backup FAILED —
refusing to start with pending migrations and no safety net.` Es deliberado: migrar sin red no es
una opción que se tome por accidente. Para saltárselo a conciencia,
`FUTUREFIN_PREMIGRATION_BACKUP=off`.

**Retención**: los `FUTUREFIN_BACKUP_KEEP` (10 por defecto) más recientes son intocables; del
resto, se borran los de más de `FUTUREFIN_BACKUP_KEEP_DAYS` (90) días. Y si el volumen baja de
256 MB libres, se poda por presión de disco sin bajar nunca de 3 ficheros.

**Ojo con dónde vive**: dentro de un volumen Docker, en la misma máquina. Es la red de las
actualizaciones, **no** una copia de seguridad fuera de casa. Para sacarlos al host:

```bash
docker compose cp futurefin:/var/lib/futurefin/backups ./backups-auto
```

Las tres capas de respaldo y cuándo usar cada una, en [backups.md](backups.md).

## Volver a una versión anterior

Cambia `FUTUREFIN_TAG` a la versión de antes y repite `docker compose pull && docker compose up -d`.

Con una regla dura que no se puede saltar: **las migraciones solo avanzan.** No hay migraciones
"de bajada" en ningún sitio del proyecto. Si la versión nueva aplicó migraciones que el binario
antiguo no lleva dentro, ese binario **no arranca**: se para con un error claro y sin tocar los
datos.

Para saber si el rollback es seguro, compara lo aplicado con lo que trae la versión destino:

```bash
# Qué migraciones ha aplicado ESTA base de datos:
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin \
  -c "SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;"

# Qué migraciones trae la versión a la que quieres volver:
git ls-tree --name-only v3.8.0 apps/api/migrations/
```

- Si los conjuntos coinciden → el rollback es seguro.
- Si la base tiene migraciones de más → **no bajes de versión**. O avanzas a una versión corregida,
  o restauras el dump pre-migración que el propio contenedor escribió (ver
  [backups.md](backups.md)) asumiendo que pierdes lo escrito desde la actualización.

Hay una segunda barrera, esta a nivel de PostgreSQL: una imagen nunca abre un cluster creado por
un PostgreSQL **más nuevo**. Si una versión futura ya hizo `pg_upgrade` de tu volumen a la 17, no
podrás volver a la 3.x sin restaurar un dump.

### La guarda de downgrade: qué verás exactamente

Desde la 4.3.0 ese "no arranca" tiene mensaje propio en vez del error crudo de la librería de
migraciones. Si arrancas una imagen antigua sobre datos ya migrados por una posterior, el log dice:

```
─────────────────────────────────────────────────────────────────────────────
FutureFin NO ARRANCA: esta base de datos viene de una versión MÁS NUEVA.
─────────────────────────────────────────────────────────────────────────────
La base tiene aplicada la migración <N>, que este binario (versión X.Y.Z)
no conoce. Es la firma de haber arrancado una imagen antigua sobre datos ya
migrados por una imagen posterior.

TUS DATOS ESTÁN INTACTOS: no se ha tocado nada. FutureFin prefiere no arrancar
antes que ejecutar un esquema viejo sobre datos nuevos.
```

Y da las dos salidas: corregir `FUTUREFIN_TAG` para volver a la versión más nueva —lo normal— o
restaurar el `pre-migration-*.sql.gz` si de verdad quieres quedarte en la antigua. La detección no
añade ninguna comprobación nueva: es el mismo fallo de siempre, contado para que se pueda accionar.
Cualquier otro error de migración (un desajuste de checksum, por ejemplo) **conserva su mensaje
original** y sigue sin auto-repararse.

## Actualizar el add-on de Home Assistant

Si lo instalaste como add-on ([home-assistant.md](home-assistant.md)), el canal es otro: ahí no hay
`FUTUREFIN_TAG` que tocar.

- **La versión del add-on ES la versión de la imagen.** El mismo workflow que publica una versión
  sube ese número en `main` cuando la imagen ya está verificada en el registry y el Release creado:
  la tienda nunca anuncia una versión que no exista.
- **Puede ir una versión por detrás durante un rato**, entre la publicación y el siguiente refresco
  del índice de repositorios del Supervisor. No es un error; **Buscar actualizaciones** lo fuerza.
- **La actualización automática de Home Assistant está soportada.** El backup automático
  pre-migración salta igual que en Compose, dentro de `/data/state/backups`.
- **El rollback es restaurar la copia de Home Assistant** que hiciste antes de actualizar, no
  reinstalar una versión anterior del add-on: se topará con la guarda de downgrade de arriba.

## Watchtower y otros actualizadores automáticos

Funciona sin intervención, con **una configuración imprescindible**:

```
WATCHTOWER_TIMEOUT=60s
```

Watchtower **ignora el `stop_grace_period` del compose** y usa su propio timeout, 10 segundos por
defecto. Eso puede mandar un SIGKILL al contenedor en mitad del checkpoint de cierre de
PostgreSQL. No corrompe nada —para eso existe el WAL—, pero el arranque siguiente paga una
recuperación, y en una base grande esa recuperación puede pasarse del `start_period` del
healthcheck y dejar el contenedor dando tumbos en `unhealthy`.

Con `FUTUREFIN_TAG=latest`, watchtower te subirá de versión sin preguntar. Es cómodo y es
exactamente lo que no quieres en una instalación a la que tengas cariño: fija la versión.

## Vengo de 2.x o tengo una base de datos externa

Hasta la 2.x el stack eran **dos contenedores**: `futurefin` y `futurefin-database`. Desde la 3.0.0
es **uno solo**, con PostgreSQL dentro de la imagen. Y desde la **4.0.0 la imagen ya no sabe hablar
con ninguna base de datos que no sea la suya**: el modo externo, marcado como deprecado desde la
3.0.0, se ha retirado.

Empieza por identificar tu caso:

| Tu situación | Qué hacer |
|---|---|
| Compose de la 2.x (dos contenedores), tus datos en el volumen `pgdata` | **Caso A**: sustituye el compose y reutiliza el volumen. No hace falta pasar por ninguna versión intermedia. |
| Tu PostgreSQL vive fuera del compose: gestionado, en otra máquina, en otro stack | **Caso B**: pasa **una vez** por la 3.9.0 para traértelo dentro. |
| Te ha llegado la 4.0.0 sola (watchtower + `:latest`) y el contenedor no arranca | **Caso C**: no has perdido nada. Léelo y vuelve al caso A o al B. |

Lo que no cambia en ninguno de los tres: **la 4.0.0 no escribe jamás en una base externa, y no
arranca con una base vacía fingiendo que no pasa nada.** Si no puede continuar, se para y lo
explica.

### Antes de tocar nada

Las dos capas, no una:

```bash
# 1. Capa de aplicación: que cada usuario exporte su .ffbackup desde Ajustes.
# 2. Capa de infraestructura: un pg_dump con el stack antiguo todavía en marcha.
ENV_FILE=.env ./scripts/backup-postgres.sh
```

### Caso A — compose de 2.x, datos en el volumen `pgdata`

**Sin pérdida de datos**: el volumen `pgdata` de la 2.x conserva el mismo nombre y la misma ruta,
así que la imagen actual lo **adopta en el sitio**. No hay copia, ni conversión, ni versión
intermedia.

Sustituye tu `docker-compose.yml` por el de este repositorio y:

```bash
docker compose pull && docker compose up -d --remove-orphans
```

El `--remove-orphans` es lo que retira el contenedor `futurefin-database`, ya inútil. **No borres el
volumen `pgdata`**: el compose nuevo monta exactamente ese nombre en exactamente esa ruta, y ahí
está tu vida.

Y **quita `DATABASE_URL`** del compose: la 4.0.0 ya no la usa. Si se queda puesta y el volumen ya
tiene datos, se ignora con un aviso en los logs; si el volumen está vacío, el contenedor se para
(caso C).

### Caso B — base de datos externa de verdad

Una gestionada aparte, en otra máquina o en otro stack: no hay ningún volumen con tus datos que la
4.0.0 pueda adoptar, así que se niega a arrancar. La última versión que sabe traérselos es la
**3.9.0**, y lo hace en un solo arranque.

Necesita tres cosas a la vez: la imagen `3.9.0`, la `DATABASE_URL` de siempre **llegando dentro del
contenedor**, y un volumen montado en `/var/lib/postgresql/data` que esté **vacío**. Ojo con la
segunda: el `docker-compose.yml` de este repositorio **no** pasa `DATABASE_URL` al contenedor
(no la lista en `environment:` ni usa `env_file:`), así que ponerla en el `.env` no basta —
tienes que declararla:

```yaml
services:
  futurefin:
    image: maxlainz/futurefin:3.9.0        # temporal, solo para este arranque
    environment:
      DATABASE_URL: postgres://usuario:contraseña@tu-host:5432/futurefin
    volumes:
      - pgdata:/var/lib/postgresql/data    # vacío: es el destino
      - ffdata:/var/lib/futurefin          # aquí se guarda el dump intermedio
```

```bash
docker compose up -d
docker compose logs -f futurefin | grep -E "automigration completed|FATAL"
```

Qué hace por dentro, y por qué puedes dejarlo trabajar tranquilo:

1. Espera hasta 60 segundos a que la base externa conteste (`FUTUREFIN_EXTERNAL_WAIT_SECS`). Si no
   contesta, **se para**: nunca arranca vacío.
2. Le hace un `pg_dump` **de solo lectura** y lo guarda en `ffdata` como `pre-automigration-<ts>.sql.gz`.
3. Crea el cluster embebido y restaura ahí el dump.
4. **Verifica contando filas tabla a tabla** contra el origen. Si el censo no coincide, marca la
   migración como fallida y aborta — con tu base externa intacta y el dump guardado.
5. Loguea `automigration completed: N rows across M tables`.

Cuando lo veas: quita del compose la `DATABASE_URL` y el pin a `3.9.0`, y sube a la versión actual.

```bash
docker compose pull && docker compose up -d
```

Tu base externa se queda donde está, **intacta y sin usar**. Apágala cuando hayas comprobado que
todo funciona desde dentro.

Si prefieres no pasar por la 3.9.0, la alternativa manual es un `pg_dump` de la externa y un
restore en un volumen nuevo con `scripts/restore-postgres.sh` (ver [backups.md](backups.md)).

### Caso C — la 4.0.0 ya te ha llegado y el contenedor no arranca

`FUTUREFIN_TAG=latest` incluye los saltos de major: watchtower puede haberte subido de la 3.x a la
4.0.0 sin preguntar. Si tu `DATABASE_URL` apuntaba fuera y el volumen del contenedor no tenía una
base embebida, el contenedor sale con código de error y en los logs pone:

```
DATABASE_URL apunta a una base de datos EXTERNA y este volumen no contiene
todavía una base embebida.
```

**No has perdido nada.** La 4.0.0 se para *antes* de tocar nada, precisamente para que arrancar
vacío no se lea como una pérdida de datos: ni la base externa ni el volumen se han modificado. El
caso típico es el del compose 2.x sin tocar, donde el contenedor de la app no tiene ningún volumen
montado en su `PGDATA` y sus datos siguen en el contenedor `futurefin-database`.

Sal de ahí por el caso A (si tus datos están en el volumen `pgdata`) o por el caso B (si están en
una base externa de verdad). Y mientras te organizas, siempre puedes volver atrás fijando
`FUTUREFIN_TAG` a tu versión anterior.

Para que no vuelva a pasar: **fija la versión** en el `.env` (`FUTUREFIN_TAG=4.0.0`). La etiqueta
`:3` se queda en la última 3.x y nunca salta de major sola.

### El primer arranque tarda más — una única vez

Cuando el contenedor adopta un volumen de la 2.x (caso A), antes de que la API llegue a levantar
hace tres cosas que solo se hacen una vez:

1. **Adopción de permisos.** El cluster de la 2.x lo creó `postgres:16.4-alpine` (uid 70); la imagen
   actual corre el postmaster como el `postgres` de Debian (uid 999). El entrypoint hace `chown -R` y
   loguea `adopting ownership of PGDATA (uid 70 -> 999)`.
2. **Reindexado por colación.** Alpine usa musl y Debian usa glibc, y ordenan el texto de forma
   distinta: todos los índices de texto heredados de la 2.x son sospechosos (un índice UNIQUE
   corrupto aceptaría en silencio nombres de usuario duplicados). Se ejecuta un `REINDEX DATABASE`
   completo. En una base grande esto puede tardar. Es idempotente: se apunta en `ffdata` y no se
   repite para el mismo cluster.
3. **Backup automático pre-migración**, escrito en `ffdata` antes de que ninguna migración toque el
   esquema. Si falla, el arranque se aborta.

Por eso el healthcheck lleva `start_period: 120s`. Si el reindexado se pasa de ahí, el contenedor
aparecerá `unhealthy` un rato y luego se recuperará solo: **sigue los logs, no el `docker ps`**.

```bash
docker compose logs -f futurefin
```

Espera a que `/v1/ready` devuelva 200.

### Detalles de la 2.x que siguen importando

- **`POSTGRES_PASSWORD` ya no hace falta.** La base es local al contenedor, por socket Unix, sin
  ningún puerto TCP. Si la dejas puesta, se aplica al rol y no hace nada más.
- **Si personalizaste `POSTGRES_USER` o `POSTGRES_DB` en la 2.x, consérvalos en el `.env`.** El
  superusuario del cluster adoptado *es* ese rol; sin el valor correcto el arranque muere con
  `cannot connect as role 'futurefin'. If your 2.x install used a custom POSTGRES_USER, set the
  same value now.`
- **`FUTUREFIN_DB_MODE=external` ya no existe.** Si lo arrastras de un compose de la 3.x, el
  contenedor aborta con un mensaje que te dice qué quitar. Los valores válidos son `auto` (el de por
  defecto) y `embedded`, y hoy significan lo mismo.

### Volver a la 2.x

Es aburrido a propósito, porque no se cambia la forma del cluster en disco:

```bash
docker compose down
# restaura tu docker-compose.yml de 2.x y vuelve a poner POSTGRES_PASSWORD en el .env
docker compose up -d
```

- El volumen `pgdata` no necesita conversión: la imagen Alpine vuelve a hacer `chown` a su propio
  uid al arrancar, igual que la imagen nueva hizo en la otra dirección.
- El volumen `ffdata` queda **huérfano**. No lo borres si quieres conservar los backups
  automáticos: sácalos antes con `docker compose cp`.
- Sigue vigente la regla dura: **si la versión nueva aplicó migraciones que el binario 2.x no lleva,
  no arranca.** Comprueba `_sqlx_migrations` como se explica arriba.

## `pg_upgrade` automático de un volumen antiguo

Cada imagen lleva el PostgreSQL actual **y el anterior**: la imagen de hoy lleva el 16 (activo) y
el 15 (solo para actualizar volúmenes viejos). Si el volumen está en la 15, el contenedor hace el
`pg_upgrade` él solo al arrancar, y cada paso está diseñado para que el cluster viejo sobreviva a
un fallo:

- Comprueba que hay **el triple del tamaño actual** libre en `ffdata`; si no, aborta con las cifras
  exactas.
- Escribe un `pg_dumpall` **obligatorio** (`pre-pgupgrade-15-to-16-<ts>.sql.gz`) antes de nada.
- Hace el `pg_upgrade` en **modo copia**, no `--link`: el cluster viejo tiene que seguir usable.
- Verifica el resultado por **censo de filas** antes de promoverlo.
- Deja el cluster antiguo en `pgdata_old_15/`, dentro del mismo volumen. **Bórralo tú a mano**
  cuando estés tranquilo: hasta entonces ocupa el tamaño completo de la base.

El entrypoint **nunca borra un cluster**. Lo aparta con `mv` y lo deja ahí.

Consecuencia de la política de dos majors: un volumen que siga en la 15 tiene que pasar por una
imagen que todavía lleve el 15 antes de que salga la primera que ya no lo lleve. Un volumen creado por un PostgreSQL más nuevo que
el de la imagen se rechaza en seco.

## Ver también

- [Copias de seguridad](backups.md) — las tres capas, y cómo restaurar un dump
- [Configuración](configuracion.md) — todas las variables citadas aquí
- [Instalación](instalacion.md) — volúmenes, primer arranque, roles
