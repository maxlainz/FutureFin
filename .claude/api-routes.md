# API Route Map

All routes in `apps/api/src/routes/mod.rs`. Routes under `/v1/` require valid session cookie `ff_session` unless noted.

## Top-level (no auth required)
| Method | Path | Handler |
|--------|------|---------|
| GET | `/health` | `health::health_check` |
| GET | `/openapi.json` | `openapi::openapi_json` |

## /v1 routes

### Health
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/health` | No auth |
| GET | `/v1/ready` | DB ping |

### Auth (`/v1/auth/`)
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/auth/register` | No prior session needed. First user auto-becomes installation owner. |
| POST | `/v1/auth/login` | Sets `ff_session` cookie |
| POST | `/v1/auth/logout` | Clears cookie + DB session |
| GET | `/v1/auth/me` | Current user info |
| PATCH | `/v1/auth/me` | Update `birth_date` |

### Installation
| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/installation/session-context` | Returns `{installation_initialized, access}` — used for routing the UI gate |
| GET | `/v1/installation` | Own membership + installation snapshot |
| PATCH | `/v1/installation` | Owner only. Updates tz, inflation, target_age, show_age_mode, fire_settings |
| POST | `/v1/installation/setup` | Creates singleton installation (409 if exists) |

### Pending users (`/v1/installation/pending-users/`)
Owner-only management of users awaiting approval.

### Categories (`/v1/categories/`)
Scopes: `asset`, `liability`, `income`, `expense`. Per-installation.

### Assets (`/v1/assets/`)
Accepts `?view=mine` to filter by `owner_user_id`. The asset record no longer carries contribution fields — those live in `/v1/allocation-rules/`.

### Allocation rules (`/v1/allocation-rules/`)
Cascade rules that route the monthly surplus (`income − expense − debt_service`) into assets, in priority order. Accepts `?view=mine` to scope by `owner_user_id`.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/v1/allocation-rules` | List ordered by `priority ASC`. |
| POST | `/v1/allocation-rules` | Body: `{target_asset_id, kind, amount?, cap_kind?, cap_value?, notes?, enabled?}`. Auto-assigns the next `priority` in the scope. |
| PATCH | `/v1/allocation-rules/{id}` | Updates fields. `amount` accepts a string, number, or `null` (only for `remainder`). `cap` accepts `{kind, value}` or `null`. Does **not** change `priority`. Rejects with 400 + `remainder_required` if it would orphan the scope. |
| DELETE | `/v1/allocation-rules/{id}` | Returns 400 `remainder_required` if deleting the last `remainder` rule in scope. |
| POST | `/v1/allocation-rules/reorder` | Body: `{ids: [uuid,...]}`. Must list exactly the rules in the current scope; reassigns `priority` 1..N in the given order in one transaction. |

Rule kinds:
- `fixed` — €/mes; `amount` required, ≥ 0.
- `percent` — `amount` ∈ [0, 100], applied to the **surplus remaining at this step** (cascade pure).
- `remainder` — `amount` ignored (must be NULL). At least one per scope is enforced server-side.

Cap kinds (all optional; `cap_kind`/`cap_value` are paired):
- `amount` — absolute € target value for the destination asset.
- `months_expense` — N × (monthly expense + debt service); evaluated per-month against current state.
- `income_multiple` — N × monthly income.

### Liabilities (`/v1/liabilities/`)
Accepts `?view=mine`. `principal_derived_from_plan` flag indicates auto-derived principal from planning flows.

**Expiration filter**: rows with `payment_end_date < today` are hidden from `GET /v1/liabilities` (and from totals/breakdowns in `/summary`, derived lines in `/budget`, debt service in `/projection`). The rows are not deleted — the filter is `WHERE (payment_end_date IS NULL OR payment_end_date >= $today)`. Use `installation.calendar_tz` to compute `today`.

### Summary (`/v1/summary/`)
Aggregated net worth, financial health metrics, category breakdowns. Accepts `?view=mine`. `total_liabilities` and breakdowns exclude expired rows (see Liabilities note above).

### Budget (`/v1/budget/`)
Income/expense entries + derived lines from liabilities. Accepts `?view=mine`. Derived lines only show liabilities with `payment_end_date > today`.

### Planning (`/v1/planning/`)
Upcoming cash flows (one-off inflows/outflows) with due dates.

### Projection (`/v1/projection/`)
Net-worth series via `futurefin-engine`. Accepts `?view=mine` and `?months=N` (12–840).

Response (`ProjectionSeriesResponse`) includes:
- `points[]` — `{month_index, net_worth, contributed_capital}` for months 0..=N
- `months`, `horizon_years`, `horizon_basis` — effective horizon (`mac_target_age` | `mac_fallback_no_demographics` | `months_override`)
- `starting_net_worth`, `monthly_delta_assumption` — snapshot values at month 0
- `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date` — UI axis helpers
- `milestones[]` — next 3 net-worth milestones (1/2.5/5×10ⁿ thresholds), each with `target`, `reached_month_index`, `reached_date_ymd`
- `compound_outpaces_true_savings_month_index` — first month where compound return > base monthly savings (optional)

### Backup
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/backup/user-export` | Returns `.ffbackup` binary for the **current user only**. Body: `{password, ui_preferences?}`. Encrypted with the user's account password (Argon2id KDF → AES-256-GCM). Any role. |
| POST | `/v1/backup/user-import/preview` | Body: `{file_b64, password}`. Returns counts of what would be imported. Write role required. |
| POST | `/v1/backup/user-import` | Body: `{file_b64, password, confirm_replace: true}`. **Destructive**: replaces all `owner_user_id = current_user` rows in `assets/liabilities/budget_entries/planning_flows` in a transaction. Write role required. |

The `.ffbackup` format is a versioned, encrypted binary container — see [`backup_user/crypto.rs`](../apps/api/src/handlers/backup_user/crypto.rs) for the frame layout and [`backup_user/schema.rs`](../apps/api/src/handlers/backup_user/schema.rs) for the payload schema + migration layer (`schema_version`).

## Auth pattern in handlers

Every protected handler calls:
```rust
let user = require_session_user(&jar, &state.pool).await?;
let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
// For write ops:
if !role_can_write(role.as_str()) { return Err(ApiError::Forbidden); }
```

## View-scoping pattern

For any endpoint that accepts `?view=mine`, **do not** write two `match view { Household => sqlx::query_as("…installation_id = $1…"), Mine => sqlx::query_as("…installation_id = $1 AND owner_user_id = $2…") }` branches. Use the helpers in `handlers/person_view.rs`:

```rust
let view = q.resolve(); // Query<LedgerViewQuery>
let scope = view.scope_where("a"); // table alias optional; "" = no prefix
let today_ph = view.next_arg_index(); // 2 (Household) or 3 (Mine)
let sql = format!(
    "SELECT ... FROM assets a WHERE {scope} AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph}) ORDER BY ...",
);
let rows: Vec<MyRow> = view
    .bind_scope_as(sqlx::query_as(&sql), iid, user.id.0)
    .bind(today)
    .fetch_all(pool)
    .await?;
```

For `sqlx::query_scalar`, use `bind_scope_scalar` instead. The helpers guarantee placeholder order ($1=iid, optional $2=owner_user_id) so the two branches can never drift.

## Error mapping

`impl From<sqlx::Error> for ApiError` (in `error.rs`) auto-detects:
- `23505` (unique_violation) → `ApiError::Conflict` (409)
- `23503` (foreign_key_violation) → `ApiError::BadRequest("referenced record missing")`

Handlers should just `?` any `sqlx::Error`; never write per-call `.map_err(...)` to translate codes.
