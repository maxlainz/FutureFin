# FutureFin — Especificación de producto (línea self-hosted)

Este directorio agrupa la **documentación funcional** del producto FutureFin (self-hosted: Docker + web), multi-usuario.

## Publicación del repositorio

Instrucciones paso a paso para crear **FutureFin** en GitHub y subir `main` y `dev`: [GITHUB_SETUP.md](./GITHUB_SETUP.md).

## Índice de documentos

| Documento | Contenido |
|-----------|-----------|
| [spec/AUTH_MODEL.md](spec/AUTH_MODEL.md) | Singleton por instalación, invitaciones (owner), roles, vistas individual/conjunta, autorización API |
| [spec/PARITY_CHECKLIST.md](spec/PARITY_CHECKLIST.md) | Checklist de alcance MVP y criterios de terminado |
| [spec/BACKUP_AND_CSV_SPEC.md](spec/BACKUP_AND_CSV_SPEC.md) | Backup monofichero cifrado + ZIP CSV |
| [VERSIONING_AND_REPOSITORY.md](VERSIONING_AND_REPOSITORY.md) | Repo, semver, versiones de backup/API |

## Principios recordados

- **Excel con esteroides:** UI reactiva, sin botón global de recalcular.
- **Sin datos demo ni categorías por defecto** en primer arranque.
- **Implementación libre** si la checklist de capacidades se cumple.
