# FutureFin — Especificación de producto (línea self-hosted)

Este directorio materializa el **dossier de producto** para la refactorización completa de FutureFin como aplicación **Docker / web**, multi-usuario, con **paridad de capacidades** respecto al cliente macOS Swift de referencia — salvo exclusiones documentadas (artefactos desktop, datos demo, compatibilidad legacy).

## Publicación del repositorio

Instrucciones paso a paso para crear **FutureFin** en GitHub y subir `main` y `dev`: [GITHUB_SETUP.md](./GITHUB_SETUP.md).

## Plan Cursor y contexto de conversación

- Copia archivada del dossier (YAML frontmatter + contenido): [plan/PRODUCT_DOSSIER_PLAN.md](plan/PRODUCT_DOSSIER_PLAN.md)
- Resumen de decisiones y sesión para continuar en otra ventana: [plan/CURSOR_SESSION_CONTEXT.md](plan/CURSOR_SESSION_CONTEXT.md)

## Índice de documentos

| Documento | Contenido |
|-----------|-----------|
| [spec/AUTH_MODEL.md](spec/AUTH_MODEL.md) | Singleton por instalación, invitaciones (owner), roles, vistas individual/conjunta, autorización API |
| [spec/PARITY_CHECKLIST.md](spec/PARITY_CHECKLIST.md) | Inventario must-have MVP vs vistas Swift |
| [spec/BACKUP_AND_CSV_SPEC.md](spec/BACKUP_AND_CSV_SPEC.md) | Backup monofichero cifrado + ZIP CSV |
| [spec/ORACLE_TESTS.md](spec/ORACLE_TESTS.md) | Tests Swift como oráculos numéricos |
| [VERSIONING_AND_REPOSITORY.md](VERSIONING_AND_REPOSITORY.md) | Repo nuevo, semver, versiones de backup/API |
| [MAC_CLIENT_SUNSET.md](MAC_CLIENT_SUNSET.md) | Mensajes de deprecación del cliente macOS |

## Fuente de verdad del comportamiento legado

- **Código Swift:** `src/core/`, `renderer/`, `tests/` en el repositorio **FutureFin / FinFuture** (macOS).
- **No** usar solo el README del repo Swift: está desactualizado frente al código (p. ej. número de pestañas, FIRE en Summary).

## Alineación futura del README del repo de implementación

Cuando se cree el repositorio que contenga la aplicación Docker/web:

1. El **README** debe describir despliegue (Docker Compose, variables de entorno, HTTPS), modelo multi-usuario y backups **según** `spec/BACKUP_AND_CSV_SPEC.md`.
2. Las **reglas micro** de negocio deben resumirse en README solo a alto nivel; el detalle normativo permanece en este dossier y en los tests portados desde [`spec/ORACLE_TESTS.md`](spec/ORACLE_TESTS.md).
3. Mantener un enlace a este dossier o fusionarlo en `/docs` del monorepo de producto.

## Principios recordados

- **Excel con esteroides:** UI reactiva, sin botón global de recalcular.
- **Sin datos demo ni categorías por defecto** en primer arranque.
- **Implementación libre** si los oráculos numéricos y la checklist de capacidades se cumplen.
