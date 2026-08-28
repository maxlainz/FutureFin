# Política de seguridad

## Versiones con soporte

FutureFin lo mantiene una sola persona. Solo la **última versión menor publicada** recibe arreglos
de seguridad; no hay ramas de mantenimiento hacia atrás.

| Versión | Soporte |
|---|---|
| La última menor publicada — la de arriba del todo en [releases](https://github.com/maxlainz/FutureFin/releases) | Sí |
| Cualquier versión anterior | No |

La última versión está en las [releases del repositorio](https://github.com/maxlainz/FutureFin/releases).
Si usas el tag `:latest` de la imagen, ya vas por ahí; si has pineado una versión concreta —lo
recomendable para producción—, revisa el CHANGELOG antes de subir.

## Reportar una vulnerabilidad

**No abras un issue público.** Los issues de este repositorio son visibles para cualquiera, y
publicar un fallo explotable antes de que exista el arreglo deja expuestas las instalaciones de
otras personas.

Usa el canal privado de GitHub: **[Security → Report a vulnerability](https://github.com/maxlainz/FutureFin/security/advisories/new)**.
Crea un *advisory* privado que solo ven quien mantiene el repositorio y quien lo reporta, y donde
se puede discutir y preparar el parche antes de hacerlo público.

Incluye, en la medida de lo posible:

- La versión afectada y cómo está desplegada (Compose, `docker run`, desarrollo).
- Qué consigue quien ataca: leer datos ajenos, escribir, elevar privilegios, tumbar el servicio.
- Los pasos para reproducirlo, con peticiones concretas si aplica.
- Si hace falta autenticación previa y con qué rol.

Y **no pegues datos reales**: ni tuyos ni de nadie. Un *proof of concept* funciona igual con
importes y nombres inventados.

**Qué esperar.** Esto es un proyecto de una persona, sin acuerdo de nivel de servicio. El
compromiso es responder en cuanto se lea el aviso, mantenerte al tanto mientras se trabaja en el
arreglo y darte crédito en el advisory y en el CHANGELOG si lo quieres. La publicación es
coordinada: primero la versión con el arreglo, después el advisory. No hay programa de
recompensas.

## Qué debes saber si la autoalojas

Todo lo de esta sección está verificado en el código, no es una aspiración.

### FutureFin no gestiona TLS

La aplicación sirve **HTTP en texto plano** sobre un `TcpListener` (`0.0.0.0:$PORT`, 8080 por
defecto). El binario no incluye ninguna biblioteca TLS: no termina HTTPS y no lo va a hacer.

**No la expongas a internet tal cual.** Si tiene que salir de tu red, ponla detrás de un proxy
inverso —Caddy, Nginx, Traefik— que termine TLS, y **activa `COOKIE_SECURE=1`** para que la cookie
de sesión no viaje nunca por HTTP. Sin proxy, la cookie de sesión y las contraseñas del formulario
de acceso van en claro por la red.

`COOKIE_SECURE` está **desactivado por defecto** (lo normal es acceder por `http://localhost`).
Solo lo activan los valores exactos `1`, `true`, `TRUE`, `yes` o `YES`; cualquier otra cosa
—incluidos `True` u `on`— se lee como falso.

### Sesiones

La sesión es una cookie **`ff_session`** con `HttpOnly`, `SameSite=Lax`, `Path=/` y sin `Domain`
(solo la sirve el host que la puso). Su `Max-Age` es `SESSION_TTL_DAYS`, 30 días por defecto
(rango admitido 1-400).

El valor es un UUID opaco: no lleva información dentro. La sesión de verdad vive en la tabla
`sessions` y **se revalida contra la base de datos en cada petición**, así que cerrar sesión o
borrar la fila corta el acceso al instante. No hay JWT que siga siendo válido después de
revocarlo.

El **anti-clickjacking es condicional** desde la 4.3.0: por defecto toda respuesta lleva
`X-Frame-Options: DENY`, como siempre. La única excepción es un despliegue tras un **ingress de
confianza** —el add-on de Home Assistant, que pinta la app dentro de un iframe del mismo origen que
HA—, y exige **las dos cosas a la vez**: que la IP del peer esté en `FUTUREFIN_TRUSTED_PROXY_IPS`
**y** que la petición traiga la cabecera `X-Ingress-Path`. Solo entonces el `DENY` se sustituye por
`Content-Security-Policy: frame-ancestors 'self'`, que sigue prohibiendo el embebido
**cross-origin**, que es el vector real del clickjacking. Sin peer de confianza —el valor por
defecto—, mandar esa cabecera a mano no relaja nada: la respuesta lleva `DENY` igual.

El CORS nunca usa comodín: se sirve con una lista explícita de orígenes (`CORS_ORIGINS`, con los de
`localhost` por defecto) y el arranque **aborta** si esa lista queda vacía.

Desde la 4.4.0 esa misma lista gobierna **dos capas con privilegios distintos**: la de `/v1/*` (y
`/oauth/*`, `/.well-known/*`) permite credenciales —la cookie `ff_session`— porque es la que
protege el resto de la app; la de `/mcp` **no** las permite, porque su credencial es la cabecera
`Authorization`, no una cookie. Añadir un origen a `CORS_ORIGINS` para que funcione un cliente MCP
de navegador (el MCP Inspector, por ejemplo) no concede de paso acceso con cookie a `/v1`. `/mcp`
además rechaza con **403** cualquier petición cuya cabecera `Origin` no esté en la lista — pero una
petición **sin** `Origin` (Claude Desktop, Claude Code, `curl`) sigue pasando siempre, así que esto
solo afecta a clientes de navegador.

### Contraseñas

Se guardan como hash **Argon2id** (versión 0x13) con salt aleatorio por usuario, en formato PHC,
con los parámetros recomendados por OWASP: 19 MiB de memoria, 2 iteraciones, paralelismo 1, salida
de 32 bytes. La política de contraseñas es solo de longitud: entre 12 y 256 caracteres.

**Puedes cambiar tu contraseña** con `POST /v1/auth/password` (`{current_password, new_password}`).
Hasta la 4.0.0 no se podía: el hash solo se escribía al registrarse, así que una cookie robada o una
contraseña filtrada en otro servicio daban acceso hasta que caducara la sesión, sin nada que
hacer. El cambio **revoca en la misma transacción** las demás sesiones abiertas, los tokens de API
(`ffp_…`) y las concesiones OAuth de tu usuario; la sesión desde la que lo pides sobrevive. Es el
comportamiento seguro por defecto: si cambias la contraseña porque sospechas un compromiso, dejar
viva una credencial que no caduca haría el cambio decorativo. **Todavía no tiene botón en la
interfaz**: se llama por API.

Verificar la contraseña cuesta Argon2id, así que se hace **fuera del reactor** (`spawn_blocking`) en
los cinco sitios donde se usa. Antes corría en línea y bastaban unas pocas peticiones simultáneas de
registro —endpoint sin autenticación por diseño— para dejar el proceso entero sin responder, sonda
de salud incluida. Y el acceso verifica **siempre** contra un hash de descarte aunque el usuario no
exista, para que un usuario inexistente no responda en 1 ms y uno existente en 80: esa diferencia
enumeraba quién tiene cuenta.

**No hay límite de intentos en el acceso**: ni contador, ni retardo progresivo, ni captcha. Sigue
siendo cierto en la 4.0.0 — no hay ningún middleware de rate limiting en el binario. El único freno
frente a la fuerza bruta es el coste del propio Argon2id. Si la instalación es accesible desde fuera
de tu red, pon el límite en el proxy inverso.

### Copias de seguridad `.ffbackup`

El fichero se cifra en el servidor antes de descargarse: la clave de 256 bits se deriva con
**Argon2id** (19 MiB / 2 iteraciones / paralelismo 1, con un salt de 16 bytes distinto en cada
exportación) y el contenido, ya comprimido, se cifra con **AES-256-GCM** con un nonce aleatorio de
12 bytes. El manifiesto va autenticado como AAD, así que manipular el fichero se detecta al
descifrar.

**La contraseña es la de tu cuenta.** El servidor la verifica contra tu hash y deriva de ella la
clave. Consecuencia práctica: un `.ffbackup` queda atado a la contraseña que tenías **cuando lo
generaste**. Si la cambias después, los ficheros antiguos siguen necesitando la anterior — no se
recifran solos. Guárdala si guardas backups viejos. Desde la 4.0.0 esto **importa de verdad**,
porque ya se puede cambiar la contraseña: si la rotas por sospecha de compromiso, tu copia antigua
se sigue abriendo con la contraseña filtrada. Si eso te preocupa, exporta una copia nueva después
del cambio y destruye la vieja.

**Al importar, un `.ffbackup` es un fichero que trae quien sea.** El manifiesto viaja en claro y
fuera de la parte autenticada, así que sus parámetros de derivación de clave los elige quien
fabrica el fichero. Desde la 4.0.0 están **acotados** (memoria ≤ 256 MiB, iteraciones ≤ 10,
paralelismo ≤ 4) y el texto en claro descomprimido tiene un techo de 128 MiB. Antes no: un fichero
de 200 bytes pidiendo 8 GB de memoria se llevaba por delante el contenedor entero —con el
PostgreSQL embebido dentro— **desde el endpoint de vista previa**, que ni siquiera escribe nada. El
cifrado no defiende de esto: quien ataca cifra con su propia contraseña, así que su bomba pasa la
autenticación intacta.

Los respaldos automáticos previos a cada migración que escribe el contenedor en el volumen
`ffdata` son otra cosa: son volcados de `pg_dump` **sin cifrar**. Protege ese volumen como
protegerías la base de datos.

### El servidor MCP y OAuth se pueden apagar

`FUTUREFIN_MCP_ENABLED` controla el endpoint `/mcp` y **todo** el protocolo OAuth 2.1 embebido.
Está **activado por defecto** (variable sin definir = activo). Lo activan explícitamente `1`,
`true`, `TRUE`, `yes` o `YES`; **cualquier otro valor lo desactiva**, así que `FUTUREFIN_MCP_ENABLED=0`
hace lo que esperas.

Desactivado, las rutas de `/mcp`, de los `.well-known` de metadata y de `/oauth/register`,
`/oauth/token` y `/oauth/revoke` **siguen montadas**, pero responden **404** con un cuerpo JSON
(`code: "mcp_disabled"`) a cualquier método, en vez de desaparecer del router: desmontarlas del
todo se comportaba distinto en la imagen publicada (un `POST /mcp` sin ruta caía en un `405` con
cuerpo vacío, y un `GET` a un `.well-known` en el HTML de la propia SPA), y un cliente como el
conector de claude.ai no sabía interpretar ninguna de las dos cosas — mostraba «conexión fallida»
sin explicar que el servidor la tenía apagada a propósito. Los dos endpoints de la pantalla de
consentimiento (`/v1/oauth/authorize*`) no dependen de este interruptor. Se mantienen a propósito
el panel de conexiones y el CRUD de tokens de API: apagar la integración nunca debe quitarte la
capacidad de **revocar** credenciales que ya concediste.

Sobre las credenciales: tanto los tokens de API (`ffp_…`) como las de OAuth (`ffc_`, `ffcs_`,
`ffo_`, `ffr_`) son **32 bytes de aleatoriedad del sistema** y se guardan **solo como SHA-256** —
el secreto en claro se enseña una vez y no se puede recuperar. Un hash rápido es lo correcto aquí
precisamente porque el secreto no es una contraseña humana, sino 256 bits aleatorios. Ninguna
credencial congela nada: el rol y la pertenencia a la instalación se resuelven de nuevo en cada
petición, y la escritura por MCP exige además el interruptor de la propia instalación.

**Un token de API puede limitarse a solo lectura.** Cada token lleva un `scope`
(`read_write` por defecto, o `read_only`), elegible al crearlo. Un token `read_only` no escribe
nunca, aunque tu rol pueda y aunque la instalación tenga la escritura activada: es una restricción
que le pones al propio token, independiente de tu cuenta. Los conectores OAuth (`ffo_…`) no
negocian `scope` — siempre pueden escribir, con el mismo techo que el rol vivo y el interruptor.

**Cada intento de escritura por MCP queda registrado.** La tabla `mcp_write_audit` guarda, por
llamada, quién la hizo, con qué credencial, con qué rol, qué herramienta, el desenlace
(`denied`/`ok`/`failed`) y qué filas mutó — nunca los argumentos de la llamada. Retención de
**365 días**, podada de forma perezosa en la propia escritura. No está expuesta por ninguna ruta
HTTP ni por ninguna herramienta MCP, y no entra en el `.ffbackup`: es un rastro operativo, no un
dato del hogar.

**Las herramientas más destructivas piden un segundo secreto.** Además del `confirm: true`
habitual, las siete herramientas de mayor radio (borrar un activo, un pasivo, un snapshot o un
lote de importación; aplicar una regla de categorización en bloque; hacer converger los movimientos
recurrentes; desconciliar una transferencia) exigen un `confirm_token` de un solo uso que solo se
emite dentro de su propio *preview* y caduca a los 10 minutos. Resuelve un hueco real: el booleano
`confirm: true` lo escribe el propio modelo, así que por sí solo nunca demuestra que hubo un
*preview* de por medio.

El registro dinámico de clientes OAuth (`POST /oauth/register`) **sigue abierto sin
autenticación** en la 4.0.0, como exige el RFC 7591. Registrar un cliente no concede acceso a nada
por sí solo: la puerta sigue siendo tu inicio de sesión y la pantalla de consentimiento. Contra el
flood hay recolección perezosa —en el propio POST— de los clientes de más de 24 horas que no tienen
ninguna autorización concedida, y un tope duro de 1.000 clientes registrados, pasado el cual el
registro responde «vuelve más tarde» en vez de crecer sin fin.

### `?view=mine` no es una frontera de autorización

FutureFin es una aplicación de **hogar compartido**: todos los datos financieros pertenecen a la
instalación, y **cualquier miembro aprobado puede verlos todos**. El parámetro `?view=mine` filtra
la vista a las filas atribuidas a tu usuario, pero es una comodidad de presentación: basta con
omitirlo para recibir el agregado del hogar completo, y ningún endpoint de lectura consulta el rol.

Las únicas fronteras reales son:

| Frontera | Qué separa |
|---|---|
| Pertenencia a la instalación | Quien no está aprobado recibe 403 en todo endpoint de datos |
| Rol (`owner`, `member`, `viewer`) | `viewer` solo lee; `owner` además administra usuarios y ajustes |

**Y desde la 4.0.0 la pertenencia se puede retirar.** Quien es propietario puede degradar el rol de
un miembro o revocarle el acceso (`PATCH` / `DELETE /v1/installation/members/{user_id}`); antes esta
página prometía que la membresía era el corte real y no había ninguna forma de accionarlo salvo
entrar a la base de datos a mano. El corte es **inmediato y completo**: en la misma transacción se
borra la membresía, se cierran sus sesiones, se revocan sus tokens de API y se revocan sus
concesiones OAuth. Ninguna credencial congela el rol, así que la siguiente petición ya llega sin
acceso. **Los datos de esa persona no se borran**: siguen atribuidos a su usuario y los recupera
intactos si se la vuelve a aprobar. Hay una guardia contra dejar la instalación sin ningún
propietario. Todavía no tiene interfaz: se hace por API.

**No uses FutureFin esperando privacidad entre miembros del mismo hogar.** Si dos personas no
deben verse las cuentas, necesitan dos instalaciones.

### Registro y primer usuario

El registro está **abierto**: cualquiera que llegue al puerto puede crearse una cuenta. No hay
invitaciones ni interruptor para cerrarlo.

Eso no le da acceso a los datos —quien no tiene membresía recibe 403 en todo endpoint de datos, y
solo quien es propietario aprueba pendientes desde `Ajustes → Usuarios`—, pero sí puede crear
cuentas y comprobar si la instalación ya está inicializada. Es otra razón para no dejarla expuesta
a internet sin control de acceso delante.

**La primera persona que se registra se convierte en propietaria** de la instalación, en la misma
transacción del registro. Regístrate tú antes que nadie.

### Confianza en cabeceras de proxy

Desde la 4.3.0 FutureFin puede aceptar que un proxy delantero le diga **quién** es quien llama
(`POST /v1/auth/sso`, cabecera `X-Remote-User-Id`), y convertir eso en una sesión normal suya. Una
cabecera de identidad es una afirmación sin prueba, así que la puerta es **doble y opt-in**:

1. **`FUTUREFIN_TRUSTED_PROXY_AUTH=1`** — sin ella el endpoint responde `401 sso_disabled` aunque
   la cabecera venga. La ruta se monta siempre; lo que decide es el estado, no la forma del router.
2. **`FUTUREFIN_TRUSTED_PROXY_IPS`** — la IP del peer TCP tiene que estar en esa lista (o la lista
   ser `any`). Un peer que no lo esté recibe `401 sso_untrusted_peer`. Sin la lista definida,
   **nadie** es de confianza.

Las dos son necesarias: activar la primera sin la segunda **aborta el arranque** en vez de arrancar
aceptando identidad de cualquiera. Y `FUTUREFIN_TRUSTED_PROXY_IPS=any` **se rechaza** cuando
`FUTUREFIN_TRUSTED_PROXY_AUTH=1`: el comodín existe para el prefijo, que es inocuo, pero combinarlo
con el canje de identidad significaría «cualquiera que alcance el puerto puede decir quién es», que
es exactamente el fallo que estas dos variables existen para impedir. El arranque falla con un
mensaje explícito en vez de quedarse abierto. En el add-on de Home Assistant, el entrypoint pone la
lista al único peer que alcanza al contenedor por el ingress (`172.30.32.2`).

**Co-tenancy del add-on: los add-ons de Home Assistant no están aislados entre sí.** Por el ingress
todos cuelgan del **mismo origen** (el de Home Assistant) y se distinguen solo por el path. El
aislamiento de origen del navegador —el que impide que una web lea otra— **no existe entre dos
paneles de ingress**: un add-on malicioso o comprometido puede, desde su propio panel, abrir el de
FutureFin y hablarle con las credenciales de la persona que ha iniciado sesión. FutureFin acota lo
que puede acotar: la cookie `ff_session` se emite con `Path` restringido a su propio prefijo de
ingress (solo cuando el peer es de confianza — si no lo es, no se acota nada, porque el prefijo
vendría de una cabecera sin verificar) y fuera del ingress de confianza se mantiene
`X-Frame-Options: DENY`. Pero eso son mitigaciones, no una frontera. **La frontera real es
operativa: instala solo add-ons en los que confíes.** Si eso no te vale para tus datos financieros,
la respuesta es un despliegue por Compose separado, no una opción del add-on.

Consecuencia práctica: **si publicas el puerto directo del add-on, ese puerto nunca honra
`X-Remote-User-*`** — quien llega por ahí tiene otra IP de origen y no pasa el filtro. Por ese
camino hace falta sesión propia, token de API u OAuth, como en cualquier otro despliegue.

Lo que **no** exige peer de confianza es la detección del **prefijo** (`X-Forwarded-Prefix`,
`X-Ingress-Path`, `FUTUREFIN_BASE_PATH`). Es deliberado: un prefijo falsificado solo deforma la
respuesta del propio atacante (assets que no cargan). Lo que sí lo exige es relajar el
anti-clickjacking y aceptar identidad, que es lo descrito arriba.

**Cuentas sin contraseña, y un trade-off asumido.** Un usuario creado por esta vía tiene el hash de
contraseña a `NULL`: no puede entrar por el formulario de acceso, no puede fijarse una contraseña
en esta versión, y **no puede exportar su `.ffbackup`** (la clave del archivo se deriva de la
contraseña; sin ella no hay nada de donde derivarla). Los tres casos responden un
`401 sso_account_no_password` **hablado**. Eso revela que ese nombre existe como cuenta SSO, y es
un intercambio buscado: sin el mensaje, la persona se quedaría tecleando para siempre una
contraseña que nunca se fijó. Es la misma postura que el `username_taken` del registro, que ya
distingue "ese nombre está cogido" de cualquier otro error.

### Home Assistant como proveedor de identidad

Desde la 4.3.1 FutureFin puede además **entrar como cliente** en el Home Assistant de quien lo
aloja: `GET /v1/auth/ha/start` y `GET /v1/auth/ha/callback` implementan el flujo de código de
autorización de HA («Entrar con Home Assistant»), y la identidad que devuelve se canjea por una
sesión normal. Es la contrapartida del apartado anterior: allí un proxy **afirma** quién eres; aquí
la prueba es un round-trip por tu propio navegador contra HA.

**Solo existe en modo add-on.** Se activa con `FUTUREFIN_HA_SSO_URL` (el origen público de tu HA) y
**solo se honra con `FUTUREFIN_HA_ADDON=1`**, que exporta únicamente el entrypoint cuando detecta el
Supervisor. La URL sin el flag **aborta el arranque**: es una decisión de diseño, no un descuido —
no queríamos un «login con cualquier IdP» de propósito general entrando por la puerta de atrás. Las
dos rutas se montan siempre; sin URL configurada responden `ha_sso_disabled`.

**El modelo de defensa, y por qué no hay PKCE.** Home Assistant no soporta PKCE, ni `client_secret`,
ni `scope`. Lo que sí exige es que el `client_id` y el `redirect_uri` sean del **mismo origen**, y
FutureFin manda como `client_id` su propio origen público, byte a byte el mismo en la autorización
y en el canje: un código emitido para esta instalación no sirve para redirigirlo a otra parte. La
segunda pata es una cookie de estado propia, **`ff_ha_state`**: `HttpOnly`, `SameSite=Lax`,
`Max-Age` de 10 minutos, `Secure` si `COOKIE_SECURE` lo está, con el `Path` acotado al prefijo del
propio despliegue y **de un solo uso** — se borra en el callback pase lo que pase, así que un
`state` no se puede reproducir. El `state` devuelto por HA se compara con el de la cookie en
**tiempo constante**; sin coincidencia, no se llama a HA para nada.

`SameSite=Lax` no es una relajación: es el único valor que funciona. El callback llega como
navegación de nivel superior desde el dominio de Home Assistant, y `Strict` no manda la cookie en
una navegación cross-site — el flujo fallaría **siempre**. `None` exigiría `Secure`, que no se puede
dar por hecho en una LAN por HTTP.

**La ruta de retorno viaja dentro de la cookie, no en el `state`.** Es deliberado: así el destino
final es un valor que el servidor puso, no algo que el navegador trajo de vuelta. Aun así se sanea
antes de guardarlo y el destino se acota a esta misma aplicación — tiene que empezar por `/`, no
puede ser `//host` ni contener `\`, ni `://` o `@` en la parte de ruta, ni caracteres de control (un
`\r\n` partiría la cabecera `Location`), y como mucho 512 caracteres; cualquier duda cae a la raíz.
Esa disciplina es una obligación, no un extra: **este origen no puede tener ni un open-redirect**.
Un redirect abierto en FutureFin permitiría fabricar un `redirect_uri` legítimo para HA que acabe
entregando el código en otro sitio.

**Qué queda guardado de tu Home Assistant: nada.** El token de refresco que HA emite se **revoca
inmediatamente** después de leer la identidad, antes incluso de tocar la base de datos, y el de
acceso muere con la petición. Ningún token de HA se escribe en el log (solo longitudes, y a nivel
`debug`). Si la revocación falla —HA caído en ese instante—, el login sigue adelante y queda un
**aviso en el log**: si lo ves, borra ese token a mano en **Home Assistant → Perfil → Seguridad →
Tokens de actualización**.

**Lo que heredas de tu Home Assistant, y lo que no.** FutureFin acepta a quien HA diga que ha
entrado, **con cualquier proveedor de autenticación**: si tu HA usa `trusted_networks`, quien esté
en esa red es ese usuario en HA y, por tanto, también aquí — sin teclear nada. HA es la fuente de
**identidad**; el rol y la pertenencia al hogar los sigue decidiendo FutureFin, con las mismas
reglas que el resto de las altas: la primera identidad de la instalación se queda la propiedad y
las siguientes entran **pendientes** hasta que el propietario las apruebe. La otra cara: quien autentica es HA,
así que los intentos fallidos que pasan por este botón cuentan para el `ip_ban` de Home Assistant.
FutureFin sigue sin tener límite de intentos propio.

Un certificado autofirmado en esa URL **no está soportado**: el cliente verifica el certificado y no
existe opción para desactivar la verificación. Usa `http://` en la LAN o un certificado válido.

### Endpoints sin autenticación

`GET /health`, `GET /v1/health`, `GET /v1/ready` y `GET /openapi.json` responden sin credenciales.
Los de salud **publican la versión** de la aplicación: es lo primero que verá un escáner. Están así
a propósito, porque el healthcheck del contenedor y los orquestadores los necesitan.

### La base de datos embebida

En producción PostgreSQL corre dentro del contenedor y escucha **solo en un socket Unix**: no
publica ningún puerto TCP, ni dentro ni fuera. Por eso la autenticación local es `trust` — quien ya
esté dentro de ese contenedor tiene el sistema de ficheros de la base de datos de todas formas.

Existe una variable de escape, `FUTUREFIN_PG_LISTEN`, documentada solo para depuración, que **abre
el listener TCP**. No la uses en producción: con ella puesta, ese `trust` sí deja de ser inocuo.

## Fuera de alcance

Lo siguiente es comportamiento conocido y documentado, no una vulnerabilidad. Reportarlo por el
canal privado no hace daño, pero la respuesta será un enlace a esta página:

- Que la aplicación no sirva HTTPS por sí misma.
- Que un miembro del hogar vea los datos de otro miembro, o que `?view=mine` se pueda omitir.
- Que el registro esté abierto, o que los endpoints de salud publiquen la versión.
- Que el `401 sso_account_no_password` revele que una cuenta existe y es de tipo SSO (está descrito
  arriba, y es coherente con el `username_taken` del registro).
- Que no haya límite de intentos en el acceso (está descrito arriba; ponlo en tu proxy).
- Hallazgos de un escáner automático sin un impacto demostrado.
- Ataques que requieren acceso previo al host, al contenedor o al volumen de datos.
- Proyecciones que no cuadran con tus expectativas: eso es un
  [issue de fallo](https://github.com/maxlainz/FutureFin/issues), no un problema de seguridad.
