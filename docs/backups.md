# Copias de seguridad

FutureFin tiene **tres capas de respaldo** que no se sustituyen entre sí. Merece la pena tener
claro qué cubre cada una antes de necesitarlas.

| Capa | Qué copia | Alcance | Quién la ejecuta |
|---|---|---|---|
| **1. `.ffbackup` por usuario** | Los datos de **una** persona, cifrados con su contraseña | Sus activos, pasivos, presupuesto, movimientos… | Cada usuario, desde la app |
| **2. Backup automático pre-migración** | La base de datos entera, antes de cada actualización con migraciones | Todo | El contenedor, solo, sin que nadie lo pida |
| **3. `pg_dump` manual** | La base de datos entera, cuando tú quieras | Todo | Tú, a mano o por cron |

En corto: la **capa 2** es la red de las actualizaciones, la **capa 3** es tu copia de desastre y
la **capa 1** es portabilidad de datos por persona. Las capas 1 y 2 viven dentro de la máquina
(volúmenes Docker): **si se te quema el servidor, se van con él**. La única que puedes sacar de
casa es la 3 — y eso hay que hacerlo aparte, porque el script deja los ficheros en el host y ahí
se quedan.

---

## Capa 1 — El `.ffbackup` de cada usuario

Un archivo binario, cifrado, con **tus** datos y solo los tuyos. Es la copia portable: sirve para
llevarte tus números a otra instalación, o para tener algo tuyo que no dependa de que un volumen
sobreviva.

**Cómo se hace**: `Ajustes → Copias de seguridad → Copia de seguridad personal (.ffbackup)`, botón
**Exportar mis datos**. Te pedirá tu contraseña de la cuenta.

**Qué lleva dentro**: las filas cuyo dueño eres tú — activos, reglas de reparto, pasivos, entradas
de presupuesto, planificación, las categorías que usas, tus snapshots de histórico, tus
movimientos, importaciones y reglas de categorización, tus reglas recurrentes, tu fecha de
nacimiento y tus preferencias de interfaz. Incluye además una foto **informativa** de los ajustes
de la instalación (divisa, zona horaria, inflación, supuestos FIRE) que **no se aplica al
importar**: es contexto, no configuración.

**Cómo está cifrado**: AES-256-GCM con una clave derivada de tu contraseña de cuenta mediante
Argon2id, con sal y nonce aleatorios en cada exportación, sobre una carga comprimida con gzip. El
manifiesto del fichero va en claro —lo justo para que el servidor pueda rechazar una versión que
no entiende sin descifrar nada—, y el resto no.

> **Si olvidas la contraseña que tenías en el momento de exportar, el archivo es irrecuperable.**
> No hay puerta de atrás, y una contraseña equivocada da un error genérico indistinguible de un
> fichero corrupto. Es a propósito.

**Cómo se restaura**: en la misma pantalla, **Importar backup**. Dos cosas que hay que saber antes
de pulsar:

- **La importación reemplaza, no fusiona.** Borra *todas* tus filas actuales y mete las del
  archivo, en una sola transacción: o entra todo o no entra nada. No existe modo mezcla.
- Antes de aplicar nada, la app enseña un **preview** con los recuentos de lo que va a entrar.
  Léelo. Es la última oportunidad de darte cuenta de que ese no era el archivo.

**Compatibilidad entre versiones**: cada archivo lleva un `schema_version` (hoy el **9**, agosto de
2026). Todos los formatos antiguos, del 1 al 9, se siguen importando: se migran en memoria al
llegar. Al revés no: un archivo de una versión **más nueva** que el servidor se rechaza limpiamente
con un "actualiza FutureFin para importar este backup" — rechazo claro, nunca datos a medias.

La API por debajo, si prefieres automatizarlo (todos con cookie de sesión):

| Endpoint | Notas |
|---|---|
| `POST /v1/backup/user-export` | Cuerpo `{"password": "..."}`. Devuelve el binario. |
| `POST /v1/backup/user-import/preview` | Descifra y devuelve recuentos **sin cambiar nada**. |
| `POST /v1/backup/user-import` | Lo mismo, **más `"confirm_replace": true`**. Sin ese campo, 400. |

Los dos de importación aceptan hasta 16 MiB de cuerpo.

---

## Capa 2 — El backup automático pre-migración

Se dispara solo, y es la razón por la que actualizar es aburrido.

**Cuándo**: cuando la versión de la app cambia respecto al último arranque, o cuando la imagen trae
migraciones que la base todavía no ha aplicado. Una instalación recién creada se lo salta: no hay
nada que perder.

**Dónde y cómo se llama**: dentro del volumen `ffdata`, comprimido:

```
/var/lib/futurefin/backups/pre-migration-3.8.0-to-3.9.0-20260821T031500Z.sql.gz
```

En el mismo directorio pueden aparecer otros dos tipos, con la misma retención:
`pre-pgupgrade-*` (antes de un `pg_upgrade` de PostgreSQL, ese es **obligatorio**) y
`pre-automigration-*` (antes de traerse los datos de una base externa).

**Si falla, el arranque se aborta**: `pre-migration backup FAILED — refusing to start with pending
migrations and no safety net.` Migrar sin red no es algo que pase por accidente. Se puede desactivar
a conciencia con `FUTUREFIN_PREMIGRATION_BACKUP=off`.

**Retención**: los `FUTUREFIN_BACKUP_KEEP` (10 por defecto) más recientes no se tocan nunca; del
resto se borran los de más de `FUTUREFIN_BACKUP_KEEP_DAYS` (90) días. Si el volumen baja de 256 MB
libres, se poda por presión de disco sin bajar nunca de 3 ficheros.

**Sácalos al host** — sobre todo antes de desmontar un stack, y después de venir de la 2.x:

```bash
docker compose cp futurefin:/var/lib/futurefin/backups ./backups-auto
ls -lh backups-auto/
```

Y recuerda: `docker compose down -v` borra el volumen `ffdata` y **se lleva estos backups con él**.

---

## Capa 3 — `pg_dump` manual con los scripts del repositorio

### Hacer una copia: `scripts/backup-postgres.sh`

```bash
./scripts/backup-postgres.sh
```

Comprueba que el servicio está corriendo y hace un `pg_dump` **por el socket Unix del contenedor**,
comprimiéndolo en el host:

```
./backups/futurefin-postgres-20260822T031500Z.sql.gz
```

Variables opcionales:

| Variable | Por defecto | Qué hace |
|---|---|---|
| `BACKUP_DIR` | `./backups` | Dónde deja los ficheros. |
| `KEEP_BACKUPS` | `30` | Cuántos conserva; el resto se borran. |
| `SERVICE` | `futurefin` | Nombre del servicio de Compose. |
| `ENV_FILE` | vacío | `--env-file` para Compose. **Ya no es obligatorio.** |
| `POSTGRES_USER` / `POSTGRES_DB` | `futurefin` | Solo si los personalizaste. |

Una línea de cron razonable, todos los días a las 3:15:

```
15 3 * * * cd /srv/futurefin && ./scripts/backup-postgres.sh >> backups/backup.log 2>&1
```

**Estos ficheros se quedan en el host.** Copiarlos a otra máquina, a un NAS o a almacenamiento
remoto es cosa tuya, y es lo que convierte esta capa en una copia de desastre de verdad.

### Restaurar: `scripts/restore-postgres.sh`

Sirve tanto para tus dumps manuales como para los automáticos pre-migración:

```bash
./scripts/restore-postgres.sh backups/futurefin-postgres-20260822T031500Z.sql.gz
./scripts/restore-postgres.sh backups-auto/pre-migration-3.8.0-to-3.9.0-*.sql.gz --yes
```

Sin `--yes` pide confirmación por teclado, porque la operación **borra la base actual**.

Resuelve por dentro la parte incómoda —no se puede tirar una base de datos a la que la API está
conectada— en seis pasos, cada uno anunciado por pantalla:

1. Para el servicio normal, con apagado ordenado.
2. Levanta un contenedor temporal en **modo rescate** (`FUTUREFIN_MODE=db-only`): PostgreSQL sin
   API, sobre los mismos volúmenes. Ese contenedor aparecerá `unhealthy` — es esperado, no hay API
   detrás de `/v1/ready`.
3. Hace un **censo de filas antes**.
4. Recrea la base y restaura el dump con `ON_ERROR_STOP=1`.
5. **Censo de filas después**, y para el contenedor de rescate con un cierre limpio.
6. Vuelve a levantar el stack y espera a `/v1/ready` hasta 120 segundos.

Si el dump es **más antiguo** que la imagen que está corriendo, el arranque normal aplica hacia
delante las migraciones que falten (`docker compose logs futurefin | grep 'migrations applied'`).
Si el dump es **más nuevo** que la imagen, se aplican las reglas de rollback de
[actualizar.md](actualizar.md): las migraciones solo avanzan.

---

## Qué copiar antes de cada cosa

| Vas a… | Haz al menos |
|---|---|
| Actualizar dentro de la 3.x | Nada obligatorio: la capa 2 salta sola. Un `backup-postgres.sh` es barato. |
| Actualizar desde la 2.x | Las **dos**: que cada usuario exporte su `.ffbackup` y un `pg_dump` con el stack 2.x en marcha. |
| Mover la instalación a otra máquina | `pg_dump` (capa 3) y llévate el fichero. |
| Desmontar o rehacer el stack | Saca los backups automáticos con `docker compose cp` antes de tocar los volúmenes. |
| Experimentar con algo raro | `pg_dump`. Siempre. |

## Ver también

- [Actualizar y volver atrás](actualizar.md) — cuándo salta la capa 2 y cómo funciona el rollback
- [Configuración](configuracion.md) — las variables `FUTUREFIN_BACKUP_*` y `FUTUREFIN_MODE`
- [Instalación](instalacion.md) — qué guarda cada volumen
