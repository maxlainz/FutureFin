-- Projection engine inputs on assets; household FIRE JSON + optional inflation % on installation.

ALTER TABLE assets
ADD COLUMN expected_annual_return_percent NUMERIC(18, 6),
ADD COLUMN monthly_contribution_fixed NUMERIC(18, 4) NOT NULL DEFAULT 0,
ADD COLUMN contribution_remainder_weight NUMERIC(18, 6) NOT NULL DEFAULT 0;

ALTER TABLE assets
ADD CONSTRAINT assets_contribution_remainder_weight_nonneg CHECK (
    contribution_remainder_weight >= 0
);

ALTER TABLE installation
ADD COLUMN annual_inflation_assumption_percent NUMERIC(18, 6),
ADD COLUMN fire_settings JSONB NOT NULL DEFAULT '{}'::jsonb;
