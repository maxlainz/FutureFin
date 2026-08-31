# Data Model & DB Schema

Migrations in `apps/api/migrations/`. SQLx embeds and runs them on startup.

## Core tables (chronological migration order)

### Users & Sessions
- `users`: `id (uuid PK)`, `username (unique)` (restricción `users_username_key`), `password_hash (text, NULLABLE)`, `birth_date (date nullable)`, `created_at`, `external_user_id (uuid nullable)`. Los dos últimos cambios llegan con `20260827120000_users_trusted_header_identity.sql` (SSO por cabeceras de un proxy de confianza), ambos **aditivos** para las cuentas que ya existían:
  - **`password_hash` deja de ser `NOT NULL`**. `NULL` = cuenta SSO **sin contraseña**: la autenticación la hizo el proveedor antes de que la petición llegara. Inventarle una contraseña aleatoria sería peor (una credencial que nadie conoce y que el cambio de contraseña podría rotar), así que la ausencia se modela como ausencia. Los tres flujos de contraseña la rechazan con **401 `sso_account_no_password`**: `POST /v1/auth/login`, `POST /v1/auth/password` y `POST /v1/backup/user-export` (su clave se deriva de la contraseña de cuenta).
  - **`external_user_id`** guarda la identidad estable del proveedor (`X-Remote-User-Id`). Es la clave de resolución del SSO: cambiar el nombre para mostrar en Home Assistant no crea una cuenta nueva. `NULL` en las cuentas de usuario+contraseña.
  - **Índice `users_external_user_id_key`: UNIQUE PARCIAL**, `CREATE UNIQUE INDEX … ON users (external_user_id) WHERE external_user_id IS NOT NULL`. El `WHERE` deja explícito que las filas sin identidad externa no compiten (aunque en Postgres `NULL` ya sea distinto de `NULL`) y mantiene el índice pequeño. **Consecuencia para los handlers**: `username` ya **no es el único UNIQUE de la tabla**, así que un 23505 sobre `users` hay que discriminarlo por `db.constraint()` antes de traducirlo — `register` lo hace (solo `users_username_key` → `username_taken`) y `handlers/sso.rs` distingue las tres salidas (nombre cogido → siguiente candidato; identidad duplicada → devolver el usuario que ganó la carrera; otra cosa → error).
  - Los dos `COMMENT ON COLUMN` de la migración son la documentación in-situ; `\d+ users` en `psql` los muestra.
- `sessions`: `id (uuid PK)`, `user_id (FK users)`, `expires_at`, `created_at`
- `api_tokens` (v3.0.0, `20260816120000_api_tokens.sql`; `scope` añadida en `20260828140100_api_tokens_scope.sql`, Fase 3/issue #84): `id (uuid PK)`, `user_id (FK users ON DELETE CASCADE)`, `label (1..64)`, `token_hash (text UNIQUE — SHA-256 hex del secreto; el secreto `ffp_…` jamás se persiste)`, `token_prefix (primeros 12 chars, para la UI)`, `scope (text NOT NULL DEFAULT 'read_write', CHECK ∈ {read_write, read_only})`, `created_at`, `expires_at (nullable)`, `last_used_at (nullable, throttle 60 s)`, `revoked_at (nullable — soft-revoke, la fila queda como auditoría)`. Credencial Bearer del servidor MCP (`/mcp`). **Excluida a propósito del `.ffbackup`**: son credenciales de la instalación, no datos financieros — un restore no debe resucitar secretos. Sin `installation_id`: el rol/installation se re-resuelven vivos en cada uso vía `require_installation_member`. `scope` se lee **vivo** en el mismo SELECT que autentica; `ADD COLUMN … NOT NULL DEFAULT` no reescribe la tabla (PG 11+, el default vive en el catálogo) así que los tokens ya emitidos siguen funcionando byte a byte. Detalle de las tres puertas de escritura (rol → scope → toggle) y del reparto de responsabilidades: [`auth-and-membership.md`](auth-and-membership.md) §API tokens.

### Installation (singleton)
- `installation`: `id (uuid PK)`, `base_currency (char 3)`, `calendar_tz (text)`, `annual_inflation_assumption_percent (decimal NOT NULL DEFAULT 0; 0 = target FIRE plano, >0 = target móvil que crece con la inflación)`, `show_age_mode (text: 'dates'|'ages')`, `fire_settings (jsonb nullable)`, `mcp_write_enabled (bool NOT NULL DEFAULT TRUE, `20260818120000` — kill-switch vivo de las tools de escritura MCP; toggle owner-only en Ajustes → Integraciones; NO se exporta en el `.ffbackup`)`, `created_at`
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

**liabilities**: `id`, `installation_id`, `owner_user_id`, `category_id`, `expense_category_id (uuid nullable, FK categories ON DELETE SET NULL — 3.4.0)`, `label`, `type_tag`, `principal (decimal)`, `apr_percent (decimal nullable)`, `payment_amount (decimal nullable)`, `payment_frequency ('monthly'|'weekly' nullable)`, `payment_end_date (date nullable)`, `principal_derived_from_plan (bool)`, `repayment_model (text NOT NULL DEFAULT 'french' — default 'fixed_payments' en 4.2.0, 'french' desde 4.7.0/#144)`, `min_payment_pct (numeric(8,4) nullable, CHECK 0..100 — 4.7.0, cuota mínima revolving en % del saldo)`, `min_payment_eur (numeric(18,4) nullable >= 0 — 4.7.0, suelo en € de esa cuota)`, `notes`, `sort_index`. `repayment_model` acota su dominio con un CHECK, no con un ENUM de Postgres (`liabilities_repayment_model_chk`: `IN ('fixed_payments','french','interest_only','revolving')`, migración `20260825120000_liabilities_repayment_model.sql`) — añadir un modelo es un `ALTER` del CHECK, sin las servidumbres de tipo que un ENUM arrastra a las migraciones y al backup por usuario. **Historia del DEFAULT**: en 4.2.0 fue `fixed_payments` (reproducía el modelo pre-4.2.0 y nadie veía moverse un número al actualizar); en 4.7.0 (#144, migración `20260901120000_liabilities_repayment_model_french_default.sql`, DATA-CHANGING firmada por el owner) pasó a `french` Y las filas `fixed_payments` con TIN > 0 y cuota mensual se CONVIRTIERON a `french` (su proyección empezó a cobrar intereses — el número honesto); el TIN residual inexpresable se anuló. La validación de escritura acopla modelo⇔campos (`apr_forbidden_for_model`, `revolving_minimum_required`) — los CHECKs de columna se quedan laxos a propósito para que los backups viejos importen. `apr_percent` es, en la práctica, el **TIN nominal anual**: el engine lo usa como `i = apr/1200` (ver [`engine.md`](engine.md)). `expense_category_id` es la categoría de GASTO donde vive la cuota (atribución en `/v1/budget` y en la comparativa de Movimientos); **obligatoria al crear vía API** desde 3.4.0, `NULL` solo en filas anteriores sin asignar (y en imports de backups viejos). No cuenta en `category_delete_effects` (no bloquea borrar la categoría; el remap de categorías de gasto sí la arrastra). **Expired rows persist** in the table — y desde 4.7.0 (#145) el predicado de visibilidad de las lecturas es `WHERE (payment_end_date IS NULL OR payment_end_date >= $today OR principal > 0)`: el plan vencido con SALDO VIVO sigue visible (congelado, marcado `plan_expired_with_balance` en la respuesta); solo el vencido y saldado (`principal = 0`) se filtra. There is no scheduled purge.

**budget_entries**: `id`, `installation_id`, `owner_user_id`, `category_id`, `scope ('income'|'expense')`, `amount (decimal, monthly)`, `notes`, `sort_index`, `persists_after_retirement (bool, default false)` — income entries only: whether this income continues after the FIRE-crossover month (used to compute `income_retirement_monthly` in projection) — plus `ends_at_retirement (bool, default false)` and `expense_end_date (date nullable)` — expense entries only: stop computing the expense at the FIRE crossover or at a fixed date (`20260514120000_budget_entries_add_expense_end.sql`, v1.0.8)

**planning_flows**: `id`, `installation_id`, `owner_user_id`, `category_id`, `direction ('inflow'|'outflow')`, `title`, `expected_amount (decimal)`, `due_date (date nullable)`, `notes`, `sort_index`

### History snapshots (perspectiva histórica — v1.5.0)
`20260706203746_history_snapshots.sql`. Snapshots manuales per-user del patrimonio: el ledger solo guarda el valor presente (`assets.current_value`, `liabilities.principal` son escalares mutables sin historial), así que estas dos tablas conservan fotos puntuales de las que el servidor interpola la serie histórica de net worth (ver `GET /v1/history/series`). Totales derivados, nunca almacenados.

**history_snapshots** (cabecera): `id`, `installation_id (FK installation ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `kind (text 'asset'|'liability'; singular, como el resto del código)`, `snapshot_date (date; día civil en calendar_tz)`, `source (text 'capture'|'backfill')`, `created_at`, `updated_at`.
- **Upsert key**: `UNIQUE (installation_id, owner_user_id, kind, snapshot_date)` (`history_snapshots_unique_per_day`). La captura del mismo día sobrescribe silenciosamente (`ON CONFLICT DO UPDATE`); el backfill sobre una fecha ya ocupada da 409 (SQLSTATE 23505 → `Conflict` vía `error.rs`, sin código custom).
- Índice `history_snapshots_installation_date_idx` sobre `(installation_id, snapshot_date)` — sirve el filtro por rango de año del listado.
- **`owner_user_id` es `NOT NULL` con `ON DELETE CASCADE`**, deliberadamente más estricto que el `ON DELETE SET NULL` del resto del ledger. Un snapshot sin dueño no significa nada (la interpolación es per-user y el household es la suma de las series de cada usuario), y el export per-user queda resuelto con un simple `WHERE owner_user_id = caller.id`. Por eso, al borrar un usuario, sus snapshots desaparecen en cascada en vez de quedar huérfanos como filas compartidas.

**history_snapshot_items** (contenido): `id`, `snapshot_id (FK history_snapshots ON DELETE CASCADE)`, `source_item_id (uuid NOT NULL)`, `label (text; 1..=200 chars, no vacío)`, `value (numeric(18,4) >= 0)`, `apr_percent (numeric(8,4) nullable >= 0)`, `payment_amount (numeric(18,4) nullable >= 0)`, `payment_frequency (text nullable 'monthly'|'weekly')`, `repayment_model (text nullable, CHECK ∈ los cuatro literales — 4.7.0/#129, migración `20260901120100`; NULL = snapshot pre-4.7.0 ⇒ interpolación LINEAL, el default de su época; NO se backfilla desde `liabilities` — el modelo de HOY no es el de la foto)`. Los items copian `label` y — solo en pasivos — los términos del préstamo (`apr_percent`, `payment_amount`, `payment_frequency`, y desde 4.7.0 el `repayment_model`) para poder reconstruir la curva de amortización con la LEY de su momento aunque el pasivo original se edite o se borre.
- `CONSTRAINT history_snapshot_items_unique_item UNIQUE (snapshot_id, source_item_id)`.
- **`source_item_id` NO es una FK a `assets`/`liabilities`, a propósito.** Es la clave de interpolación/series, no una referencia: en captura vale el id del asset/liability vivo; en backfill vale un UUID del cliente (que enlaza el mismo item entre snapshots) o uno generado por el servidor. La copia debe **sobrevivir al borrado** de la fila de ledger (un asset borrado sigue apareciendo en su histórico y cae a 0 tras su último snapshot), así que una FK sería incorrecta.

**Limitación documentada**: la captura solo toma filas del ledger con `owner_user_id = usuario`. Las filas compartidas del household (`owner_user_id IS NULL`, legacy/compartidas) **nunca se capturan** ni participan en la serie histórica. Consecuencia esperada: `history(mes 0)` puede diferir de `starting_net_worth` de la proyección cuando existen filas compartidas o usuarios sin snapshots; es un desajuste conocido y no un bug.

### Transactions (histórico de gasto mensual — v1.6.0)
`20260707120000_transactions_and_rules.sql`. Tres tablas nuevas para el histórico REAL de gasto (importado de CSV bancario o metido a mano como efectivo), su categorización y las reglas aprendidas. Todas per-user (`owner_user_id NOT NULL`). **Contrato de cache condicionado al modo `fire_settings.savings_source`**: en modo A (`budget`, default) las transacciones **no son inputs del engine de proyección** → sus mutaciones **nunca invalidan la cache**; en los modos que usan transacciones (B `transactions_avg` y C `budget_income_real_expense` → `SavingsSource::uses_transactions()`) el ahorro de la simulación deriva del promedio real 12m de las transacciones **no conciliadas** (3.5.0) → **sí son inputs**, y las mutaciones invalidan la cache (`invalidate_projection_if_savings_uses_transactions`); conciliar/desconciliar también es una mutación del conjunto y también invalida. Los tres casos fijados por `transactions_projection_cache.rs`. Ningún `CHECK` de valores sobre `source` (un banco nuevo no debe exigir migración; se valida en Rust).

**transaction_imports** (cabecera de un lote de import = un CSV): `id`, `installation_id (FK installation ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `source (text NOT NULL; `myinvestor`|`n26`|… validado en Rust)`, `account_asset_id (uuid nullable FK assets **ON DELETE SET NULL**; cuenta origen, metadata)`, `original_filename (text nullable, CHECK ≤300 chars)`, `created_at`. Índice `(installation_id, owner_user_id, created_at DESC)`. Borrar la cabecera deshace el import: sus `transactions` caen en cascada (`import_id ON DELETE CASCADE`).

**transactions** (un movimiento datado, **firmado**): `id`, `installation_id (FK ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `import_id (uuid nullable FK transaction_imports **ON DELETE CASCADE**; NULL = manual/efectivo)`, `source (text NOT NULL)`, `op_date (date NOT NULL; fecha de operación, referencia de la huella y del mes)`, `value_date (date nullable)`, `concept (text NOT NULL, CHECK ≤500 y trim no vacío)`, `amount (numeric(18,4) NOT NULL; **firmado**: negativo = cargo)`, `currency (char(3) NOT NULL DEFAULT 'EUR', CHECK = 'EUR')`, `kind (text nullable, CHECK ∈ {expense, income, savings})`, `category_id (uuid nullable FK categories **ON DELETE RESTRICT**; NULL = "Sin categoría")`, `fingerprint (text NOT NULL)`, `fingerprint_ordinal (int NOT NULL DEFAULT 0)`, `linked_asset_id (uuid nullable FK assets **ON DELETE SET NULL**; destino de un ahorro, metadata)`, `linked_liability_id (uuid nullable FK liabilities **ON DELETE SET NULL**; cuota de préstamo, metadata)`, `notes (text nullable, CHECK ≤4000)`, `created_at`, `updated_at`.
- **Dedup por huella**: `CONSTRAINT transactions_unique_fingerprint UNIQUE (installation_id, owner_user_id, fingerprint, fingerprint_ordinal)`. La `fingerprint` se **computa en Rust** (nunca se almacena en el CSV/backup): `source · op_date ISO · importe canónico 4dp · concepto normalizado`, unidos con `\u{1f}` (`schema::compute_fingerprint`). El `fingerprint_ordinal` (`MAX+1` por `(owner, fingerprint)`) distingue ocurrencias repetidas del **mismo** movimiento en el mismo archivo; forzar una fila `already_imported` incrementa el ordinal en vez de dar 409.
- **`kind='savings'` no lleva categoría** (`category_id` debe ser NULL → `savings_no_category`, validado en Rust); `expense`/`income` con categoría exigen que el `scope` de la categoría coincida con el kind (`category_scope_mismatch`).
- **Edición y huella (manuales vs importadas)**: `op_date`/`amount`/`concept` son **editables por PATCH tanto en manuales como en importadas** (ya no existe `immutable_field`). La diferencia está en la huella: en **manuales** se **recomputa** al cambiar esos campos (toma un ordinal libre por `(owner, fingerprint)` y libera el anterior); en **importadas** la huella queda **anclada** a la del CSV original y **nunca** se recomputa, para que un re-import del mismo archivo siga detectando el duplicado aunque el usuario haya reubicado la fecha o corregido importe/concepto.
- **Contraste de ON DELETE**: `linked_asset_id`/`linked_liability_id` son `SET NULL` (el movimiento **sobrevive** al borrado de la fila de ledger, contraste deliberado con `source_item_id` de history que ni siquiera es FK); `category_id` es `RESTRICT` (una categoría en uso **no** se borra sin remap — `categories.rs` la incluye en `category_delete_effects` y la remapea en la transacción de borrado); `import_id` es `CASCADE`.
- Índices: `(installation_id, op_date)`, `(installation_id, owner_user_id, op_date)`, `(installation_id, category_id)`, `(import_id)`.

**categorization_rules** (reglas que PRE-asignan kind+categoría en el preview del import): `id`, `installation_id (FK ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `match_kind (text NOT NULL DEFAULT 'substring', CHECK ∈ {substring, prefix, exact})`, `pattern (text NOT NULL, CHECK ≤500 y trim no vacío; normalizado como el concepto)`, `source (text nullable; NULL = regla **agnóstica**, aplica a cualquier banco)`, `assign_kind (text nullable, CHECK ∈ {expense, income, savings})`, `assign_category_id (uuid nullable FK categories **ON DELETE SET NULL**; una regla degradada nunca bloquea el borrado de una categoría)`, `created_at`, `updated_at`. `CONSTRAINT categorization_rules_unique UNIQUE (installation_id, owner_user_id, source, pattern)` (el aprendizaje al confirmar un import hace upsert sobre ella). **Esa constraint NO cubre las reglas agnósticas** (`source IS NULL`): en SQL `NULL <> NULL`, así que dos reglas sin `source` y con el mismo patrón nunca colisionaban, y eran justo las que se crean por defecto — dos POST idénticos devolvían 200 los dos. Desde `20260828120000_categorization_rules_unique_agnostic` la mitad que falta la cubre el índice único **parcial** `categorization_rules_unique_agnostic (installation_id, owner_user_id, pattern, match_kind) WHERE source IS NULL`, precedido de un dedup que borra solo filas **inalcanzables** por la precedencia (misma clave completa; sobrevive la de `updated_at` mayor) — las que difieren en `match_kind` matchean otros conceptos y NO se tocan. La validación de la aplicación es más ancha que el índice: `create_categorization_rule_core` rechaza con **409 `rule_duplicate`** cualquier `(COALESCE(source,''), pattern)` repetido, sin mirar `match_kind`, que es lo que promete el contrato. Índice `(installation_id, owner_user_id)`. Precedencia de matching (en Rust): source-específica > agnóstica → exact > prefix > substring → patrón más largo → `updated_at` más reciente.

### Recurring transaction rules (plantillas de movimiento recurrente — v1.8.0; resolución mensual desde 3.2.0)
`20260708090000_recurring_transaction_rules.sql` (+ `20260817130000_recurring_rules_monthly_resolution.sql`, que **elimina `day_of_month`**). Una **plantilla** per-user (nómina, alquiler, aportación mensual…) que **materializa** movimientos reales en `transactions`. Desde 3.2.0 las reglas tienen **resolución mensual**: la instancia del mes M se fecha en el **último día de M** y solo se materializa con M ya **cerrado** (el mes en curso jamás — así el mes abierto no muestra movimientos sintéticos que distorsionen sus estadísticas). Materializar invalida la cache de proyección **solo en los modos que usan transacciones** (B/C — regresión en `transactions_projection_cache.rs`).

**recurring_transaction_rules**: `id`, `installation_id (FK installation ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `concept (text NOT NULL, CHECK ≤500 y trim no vacío)`, `amount (numeric(18,4) NOT NULL; **firmado**, negativo = cargo; CHECK <> 0)`, `kind (text NOT NULL, CHECK ∈ {expense, income, savings})`, `category_id (uuid nullable FK categories **ON DELETE RESTRICT**)`, `linked_asset_id (uuid nullable FK assets **ON DELETE SET NULL**)`, `linked_liability_id (uuid nullable FK liabilities **ON DELETE SET NULL**)`, `notes (text nullable, CHECK ≤4000)`, `origin_month (date NOT NULL)`, `created_at`, `updated_at`. Índice `(installation_id, owner_user_id)` y **UNIQUE parcial** `transactions (recurring_rule_id, date_trunc('month', op_date::timestamp)) WHERE recurring_rule_id IS NOT NULL`.
- **`origin_month` = ANCLA de la regla** (3.9.0; sustituye al cursor monotónico `last_materialized_month`): el mes en que arrancó. La **convergencia** (`converge_recurring_for_installation`) lleva las instancias al estado que define la invariante — *una instancia de R existe en el mes M ⟺ M es un mes **activo** de la instalación y `M >= R.origin_month`* —, creando lo que falta y **podando** lo que sobra tras cada mutación de transacciones. **Mes activo** = mes civil cerrado con ≥1 movimiento real (`recurring_rule_id IS NULL`) no conciliado; el ámbito es de **instalación**, no de owner. La idempotencia es **por existencia**, respaldada por el índice UNIQUE parcial: el cursor era monotónico y por tanto incapaz de materializar un mes antiguo que un CSV activara hoy. Consecuencias: borrar una instancia a mano **ya no la borra para siempre** (vuelve mientras su mes siga activo; para quitarla se borra la plantilla), y el alta con fecha pasada **ya no backfillea** meses vacíos. El mes de origen está exento de la **poda** (su instancia es el movimiento que dio de alta la recurrencia y no cuenta como real), pero no de la **inserción** (`>= origin_month` + comprobación de existencia).
- **`category_id` es `ON DELETE RESTRICT`** (igual que `transactions.category_id`, y a diferencia del `SET NULL` de `categorization_rules`): la regla genera transacciones reales con esa categoría, así que una categoría en uso por una regla **no** se puede borrar sin remap. `categories.rs` la cuenta en `category_delete_effects` y la remapea (`UPDATE ... SET category_id = target`) dentro de la transacción de borrado, junto a las `transactions`. `linked_asset_id`/`linked_liability_id` son `SET NULL` (metadata; la regla sobrevive al borrado de la fila de ledger).

**Columna nueva en `transactions`**: `recurring_rule_id (uuid nullable FK recurring_transaction_rules **ON DELETE SET NULL**)` + índice `transactions_recurring_rule_idx (recurring_rule_id)`. Enlaza cada instancia (y la de origen) a su regla; borrar la regla **conserva** las instancias (quedan como manuales sueltas).

### Conciliación de transferencias (3.5.0)
`20260819120000_transactions_transfer_reconciliation.sql`. Un movimiento **conciliado** es una pata de una transferencia interna emparejada con su contrapartida (la otra pata, normalmente de otro extracto): sigue **visible** en Movimientos pero queda **excluido de todos los agregados de flujo** (totales del mes, comparativa por categoría, promedio real 12m que alimenta el engine en modos B/C, serie por categoría y `months[]` de `/v1/history/cashflow`; **NO** de la curva fina del cashflow — un traspaso mueve saldo real entre cuentas propias). Predicado único: `transfer_counterpart_id IS NULL`.

**Columnas nuevas en `transactions`**: `transfer_counterpart_id (uuid nullable **self-FK** transactions **ON DELETE SET NULL**)`, `transfer_reconciled_at (timestamptz nullable)`, `transfer_reconciled_source (text nullable, CHECK ∈ {auto, manual})`, `CHECK (transfer_counterpart_id <> id)`.
- **La fuente de verdad de «conciliada» es `transfer_counterpart_id IS NOT NULL`**: el `ON DELETE SET NULL` no limpia `reconciled_at/source`, así que esos dos campos solo se interpretan (y se serializan) cuando hay contrapartida.
- **`ON DELETE SET NULL` desconcilia gratis**: borrar una pata (o su import en cascada) devuelve la superviviente al gasto. Se rechazó un `transfer_group_id` porque dejaría «grupos de uno» ocultos para siempre — el bug de partida con otro nombre.
- **Inyectividad por índice UNIQUE parcial** (`transactions_transfer_counterpart_uniq`): nadie es contrapartida de dos movimientos. La **simetría** (A→B ∧ B→A) la escribe siempre `handlers/transactions/reconcile.rs` en una única transacción.
- Índice del matcher: `transactions_transfer_match_idx (installation_id, owner_user_id, amount, op_date) WHERE transfer_counterpart_id IS NULL`.
- **Auto-matching** (determinista, punto fijo): mismo owner + misma divisa + importes exactamente opuestos + `|Δop_date| ≤ 5 días` + par no rechazado; corre post-commit tras toda mutación del conjunto y vía `POST /v1/transactions/reconcile`. La conciliación **manual** de un par exige importes opuestos y misma divisa pero **no** la ventana de fecha.

**transfer_match_rejections** (memoria anti-resurrección del matcher): `id`, `installation_id (FK ON DELETE CASCADE)`, `owner_user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `transaction_a_id`/`transaction_b_id (uuid NOT NULL FK transactions **ON DELETE CASCADE**)`, `created_at`; `CHECK (transaction_a_id < transaction_b_id)` (par canónico) + `UNIQUE (transaction_a_id, transaction_b_id)`. Se inserta al **desconciliar a mano** (los pases posteriores no re-emparejan ese par); la conciliación manual del mismo par lo borra. Un PATCH que cambia `amount`/`op_date` rompe el par **sin** crear rechazo (no es una decisión del usuario sobre el par). **Limitación documentada**: deshacer un import y re-importarlo da UUIDs nuevos → el rechazo cascadea y el par vuelve a ser candidato.

## OAuth (v3.1.0)

`20260817090000_oauth.sql`. Cinco tablas para el authorization server embebido (ver
[`api-routes.md`](api-routes.md) §OAuth 2.1 y [`auth-and-membership.md`](auth-and-membership.md)).
Mismo contrato de credenciales que `api_tokens`: **solo se persiste el SHA-256 hex** del secreto,
nada se congela (rol y membership se re-resuelven vivos en cada request), revocación = una fila.
**Las expiries las calcula Postgres** (`now() + $n::interval`), nunca Rust — así el TTL no depende
del reloj del proceso.

**oauth_clients** (apps registradas por DCR): `id (uuid PK)`, `client_id (text NOT NULL UNIQUE; el
público `ffc_…`)`, `client_secret_hash (text nullable — SHA-256 hex de `ffcs_…`; **NULL** para
clientes públicos)`, `client_name (text NOT NULL)`, `client_uri (text nullable)`,
`redirect_uris (text[] NOT NULL)`, `token_endpoint_auth_method (text NOT NULL)`, `created_at`,
`last_used_at (nullable, throttle 60 s)`.
- CHECKs: `oauth_clients_name_len` (`client_name` 1..120), `oauth_clients_redirects_card`
  (`cardinality(redirect_uris)` 1..5), `oauth_clients_auth_method` (∈ {`none`,
  `client_secret_basic`, `client_secret_post`}) y `oauth_clients_secret_presence` — el emparejamiento
  que hace imposible una fila incoherente: `none` ⇒ `client_secret_hash IS NULL`, cualquier otro
  método ⇒ `IS NOT NULL`.
- Índice `oauth_clients_created_at_idx (created_at)`: lo consume el **GC perezoso** de registros
  huérfanos de `POST /oauth/register` (clientes de >24 h sin ningún grant).
- **Sin `installation_id`**: un cliente registrado no pertenece a nadie ni da acceso a nada. El gate
  real es el consentimiento del usuario, o sea una fila en `oauth_grants`.

**oauth_grants** (el consentimiento — **la unidad de todo**): `id (uuid PK)`,
`client_id (uuid NOT NULL FK oauth_clients ON DELETE CASCADE)`,
`user_id (uuid NOT NULL FK users ON DELETE CASCADE)`, `scope (text nullable)`,
`resource (text nullable)`, `created_at`, `last_used_at (nullable, throttle 60 s — el «Último uso»
del panel)`, `revoked_at (nullable)`, `revoked_reason (text nullable)`.
- **`CREATE UNIQUE INDEX oauth_grants_active_uniq ON oauth_grants (client_id, user_id) WHERE revoked_at IS NULL`
  — índice UNIQUE parcial, y el `WHERE` es todo el diseño.** Habilita el upsert de
  `issue_authorization_code` (`ON CONFLICT (client_id, user_id) WHERE revoked_at IS NULL DO UPDATE`):
  re-consentir la misma app **refresca el grant vivo** en vez de duplicar la fila que ve el panel. Y
  como la unicidad solo aplica a las filas vivas, un grant revocado **no bloquea** un consentimiento
  posterior — el historial de revocaciones se acumula sin colisionar.
- **Soft-revoke con motivo**: revocar es `revoked_at = now()` + `revoked_reason` ∈ `user_panel`
  (botón del panel) \| `refresh_token_reuse` \| `code_reuse` (señales de robo, OAuth 2.1 §4.3.1/§7.5)
  \| `rfc7009` (revocación del cliente) \| **`password_change`** (4.0.0, `POST /v1/auth/password`)
  \| **`membership_revoked`** (4.0.0, `DELETE /v1/installation/members/{user_id}`). Los dos nuevos
  se escriben en la MISMA transacción que su acción: si el motivo del cambio de contraseña es un
  compromiso, dejar viva una credencial que no caduca haría el cambio decorativo, y expulsar a
  alguien dejándole el `ffo_` vivo no sería expulsarlo. Los motivos, reproducibles:
  `grep -rn 'revoked_reason' apps/api/src` (ojo: `code_reuse` y `refresh_token_reuse` no viajan como
  literal en el `UPDATE`, se pasan como parámetro desde `oauth/token.rs`). La fila queda como auditoría. **Es el único punto de corte
  que hay que tocar**: `oauth/access.rs` valida el access token con un JOIN que exige
  `g.revoked_at IS NULL`, así que revocar el grant mata todos sus tokens sin actualizarlos (misma
  filosofía que borrar una sesión).
- Índices `oauth_grants_user_id_idx (user_id)` (lo consume el panel) y
  `oauth_grants_client_id_idx (client_id)`.

**oauth_authorization_codes**: `code_hash (text PK — el SHA-256 hex es la PK, el code en claro
nunca se persiste)`, `grant_id (uuid NOT NULL FK oauth_grants ON DELETE CASCADE)`,
`redirect_uri (text NOT NULL)`, `code_challenge (text NOT NULL)`,
`code_challenge_method (text NOT NULL)`, `resource`, `scope`, `created_at`,
`expires_at (NOT NULL; **2 min**)`, `consumed_at (nullable — un solo uso)`.
- CHECKs: `oauth_codes_pkce_s256` (`code_challenge_method = 'S256'` — PKCE S256 **obligatorio a
  nivel de schema**, no solo en Rust) y `oauth_codes_challenge_len` (43..128 chars).
- Índices `(grant_id)` y `(expires_at)`.

**oauth_access_tokens** (`ffo_`, 1 h): `id (uuid PK)`,
`grant_id (uuid NOT NULL FK oauth_grants ON DELETE CASCADE)`, `token_hash (text NOT NULL UNIQUE)`,
`created_at`, `expires_at (NOT NULL)`, `revoked_at (nullable)`. Índices `(grant_id)` y
`(expires_at)`. `revoked_at` aquí solo lo usa RFC 7009 con un `ffo_` explícito; el corte normal es
revocar el grant.

**oauth_refresh_tokens** (`ffr_`, 90 días **sin uso**): `id (uuid PK)`,
`grant_id (uuid NOT NULL FK oauth_grants ON DELETE CASCADE)`, `token_hash (text NOT NULL UNIQUE)`,
`created_at`, `expires_at (NOT NULL)`, `consumed_at (nullable)`,
`replaced_by (uuid nullable FK oauth_refresh_tokens **ON DELETE SET NULL** — self-FK)`,
`revoked_at (nullable)`. Índice `(grant_id)`.
- **`consumed_at` + `replaced_by` son la rotación**: cada canje consume el actual, emite uno nuevo
  con `expires_at = now() + 90 días` (sliding por construcción) y los **encadena** para poder
  auditar la cadena. Que `replaced_by` sea `SET NULL` y no `CASCADE` es deliberado: purgar tokens
  viejos no debe borrar los nuevos.
- **Sin `UNIQUE (grant_id)`**: un grant puede tener varios refresh vivos (cadenas de rotación en
  paralelo). La detección de robo no la da una constraint sino `consumed_at` (ver `oauth/token.rs`,
  `FOR UPDATE` sobre la fila).

**Cascadas, de fuera a dentro**: borrar un `users` → sus grants → sus codes y tokens; borrar un
`oauth_clients` → sus grants → idem. Ningún token queda huérfano por construcción, así que no hace
falta un job de limpieza para la integridad (los índices por `expires_at` están para una purga
futura de filas caducadas, que hoy no existe).

**Excluidas del `.ffbackup` por construcción.** Son credenciales de la instalación, no datos
financieros: un restore no debe resucitar accesos concedidos. La exclusión no es una lista negra que
haya que mantener — `backup_user/export.rs` es una **whitelist**: un `SELECT` explícito por tabla
exportada, y ninguno menciona `oauth_*`. **No los añadas ahí.** Corolario: la migración OAuth no tocó el
formato de backup (el bump a **7** llegó después, en 3.2.0, por las reglas recurrentes).

## MCP write safety — auditoría, idempotencia y confirmación en dos fases (Fase 3, issue #84)

Cuatro tablas nuevas (más la columna `api_tokens.scope` de arriba), ninguna en el `.ffbackup`: son
artefactos operativos del transporte de escritura, no datos del hogar (misma familia que
`api_tokens`/`oauth_*`, y por la misma razón — un restore no debe resucitar secretos ni estado de
protocolo). Ninguna cambia `CURRENT_SCHEMA_VERSION` (sigue en **10**). Contexto de diseño completo
(el porqué de cada decisión) en los doc-comments de cada migración; aquí solo el esquema y las
invariantes.

**mcp_write_audit** (`20260828140000_mcp_write_audit.sql`) — registro append-only de toda escritura
MCP, escrito desde `require_mcp_write` (`apps/api/src/mcp/auth.rs`): `id (uuid PK)`,
`at (timestamptz NOT NULL DEFAULT now())`, `installation_id (FK installation ON DELETE CASCADE)`,
`user_id (FK users ON DELETE CASCADE)`, `credential_kind (text NOT NULL, CHECK ∈ {api_token,
oauth})`, `credential_id (uuid NOT NULL — `api_tokens.id` u `oauth_access_tokens.id`, **sin FK a
propósito**: es polimórfico y el log tiene que sobrevivir a que la credencial se borre o caduque)`,
`role (text NOT NULL — rol VIVO en el momento de la llamada, no el de hoy)`, `tool (text NOT NULL)`,
`outcome (text NOT NULL, CHECK ∈ {attempted, ok, failed, denied})`, `error_code (text nullable —
solo el código estable, p.ej. `forbidden`; NUNCA el mensaje, que puede llevar texto escrito por la
persona)`, `target_ids (uuid[] NOT NULL DEFAULT '{}' — filas que la llamada mutó de verdad; vacío en
un preview)`, `settled_at (timestamptz nullable)`.
- **QUÉ NO SE GUARDA, y es la decisión central**: nunca los argumentos de la tool, ni en claro ni
  como digest. Los argumentos llevan contenido escrito por la persona (conceptos, notas, importes);
  guardarlos crearía un segundo domicilio para ese contenido fuera del `.ffbackup` cifrado, y al ser
  append-only convertiría el borrado del usuario en una mentira. Un digest tampoco vale: el espacio
  de entrada (fecha + importe + un concepto de vocabulario corto) es lo bastante pequeño para
  fuerza-bruta un SHA-256. El esquema es tipado sin JSONB ni texto libre **a propósito** — no cabe
  una frase que haya escrito una persona.
- **`CONSTRAINT mcp_write_audit_settled_shape CHECK ((settled_at IS NULL) = (outcome = 'attempted'))`**
  — la forma que hace el orden imposible de falsear: `attempted` nace con `settled_at NULL` y solo
  puede cerrarse una vez (write-once por `WHERE settled_at IS NULL` en el UPDATE de
  `McpWriteAudit::settle`); `denied` nace **ya cerrado** (el gate ES toda la operación). Un proceso
  que muere a mitad deja `attempted` + `settled_at IS NULL`, que es exactamente la verdad: se
  intentó, no se sabe cómo acabó. No hay otra vía de UPDATE ni de DELETE salvo la poda.
- **Retención 365 días** (`AUDIT_RETENTION_DAYS`, constante en `auth.rs`, no env var — mismo criterio
  que `MAX_ACTIVE_TOKENS_PER_USER`), podada de forma **perezosa dentro del propio camino de
  escritura** (después de cada INSERT de auditoría, nunca en un GET — D5). Autorregulado: una
  instalación parada no poda porque tampoco crece.
- Índice `mcp_write_audit_at_idx (at)`: sirve tanto la poda por rango como leer lo reciente.

**api_tokens.scope**: ver arriba (junto a `api_tokens`) y [`auth-and-membership.md`](auth-and-membership.md).

**transaction_idempotency_keys** (`20260828150000_transaction_idempotency_keys.sql`) — claves de
idempotencia opt-in del alta manual `POST /v1/transactions` (`handlers/transactions/idempotency.rs`):
`installation_id (FK installation ON DELETE CASCADE)`, `owner_user_id (FK users ON DELETE CASCADE)`,
`idempotency_key (text NOT NULL, CHECK 1..200 chars)`, `request_hash (text NOT NULL — SHA-256 del
cuerpo YA VALIDADO/normalizado, no del JSON crudo)`, `transaction_id (uuid NOT NULL FK transactions
ON DELETE CASCADE)`, `created_at (timestamptz NOT NULL DEFAULT now())`,
`PRIMARY KEY (installation_id, owner_user_id, idempotency_key)`.
- **Opt-in**: sin `idempotency_key` en el cuerpo esta tabla no se toca y el comportamiento es
  exactamente el de siempre (reenviar el mismo movimiento crea otro). Cambiar el default rompería un
  contrato ya publicado en la propia tool MCP.
- **Ámbito `(installation, owner_user_id)`**: la clave la elige el cliente, así que dos miembros
  pueden elegir la misma sin colisionar entre sí — con ámbito de instalación, la clave de Bob
  «reproduciría» el movimiento de Alice y le devolvería una fila ajena.
- **`request_hash` es del cuerpo normalizado**: dos peticiones que describen el mismo movimiento con
  distinta forma (`"10"` vs `"10.00"`) son el mismo reintento y se reproducen; el `money_out` fija la
  escala a 4 decimales antes de hashear.
- **Reclamada DENTRO de la misma transacción que el INSERT** del movimiento (`ON CONFLICT DO
  NOTHING`, nunca se mira el SQLSTATE 23505 a mano — el mapeo central vive en `error.rs`, I10): o
  existen las dos filas o ninguna, lo que resuelve la carrera de dos reintentos simultáneos sin
  dejar el duplicado que esta tabla existe para evitar. El perdedor de la carrera hace un segundo
  `lookup`: si encuentra la fila del ganador, reproduce su respuesta (mismo camino que el replay
  normal); si no la encuentra —la única forma es que la fila del ganador se borrara entre medias—
  devuelve **409 `idempotency_key_in_flight`** en vez de inventar un desenlace: ni es un duplicado
  ni un éxito, así que el cliente debe reintentar sin más.
- **`ON DELETE CASCADE` hacia `transactions`**: borrar el movimiento libera la clave — reintentar
  después vuelve a crear, correctamente, porque borrar es una intención posterior y explícita.
- **Retención 24 h**, poda perezosa dentro del propio `POST /v1/transactions` (D5). La ventana útil
  de una clave son segundos (protege un reintento en vuelo); 24 h es tres órdenes de magnitud de
  margen.
- **`POST /v1/transactions/batch`: clave POR ÍTEM rechazada, clave DEL LOTE aceptada desde 4.4.0
  (Fase 6, issue #87).** La Fase 3 rechazaba la idempotencia de lote entera con
  `idempotency_key_batch_unsupported` porque «reproducir parcialmente» no tiene semántica. El
  razonamiento que la reabre no la contradice, la afina: **el lote es UNA unidad atómica, así que
  lleva UNA clave**, en la **raíz** del body (1..180 chars — 20 menos que los 200 del alta
  individual, para el sufijo derivado). Una clave por ítem sigue siendo 400, y ahora el mensaje dice
  dónde ponerla. **Sin tabla ni columna nueva**: como esta tabla guarda UN `transaction_id` por fila
  y un lote crea N, se escriben **N filas con clave derivada `{clave}#b{i}`**, todas con el
  `request_hash` del **lote entero** (marcador `batch-v1` + nº de ítems + los ítems ya validados, en
  orden — así cambiar el orden o el número de ítems mueve la huella). El replay sondea el ancla
  `{clave}#b0` y exige que **las N** filas existan con ese hash: si falta alguna (un movimiento
  borrado — la FK es `ON DELETE CASCADE`) el resultado es un **409 ruidoso**, nunca medio lote. Las
  N claves se reclaman en la MISMA transacción que los N INSERT, así que «3 de 5» no puede ocurrir.
  Regresión: `apps/api/tests/transactions_batch_idempotency.rs`.
- Índice `transaction_idempotency_keys_created_idx (created_at)`.

**mcp_confirm_tokens** (`20260828160000_mcp_confirm_tokens.sql`) — confirmación en dos fases de las
escrituras MCP irreversibles (`apps/api/src/confirm_token.rs`, `pub` fuera de `mcp/` porque no hay
camino HTTP con dos fases que la comparta): `token_hash (text PK — SHA-256 hex del secreto `ffpv_…`,
el secreto viaja una única vez, en la respuesta del preview)`, `installation_id (FK installation ON
DELETE CASCADE)`, `user_id (FK users ON DELETE CASCADE — el token es de quien previsualizó; otro
miembro no puede confirmar tu borrado con tu token)`, `tool (text NOT NULL — un token de
`delete_import` no confirma un `delete_asset`)`, `args_hash (text NOT NULL — SHA-256 de los
argumentos normalizados del preview)`, `effects_hash (text NOT NULL — SHA-256 del bloque `effects`
que el preview publicó)`, `created_at`, `expires_at (NOT NULL — TTL 10 min)`,
`consumed_at (timestamptz nullable — un solo uso)`.
- **Por qué existe**: `confirm: true` es un booleano del propio esquema de la tool, así que el
  modelo puede escribirlo en la PRIMERA llamada — sin este token, `confirm` nunca fue un control de
  dos fases real, solo *prompting*. La confirmación exige el token que **solo** el preview emite.
- **Ligado a la huella de los EFECTOS, no solo a la tool y los argumentos**: si entre el preview y el
  confirm el mundo se movió (el lote creció, el pasivo ganó movimientos vinculados), la huella
  recalculada en la confirmación no casa y `confirm_token_stale` — la ventana que un `confirm`
  booleano no podía ni ver.
- **Digest con orden de claves canónico** (`confirm_token::digest`, claves de objeto ordenadas a
  todos los niveles + longitud delante de cada string): un cambio de dependencia o de estilo en el
  `json!` que construye `effects` no puede mover la huella y producir un `confirm_token_stale`
  intermitente sobre efectos idénticos.
- **Un solo uso, TTL 10 min**: precedente exacto `oauth_authorization_codes` (`consumed_at` marcado
  dentro del mismo UPDATE que valida — el consumo es atómico, dos confirmaciones simultáneas no
  pueden ganar las dos). El TTL es 5× el de un código OAuth (2 min) porque aquí hay una persona
  leyendo un preview en un chat, no una máquina respondiendo al instante.
- **Solo hashes, nunca argumentos/efectos en claro**: a diferencia de `mcp_write_audit` (donde un
  digest sería fuerza-brutable por baja entropía), aquí el hash no es una medida de privacidad sino
  de igualdad — la fila vive 10 minutos y se poda. Se hashea de todos modos porque una tabla
  operativa no tiene por qué contener el concepto de un movimiento ni el nombre de un activo.
- **Solo 8 de las 17 tools con preview lo exigen** (7 de 14 hasta la Fase 6) — las de cascada de
  tamaño no acotado (`delete_import`, `delete_asset`, `delete_liability`,
  `apply_categorization_rule`, `materialize_recurring`) y las puertas de un solo sentido
  (`unreconcile_transfer`, `delete_snapshot` y, desde 4.4.0, `delete_allocation_rule` — recrear la
  regla no restaura su prioridad, y mientras tanto TODO el sobrante mensual se ha ido por otro
  sitio). Los borrados de una fila cuyo contenido íntegro viaja en el preview no lo
  piden: el agente puede recrearlos desde su propio contexto, y encarecer cada borrado trivial a dos
  viajes es la forma más rápida de que la ceremonia se lea como ruido. Detalle por tool:
  [`api-routes.md`](api-routes.md) §MCP.
- **Poda perezosa en la emisión** (`gc_expired`, antes de cada `issue`, D5), **estricta al emitir**:
  si el INSERT falla, el preview entero falla — prometer un token que no existe dejaría al llamante
  en un bucle de `confirm_token_invalid` sin explicación.
- Índice `mcp_confirm_tokens_expires_at_idx (expires_at)`.

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
  ],
  "savings_source": "budget|transactions_avg|budget_income_real_expense",
  "income_avg_window_months": 3, "income_avg_window_mode": "data|calendar",
  "expense_avg_window_months": 12, "expense_avg_window_mode": "data|calendar"
}
```
Defaults (Spain): SWR 3.5%, 5-bracket capital gains schedule (IRPF). Last bracket must have `up_to: null`.
`fire_settings` is nullable; when null, defaults apply on read (handler calls `resolve_fire_settings`).

**`savings_source`** (`SavingsSource` enum, default `budget`) — fuente del ahorro mensual de la simulación FIRE, **tres modos**:
- `budget` (modo A, presupuesto — histórico): budget entries + cuota derivada de pasivos time-limited (el engine cobra el debt service y amortiza el principal).
- `transactions_avg` (modo B): income y gasto del promedio ponderado real 12m de las transacciones, **crudo** (reforma 3.4.0: las cuotas de pasivo ya viven dentro de los movimientos; los pasivos no tocan la caja de la simulación y solo restan su principal pendiente al NW, constante en todo el horizonte — `monthly_payment` se anula en memoria antes de entrar al engine).
- `budget_income_real_expense` (modo C): income del **presupuesto** + gasto **real** (mismo promedio crudo que B, mismo contrato de pasivos). Target FIRE `annual_expense` usa el gasto real, `current_income` usa el income del presupuesto.

El promedio 12m que alimenta el engine cuenta solo **meses reales** (≥1 transacción `recurring_rule_id IS NULL`); meses solo-recurrentes se excluyen por completo (ver §Transactions en `api-routes.md`). Aditivo, **sin migración** (`FireSettings` tiene `#[serde(default)]` a nivel struct, así que un JSONB sin el campo → `budget`; backups viejos siguen cargando). En B y C las **transacciones se vuelven input del engine** (gate `SavingsSource::uses_transactions()`; ver nota en §Transactions). Semántica completa en `futurefin-fire-domain-reference` y `futurefin-config-and-flags`.

**Deserialization is strict**: `fire_number_mode` only accepts `manual | annual_expense | current_income`; `savings_source` only `budget | transactions_avg | budget_income_real_expense` (unknown → 422, like `FireNumberMode`; el error lista las tres variantes válidas). The legacy alias `annual_expense_adjusted` is mapped to `annual_expense` for backwards-compat with old backups, but any other value returns 422 (was silently coerced to default before May 2026). The field `fire_number_expense_adjustment_pct` was removed — it had no consumer.

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
- **Forward compat**: each payload variant lives behind `BackupPayloadVN` + a `migrate_to_current` chain. Backups with `schema_version > CURRENT_SCHEMA_VERSION` are rejected with `409` and a clear error. La fuente de verdad es la constante `CURRENT_SCHEMA_VERSION` en `apps/api/src/handlers/backup_user/schema.rs` — compruébala con `grep -n 'CURRENT_SCHEMA_VERSION' apps/api/src/handlers/backup_user/schema.rs` antes de citar un número aquí (esta línea dijo `= 7` mucho después de que el código fuera por el 9).
  - **`CURRENT_SCHEMA_VERSION = 11`** (4.7.0, #129): `BackupPayloadV11` = V10 con los snapshots como `BackupSnapshotV11` (alias actual `BackupSnapshot`), cuyo item añade `repayment_model?`. `payload_v10_to_v11` mapea cada item con `None` («la foto no lo sabía» ⇒ ley lineal al interpolar). El bump existe EN VEZ de un campo aditivo porque `parse_payload` rechaza versiones futuras: es lo que impide que un servidor 4.6.0 lea a medias un backup 4.7.0 y pierda el modelo en silencio. Además el import de CUALQUIER versión ≤ v10 aplica la normalización firmada de #144 a los pasivos (fixed+TIN+mensual → french; residuo pierde el TIN; revolving sin mínimos → backfill pct 0 / suelo = cuota), y `BackupLiabilityV10` ganó `min_payment_pct?`/`min_payment_eur?` como campos ADITIVOS sin bump (patrón `expense_category_ref`).
  - **`CURRENT_SCHEMA_VERSION = 10`** (4.2.0, verificado 2026-08-25): `BackupPayloadV10` = V9 con los pasivos como `BackupLiabilityV10`, que añade `repayment_model`. `payload_v9_to_v10` es una copia literal que rellena el campo con **`fixed_payments`** en cada pasivo. `BackupLiability` es un alias que apunta siempre a la variante actual.
  - **Un `.ffbackup` v10 NO importa en ≤ 4.1.0**: la versión anterior rechaza con `409` cualquier `schema_version` por encima de la suya. Es el contrato de siempre (falla ruidosamente en vez de tragarse un payload que no entiende), pero conviene decirlo al actualizar: exporta *antes* de subir si quieres un backup que la imagen vieja pueda leer.
  - Historia previa de la cadena: v7 (3.2.0) = V6 **menos** el `day_of_month` de cada regla recurrente (`payload_v6_to_v7` lo descarta — las reglas pasaron a resolución mensual); v6 (v1.8.0) había añadido `recurring_transaction_rules` + `BackupTransaction.recurring_rule_index` sobre v5. La cadena v1→…→v9→v10 sigue intacta.
- **History snapshots (v4; item ampliado en v11)**: `BackupSnapshot {kind, snapshot_date, source, items}`; `BackupSnapshotItem {ledger_index: Option<usize>, item_key: Uuid, label, value, apr_percent?, payment_amount?, payment_frequency?, repayment_model? (v11)}` — los payloads v4..v10 congelan la forma sin modelo (`BackupSnapshotV10`/`BackupSnapshotItemV10`).
  - **`item_key`** = the original `source_item_id`, **always present**.
  - **`ledger_index`** = position of the referenced row in **this payload's** `assets` vec (kind=asset) or `liabilities` vec (kind=liability), set **only** when that ledger row still existed at export (miss → `None`).
  - **Re-link on import**: `ledger_index: Some(i)` → the re-inserted item's `source_item_id` becomes the **fresh UUID** of the ledger row re-created at index `i` (preserves cross-snapshot linkage *and* the join-to-today at month 0 that `GET /v1/history/series` relies on). `None` → `item_key` kept **verbatim** (deleted rows / free-form backfill items stay linked to each other). An out-of-bounds `ledger_index` → `400 BadRequest` and the whole import rolls back. Export builds the indices from `fetch_assets`/`fetch_liabilities` (both now return an `id → index` map); `fetch_snapshots` assembles the items.
- **Transactions (v5)**: `BackupTransactionImport {source, account_asset_index?, original_filename?}`, `BackupTransaction {import_index?, source, op_date, value_date?, concept, amount, currency, kind?, category_ref?, fingerprint_ordinal, linked_asset_index?, linked_liability_index?, notes?, recurring_rule_index? (v6), transfer_counterpart_index?/transfer_reconciled_at?/transfer_reconciled_source? (v8)}`, `BackupCategorizationRule {match_kind, pattern, source?, assign_kind?, assign_category_ref?}`. **Refs por índice** en los vecs de este payload: `account_asset_index`/`linked_asset_index` → `assets`, `linked_liability_index` → `liabilities`, `import_index` → `transaction_imports` (`None` = manual/efectivo), `recurring_rule_index` → `recurring_transaction_rules` (`None` = movimiento suelto). Las FK metadata (`account_asset_index`/`linked_*`) van `None` cuando la fila de ledger ya no existía al exportar (son `ON DELETE SET NULL`). **La `fingerprint` NO se exporta** — se **recomputa al importar** (`compute_fingerprint` sobre source·op_date·amount·concepto); solo se lleva `fingerprint_ordinal` para preservar el ordinal de dedup de ocurrencias repetidas. Categorías por `category_ref (scope, name)` como el resto del payload. **`categorization_rules` inserta con `ON CONFLICT DO NOTHING` desde 4.4.0** (issue #82): antes de la constraint parcial `categorization_rules_unique_agnostic` (ver §`categorization_rules` arriba) una instalación pudo acumular reglas agnósticas duplicadas y exportarlas en su `.ffbackup`; sin la cláusula, reimportar ese fichero daría `23505` en el segundo INSERT → 409, y el archivo pasaría a ser **inimportable** — rompiendo el no-negociable «un backup antiguo nunca deja de importar, es la única vía de recuperación del usuario». La copia duplicada se descarta silenciosamente y `ImportCounts.categorization_rules` cuenta lo que **entró**, no lo que traía el payload. Test: `backup_user_roundtrip.rs::a_legacy_backup_with_duplicate_agnostic_rules_still_imports`.
- **Recurring rules (v6, sin `day_of_month` desde v7)**: `BackupRecurringRule {concept, amount, kind, category_ref?, linked_asset_index?, linked_liability_index?, notes?, origin_month}` (**v9**; la variante ≤v8, `BackupRecurringRuleV8`, lleva `last_materialized_month` y `payload_v8_to_v9` la ancla en la instancia MÁS ANTIGUA del payload — el cursor iba por delante del origen, así que copiarlo tal cual impediría materializar los meses intermedios) (la variante v6, `BackupRecurringRuleV6`, aún lleva `day_of_month`; `payload_v6_to_v7` lo descarta al importar). Categoría por `category_ref (scope, name)`; `linked_*_index` a los vecs `assets`/`liabilities` (`None` cuando la FK ya estaba en `SET NULL` al exportar). La idempotencia de una re-materialización tras el import es **por existencia de instancia** (desde 3.9.0), no por cursor: el antiguo `last_materialized_month` solo sobrevive dentro de la variante ≤v8, y `payload_v8_to_v9` lo convierte en el ancla `origin_month` como se describe arriba.
- **Conciliación de transferencias (v8)**: además de los tres campos v8 de `BackupTransaction` (arriba), el payload lleva `transfer_match_rejections: [{transaction_a_index, transaction_b_index}]` — los pares **rechazados** al desconciliar a mano, por índice en el vec `transactions` del propio payload. **Sin exportar los rechazos, un restore los resucitaría** en el primer pase de auto-conciliación post-import (es su única razón de ser, igual que en la tabla viva). `transfer_counterpart_index` es **simétrico** (ambas patas se apuntan); al importar, las transacciones se insertan primero con `transfer_counterpart_id` NULL y una **segunda pasada** enlaza las parejas con los UUIDs frescos (índice fuera de rango → 400 `backup_reference_out_of_range`). Un payload ≤v7 deserializa con conciliación vacía (`#[serde(default)]`; test «v7→v8 must default transfer_match_rejections to empty» en `schema.rs`). Con esto, las **cinco** tablas del histórico de gasto (`transaction_imports`, `transactions`, `categorization_rules`, `recurring_transaction_rules`, `transfer_match_rejections`) viajan en el `.ffbackup`.
- **Import semantics**: replace-only. Las tablas user-scoped se vacían (`WHERE installation_id = $1 AND owner_user_id = $2`) y se reinsertan con UUIDs frescos en la misma transacción. Borrado (para respetar las FK): `history_snapshots` (sus items en cascada), `transactions` (anulando primero el self-FK `transfer_counterpart_id` para un orden determinista; las `transfer_match_rejections` caen por `ON DELETE CASCADE`), `transaction_imports`, `categorization_rules`, `recurring_transaction_rules` (tras las transactions, que ya limpiaron su `recurring_rule_id`, y antes de assets/liabilities para que sus FK `SET NULL` no disparen a mitad del wipe), `allocation_rules`, `assets`, `liabilities`, `budget_entries`, `planning_flows`. Reinserción: primero `categories`/`assets`/`allocation_rules`/`liabilities`/`budget_entries`/`planning_flows`, luego `history_snapshots` (necesitan los UUIDs frescos de assets/liabilities para el re-link) y por último `transaction_imports` → `recurring_transaction_rules` → `transactions` → `categorization_rules` (resuelven sus refs por índice a las filas recién insertadas — las `transactions` necesitan que las reglas recurrentes ya existan para su `recurring_rule_index`). `users.birth_date` is updated if the backup differs. Tras el `commit`, el import invalida la cache de proyección (`refresh_projection_after_mutation`) — antes de v1.5.0 no lo hacía y la proyección quedaba stale hasta 60 min. `ImportCounts` incluye `snapshots`/`snapshot_items` (v4), `transaction_imports`/`transactions`/`categorization_rules` (v5) y `recurring_transaction_rules` (v6), visibles en preview y apply.
