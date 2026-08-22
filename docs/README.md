# Documentación de FutureFin

Todo lo que no cabe en el [README principal](../README.md): la parte operativa, ordenada por lo que
quieres hacer.

| Documento | Para cuando quieres… |
|---|---|
| **[Instalación](instalacion.md)** | Poner la app en marcha desde cero: Docker Compose o `docker run`, los dos volúmenes, por qué el contenedor se niega a arrancar sin uno, el primer registro y cómo aprobar a más gente. |
| **[Actualizar](actualizar.md)** | Subir de versión y volver atrás con `FUTUREFIN_TAG`, entender el backup automático pre-migración, configurar watchtower, y la ruta completa de la 2.x de dos contenedores a la 3.x. |
| **[Configuración](configuracion.md)** | Saber cómo se llama una opción, qué vale por defecto y quién la lee. La tabla completa de variables de entorno, lo deprecado marcado como tal, y los ajustes que viven dentro de la app. |
| **[Copias de seguridad](backups.md)** | Entender las tres capas —`.ffbackup` por usuario, backup automático pre-migración y `pg_dump` manual—, qué cubre cada una y cómo restaurar. |
| **[Conectar Claude](mcp.md)** | Enchufar el servidor MCP: el conector OAuth de claude.ai, los tokens de API para Claude Code, qué puede leer, qué puede escribir y cómo se apaga. |
| **[Desarrollo](desarrollo.md)** | Levantar el entorno local (`split-dev`), ejecutar las pruebas y construir la imagen Docker sin publicarla. |

## Atajos

**Acabo de instalarlo y no sé por dónde empezar** → [Instalación · El primer
registro](instalacion.md#el-primer-registro-quien-llega-primero-es-el-propietario).

**Quiero que no me salte de versión sola** → [Actualizar · Cuál
elegir](actualizar.md#cuál-elegir).

**¿Qué pasa si se me rompe el servidor?** → [Copias de seguridad](backups.md). Aviso rápido: las
copias automáticas viven en un volumen de la misma máquina.

**Vengo de la 2.x** → [Actualizar · Actualizar desde
2.x](actualizar.md#actualizar-desde-2x-dos-contenedores-a-3x).

**Voy a exponerlo a internet** → [Instalación · Ponerlo detrás de
HTTPS](instalacion.md#ponerlo-detrás-de-https), y `COOKIE_SECURE=true` en
[Configuración](configuracion.md).

**Quiero contribuir código** → [Desarrollo](desarrollo.md).
