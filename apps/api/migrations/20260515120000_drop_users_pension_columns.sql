ALTER TABLE users DROP CONSTRAINT IF EXISTS users_pension_age_range;
ALTER TABLE users DROP CONSTRAINT IF EXISTS users_pension_amount_nonneg;
ALTER TABLE users DROP COLUMN IF EXISTS pension_enabled;
ALTER TABLE users DROP COLUMN IF EXISTS pension_start_age;
ALTER TABLE users DROP COLUMN IF EXISTS pension_annual_net;
