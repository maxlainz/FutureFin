# FutureFin

Finanzas del hogar y planificación FIRE, autoalojadas. La misma imagen que se
publica para Docker Compose, empaquetada como add-on: PostgreSQL va dentro del
contenedor y todos los datos viven en `/data`, que Home Assistant respalda.

## Instalación

1. Añade este repositorio a la tienda de add-ons:

   [![Añadir repositorio a Home Assistant](https://my.home-assistant.io/badges/supervisor_add_addon_repository.svg)](https://my.home-assistant.io/redirect/supervisor_add_addon_repository/?repository_url=https%3A%2F%2Fgithub.com%2Fmaxlainz%2FFutureFin)

   O a mano: **Ajustes → Add-ons → Tienda de add-ons → ⋮ → Repositorios** y pega
   `https://github.com/maxlainz/FutureFin`.

2. Instala **FutureFin** y pulsa **Iniciar**. El primer arranque crea la base de
   datos; puede tardar un minuto.

3. Abre **FutureFin** en la barra lateral.

## Primer arranque y usuarios

- La primera persona de Home Assistant que abre el panel se convierte en
  **propietaria** de la instalación.
- El resto entra como **pendiente**: ve la aplicación pero no ve datos hasta que
  el propietario la aprueba en **Ajustes → Usuarios**.
- Con la opción `sso` desactivada no hay identidad de Home Assistant: FutureFin
  muestra su login clásico (correo y contraseña) y el primer registro es el
  propietario.
- Las cuentas creadas por SSO **no tienen contraseña** en esta versión. Si
  desactivas `sso` después, esas cuentas se quedan sin forma de entrar por el
  login clásico.

## Opciones

| Opción | Por defecto | Qué hace | Cuándo tocarla |
|---|---|---|---|
| `log_level` | `info` | Verbosidad del log del add-on. | `debug` o `trace` solo para diagnosticar; generan mucho ruido. |
| `sso` | `true` | Usa la identidad de Home Assistant para entrar sin contraseña a través del ingress. | Desactívala si prefieres el login clásico de FutureFin. |
| `mcp` | `true` | Monta el servidor MCP y el OAuth embebido (`/mcp`, `/.well-known/*`). | Desactívalo si no vas a conectar ningún cliente MCP. |
| `cors_origins` | *(vacío)* | Orígenes extra permitidos, separados por comas. | Solo si consumes la API desde otra web. |
| `public_url` | *(vacío)* | URL pública con la que FutureFin se anuncia (issuer de OAuth). | Obligatoria si expones el add-on por un túnel o proxy con dominio propio. |
| Puerto directo `8080/tcp` | deshabilitado | Publica el puerto del contenedor en la red local. | Necesario para MCP y OAuth. Ver abajo. |

El puerto directo se activa en la pestaña **Configuración** del add-on, sección
**Red**: escribe `8080` (o el puerto del host que prefieras) y reinicia.

## MCP y claude.ai

El servidor MCP **no funciona a través del ingress**. El descubrimiento de OAuth
2.1 exige servir `/.well-known/oauth-authorization-server` y
`/.well-known/oauth-protected-resource` en la **raíz del origen**, y esa raíz es
de Home Assistant, no del add-on: el ingress cuelga la aplicación de una ruta
larga y con sesión propia. No hay forma de arreglarlo desde este lado.

La receta es publicar el puerto directo y apuntar el cliente ahí.

### En la red local

1. Activa el puerto `8080/tcp` (sección **Red** del add-on) y reinicia.
2. Entra en `http://IP-DEL-HOST:8080` y crea un token en
   **Ajustes → Integraciones**, o conecta por OAuth desde el cliente.
3. Endpoint MCP: `http://IP-DEL-HOST:8080/mcp`.

Sirve para clientes que corren en tu red. El conector de claude.ai web necesita
una URL pública con HTTPS.

### Con Cloudflare Tunnel

1. Instala el add-on **Cloudflared** y, en su configuración, añade un
   `additional_hosts`:

   ```yaml
   additional_hosts:
     - hostname: finanzas.tudominio.com
       service: http://IP-DEL-HOST:8080
   ```

2. Crea el CNAME correspondiente en Cloudflare (el propio add-on de Cloudflared
   lo hace si gestionas el dominio ahí).
3. Pon `public_url: "https://finanzas.tudominio.com"` en las opciones de
   FutureFin y reinicia: es la URL con la que anuncia su issuer de OAuth.
4. Endpoint para el conector: `https://finanzas.tudominio.com/mcp`.

> **Aviso**: ese hostname **no puede estar detrás de Cloudflare Access**. Access
> intercepta las peticiones con su propio login y el flujo OAuth del cliente MCP
> nunca llega a FutureFin. La autenticación la pone FutureFin (token o OAuth con
> PKCE); duplicarla con Access rompe la conexión.

## Copias de seguridad

- Las copias de Home Assistant cubren `/data` **entero**, PostgreSQL incluido.
  Como el add-on declara `backup: cold`, el Supervisor lo **para** mientras copia:
  cuenta con 1–2 minutos de indisponibilidad.
- Antes de cada migración de esquema, el propio contenedor escribe un volcado
  automático en `/data/state/backups`. Es una red de seguridad interna, no
  sustituye a la copia de Home Assistant.
- La exportación cifrada **`.ffbackup`** (por usuario, desde Ajustes) sigue siendo
  la vía de portabilidad y migración entre instalaciones.

## Migrar desde docker-compose

1. En la instalación antigua, cada usuario exporta su `.ffbackup` desde
   **Ajustes → Copia de seguridad** (elige una contraseña y no la pierdas).
2. En el add-on, entra con la cuenta que debe quedarse los datos e **importa** el
   fichero.
3. Quien importa se convierte en **propietario de los datos importados**. La
   contraseña del backup es la única autorización: quien la tiene, puede
   descifrarlo.

## Actualizaciones

- Cuando se publica una versión nueva de la imagen, la versión del add-on se
  actualiza sola poco después. Puede ir **una versión por detrás** durante unos
  minutos; no es un error.
- La actualización automática de Home Assistant está soportada.
- Para volver atrás: **restaura la copia de Home Assistant que hiciste antes de
  actualizar**. Instalar una imagen anterior sobre datos ya migrados no funciona
  — el arranque se niega a seguir (guardia de downgrade) en lugar de corromper la
  base de datos.

## Limitaciones

- **Sin enlaces profundos**: la ruta `/api/hassio_ingress/<token>` es estable
  mientras el add-on siga instalado, pero la **sesión** de ingress solo la crea
  Home Assistant al abrir el panel desde la barra lateral. Por eso un enlace
  guardado a una vista concreta no te llevará ahí: la entrada es siempre el
  icono de la barra lateral.
- Solo **amd64** y **aarch64**. No hay imagen para armv7 ni i386.

## Si algo no arranca

1. Abre la pestaña **Log** del add-on: el entrypoint escribe cada hito del
   arranque (adopción del volumen, migraciones, PostgreSQL listo, API escuchando).
2. En modo add-on el log lleva el marcador `ha_addon=1`. Si no aparece, el
   contenedor no ha detectado `/data/options.json` y está arrancando en modo
   Compose.
3. Documentación completa del proyecto:
   [docs/](https://github.com/maxlainz/FutureFin/tree/main/docs) — instalación,
   configuración, backups, actualización y MCP.
