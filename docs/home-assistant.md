# FutureFin como add-on de Home Assistant

FutureFin se instala también como **add-on de Home Assistant**: un panel más en la barra lateral,
sin tocar Docker ni escribir un `docker-compose.yml`, y con los datos dentro de las copias de
seguridad que Home Assistant ya hace.

Es **la misma imagen** que se publica para Docker Compose, empaquetada. PostgreSQL sigue viviendo
dentro del contenedor; lo único que cambia es dónde: en el add-on todo cuelga de `/data`, el único
directorio persistente que el Supervisor monta.

> La ficha corta que se ve dentro de Home Assistant está en
> [`addon/futurefin/DOCS.md`](../addon/futurefin/DOCS.md). Esta página es la versión larga: lo
> mismo, más el porqué de cada cosa y los casos que no caben en una pestaña.

---

## 1. Instalación

### Añadir el repositorio

Este repositorio de GitHub es **también** una tienda de add-ons: el fichero
[`repository.yaml`](../repository.yaml) de la raíz es lo que hace que el Supervisor lo reconozca, y
el add-on en sí vive en `addon/futurefin/`.

[![Añadir repositorio a Home Assistant](https://my.home-assistant.io/badges/supervisor_add_addon_repository.svg)](https://my.home-assistant.io/redirect/supervisor_add_addon_repository/?repository_url=https%3A%2F%2Fgithub.com%2Fmaxlainz%2FFutureFin)

Ese botón abre el diálogo de "añadir repositorio" ya relleno en tu propia instancia. Si prefieres
hacerlo a mano: **Ajustes → Complementos → Tienda de complementos → ⋮ → Repositorios**, y pega

```
https://github.com/maxlainz/FutureFin
```

### Instalar y arrancar

1. En la tienda, recarga y busca **FutureFin**. Pulsa **Instalar**. El Supervisor descarga la
   imagen ya publicada (`maxlainz/futurefin`, Docker Hub); **el add-on no compila nada**, así que el
   tiempo es el de la descarga.
2. Pulsa **Iniciar**. El primer arranque crea el cluster de PostgreSQL desde cero y aplica todas
   las migraciones: puede tardar un minuto largo. La pestaña **Log** lo va contando.
3. Activa **Mostrar en la barra lateral**. El panel se llama **FutureFin** y usa el icono
   `mdi:currency-usd`.
4. Abre el panel. Ya estás dentro.

No hay que publicar ningún puerto, ni configurar una URL, ni tocar TLS: el acceso normal va por el
**ingress** del Supervisor, que ya es el HTTPS y la autenticación de tu Home Assistant.

### Arquitecturas

La imagen se publica solo para **amd64** y **aarch64**. Un Home Assistant sobre Raspberry Pi de
64 bits o sobre un mini PC x86 vale; un armv7 o un i386 no verán el add-on en la tienda.

---

## 2. Primer arranque y usuarios

FutureFin es de **hogar compartido**: una instalación, varias personas, los mismos datos, con tres
roles (`owner`, `member`, `viewer`). Eso no cambia en el add-on. Lo que cambia es **cómo se
identifica cada persona**.

> **Quién ve el panel.** El add-on no declara `panel_admin`, así que vale el valor por defecto del
> Supervisor: **solo los administradores de Home Assistant** ven el icono de FutureFin en la barra
> lateral. Eso acota la frase «la primera persona que abre el panel se convierte en propietaria» —
> esa persona solo puede ser un administrador de HA, no cualquier usuario de la casa. Aun así,
> **instálalo y ábrelo tú primero**: si hay varios administradores, la propiedad se la lleva quien
> llegue antes, y recuperarla después es trabajo manual.

### Con `sso` activada (el valor por defecto)

El ingress del Supervisor ya ha autenticado a la persona antes de que la petición llegue al add-on,
y le añade una cabecera con su identidad de Home Assistant. FutureFin la canjea por una sesión
normal suya (`POST /v1/auth/sso`): la misma fila en la tabla `sessions`, la misma cookie
`ff_session`, las mismas reglas de pertenencia al hogar. No hay formulario que rellenar.

- **La primera persona de Home Assistant que abre el panel se convierte en propietaria** de la
  instalación, exactamente igual que quien se registra primero en una instalación por Compose.
- **Las siguientes entran como pendientes**: ven la aplicación, pero no ven ni un dato hasta que
  la propietaria las aprueba en `Ajustes → Usuarios` y les asigna rol.
- El nombre de usuario sale del nombre para mostrar de Home Assistant (o, si no viene, del nombre
  de cuenta); si ya estuviera cogido, se le añade un sufijo.

Si algo falla en ese canje —el proxy no responde, la cabecera no llega—, la app **cae al formulario
de acceso de siempre**. El SSO es un atajo, nunca la única puerta.

### Con `sso` desactivada

No hay identidad delegada: el panel enseña el **login clásico** de FutureFin (usuario y contraseña)
dentro del iframe del ingress, y el primer registro es el que se lleva la propiedad. Es lo que
quieres si prefieres que la sesión de Home Assistant y la de FutureFin sean cosas separadas.

### Las cuentas SSO no tienen contraseña (y qué implica)

Una cuenta creada por SSO nace con el hash de contraseña a `NULL`. En esta versión **no puede
ponerse una**: `POST /v1/auth/password` responde `401 sso_account_no_password` en vez de fingir un
"contraseña actual incorrecta". El motivo es deliberado: fijar una contraseña desde ahí crearía una
segunda vía de acceso a una cuenta cuya autenticación pertenece a Home Assistant.

Consecuencias prácticas, en orden de sorpresa:

- **Si desactivas `sso` después**, esas cuentas se quedan sin forma de entrar por el login clásico.
- **Una cuenta SSO no puede exportar su `.ffbackup`.** La exportación cifra con una clave derivada
  de la contraseña de la cuenta ([backups.md](backups.md) §Capa 1); sin contraseña no hay clave, y
  el servidor responde el mismo `sso_account_no_password` en vez de generar un fichero que nadie
  podría descifrar. La vía para llevarte datos fuera es **exportar desde una cuenta que sí tenga
  contraseña** (por ejemplo, una creada por el login clásico), o usar la copia de seguridad de Home
  Assistant, que se lo lleva todo.

---

## 3. Opciones del add-on

Se editan en la pestaña **Configuración** del add-on. Cambiarlas requiere **reiniciar** el add-on:
el entrypoint las lee de `/data/options.json` una sola vez, al arrancar, y las traduce a las
variables de entorno de siempre ([configuracion.md](configuracion.md)).

| Opción | Por defecto | Qué hace exactamente | Cuándo tocarla |
|---|---|---|---|
| `log_level` | `info` | Verbosidad del log. `debug` y `trace` fijan `RUST_LOG=futurefin_api=debug,tower_http=debug,sqlx=warn`; `warn` y `error` bajan a `warn`. `info` no toca nada. | Solo para diagnosticar. `debug` genera mucho ruido y `trace` hoy hace lo mismo que `debug`. |
| `sso` | `true` | Activa la identidad delegada: exporta `FUTUREFIN_TRUSTED_PROXY_AUTH=1` y confía en el peer `172.30.32.2` (el ingress del Supervisor, y solo él). | Desactívala si prefieres el login clásico de FutureFin. Ver §2. |
| `mcp` | `true` | Monta `/mcp` y todo el protocolo OAuth embebido. En `false` exporta `FUTUREFIN_MCP_ENABLED=0` y los desmonta (el panel de Conexiones sigue, para poder revocar). | Ponla en `false` si no vas a conectar ningún cliente de IA. **Ojo**: dejarla en `true` no basta para que MCP funcione — ver §4. |
| `cors_origins` | *(vacío)* | Orígenes extra permitidos (`CORS_ORIGINS`), separados por comas. Vacío = los de por defecto. **Una entrada inválida aborta el arranque** a propósito. | Solo si llamas a la API de FutureFin desde otra página web. |
| `public_url` | *(vacío)* | Origen público con el que FutureFin se anuncia como issuer de OAuth (`FUTUREFIN_PUBLIC_URL`). Tiene que ser un origen pelado, sin path ni barra final; **si está y es inválido, no arranca**. | Obligatoria si expones el add-on por un túnel o un proxy con dominio propio. Ver §4. |
| Puerto directo `8080/tcp` | **no publicado** | Publica el puerto del contenedor en la red local. Se configura en la sección **Red** de la misma pestaña: escribe el puerto del host (`8080`, o el que quieras) y reinicia. | Necesario para MCP y OAuth, y **solo** para eso. Ver §4 y el aviso de seguridad. |

### Lo que el add-on decide por ti

Estas no son opciones, son consecuencias del empaquetado, y conviene saberlas:

| | Valor en el add-on | Por qué |
|---|---|---|
| `PGDATA` | `/data/pgdata` | `/data` es el **único** bind persistente que monta el Supervisor. La ruta por defecto de la imagen (`/var/lib/postgresql/data`) caería fuera y se perdería al recrear el contenedor. |
| `FUTUREFIN_STATE_DIR` | `/data/state` | Mismo motivo: ahí van los backups automáticos pre-migración y el estado del entrypoint. |
| Modo de backup | `cold` | El Supervisor **para** el add-on antes de copiar `/data`. Copiar en caliente el directorio de datos de un PostgreSQL vivo no da una copia consistente. |
| Sin *watchdog* | — | El único endpoint que podría vigilar (`/v1/ready`) solo es alcanzable por el puerto directo, que está cerrado por defecto. Un watchdog apuntando ahí reiniciaría el add-on en bucle en la instalación normal. |

---

## 4. MCP y claude.ai: por qué hace falta el puerto directo

**El servidor MCP no funciona a través del ingress.** No es un fallo del empaquetado ni algo que
se arregle con una opción: es una consecuencia del protocolo.

El descubrimiento de OAuth 2.1 que usan los clientes MCP (RFC 8414 y RFC 9728) exige servir
`/.well-known/oauth-authorization-server` y `/.well-known/oauth-protected-resource` **en la raíz
del origen**. Bajo el ingress, la raíz del origen es la de Home Assistant, no la del add-on: el
Supervisor cuelga la aplicación de una ruta larga y efímera (`/api/hassio_ingress/<token>`) y con
su propia sesión. El cliente pediría `https://tu-home-assistant/.well-known/…` y ahí no hay ningún
FutureFin al que preguntar.

La solución es la misma que en cualquier despliegue: **publicar el puerto directo** y apuntar el
cliente ahí. Lo demás (tokens, permisos, roles, el interruptor de escritura) funciona igual que
siempre — ver [mcp.md](mcp.md).

### Receta A — un cliente MCP en tu red local

Para Claude Code o cualquier cliente que corra en la misma red.

1. Publica el puerto: **Configuración → Red → 8080/tcp → `8080`**, y reinicia el add-on.
2. Abre `http://IP-DE-TU-HOME-ASSISTANT:8080` en el navegador. **Ojo**: por ahí no hay SSO (el peer
   ya no es el ingress), así que necesitas una cuenta con contraseña; si tu única cuenta es SSO,
   crea una por el login clásico o usa la receta B con OAuth desde el panel.
3. `Ajustes → Integraciones → Tokens de API (MCP)` → **Crear token**. Cópialo: solo se enseña una
   vez.
4. Endpoint MCP: `http://IP-DE-TU-HOME-ASSISTANT:8080/mcp`, con
   `Authorization: Bearer ffp_…`.

Esto **no** sirve para el conector de claude.ai web: sus peticiones salen de la infraestructura de
Anthropic, no de tu navegador, y necesitan una URL pública con HTTPS.

### Receta B — claude.ai web, con Cloudflare Tunnel

1. Publica el puerto directo (paso 1 de la receta A).
2. Instala el add-on **Cloudflared** y añade a su configuración un `additional_hosts` que apunte al
   puerto que acabas de abrir:

   ```yaml
   additional_hosts:
     - hostname: finanzas.tudominio.com
       service: http://IP-DE-TU-HOME-ASSISTANT:8080
   ```

3. Crea el **CNAME** de `finanzas.tudominio.com` hacia el túnel. Si gestionas el dominio en
   Cloudflare, el propio add-on de Cloudflared lo hace.
4. En las opciones de **FutureFin**, pon:

   ```yaml
   public_url: "https://finanzas.tudominio.com"
   ```

   y reinicia. Es la URL con la que anuncia su issuer de OAuth; sin ella, si el túnel no manda
   `X-Forwarded-Proto`/`Host` como espera, el servidor anunciaría una URL a la que nadie puede
   llegar y claude.ai diría que la conexión falló.
5. En claude.ai: `Configuración → Conectores → Añadir conector personalizado`, y pega
   `https://finanzas.tudominio.com/mcp`. El registro de la aplicación es automático.

> **No pongas Cloudflare Access delante de ese hostname.** Access intercepta las peticiones con su
> propio login y el flujo OAuth del cliente MCP nunca llega a FutureFin. La autenticación ya la
> pone FutureFin (token de API, u OAuth 2.1 con PKCE); duplicarla rompe la conexión.

### Aviso de seguridad del puerto directo

Con el puerto publicado, **FutureFin queda expuesto a tu red local sin la autenticación de Home
Assistant delante**. Sigue exigiendo lo suyo —sesión propia, token, u OAuth— y las cabeceras de
identidad del ingress **nunca** se honran por ahí (el peer no está en la lista de confianza), pero
el filtro de HA ya no está en medio: el registro de FutureFin está abierto y sus endpoints de salud
publican la versión. Si no necesitas MCP, deja el puerto cerrado.

Detalles del modelo de confianza en [SECURITY.md](../SECURITY.md).

---

## 5. Copias de seguridad

En el add-on hay **tres capas**, igual que siempre, con una que cambia de dueño:

| Capa | En el add-on | Qué cubre |
|---|---|---|
| **Copia de Home Assistant** (sustituye al `pg_dump` manual) | La haces desde `Ajustes → Sistema → Copias de seguridad`, incluyendo el add-on | `/data` **entero**: el cluster de PostgreSQL, los backups automáticos y el estado del entrypoint. Es la copia completa. |
| **Backup automático pre-migración** | `/data/state/backups/pre-migration-*.sql.gz` | La base de datos entera, justo antes de cada actualización que traiga migraciones. La escribe el contenedor solo. |
| **`.ffbackup` por usuario** | `Ajustes → Copias de seguridad` dentro de la app | Los datos de una persona, cifrados con su contraseña. Es la capa de **portabilidad**, no de desastre. |

Tres cosas que conviene saber:

- **La copia de HA es en frío.** El add-on declara `backup: cold`, así que el Supervisor lo **para**
  mientras copia `/data` y lo vuelve a arrancar al terminar. Cuenta con **1–2 minutos** de
  indisponibilidad. Es a propósito: copiar en caliente el directorio de datos de un PostgreSQL en
  marcha no da una copia consistente, y una copia que no restaura no es una copia.
- **Los `pre-migration-*` viven dentro de `/data`**, así que la copia de HA se los lleva también.
  Son la red de las actualizaciones, no una copia fuera de casa.
- **Cuál usar para qué**: para volver atrás tras una actualización, la copia de HA. Para llevarte
  tus números a otra instalación (o al revés), el `.ffbackup`. Recuerda que **una cuenta SSO no
  puede exportar `.ffbackup`** (§2).

Todo el detalle de las tres capas, en [backups.md](backups.md).

---

## 6. Migrar desde docker-compose

No hay importador de volúmenes: la ruta soportada es el **`.ffbackup` por usuario**.

1. **En la instalación antigua**, cada persona abre `Ajustes → Copias de seguridad → Copia de
   seguridad personal (.ffbackup)` y pulsa **Exportar mis datos**. Le pedirá su contraseña de
   cuenta, que es también la que cifra el archivo. **Si la olvida, el archivo es irrecuperable.**
2. **En el add-on**, entra con la cuenta que debe quedarse esos datos y usa **Importar backup** en
   la misma pantalla. Antes de aplicar nada verás un **preview** con los recuentos: léelo.
3. **Quien importa se convierte en dueño de lo importado.** La contraseña del backup es la única
   autorización que existe: quien la tiene, puede descifrarlo y quedárselo.

Dos avisos que valen aquí igual que en cualquier importación:

- **La importación reemplaza, no fusiona**: borra todas tus filas actuales y mete las del archivo,
  en una sola transacción.
- La foto de los ajustes de la instalación que viaja dentro del fichero (divisa, zona horaria,
  inflación, supuestos FIRE) es **informativa** y no se aplica: configúralos en el add-on.

Si prefieres mudar la instalación entera en vez de por usuario, la vía es la de siempre: un
`pg_dump` desde el stack de Compose y una restauración, con las reglas de
[backups.md](backups.md#capa-3--pg_dump-manual-con-los-scripts-del-repositorio). No está integrada
con el add-on.

---

## 7. Actualizaciones

El add-on **no se versiona aparte**: su `version:` es siempre el número de la imagen de FutureFin
a la que apunta. Cuando se publica una versión nueva, el mismo workflow que la construye —una vez
la imagen está verificada en el registry y el Release creado— actualiza ese `version:` en `main`,
y la tienda de add-ons lo ve en su siguiente refresco.

- **Puede ir una versión por detrás durante un rato.** Entre que la imagen se publica y que el
  Supervisor refresca el índice del repositorio, la tienda sigue enseñando la anterior. No es un
  error: no hay nada que hacer salvo esperar o pulsar **Buscar actualizaciones**.
- **La actualización automática de Home Assistant está soportada.** Si la activas, el add-on se
  actualizará solo, con la misma red de seguridad de siempre: el contenedor escribe su backup
  pre-migración antes de aplicar ninguna migración, y si ese backup falla **se niega a arrancar**
  en vez de migrar sin red.
- **Volver atrás = restaurar la copia de Home Assistant** que hiciste antes de actualizar. No basta
  con instalar la versión anterior del add-on, y ese es justo el punto siguiente.

### La guarda de downgrade

Las migraciones de FutureFin **solo avanzan**: no existen migraciones de bajada. Si arrancas una
imagen antigua sobre datos que ya migró una imagen posterior, el binario **no arranca** — se para
antes de tocar nada y escribe en el log del add-on:

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

El mensaje da las dos salidas: volver a la versión más nueva (lo normal), o restaurar el backup
pre-migración de `/data/state/backups` si de verdad quieres quedarte en la versión antigua.

Más contexto sobre rollback en [actualizar.md](actualizar.md#volver-a-una-versión-anterior).

---

## 8. Limitaciones

- **Sin enlaces profundos.** Lo que caduca **no es el token del path**: `/api/hassio_ingress/<token>`
  es estable mientras el add-on siga instalado (solo cambia si lo reinstalas o el Supervisor
  regenera el token). Lo que caduca —y lo que hace inútil un enlace guardado— es la **sesión de
  ingress**: la cookie que el Supervisor solo crea y refresca cuando abres el panel desde la barra
  lateral de Home Assistant. Pegar la URL en una pestaña nueva no la crea. La entrada es siempre
  por el icono de la barra lateral; dentro de la app la navegación funciona con normalidad.
- **MCP y OAuth no funcionan por el ingress** (§4). Requieren el puerto directo.
- **Solo amd64 y aarch64.** No hay imagen para armv7 ni i386.
- **Las cuentas SSO no tienen contraseña** en esta versión, y por tanto no exportan `.ffbackup`
  (§2).
- **La interfaz está solo en español**, y la divisa es **una por instalación** (EUR, USD o GBP). No
  hay multidivisa. Esto no es propio del add-on: es así en cualquier despliegue.

---

## 9. Si algo no arranca

1. **Abre la pestaña Log del add-on.** El entrypoint escribe cada hito del arranque: adopción del
   volumen, backup pre-migración, `starting embedded PostgreSQL 16`, `migrations applied`,
   `listening on http://0.0.0.0:8080`. La primera línea que falta es la pista.
2. **Comprueba el marcador `ha_addon=1`.** La línea de arranque tiene esta forma:

   ```
   [futurefin-entrypoint] FutureFin X.Y.Z — mode=serve db_mode=auto ha_addon=1 postgres_majors=15 16
   ```

   Si pone `ha_addon=0`, el contenedor no ha visto `/data/options.json` y está arrancando en modo
   Compose — con `PGDATA` apuntando fuera de `/data`. En un add-on eso no debería pasar; si pasa,
   abre un issue con esa línea.
3. **`FATAL: no persistent volume is mounted`** significa que `/data` no llegó montado. Es la misma
   protección que en Compose: sin volumen, tus datos morirían al recrear el contenedor.
4. **El panel sale en blanco dentro de Home Assistant** → mira el log en busca de errores del
   arranque de la API. Si el add-on está corriendo y el panel sigue vacío, prueba a reiniciarlo:
   la relajación del anti-clickjacking que permite pintar la app dentro del iframe del ingress
   depende de que el add-on reconozca al Supervisor como peer de confianza.
5. **Se queda en el login clásico con `sso: true`** → el canje de identidad falló y la app cayó al
   formulario, que es el comportamiento previsto. Entra por ahí si tienes contraseña y revisa el
   log.

Punteros: [instalacion.md](instalacion.md) (arranque, volúmenes, roles del hogar),
[backups.md](backups.md) (las tres capas y cómo restaurar), [configuracion.md](configuracion.md)
(qué variable de entorno hay detrás de cada opción), [mcp.md](mcp.md) (tokens, permisos y el
interruptor de escritura).
