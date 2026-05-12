# Data Model & DB Schema

Migrations in `apps/api/migrations/`. SQLx embeds and runs them on startup.

## Core tables (chronological migration order)

### Users & Sessions
- `users`: `id (uuid PK)`, `username (unique)`, `password_hash`, `birth_date (date nullable)`, `created_at`
- `sessions`: `id (uuid PK)`, `user_id (FK users)`, `expires_at`, `created_at`

### Installation (singleton)
- `installation`: `id (uuid PK)`, `base_currency (char 3)`, `calendar_tz (text)`, `projection_includes_inflation (bool)`, `annual_inflation_assumption_percent (decimal nullable)`, `projection_target_age (smallint nullable)`, `show_age_mode (text: 'dates'|'ages')`, `fire_settings (jsonb nullable)`, `created_at`
  - Singleton: only one row ever exists. First user auto-creates it on register.
  
- `installation_memberships`: `installation_id (FK)`, `user_id (FK)`, `role (text: 'owner'|'member'|'viewer')`, `created_at`

### Ledger entities
All financial tables have `installation_id (FK)` and `owner_user_id (uuid nullable FK users)`.
- `owner_user_id = NULL` → household-level row (legacy or shared)
- `owner_user_id = user.id` → attributed to specific user (`?view=mine` filter)

**categories**: `id`, `installation_id`, `scope ('asset'|'liability'|'income'|'expense')`, `name`, `sort_index`

**assets**: `id`, `installation_id`, `owner_user_id`, `category_id`, `name`, `current_value (decimal)`, `purchase_price (decimal nullable)`, `is_liquid (bool)`, `expected_annual_return_percent (decimal nullable)`, `monthly_contribution_fixed (decimal)`, `contribution_remainder_weight (decimal)`, `contribution_frequency ('monthly'|'weekly')`, `notes`, `sort_index`

**liabilities**: `id`, `installation_id`, `owner_user_id`, `category_id`, `label`, `type_tag`, `principal (decimal)`, `apr_percent (decimal nullable)`, `payment_amount (decimal nullable)`, `payment_frequency ('monthly'|'weekly' nullable)`, `payment_end_date (date nullable)`, `principal_derived_from_plan (bool)`, `notes`, `sort_index`

**budget_entries**: `id`, `installation_id`, `owner_user_id`, `category_id`, `scope ('income'|'expense')`, `amount (decimal, monthly)`, `notes`, `sort_index`

**planning_flows**: `id`, `installation_id`, `owner_user_id`, `category_id`, `direction ('inflow'|'outflow')`, `title`, `expected_amount (decimal)`, `due_date (date nullable)`, `notes`, `sort_index`

## FIRE settings (JSONB in installation.fire_settings)
```json
{
  "fire_number_mode": "annual_expense|current_income|manual",
  "fire_number_manual_amount": "decimal string or null",
  "fire_number_expense_adjustment_pct": "decimal string or null",
  "swr_pct": "3.5",
  "taxes_enabled": true,
  "tax_brackets": [
    { "up_to": "6000", "pct": "19" },
    { "up_to": null, "pct": "30" }
  ]
}
```
Defaults (Spain): SWR 3.5%, tax brackets for capital gains (Spanish IRPF schedule). Last bracket must have `up_to: null`.

## Key invariants
- `Decimal` for all monetary/percentage columns — never `f64` in schema or Rust code
- `calendar_tz` validated as IANA timezone via `chrono_tz`
- `base_currency` validated as 3-letter code, MVP supports EUR/USD/GBP only
- `swr_pct` bounded 0–4 (percent, not ratio)
- `projection_target_age` bounded 65–105 when set
