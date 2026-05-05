ALTER TABLE installation
    ADD COLUMN fire_settings JSONB;

ALTER TABLE users
    ADD COLUMN pension_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN pension_start_age SMALLINT,
    ADD COLUMN pension_annual_net NUMERIC(18, 2);

ALTER TABLE users
    ADD CONSTRAINT users_pension_age_range CHECK (
        pension_start_age IS NULL
        OR pension_start_age BETWEEN 50 AND 90
    );

ALTER TABLE users
    ADD CONSTRAINT users_pension_amount_nonneg CHECK (
        pension_annual_net IS NULL
        OR pension_annual_net >= 0
    );
