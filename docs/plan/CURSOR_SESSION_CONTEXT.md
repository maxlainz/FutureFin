# Contexto de sesiones Cursor — FutureFin

Resumen de lo acordado en chat (planificación e implementación de docs/repo). Sirve como puente si continuás en **otra ventana** u otro agente.

## Decisiones de producto

1. **Línea única:** el futuro del producto es **self-hosted (Docker/web)** con refactor **completa e independiente**; el cliente **macOS (FinFuture/FutureFin Swift)** queda **deprecado**, sin roadmap de features.
2. **Sin migración legacy:** no hay import ni compatibilidad con datos, `.ffbackup` ni CSV generados por la app Mac obsoleta. Solo formatos **nuevos** del servidor.
3. **Paridad MVP:** prácticamente **todas** las capacidades de usuario del Mac son **must-have**, salvo artefactos puramente de arquitectura desktop y el primer arranque legacy del Mac.
4. **Implementación:** objetivo = **misma capacidad y mismos números** (oráculos/tests Swift); **no** copiar Swift línea a línea si hay forma mejor adaptada al stack.
5. **Fuente de verdad:** **código y tests Swift**, no el README del repo Mac (estaba desactualizado).
6. **UX:** **reactiva** (“Excel con esteroides”), sin botón global de recalcular.
7. **Backups:** **monofichero cifrado** (formato nuevo) + **ZIP con 7 CSV** como capacidad equivalente al Mac; detalle en `docs/spec/BACKUP_AND_CSV_SPEC.md`.
8. **Sin demo:** no `AppState.demo()`, no hogar de muestra, **no categorías por defecto** insertadas por la app; estado vacío hasta crear/importar; fallo de persistencia = **error explícito**.
9. **Multi-usuario:** login propio; modelo en `docs/spec/AUTH_MODEL.md` — **un hogar por instalación**, nombre de hogar no editable en MVP, altas de usuarios solo con **invitación aprobada por el owner**, roles owner/member/viewer, vistas individual/conjunta en cliente (sin muros de privacidad servidor entre miembros en v1).

## Artefactos generados en el repo GitHub `FutureFin`

- `README.md` — visión corta y ramas `main` / `dev`.
- `docs/README.md` — índice de especificación.
- `docs/spec/*` — auth, paridad, backup/CSV, oráculos, versionado.
- `docs/MAC_CLIENT_SUNSET.md` — mensajes de sunset Mac.
- `docs/GITHUB_SETUP.md` — instrucciones remoto/push.
- `docs/plan/PRODUCT_DOSSIER_PLAN.md` — copia archivada del plan Cursor del dossier.
- Este archivo — contexto de conversación.

## GitHub y rutas locales

- Repo remoto: **https://github.com/maxlainz/FutureFin**
- Repo local canónico: **`/Users/maxlainz/Documents/GitHub/FutureFin`**
- En algunos setups **`~/FutureFin`** es symlink al mismo directorio.

### Push inicial

- Se hizo push de **`main`** y **`dev`** usando URL HTTPS explícita (por restricciones del sandbox al editar `.git/config` en algunos entornos).
- Conviene ejecutar en tu terminal:

```bash
cd ~/Documents/GitHub/FutureFin
git remote set-url origin https://github.com/maxlainz/FutureFin.git
git fetch origin
```

## Permisos del agente Cursor

- Los comandos del agente pueden ir **sandboxed**; a veces fallan escrituras en `.git/config` o red sin aprobación.
- Para push/commits suele hacer falta aprobar con **red** y permisos amplios; **credenciales GitHub** son las de tu máquina (token/SSH).
- No existe un “permiso absoluto permanente” documentado como único toggle; se aprueba por comando / políticas del workspace y macOS (p. ej. Full Disk Access para Cursor si bloquea rutas).

## Repo Swift de referencia (oráculo)

Ruta típica en tu máquina: **`/Users/maxlainz/Documents/GitHub/FinFuture`** (nombre puede variar: FinFuture / FutureFin).

## Siguientes pasos sugeridos

1. Commitear y pushear la carpeta `docs/plan/` desde tu máquina si esta sesión no pudo hacer `git push`.
2. Desarrollar en rama **`dev`** según `docs/spec/PARITY_CHECKLIST.md`.
3. Copiar fixtures/tests desde `SummaryServiceTests` / `AppStateMetricsTests` según `docs/spec/ORACLE_TESTS.md`.
