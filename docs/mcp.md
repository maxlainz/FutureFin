# Conectar Claude (MCP)

FutureFin lleva un **servidor MCP dentro del mismo binario y el mismo puerto**, en la ruta `/mcp`.
Sirve para que Claude —o cualquier otro cliente MCP— pueda consultar tus finanzas, simular
escenarios y, si tú lo autorizas, apuntar movimientos y mantener tu plan al día.

No hay nada que instalar aparte: si la app está corriendo, `/mcp` está ahí. Y sin credenciales no
hace absolutamente nada: responde 401 a todo.

## Qué puede hacer

A fecha de agosto de 2026 el catálogo son **50 herramientas**, que se reparten así:

| Grupo | Cuántas | Qué son |
|---|---|---|
| **Lectura** | 20 | Resumen, proyección FIRE, presupuesto, activos, pasivos, reglas de reparto, movimientos, importaciones, categorías, histórico, snapshots, ajustes. |
| **Simulación** | 1 | `simulate_projection`: un *what-if* puro («¿y si gasto 200 € más al mes?»). No persiste nada ni ensucia la cache. |
| **Escritura** | 29 | Crear, editar y borrar activos, pasivos, presupuesto, planificación, movimientos, categorías, reglas y snapshots; conciliar transferencias; cambiar los supuestos FIRE. |

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

**Revocar**: `Ajustes → Integraciones → Conexiones` → **Revocar**. El corte es inmediato y Claude
tendrá que volver a pedir permiso.

Si tu proxy no manda `X-Forwarded-Proto` y `Host` correctos, el servidor anunciará una URL a la que
nadie puede llegar y claude.ai dirá que la conexión falló. Se arregla fijando el origen público a
mano:

```env
FUTUREFIN_PUBLIC_URL=https://tu-host
```

Tiene que ser un origen pelado, sin path ni barra final. Si está y es inválido, la app no arranca:
mejor un fallo ruidoso que un OAuth roto en silencio.

---

## Claude Code y clientes MCP genéricos — token de API

1. En la app: `Ajustes → Integraciones → Tokens de API (MCP)` → **Crear token**. Ponle una etiqueta
   y, si quieres, una caducidad.
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
- Revocarlos es inmediato, desde la misma pantalla. Los revocados siguen listados, para que quede
  el rastro.
- Cualquier miembro gestiona **los suyos** — incluidos los visores, cuyos tokens solo leen.

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

Además, cambiar los supuestos FIRE (`update_fire_settings`) es exclusivo del propietario, igual que
en la app.

---

## Apagarlo del todo

```env
FUTUREFIN_MCP_ENABLED=0
```

Desmonta `/mcp` **y todo el protocolo OAuth**. Con una excepción deliberada: el panel de
`Ajustes → Integraciones → Conexiones` sigue montado. Apagar MCP nunca debe quitarte la capacidad
de revocar un acceso que ya concediste.

Con MCP activo pero sin ningún token ni conexión, `/mcp` responde 401 a todo: la superficie está
inerte hasta que tú la abres.

## Ver también

- [Configuración](configuracion.md) — `FUTUREFIN_MCP_ENABLED`, `FUTUREFIN_PUBLIC_URL` y el resto
- [Instalación](instalacion.md) — roles del hogar y cómo poner la app detrás de HTTPS
