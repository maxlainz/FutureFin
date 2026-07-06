# Data Model & DB Schema

Migrations in `apps/api/migrations/`. SQLx embeds and runs them on startup.

## Core tables (chronological migration order)

### Users & Sessions
- `users`: `id (uuid PK)`, `username (unique)`, `password_hash`, `birth_date (date nullable)`, `created_at`
- `sessions`: `id (uuid PK)`, `user_id (FK users)`, `expires_at`, `created_at`

### Installation (singleton)
- `installation`: `id (uuid PK)`, `base_currency (char 3)`, `calendar_tz (text)`, `annual_inflation_assumption_percent (decimal NOT NULL DEFAULT 0; 0 = target FIRE plano, >0 = target móvil que crece con la inflación)`, `show_age_mode (text: 'dates'|'ages')`, `fire_settings (jsonb nullable)`, `created_at`
  - `projection_target_age` fue **eliminada** (`20260516120000_drop_projection_target_age.sql`, v1.0.6): el cruce FIRE es el único trigger de jubilación.
  - Singleton: only one row ever exists. First user auto-creates it on register.
  
- `installation_memberships`: `installation_id (FK)`, `user_id (FK)`, `role (text: 'owner'|'member'|'viewer')`, `created_at`

### Ledger entities
All financial tables have `installation_id (FK)` and `owner_user_id (uuid nullable FK users)`.
- `owner_user_id = NULL` → household-level row (legacy or shared)
- `owner_user_id = user.id` → attributed to specific user (`?view=mine` filter)

**categories**: `id`, `installation_id`, `scope ('asset'|'liability'|'income'|'expense')`, `name`, `sort_index`

**assets**: `id`, `installation_id`, `owner_user_id`, `category_id`, `name`, `current_value (decimal)`, `purchase_price (decimal nullable)`, `is_liquid (bool)`, `expected_annual_return_percent (decimal nullable)`, `notes`, `sort_index`. **Contribuciones automáticas viven en `allocation_rules`, no en este registro.**

**allocation_rules**: `id`, `installation_id`, `owner_user_id`, `target_asset_id (FK assets ON DELETE CASCADE)`, `priority (int)`, `kind ('fixed'|'percent'|'remainder')`, `amount (decimal nullable; NULL para 'remainder')`, `cap_kind ('amount'|'months_expense'|'income_multiple' nullable)`, `cap_value (decimal nullable)`, `enabled (bool)`, `notes`, `created_at`. Cascade rules: el engine evalúa las reglas en orden ascendente de `priority` sobre el sobrante mensual (income − expense − debt_service). Cada regla aporta a su `target_asset_id` hasta su `cap` opcional; lo que queda fluye a la siguiente. Constraints: `kind='remainder'` ⇒ `amount IS NULL`; `cap_kind`/`cap_value` ambos NULL o ambos NOT NULL.

**liabilities**: `id`, `installation_id`, `owner_user_id`, `category_id`, `label`, `type_tag`, `principal (decimal)`, `apr_percent (decimal nullable)`, `payment_amount (decimal nullable)`, `payment_frequency ('monthly'|'weekly' nullable)`, `payment_end_date (date nullable)`, `principal_derived_from_plan (bool)`, `notes`, `sort_index`. **Expired rows persist** in the table — read endpoints filter them out via `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. There is no scheduled purge; if you need to recover an expired row, edit `payment_end_date` to a future date.

**budget_entries**: `id`, `installation_id`, `owner_user_id`, `category_id`, `scope ('income'|'expense')`, `amount (decimal, monthly)`, `notes`, `sort_index`, `persists_after_retirement (bool, default false)` — income entries only: whether this income continues after the FIRE-crossover month (used to compute `income_retirement_monthly` in projection) — plus `ends_at_retirement (bool, default false)` and `expense_end_date (date nullable)` — expense entries only: stop computing the expense at the FIRE crossover or at a fixed date (`20260514120000_budget_entries_add_expense_end.sql`, v1.0.8)

**planning_flows**: `id`, `installation_id`, `owner_user_id`, `category_id`, `direction ('inflow'|'outflow')`, `title`, `expected_amount (decimal)`, `due_date (date nullable)`, `notes`, `sort_index`

### History snapshots (perspectiva histórica — v1.5.0)
`20260706203746_history_snapshots.sql`. Snapshots manuales per-user del patrimonio: el ledger solo guarda el valor presente (`assets.current_value`, `liabilities.principal` son escalares mutables sin historial), así que estas dos tablas conservan fotos puntuales de las que el servidor interpola la serie histórica de net worth (ver `GET /v1/history/series`). Totales derivados, nunca almacenados.

**history_snapshots** (cabecera): `id`, `installation_id (FK installation ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `kind (text 'asset'|'liability'; singular, como el resto del código)`, `snapshot_date (date; día civil en calendar_tz)`, `source (text 'capture'|'backfill')`, `created_at`, `updated_at`.
- **Upsert key**: `UNIQUE (installation_id, owner_user_id, kind, snapshot_date)` (`history_snapshots_unique_per_day`). La captura del mismo día sobrescribe silenciosamente (`ON CONFLICT DO UPDATE`); el backfill sobre una fecha ya ocupada da 409 (SQLSTATE 23505 → `Conflict` vía `error.rs`, sin código custom).
- Índice `history_snapshots_installation_date_idx` sobre `(installation_id, snapshot_date)` — sirve el filtro por rango de año del listado.
- **`owner_user_id` es `NOT NULL` con `ON DELETE CASCADE`**, deliberadamente más estricto que el `ON DELETE SET NULL` del resto del ledger. Un snapshot sin dueño no significa nada (la interpolación es per-user y el household es la suma de las series de cada usuario), y el export per-user queda resuelto con un simple `WHERE owner_user_id = caller.id`. Por eso, al borrar un usuario, sus snapshots desaparecen en cascada en vez de quedar huérfanos como filas compartidas.

**history_snapshot_items** (contenido): `id`, `snapshot_id (FK history_snapshots ON DELETE CASCADE)`, `source_item_id (uuid NOT NULL)`, `label (text; 1..=200 chars, no vacío)`, `value (numeric(18,4) >= 0)`, `apr_percent (numeric(8,4) nullable >= 0)`, `payment_amount (numeric(18,4) nullable >= 0)`, `payment_frequency (text nullable 'monthly'|'weekly')`. Los items copian `label` y — solo en pasivos — los términos del préstamo (`apr_percent`, `payment_amount`, `payment_frequency`) para poder reconstruir la curva de amortización aunque el pasivo original se edite o se borre.
- `CONSTRAINT history_snapshot_items_unique_item UNIQUE (snapshot_id, source_item_id)`.
- **`source_item_id` NO es una FK a `assets`/`liabilities`, a propósito.** Es la clave de interpolación/series, no una referencia: en captura vale el id del asset/liability vivo; en backfill vale un UUID del cliente (que enlaza el mismo item entre snapshots) o uno generado por el servidor. La copia debe **sobrevivir al borrado** de la fila de ledger (un asset borrado sigue apareciendo en su histórico y cae a 0 tras su último snapshot), así que una FK sería incorrecta.

**Limitación documentada**: la captura solo toma filas del ledger con `owner_user_id = usuario`. Las filas compartidas del household (`owner_user_id IS NULL`, legacy/compartidas) **nunca se capturan** ni participan en la serie histórica. Consecuencia esperada: `history(mes 0)` puede diferir de `starting_net_worth` de la proyección cuando existen filas compartidas o usuarios sin snapshots; es un desajuste conocido y no un bug.

## FIRE settings (JSONB in installation.fire_settings)
```json
{
  "fire_number_mode": "annual_expense|current_income|manual",
  "fire_number_manual_amount": "decimal string or null",
  "swr_pct": "3.5",
  "taxes_enabled": true,
  "tax_brackets": [
    { "up_to": "6000",   "pct": "19" },
    { "up_to": "50000",  "pct": "21" },
    { "up_to": "200000", "pct": "23" },
    { "up_to": "300000", "pct": "27" },
    { "up_to": null,     "pct": "30" }
  ]
}
```
Defaults (Spain): SWR 3.5%, 5-bracket capital gains schedule (IRPF). Last bracket must have `up_to: null`.
`fire_settings` is nullable; when null, defaults apply on read (handler calls `resolve_fire_settings`).

**Deserialization is strict**: `fire_number_mode` only accepts `manual | annual_expense | current_income`. The legacy alias `annual_expense_adjusted` is mapped to `annual_expense` for backwards-compat with old backups, but any other value returns 422 (was silently coerced to default before May 2026). The field `fire_number_expense_adjustment_pct` was removed — it had no consumer.

## Key invariants
- `Decimal` for all monetary/percentage columns — never `f64` in schema or Rust code
- `calendar_tz` validated as IANA timezone via `chrono_tz`
- `base_currency` validated as 3-letter code, MVP supports EUR/USD/GBP only
- `swr_pct` bounded 0–4 (percent, not ratio)

## Per-user `.ffbackup`
The `/v1/backup/user-export` endpoint serializes a single user's slice into a versioned, encrypted binary file (see [`backup_user/schema.rs`](../apps/api/src/handlers/backup_user/schema.rs) and [`backup_user/crypto.rs`](../apps/api/src/handlers/backup_user/crypto.rs)).

- **Scope**: only rows with `owner_user_id = caller.id` are exported. Household rows (`owner_user_id IS NULL`) are excluded by design. Categories are denormalized to `(scope, name)` pairs for portability across installations.
- **Crypto**: Argon2id KDF (m=19456, t=2, p=1) → AES-256-GCM with random 16-byte salt and 12-byte nonce per export. AAD binds `schema_version`, original `user_id`, and `exported_at` to prevent manifest swap.
- **Framing**: `"FFBK"` magic + format_version (`u8`) + manifest_len (`u32` LE) + manifest JSON + ciphertext. The manifest stays in cleartext so future versions can refuse unsupported schemas without trying to decrypt.
- **Forward compat**: each payload variant lives behind `BackupPayloadVN` + a `migrate_to_current` chain. Backups with `schema_version > CURRENT_SCHEMA_VERSION` are rejected with `409` and a clear error. **`CURRENT_SCHEMA_VERSION = 4`** (v1.5.0): `BackupPayloadV4` = V3 + `snapshots: Vec<BackupSnapshot>`. Migrating a ≤v3 file fills `snapshots` empty (`payload_v3_to_v4`); v1→v2→v3→v4 chain is unbroken.
- **History snapshots (v4)**: `BackupSnapshot {kind, snapshot_date, source, items}`; `BackupSnapshotItem {ledger_index: Option<usize>, item_key: Uuid, label, value, apr_percent?, payment_amount?, payment_frequency?}`.
  - **`item_key`** = the original `source_item_id`, **always present**.
  - **`ledger_index`** = position of the referenced row in **this payload's** `assets` vec (kind=asset) or `liabilities` vec (kind=liability), set **only** when that ledger row still existed at export (miss → `None`).
  - **Re-link on import**: `ledger_index: Some(i)` → the re-inserted item's `source_item_id` becomes the **fresh UUID** of the ledger row re-created at index `i` (preserves cross-snapshot linkage *and* the join-to-today at month 0 that `GET /v1/history/series` relies on). `None` → `item_key` kept **verbatim** (deleted rows / free-form backfill items stay linked to each other). An out-of-bounds `ledger_index` → `400 BadRequest` and the whole import rolls back. Export builds the indices from `fetch_assets`/`fetch_liabilities` (both now return an `id → index` map); `fetch_snapshots` assembles the items.
- **Import semantics**: replace-only. Las **seis** tablas user-scoped se vacían (`WHERE installation_id = $1 AND owner_user_id = $2`) y se reinsertan con UUIDs frescos en la misma transacción, en este orden: `history_snapshots` (primero; sus items caen en cascada), `allocation_rules`, `assets`, `liabilities`, `budget_entries`, `planning_flows`. Los snapshots se **reinsertan al final** (necesitan los UUIDs frescos de assets y liabilities para el re-link). `users.birth_date` is updated if the backup differs. Tras el `commit`, el import invalida la cache de proyección (`refresh_projection_after_mutation`) — antes de v1.5.0 no lo hacía y la proyección quedaba stale hasta 60 min. `ImportCounts` incluye `snapshots` y `snapshot_items` (visibles en preview y apply).
