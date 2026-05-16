-- Drop columns moved to allocation_rules.
-- See 20260519120000_allocation_rules.sql for the replacement model.

ALTER TABLE assets
    DROP COLUMN IF EXISTS monthly_contribution_fixed,
    DROP COLUMN IF EXISTS contribution_remainder_weight,
    DROP COLUMN IF EXISTS contribution_frequency,
    DROP COLUMN IF EXISTS contribution_cap_kind,
    DROP COLUMN IF EXISTS contribution_cap_value;
