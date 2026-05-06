# Especificación — Backup monofichero y paquete CSV (MVP)

Objetivo: proveer **backup/restore** y **export/import** con formatos **propios** del servidor self-hosted.

## 1. Backup monofichero cifrado

### Semántica de producto

- Un único archivo descargable que permite **restaurar** el estado completo necesario para un hogar (o para la instalación — ver alcance).
- Protección por **contraseña** (usuario introduce passphrase en export e import).
- Incluye como mínimo:
  - `schemaVersion` (entero, evolución del contenido lógico).
  - `createdAt` (timestamp UTC).
  - **Snapshot** de dominio: tabla `installation` (singleton), `persons`, `categories`, `assets`, `liabilities`, `budgetEntries`, `plannedCashFlows` (misma forma conceptual que `SQLiteStore.Snapshot`).
  - **FIRE settings:** mapa `installation_id → JSON` en servidor.

### Diseño criptográfico

El formato debe usar cifrado autenticado (AEAD) y un KDF estándar para derivar clave desde la passphrase. La elección concreta queda al stack, manteniendo el contrato de producto (password + restore íntegro).

### Versionado

- `formatVersion`: versión del **contenedor** cifrado.
- `schemaVersion`: versión del **payload** interno (JSON / tablas). Import debe rechazar versiones no soportadas con error claro (`unsupportedVersion` equivalente).

### Alcance

**Decisión MVP:** hay **un único hogar lógico por instalación** (véase [`AUTH_MODEL.md`](./AUTH_MODEL.md)). El backup de ese hogar **es** el backup de la instalación para efectos de permisos (`owner`) y de alcance de datos. No se contempla en MVP export «multi-hogar» dentro de la misma base.

### Restore

- Operación **destructiva** en el hogar destino (sobrescribe datos).
- Tras restore, recalcular vistas; sin datos demo.

---

## 2. Paquete CSV (ZIP)

### Nombres de archivo obligatorios en el ZIP

Alineados con el contrato de API/UI del producto:


| Archivo                 | Contenido                     |
| ----------------------- | ----------------------------- |
| `summary_household.csv` | Hogares                       |
| `summary_people.csv`    | Personas                      |
| `categories.csv`        | Definiciones de categoría     |
| `assets.csv`            | Activos                       |
| `liabilities.csv`       | Pasivos                       |
| `budget.csv`            | Presupuesto                   |
| `planning.csv`          | Upcoming / planned cash flows |


### Headers y columnas

La implementación debe:

- Emitir CSV compatibles con los **exports actuales del servidor**.
- Aceptar imports con tolerancia razonable (campos opcionales, columnas nuevas/antiguas).

### Import

- Validación previa opcional; diagnósticos por `(section, lineNumber, reason, rawLine)` como `CSVImportDiagnostic`.
- Import completo del ZIP puede **reemplazar** el hogar actual tras confirmación explícita del usuario.
- Si `categories.csv` está vacío o ausente: **no** inventar categorías por defecto automáticamente salvo que el producto documente una regla explícita distinta del MVP acordado (por defecto: **fallar** o exigir categorías en otros archivos).

### Export

- Respuesta HTTP `application/zip` o descarga desde UI con los siete ficheros anteriores.

### Nota sobre campos de hogar

Si el modelo añade campos nuevos (ej. `show_age_mode`) no presentes en CSV antiguos, el servidor debe **extender** `summary_household.csv` de forma versionada o persistir esos campos fuera del CSV; documentar en changelog del esquema CSV.

---

## 3. Seguridad operativa

- Backups descargados son datos sensibles; forzar HTTPS.
- Limitar intentos de import con passphrase / rate limit.
- Opcional: watermark `createdAt` y `exportedByUserId` en metadata del backup sin romper privacidad local.