# Conectar Claude (MCP)

FutureFin lleva un **servidor MCP dentro del mismo binario y el mismo puerto**, en la ruta `/mcp`.
Sirve para que Claude —o cualquier otro cliente MCP— pueda consultar tus finanzas, simular
escenarios y, si tú lo autorizas, apuntar movimientos y mantener tu plan al día.

No hay nada que instalar aparte: si la app está corriendo, `/mcp` está ahí. Y sin credenciales no
hace absolutamente nada: responde 401 a todo.

## Qué puede hacer

A fecha de agosto de 2026 (4.4.0) el catálogo son **52 herramientas**, que se reparten así:

| Grupo | Cuántas | Qué son |
|---|---|---|
| **Lectura** | 20 | Resumen, proyección FIRE, presupuesto, activos, pasivos, reglas de reparto, movimientos, importaciones, categorías, histórico, snapshots, ajustes. |
| **Simulación** | 1 | `simulate_projection`: un *what-if* puro («¿y si gasto 200 € más al mes?»). No persiste nada ni ensucia la cache. |
| **Escritura** | 31 | Crear, editar y borrar activos, pasivos, presupuesto, planificación, movimientos, categorías, reglas y snapshots; conciliar transferencias; cambiar los supuestos FIRE. |

Las herramientas no son una API paralela: llaman **a las mismas funciones internas** que los
endpoints HTTP de la app. Lo que ves por MCP es exactamente lo que ves en la interfaz.

## Dos formas de conectarse

| Credencial | Prefijo | Para quién |
|---|---|---|
| **Conector OAuth** | `ffo_…` (interno, no lo ves) | claude.ai en web, móvil y Desktop |
| **Token de API** | `ffp_…` | Claude Code y cualquier cliente MCP genérico |

Las dos pueden escribir y las dos respetan tu rol. Ninguna congela nada: **cada petición vuelve a
comprobar tu pertenencia al hogar y tu rol**, así que revocar corta el acceso al instante.

---

## claude.ai (web, móvil, Desktop) — conector con OAuth

1. **Expón tu instalación por HTTPS público.** Las conexiones de claude.ai salen de la
   infraestructura de Anthropic, no de tu navegador: `localhost` no sirve. Un túnel de Cloudflare,
   un proxy inverso con TLS, lo que prefieras — FutureFin no gestiona TLS ni conectividad.
2. En claude.ai: `Configuración → Conectores → Añadir conector personalizado`, y pega
   `https://tu-host/mcp`. No hay que rellenar nada más: el registro de la aplicación es automático
   (Dynamic Client Registration).
3. Claude te lleva a la **pantalla de autorización de FutureFin**. Inicia sesión con tu usuario de
   siempre y pulsa **Autorizar**.
4. Ya está. El acceso hereda tu rol.

> **Si usas el add-on de Home Assistant**, tu cuenta probablemente no tiene contraseña: es una
> cuenta de identidad delegada. Con la opción `ha_sso_url` configurada, esa misma pantalla de
> autorización ofrece **«Entrar con Home Assistant»** y autorizas con tu cuenta de HA — sin crear
> una segunda cuenta solo para esto. Cómo activarlo, en
> [home-assistant.md](home-assistant.md#entrar-con-home-assistant-desde-fuera-del-panel).

**Revocar**: `Ajustes → Integraciones → Conexiones` → **Revocar**. El corte es inmediato y Claude
tendrá que volver a pedir permiso.

Si tu proxy no manda `X-Forwarded-Proto` y `Host` correctos, el servidor anunciará una URL a la que
nadie puede llegar y claude.ai dirá que la conexión falló. Se arregla fijando el origen público a
mano:

```env
FUTUREFIN_PUBLIC_URL=https://tu-host
```

Si sirves FutureFin en un subpath (`https://tu-host/futurefin`), pon aquí el mismo prefijo:
`FUTUREFIN_PUBLIC_URL=https://tu-host/futurefin`. Query y fragmento siguen prohibidos, y una barra
final se recorta sola. Si está y es inválido, la app no arranca: mejor un fallo ruidoso que un OAuth
roto en silencio.

---

## Claude Code y clientes MCP genéricos — token de API

1. En la app: `Ajustes → Integraciones → Tokens de API (MCP)` → **Crear token**. Ponle una etiqueta
   y, si quieres, una caducidad. También puedes marcarlo como **solo lectura**: ese token no podrá
   escribir aunque tu rol sí pueda y aunque la instalación tenga la escritura activada — el permiso
   se le queda corto al propio token, no a ti.
2. **Copia el secreto (`ffp_…`) en ese momento: solo se muestra una vez.** El servidor guarda
   únicamente su SHA-256, así que no hay forma de volver a enseñártelo.
3. En Claude Code:

   ```bash
   claude mcp add --transport http futurefin https://tu-host/mcp \
     --header "Authorization: Bearer ffp_..."
   ```

   Claude Code también puede conectarse **sin token**, por el mismo flujo OAuth de arriba:
   `claude mcp add --transport http futurefin https://tu-host/mcp` y autorizar en el navegador.
4. Cualquier otro cliente MCP funciona igual: transporte **Streamable HTTP** y cabecera
   `Authorization: Bearer <token>`, o el flujo OAuth 2.1 estándar del protocolo MCP.

Detalles de los tokens:

- Puedes tener hasta **10 activos** a la vez.
- La caducidad es opcional, entre 1 y 3650 días.
- **Heredan tu identidad y tu rol vivo**: un token no puede hacer nada que tú no puedas hacer ya.
- **Alcance de solo lectura, opcional**: por defecto un token nuevo puede escribir igual que tú;
  márcalo como solo lectura si quieres que ese cliente en concreto nunca pueda, pase lo que pase
  con tu rol o con el interruptor de escritura de la instalación. Los tokens de un visor, en
  cambio, nunca escriben —da igual el alcance— porque el rol se comprueba antes.
- Revocarlos es inmediato, desde la misma pantalla. Los revocados siguen listados, para que quede
  el rastro.
- Cualquier miembro gestiona **los suyos** — incluidos los visores, cuyos tokens solo leen.

---

## MCP en subpath: hace falta declarar el origen público

El descubrimiento de OAuth 2.1 (RFC 8414 y RFC 9728) exige servir
`/.well-known/oauth-authorization-server` y `/.well-known/oauth-protected-resource` **en la raíz
del origen que anuncias**. El servidor sigue montando sus rutas en su propia raíz —eso no cambia—,
así que si cuelgas FutureFin de una ruta (`https://tu-host/futurefin/`) tienes que decírselo, o
anunciará URLs que tu proxy no sabe enrutar y el cliente preguntará a un sitio donde no hay nadie
escuchando.

Si ya sirves FutureFin en subpath ([instalacion.md](instalacion.md#servirla-en-un-subpath-httpstu-hostfuturefin)),
la receta es declarar el mismo prefijo en el origen público:

```env
FUTUREFIN_PUBLIC_URL=https://tu-host/futurefin
```

Con eso el issuer, el `resource` MCP (RFC 8707) y los cuatro endpoints anunciados salen ya con el
prefijo (`https://tu-host/futurefin/mcp`, etc.), y el mismo proxy que recorta `/futurefin` para el
resto de la app los enruta igual de bien.

Un despliegue queda fuera de este arreglo:

- **El add-on de Home Assistant por el ingress del Supervisor**
  (`/api/hassio_ingress/<token>`). Ese prefijo lleva un **token efímero de sesión**: no hay un
  valor fijo que declarar en `FUTUREFIN_PUBLIC_URL` sin hornear ese secreto dentro del issuer de
  OAuth. Ahí la receta sigue siendo **publicar el puerto directo** del add-on y apuntar el cliente
  ahí: pasos completos, con la variante de Cloudflare Tunnel para claude.ai web, en
  [home-assistant.md §4](home-assistant.md#4-mcp-y-claudeai-por-qué-hace-falta-el-puerto-directo).

Todo lo demás —tokens, roles, el interruptor de escritura, el preview de las destructivas— funciona
igual en subpath, por el puerto directo, o por cualquier otro camino.

---

## Permisos: qué puede escribir y qué no

Escribir vía MCP pasa por **tres puertas**, y las tres tienen que estar abiertas:

1. **Tu rol.** Los visores (`viewer`) nunca escriben, ni por MCP ni por la interfaz.
2. **El interruptor de la instalación.** `Ajustes → Integraciones → Servidor MCP → Permitir
   escritura vía MCP`. Solo lo cambia el propietario, se guarda solo, y al apagarlo **las
   herramientas de escritura se cortan al instante** (las de lectura siguen). Con la escritura
   apagada, Claude recibe un error explícito: es un "no", no un fallo — no tiene sentido que
   reintente.
3. **La confirmación, en las destructivas.** Las herramientas que borran o cambian cosas en lote
   solo actúan con `confirm: true`. Sin ese campo devuelven un **preview** de lo que pasaría. Así,
   un borrado nunca ocurre "de paso".

A esas tres se le suma una cuarta si conectas con un **token de API de solo lectura**: ese token no
escribe aunque las tres anteriores estén abiertas — el límite lo pone el propio token. Los
conectores OAuth (`ffo_…`) no tienen este alcance: heredan tal cual tu rol y el interruptor de la
instalación.

Además, cambiar los supuestos FIRE (`update_fire_settings`) es exclusivo del propietario, igual que
en la app.

---

## Un origen de navegador no permitido, 403

Si conectas un cliente MCP que corre en un navegador (por ejemplo, el MCP Inspector) en vez de
Claude Desktop, Claude Code o claude.ai, `/mcp` comprueba desde la 4.4.0 que la cabecera `Origin`
de la petición esté en `CORS_ORIGINS`; si no lo está, responde **403**. Esto **no afecta** a Claude
Desktop, Claude Code ni a `curl`: ninguno manda `Origin`, y una petición sin esa cabecera sigue
pasando. Si te da 403 desde un navegador, añade tu origen a `CORS_ORIGINS`
([configuracion.md](configuracion.md)) y reinicia.

## Apagarlo del todo

```env
FUTUREFIN_MCP_ENABLED=0
```

Las rutas de `/mcp` y de todo el protocolo OAuth **siguen montadas**, pero responden **404** con un
cuerpo JSON (`code: "mcp_disabled"`) a cualquier método. Es deliberado: antes esas rutas
desaparecían del todo, y en la imagen publicada eso se traducía en un `POST /mcp` con **405 y
cuerpo vacío**, y un `GET /.well-known/oauth-authorization-server` que devolvía el **HTML de la
propia app** (el *fallback* de la SPA). El conector de claude.ai no sabía interpretar ninguna de las
dos cosas y mostraba «connection failed» sin más explicación: un interruptor de seguridad que, al
activarse, parecía una avería. Si ves ese mensaje con MCP desactivado a propósito, es esperado —
revisa el JSON de la respuesta, dirá `mcp_disabled`. Si lo ves con MCP **activado**, el problema es
otro (credenciales, origen, HTTPS por delante…).

Con una excepción deliberada: el panel de `Ajustes → Integraciones → Conexiones` sigue montado y
funcionando. Apagar MCP nunca debe quitarte la capacidad de revocar un acceso que ya concediste.

Con MCP activo pero sin ningún token ni conexión, `/mcp` responde 401 a todo: la superficie está
inerte hasta que tú la abres.

## Ver también

- [Configuración](configuracion.md) — `FUTUREFIN_MCP_ENABLED`, `FUTUREFIN_PUBLIC_URL` y el resto
- [Instalación](instalacion.md) — roles del hogar, HTTPS por delante y el modo subpath
- [Home Assistant](home-assistant.md) — el add-on, y por qué MCP necesita ahí el puerto directo
