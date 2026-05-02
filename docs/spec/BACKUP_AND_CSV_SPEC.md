# Especificación — Backup monofichero y paquete CSV (MVP)

Objetivo: reproducir las **capacidades** del cliente macOS (`BackupArchiveService`, `CSVService`, pestaña Backups en `SettingsView`) con formatos **propios** del servidor self-hosted. **No** leer `.ffbackup` del Mac ni migrar desde él.

## 1. Backup monofichero cifrado

### Semántica de producto

- Un único archivo descargable que permite **restaurar** el estado completo necesario para un hogar (o para la instalación — ver alcance).
- Protección por **contraseña** (usuario introduce passphrase en export e import).
- Incluye como mínimo lo que el Mac empaqueta en `BackupPayload` (`BackupArchiveService.swift`):
  - `schemaVersion` (entero, evolución del contenido lógico).
  - `createdAt` (timestamp UTC).
  - **Snapshot** de dominio: `households`, `persons`, `categories`, `assets`, `liabilities`, `budgetEntries`, `plannedCashFlows` (misma forma conceptual que `SQLiteStore.Snapshot`).
  - **FIRE settings por hogar**: mapa `household_id → JSON` (equivalente a `fireSettingsByHouseholdID` en Mac).

### Diseño criptográfico (referencia Swift actual)

El Mac usa **AES-GCM** sobre JSON serializado, con **salt + nonce**, KDF **PBKDF2-HMAC-SHA256** (~120 000 iteraciones) derivando clave desde passphrase (`BackupArchiveService`). El nuevo formato puede:

- Reutilizar el mismo esquema **conceptual** (envelope con `formatVersion`, salt, nonce, ciphertext, tag) con **nuevo `formatVersion`** y extensión de fichero propia (ej. `.futurefinbak`), **o**
- Sustituir por libs estándar del stack (ej. age, gpg) siempre que el contrato de producto (password + restore íntegro) se mantenga.

### Versionado

- `formatVersion`: versión del **contenedor** cifrado.
- `schemaVersion`: versión del **payload** interno (JSON / tablas). Import debe rechazar versiones no soportadas con error claro (`unsupportedVersion` equivalente).

### Alcance

**Decisión MVP:** hay **un único hogar lógico por instalación** (véase [`AUTH_MODEL.md`](./AUTH_MODEL.md)). El backup de ese hogar **es** el backup de la instalación para efectos de permisos (`owner`) y de alcance de datos. No se contempla en MVP export «multi-hogar» dentro de la misma base.

### Restore

- Operación **destructiva** en el hogar destino (sobrescribe datos como `importOfficialBackup` en Mac).
- Tras restore, recalcular vistas; sin datos demo.

---

## 2. Paquete CSV (ZIP)

### Nombres de archivo obligatorios en el ZIP

Alineados con `AppState.importExportCSVFilenames` / `exportBackupCSV`:


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

**Fuente normativa:** métodos `export`* y `import*WithDiagnostics` en `CSVService.swift`. La nueva implementación debe:

- Emitir CSV compatibles con los **exports actuales**.
- Aceptar imports con las mismas reglas de **tolerancia** (p. ej. assets legacy con columna `kind`, ausencia de columnas de contribución, etc.).

### Import

- Validación previa opcional; diagnósticos por `(section, lineNumber, reason, rawLine)` como `CSVImportDiagnostic`.
- Import completo del ZIP puede **reemplazar** el hogar actual tras confirmación explícita del usuario (equivalente al diálogo destructivo del Mac).
- Si `categories.csv` está vacío o ausente: **no** inventar categorías por defecto automáticamente salvo que el producto documente una regla explícita distinta del MVP acordado (por defecto: **fallar** o exigir categorías en otros archivos).

### Export

- Respuesta HTTP `application/zip` o descarga desde UI con los siete ficheros anteriores.

### Nota sobre campos de hogar

Si el modelo Swift añade campos (ej. `show_age_mode`) no presentes en el CSV histórico del Mac, el servidor debe **extender** `summary_household.csv` de forma versionada o persistir esos campos fuera del CSV; documentar en changelog del esquema CSV.

---

## 3. Seguridad operativa

- Backups descargados son datos sensibles; forzar HTTPS.
- Limitar intentos de import con passphrase / rate limit.
- Opcional: watermark `createdAt` y `exportedByUserId` en metadata del backup sin romper privacidad local.