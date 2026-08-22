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

Hay además una cabecera `X-Frame-Options: DENY` global, y el CORS nunca usa comodín: se sirve con
una lista explícita de orígenes (`CORS_ORIGINS`, con los de `localhost` por defecto) y el arranque
**aborta** si esa lista queda vacía.

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

Desactivado no se monta ni `/mcp`, ni los `.well-known` de metadata, ni `/oauth/register`,
`/oauth/token` o `/oauth/revoke`, ni los dos endpoints de la pantalla de consentimiento. Se
mantienen a propósito el panel de conexiones y el CRUD de tokens de API: apagar la integración
nunca debe quitarte la capacidad de **revocar** credenciales que ya concediste.

Sobre las credenciales: tanto los tokens de API (`ffp_…`) como las de OAuth (`ffc_`, `ffcs_`,
`ffo_`, `ffr_`) son **32 bytes de aleatoriedad del sistema** y se guardan **solo como SHA-256** —
el secreto en claro se enseña una vez y no se puede recuperar. Un hash rápido es lo correcto aquí
precisamente porque el secreto no es una contraseña humana, sino 256 bits aleatorios. Ninguna
credencial congela nada: el rol y la pertenencia a la instalación se resuelven de nuevo en cada
petición, y la escritura por MCP exige además el interruptor de la propia instalación.

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
- Que no haya límite de intentos en el acceso (está descrito arriba; ponlo en tu proxy).
- Hallazgos de un escáner automático sin un impacto demostrado.
- Ataques que requieren acceso previo al host, al contenedor o al volumen de datos.
- Proyecciones que no cuadran con tus expectativas: eso es un
  [issue de fallo](https://github.com/maxlainz/FutureFin/issues), no un problema de seguridad.
