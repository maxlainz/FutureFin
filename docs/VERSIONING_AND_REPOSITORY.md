# Repositorio principal y versionado

## Repositorio

- El **código base del producto FutureFin self-hosted** vive en un **repositorio nuevo** (nombre sugerido: `futurefin-server`, `futurefin-web`, o monorepo `futurefin` con paquetes `api/` y `web/`).
- El repo Swift del cliente macOS (**FutureFin / FinFuture**) queda como **referencia de oráculo** y puede archivarse o congelarse; no es el destino de nuevas features.

## Versionado semántico

- **`MAJOR.MINOR.PATCH`** para releases etiquetadas en git (`v1.0.0`).
- **MAJOR:** cambios incompatibles en API pública HTTP o en **schemaVersion** / **formatVersion** de backup que rompan restore sin migración documentada.
- **MINOR:** nuevas capacidades compatibles (nuevos campos opcionales, endpoints adicionales).
- **PATCH:** correcciones compatibles.

## Versiones embebidas

| Artefacto | Campo | Propósito |
|-----------|-------|-----------|
| Backup monofichero | `formatVersion` | Evolución del contenedor cifrado |
| Backup monofichero | `schemaVersion` | Evolución del snapshot JSON interno |
| API HTTP | header opcional `X-API-Version` o prefijo `/v1` | Contrato cliente-servidor |

## Política respecto al cliente macOS

- **Sin** releases coordinadas Mac ↔ servidor.
- Issue tracker del Mac: solo **críticos de seguridad** o cierre formal; sin roadmap de features.

## Changelog

- Mantener `CHANGELOG.md` en el repo nuevo con secciones por release y notas de migración de backup/schema cuando aplique.
