# Documentación de FutureFin

Todo lo que no cabe en el [README principal](../README.md): la parte operativa, ordenada por lo que
quieres hacer.

| Documento | Para cuando quieres… |
|---|---|
| **[Instalación](instalacion.md)** | Poner la app en marcha desde cero: Docker Compose o `docker run`, los dos volúmenes, por qué el contenedor se niega a arrancar sin uno, el primer registro y cómo aprobar a más gente. |
| **[Home Assistant](home-assistant.md)** | Instalarla como add-on: panel en la barra lateral, entrar con tu usuario de Home Assistant, las opciones del add-on, por qué MCP necesita el puerto directo y cómo encajan las copias de seguridad. |
| **[Actualizar](actualizar.md)** | Subir de versión y volver atrás con `FUTUREFIN_TAG`, entender el backup automático pre-migración, configurar watchtower, y la ruta completa de la 2.x de dos contenedores a la 3.x. |
| **[Tu plan de jubilación](jubilacion.md)** | Elegir tu estrategia (cuanto antes, a una edad fija, Coast FIRE, media jornada, puente hasta la pensión), declarar tu pensión con su fecha, elegir cómo retiras el dinero, y leer la sección «Riesgo»: qué son las bandas, qué significa «éxito» y dónde se declara la volatilidad. Incluye el control «Yo \| Hogar». |
| **[Configuración](configuracion.md)** | Saber cómo se llama una opción, qué vale por defecto y quién la lee. La tabla completa de variables de entorno, lo deprecado marcado como tal, y los ajustes que viven dentro de la app. |
| **[Copias de seguridad](backups.md)** | Entender las tres capas —`.ffbackup` por usuario, backup automático pre-migración y `pg_dump` manual—, qué cubre cada una y cómo restaurar. |
| **[Conectar Claude](mcp.md)** | Enchufar el servidor MCP: el conector OAuth de claude.ai, los tokens de API para Claude Code, qué puede leer, qué puede escribir y cómo se apaga. |
| **[Desarrollo](desarrollo.md)** | Levantar el entorno local (`split-dev`), ejecutar las pruebas y construir la imagen Docker sin publicarla. |

## Atajos

**Acabo de instalarlo y no sé por dónde empezar** → [Instalación · El primer
registro](instalacion.md#el-primer-registro-quien-llega-primero-es-el-propietario).

**Uso Home Assistant y no quiero pelearme con Docker** → [Home
Assistant](home-assistant.md#1-instalación): se instala como add-on y sale en la barra lateral.

**Quiero que no me salte de versión sola** → [Actualizar · Cuál
elegir](actualizar.md#cuál-elegir).

**¿Qué pasa si se me rompe el servidor?** → [Copias de seguridad](backups.md). Aviso rápido: las
copias automáticas viven en un volumen de la misma máquina.

**Vengo de la 2.x** → [Actualizar · Actualizar desde
2.x](actualizar.md#vengo-de-2x-o-tengo-una-base-de-datos-externa).

**Voy a exponerlo a internet** → [Instalación · Ponerlo detrás de
HTTPS](instalacion.md#ponerlo-detrás-de-https), y `COOKIE_SECURE=true` en
[Configuración](configuracion.md).

**¿Cuándo podría dejar de trabajar?** → [Tu plan de jubilación](jubilacion.md): cinco estrategias,
y la sección «Riesgo» para ver qué pasa si los mercados no se portan como la media.

**Acabo de actualizar y mis cifras del hogar han cambiado** → [Actualizar · Actualizar a la
5.0.0](actualizar.md#actualizar-a-la-500): en un hogar de dos o más personas eso es esperado, y ahí
se explica por qué.

**Quiero contribuir código** → [Desarrollo](desarrollo.md).
