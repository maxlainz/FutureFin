# Actualizar y volver atrás

Cómo subir de versión, qué red de seguridad se activa sola, cómo volver a la versión anterior y
cómo migrar desde la topología de dos contenedores de la 2.x.

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
un PostgreSQL **más nuevo**. Si una 4.x futura ya hizo `pg_upgrade` de tu volumen a la 17, no
podrás volver a la 3.x sin restaurar un dump.

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

## Actualizar desde 2.x (dos contenedores) a 3.x

Hasta la 2.x el stack eran **dos contenedores**: `futurefin` y `futurefin-database`. Desde la 3.0.0
es **uno solo**, con PostgreSQL dentro de la imagen. Es la operación más grande de la historia del
proyecto, y es de un solo sentido: léela entera antes de empezar.

**Sin pérdida de datos**: el volumen `pgdata` de la 2.x conserva el mismo nombre y la misma ruta en
la 3.x, así que se reutiliza tal cual.

### Antes de tocar nada

Las dos capas, no una:

```bash
# 1. Capa de aplicación: que cada usuario exporte su .ffbackup desde Ajustes.
# 2. Capa de infraestructura: un pg_dump con el stack 2.x todavía en marcha.
ENV_FILE=.env ./scripts/backup-postgres.sh
```

### La actualización

Sustituye tu `docker-compose.yml` por el de la 3.x (el de este repositorio) y:

```bash
docker compose pull && docker compose up -d --remove-orphans
```

El `--remove-orphans` es lo que retira el contenedor `futurefin-database`, ya inútil. **No borres
el volumen `pgdata`**: el compose de la 3.x monta exactamente ese nombre en exactamente esa ruta, y
ahí está tu vida.

### El primer arranque tarda más — una única vez

Antes de que la API llegue a levantar, el contenedor hace tres cosas que solo se hacen una vez:

1. **Adopción de permisos.** El cluster de la 2.x lo creó `postgres:16.4-alpine` (uid 70); la 3.x
   corre el postmaster como el `postgres` de Debian (uid 999). El entrypoint hace `chown -R` y
   loguea `adopting ownership of PGDATA (uid 70 -> 999)`.
2. **Reindexado por colación.** Alpine usa musl y Debian usa glibc, y ordenan el texto de forma
   distinta: todos los índices de texto heredados de la 2.x son sospechosos (un índice UNIQUE
   corrupto aceptaría en silencio nombres de usuario duplicados). Se ejecuta un `REINDEX DATABASE`
   completo. En una base grande esto puede tardar. Es idempotente: se apunta en `ffdata` y no se
   repite para el mismo cluster.
3. **Backup automático pre-migración**, escrito en `ffdata` antes de que ninguna migración de la
   3.x toque el esquema. Si falla, el arranque se aborta.

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
- **Si actualizaste sin tocar el compose** (el caso típico de watchtower con `:latest`): la imagen
  3.x se encuentra en una topología 2.x —sin volumen montado en su `PGDATA` y con `DATABASE_URL`
  apuntando al contenedor de base de datos— y lo detecta. Sigue funcionando **contra la base de
  datos antigua**, en modo compatibilidad externa, imprimiendo un aviso de deprecación en cada
  arranque. Nunca migrará a la capa efímera del contenedor. Termina el trabajo adoptando el compose
  nuevo cuando puedas: ese modo **se elimina en la 4.0.0**.
- **Base de datos externa de verdad** (una gestionada aparte, no el contenedor de la 2.x): al
  arrancar la 3.x con `DATABASE_URL` definida **y un volumen vacío montado**, se hace una
  **automigración de una sola vez** — dump de la externa, restauración en la embebida y
  verificación por censo de filas antes de dar el paso por bueno. La base externa solo se **lee**,
  nunca se toca. Si prefieres quedarte en la externa: `FUTUREFIN_DB_MODE=external` (deprecado,
  desaparece en la 4.0.0, y en ese modo **no hay backup automático ni `pg_upgrade`**).

### Volver a la 2.x

Es aburrido a propósito, porque la 3.0.0 no cambia la forma del cluster en disco:

```bash
docker compose down
# restaura tu docker-compose.yml de 2.x y vuelve a poner POSTGRES_PASSWORD en el .env
docker compose up -d
```

- El volumen `pgdata` no necesita conversión: la imagen Alpine vuelve a hacer `chown` a su propio
  uid al arrancar, igual que la 3.x hizo en la otra dirección.
- El volumen `ffdata` queda **huérfano**. No lo borres si quieres conservar los backups
  automáticos: sácalos antes con `docker compose cp`.
- Sigue vigente la regla dura: **si la 3.x aplicó migraciones que el binario 2.x no lleva, no
  arranca.** Comprueba `_sqlx_migrations` como se explica arriba.

## `pg_upgrade` automático de un volumen antiguo

Cada imagen lleva el PostgreSQL actual **y el anterior**: la 3.x lleva el 16 (activo) y el 15
(solo para actualizar volúmenes viejos). Si el volumen está en la 15, el contenedor hace el
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
3.x antes de que llegue una 4.x que lleve 17+16. Un volumen creado por un PostgreSQL más nuevo que
el de la imagen se rechaza en seco.

## Ver también

- [Copias de seguridad](backups.md) — las tres capas, y cómo restaurar un dump
- [Configuración](configuracion.md) — todas las variables citadas aquí
- [Instalación](instalacion.md) — volúmenes, primer arranque, roles
