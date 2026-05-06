# Repositorio principal y versionado

## Repositorio

- El **código base del producto FutureFin self-hosted** vive en este repositorio (`FutureFin`), como **monorepo**.
  - Backend/API Rust: `apps/api`
  - Frontend web: `apps/web`
  - Crates compartidos: `crates/*`

## Versionado semántico

- `**MAJOR.MINOR.PATCH`** para releases etiquetadas en git (`v1.0.0`).
- **MAJOR:** cambios incompatibles en API pública HTTP o en **schemaVersion** / **formatVersion** de backup que rompan restore sin migración documentada.
- **MINOR:** nuevas capacidades compatibles (nuevos campos opcionales, endpoints adicionales).
- **PATCH:** correcciones compatibles.

## Versiones embebidas


| Artefacto          | Campo                                           | Propósito                           |
| ------------------ | ----------------------------------------------- | ----------------------------------- |
| Backup monofichero | `formatVersion`                                 | Evolución del contenedor cifrado    |
| Backup monofichero | `schemaVersion`                                 | Evolución del snapshot JSON interno |
| API HTTP           | header opcional `X-API-Version` o prefijo `/v1` | Contrato cliente-servidor           |


## Changelog

- Mantener `CHANGELOG.md` en este repo con secciones por release y notas de migración de backup/schema cuando aplique.