# Instalación

Cómo poner FutureFin en marcha desde cero, qué se crea en tu disco y cómo se entra la primera vez.

Desde la versión 3.0.0 el despliegue es **un solo contenedor**: PostgreSQL corre dentro de la
propia imagen, sobre un socket Unix. No hay que levantar una base de datos aparte, no hay que
inventarse contraseñas y **ninguna variable de entorno es obligatoria**.

## Requisitos

- **Docker** con **Compose v2** (`docker compose`, sin guion).
- Arquitectura `linux/amd64` o `linux/arm64` — la imagen se publica para ambas, así que un
  Raspberry Pi, un NAS ARM o un VPS x86 valen igual.
- Un puerto libre en el host (por defecto el `8080`).

No necesitas instalar PostgreSQL, ni Rust, ni Node: todo eso va dentro de la imagen.

> **¿Usas Home Assistant?** Hay un **add-on** que empaqueta esta misma imagen: se instala desde la
> tienda, sale en la barra lateral y entras con tu usuario de Home Assistant, sin escribir ningún
> `docker-compose.yml`. Esta página no te hace falta: ve a
> [home-assistant.md](home-assistant.md).

## Instalar con Docker Compose

Crea un directorio vacío y guarda dentro el [`docker-compose.yml`](../docker-compose.yml) de este
repositorio. Es el fichero de referencia y contiene esto:

```yaml
name: futurefin

services:
  futurefin:
    image: ${FUTUREFIN_IMAGE:-maxlainz/futurefin}:${FUTUREFIN_TAG:-latest}
    container_name: futurefin
    restart: unless-stopped
    # PostgreSQL vive dentro de este contenedor: dale margen para el checkpoint de cierre.
    stop_grace_period: 60s
    ports:
      - "${APP_PORT:-8080}:8080"
    environment:
      RUST_LOG: "futurefin_api=info,tower_http=info,sqlx=warn"
      POSTGRES_USER: ${POSTGRES_USER:-futurefin}
      POSTGRES_DB: ${POSTGRES_DB:-futurefin}
    volumes:
      - pgdata:/var/lib/postgresql/data
      - ffdata:/var/lib/futurefin
    healthcheck:
      test: ["CMD-SHELL", "curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null"]
      interval: 15s
      timeout: 5s
      retries: 5
      start_period: 120s

volumes:
  pgdata:
  ffdata:
```

Arranca:

```bash
docker compose up -d
```

Un `.env` vacío —o directamente ningún `.env`— es una configuración válida. Si quieres cambiar el
puerto del host o fijar una versión concreta, escribe un `.env` al lado:

```env
APP_PORT=8080
FUTUREFIN_TAG=latest
```

El catálogo completo de variables está en [configuracion.md](configuracion.md); fijar la versión en
vez de seguir `latest` se explica en [actualizar.md](actualizar.md).

## Alternativa: `docker run`

Compose no es obligatorio. El equivalente mínimo:

```bash
docker run -d --name futurefin --restart unless-stopped \
  -p 8080:8080 \
  --stop-timeout 60 \
  -v futurefin_pgdata:/var/lib/postgresql/data \
  -v futurefin_ffdata:/var/lib/futurefin \
  maxlainz/futurefin:latest
```

Los dos `-v` **no son opcionales** (ver más abajo). El `--stop-timeout 60` es el equivalente del
`stop_grace_period` de Compose: le da a PostgreSQL margen para cerrar con su checkpoint hecho.

La imagen también está en GHCR si prefieres esa registry:
`ghcr.io/maxlainz/futurefin:latest`.

## Qué se crea en tu disco: los dos volúmenes

| Volumen | Ruta dentro del contenedor | Qué guarda | Si lo pierdes… |
|---|---|---|---|
| `pgdata` | `/var/lib/postgresql/data` | El cluster de PostgreSQL: **todos** tus datos, usuarios y sesiones | Lo pierdes todo. Es el volumen que hay que proteger. |
| `ffdata` | `/var/lib/futurefin` | Backups automáticos pre-migración, estado del entrypoint y el área de trabajo de `pg_upgrade` | Pierdes los backups automáticos y algunas marcas de "esto ya se hizo una vez". Recuperable, molesto. |

Con Compose los nombres reales llevan el prefijo del proyecto: `futurefin_pgdata` y
`futurefin_ffdata`. Para ver dónde viven en tu máquina:

```bash
docker volume inspect futurefin_pgdata futurefin_ffdata
```

**PostgreSQL no escucha en ningún puerto TCP** — ni fuera ni dentro del contenedor. Solo habla por
el socket Unix `/var/run/postgresql`. Para abrir un `psql`:

```bash
docker compose exec futurefin psql -h /var/run/postgresql -U futurefin -d futurefin
```

## Por qué se niega a arrancar sin volumen

Si arrancas el contenedor sin montar nada en `/var/lib/postgresql/data`, aborta con este mensaje:

```
[futurefin-entrypoint] FATAL: no persistent volume is mounted at /var/lib/postgresql/data —
your data would be LOST when the container is recreated.
```

No es un fallo: es una **protección deliberada**. Sin volumen, el cluster viviría en la capa
efímera del contenedor y desaparecería en cuanto algo lo recreara — una actualización manual, un
`docker compose up -d` tras cambiar el fichero, o un actualizador automático como watchtower. La
imagen tampoco declara un `VOLUME` en su `Dockerfile`, precisamente para que Docker no cree un
volumen anónimo que oculte el problema (los anónimos también se pierden al recrear).

Para pruebas de usar y tirar —CI, una demo de cinco minutos— puedes saltártelo:

```bash
docker run --rm -e FUTUREFIN_ALLOW_EPHEMERAL_DB=1 -p 8080:8080 maxlainz/futurefin:latest
```

Arranca avisando por los logs y **los datos mueren con el contenedor**. Nunca para algo que te
importe.

## El primer arranque

Sigue los logs, que van contando lo que pasa:

```bash
docker compose logs -f futurefin
```

En una instalación nueva verás, en este orden:

1. `initializing fresh PostgreSQL 16 cluster in /var/lib/postgresql/data`
2. `starting embedded PostgreSQL 16 (socket-only at /var/run/postgresql)`
3. `starting FutureFin API …`, y después `migrations applied` (las migraciones SQL se aplican
   solas al arrancar; no hay paso manual)
4. `listening on http://0.0.0.0:8080`

Las líneas con el prefijo `[futurefin-entrypoint]` son del supervisor del contenedor; el resto son
de PostgreSQL y de la API, todo en el mismo flujo de logs.

Comprueba que responde:

```bash
curl -sf http://127.0.0.1:8080/v1/health   # el proceso vive; devuelve versión
curl -sf http://127.0.0.1:8080/v1/ready    # 200 solo si además la base de datos contesta
```

`/v1/ready` es lo que mira el healthcheck del contenedor, con `start_period: 120s` de margen. Si
`docker ps` dice `starting` durante el primer minuto, es normal.

## El primer registro: quien llega primero es el propietario

Abre `http://localhost:8080` (o la IP de tu servidor) y **regístrate**. Solo hace falta un nombre
de usuario y una contraseña: no se pide correo electrónico porque no hay nada que enviar.

**La primera persona que se registra se convierte automáticamente en propietaria de la
instalación.** Es lo único que hay que saber sobre el orden: quien llega primero manda.

Al entrar por primera vez sale un **asistente de bienvenida** de cuatro pasos, que pregunta lo
mínimo que la app no puede adivinar y que, si se queda mal, hace que todas las cifras salgan raras:

1. **Tu hogar** — la **divisa base** (EUR, USD o GBP) y la **zona horaria**. La divisa es **una
   sola por instalación**: FutureFin no convierte ni mezcla divisas, así que esto define toda tu
   contabilidad, y también decide qué filas acepta el importador de CSV. La zona horaria viene
   rellenada con la de tu navegador; con la zona mal, "el gasto de hoy" puede caer en el día
   equivocado.
2. **Tu plan** — la **inflación anual** que asumes (hace crecer tu objetivo con el tiempo) y la
   **tasa de retirada segura**, el porcentaje de tu patrimonio que podrías gastar al año sin
   agotarlo. Los dos se cambian luego en `Ajustes → Plan`.
3. **Primer activo** — opcional. Si lo dejas en blanco, no se crea nada.
4. **Listo.**

El asistente **es saltable**. Si lo saltas, volverá a salir en la siguiente carga; y una vez
completado, el propietario puede reabrirlo cuando quiera desde `Ajustes → General → Configuración
inicial → Abrir el asistente`. No borra nada: solo vuelve a preguntar.

Un hogar recién creado **nace con categorías por defecto** en los cuatro ámbitos, para que no
aterrices en pantallas vacías sin saber por dónde empezar: `Cuenta corriente`, `Ahorro`,
`Inversión`, `Inmuebles` para activos; `Hipoteca`, `Préstamo`, `Tarjeta de crédito` para pasivos;
`Nómina` y `Otros ingresos` para ingresos; y `Vivienda`, `Supermercado`, `Suministros`,
`Transporte`, `Ocio`, `Salud`, `Otros gastos` para gastos. Se renombran, se borran y se amplían
desde `Ajustes → Categorías`.

## Aprobar a más usuarios

FutureFin es de **hogar compartido**: una instalación, varias personas, los mismos datos. Pero
registrarse no da acceso a nada.

1. La segunda persona se registra normalmente en la misma URL.
2. Al entrar ve una pantalla de espera: es un usuario **pendiente** y no ve ni un dato.
3. La propietaria abre `Ajustes → Usuarios`, ve la lista de pendientes, elige el rol y pulsa
   **Aprobar**.

Los tres roles:

| Rol | Puede |
|---|---|
| `owner` (propietario) | Todo, incluidos los ajustes de la instalación y aprobar usuarios. Es quien se registró primero. |
| `member` (miembro) | Leer y escribir datos del hogar. |
| `viewer` (visor) | Solo leer. Tampoco escribe a través de [Claude](mcp.md). |

Cada persona ve por defecto el agregado del hogar y puede filtrar a **lo suyo** con el selector de
vista de la app.

## Ponerlo detrás de HTTPS

FutureFin **no gestiona TLS ni conectividad**: sirve HTTP en su puerto y ya. Si la vas a exponer
fuera de tu red, ponla detrás de un proxy inverso (Caddy, nginx, Traefik) o de un túnel tipo
Cloudflare Tunnel.

Cuando termines TLS por delante, añade al servicio:

```yaml
    environment:
      COOKIE_SECURE: "true"
```

Eso marca la cookie de sesión como `Secure`. Si además vas a conectar Claude desde claude.ai, lee
[mcp.md](mcp.md): hay un caso en el que también hace falta `FUTUREFIN_PUBLIC_URL`.

## Servirla en un subpath (`https://tu-host/futurefin/`)

Desde la 4.3.0 FutureFin puede vivir colgada de una ruta y no solo de la raíz de un dominio. El
servidor **sigue montando todas sus rutas en la raíz**: quien quita el prefijo es tu proxy. Lo
único que depende del prefijo es lo que resuelve el navegador —los assets del HTML, las llamadas
a la API, el `Path` de la cookie de sesión—, y eso se inyecta **por petición** en el `index.html`.

Hay dos formas de decírselo, y la primera gana sobre la segunda:

| Cómo | Qué es |
|---|---|
| Cabecera `X-Forwarded-Prefix: /futurefin` | Lo normal: el proxy la manda en cada petición. También se acepta `X-Ingress-Path`, que es la que usa el add-on de Home Assistant y tiene precedencia sobre las demás. |
| Variable `FUTUREFIN_BASE_PATH=/futurefin` | Prefijo fijo, para proxies que no mandan la cabecera. |

El prefijo tiene que empezar por `/`, sin `//`, sin segmentos `.` o `..`, con el juego de
caracteres `[A-Za-z0-9._~/%-]` y como mucho 128 caracteres. Una cabecera inválida se **ignora**
(con un aviso en el log); un `FUTUREFIN_BASE_PATH` inválido **aborta el arranque** — igual que
`FUTUREFIN_PUBLIC_URL`, mejor un fallo ruidoso que HTML roto en silencio.

**Sin prefijo no cambia nada**: el `index.html` se sirve byte a byte como está en el disco.

### nginx

```nginx
location /futurefin/ {
    proxy_pass http://127.0.0.1:8080/;   # la barra final es la que quita el prefijo
    proxy_set_header X-Forwarded-Prefix /futurefin;
    proxy_set_header Host              $host;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

### Caddy

```caddy
handle_path /futurefin/* {
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Forwarded-Prefix /futurefin
    }
}
```

`handle_path` (y no `handle`) es lo que recorta el prefijo antes de reenviar.

### Traefik

```yaml
http:
  middlewares:
    futurefin-prefix:
      chain:
        middlewares: [futurefin-strip, futurefin-header]
    futurefin-strip:
      stripPrefix:
        prefixes: ["/futurefin"]
    futurefin-header:
      headers:
        customRequestHeaders:
          X-Forwarded-Prefix: "/futurefin"
```

### Lo que NO funciona en un subpath

**MCP y OAuth.** El descubrimiento de OAuth 2.1 exige servir `/.well-known/oauth-authorization-server`
y `/.well-known/oauth-protected-resource` en la **raíz del origen**, y en un subpath esa raíz es de
otro. Si vas a conectar un cliente de IA, sirve FutureFin en la raíz de un dominio (o subdominio)
propio. Ver [mcp.md](mcp.md).

## Parar, reiniciar y desinstalar

```bash
docker compose logs -f futurefin        # ver qué está pasando
docker compose restart futurefin        # reinicio completo: PostgreSQL también rebota
docker compose down --remove-orphans    # parada ordenada; LOS DATOS SE CONSERVAN
docker compose down -v                  # DESTRUCTIVO: borra pgdata y ffdata (todo)
```

El apagado es ordenado: primero se drena la API, después PostgreSQL cierra con su checkpoint. Por
eso el `stop_grace_period: 60s` del compose — no lo bajes.

`docker compose down -v` se lleva por delante **también los backups automáticos**. Antes de hacer
algo así, lee [backups.md](backups.md).

## Si algo no arranca

- **Se para nada más empezar con `FATAL: no persistent volume`** → falta el volumen; mira la
  sección de arriba.
- **`docker ps` dice `unhealthy`** → mira los logs. `/v1/health` OK + `/v1/ready` 503 significa que
  la API vive pero la base de datos no contesta.
- **El puerto 8080 está ocupado** → cambia `APP_PORT` en el `.env` y `docker compose up -d`.
- **Se para diciendo que `DATABASE_URL` apunta a una base de datos EXTERNA** → desde la 4.0.0 la
  imagen solo usa su PostgreSQL embebido. Si tus datos ya están dentro, quita esa variable del
  compose; si todavía no los has migrado, la ruta exacta está en
  [actualizar.md](actualizar.md#vengo-de-2x-o-tengo-una-base-de-datos-externa).

## Siguientes pasos

- [Actualizar y volver atrás](actualizar.md)
- [Copias de seguridad](backups.md)
- [Variables de entorno y ajustes](configuracion.md)
- [Conectar Claude](mcp.md)
