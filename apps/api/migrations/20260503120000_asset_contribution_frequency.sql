-- Recurring asset contributions may be weekly (×52/12 monthly equivalent in projection).

ALTER TABLE assets
ADD COLUMN contribution_frequency TEXT NOT NULL DEFAULT 'monthly';

ALTER TABLE assets
ADD CONSTRAINT assets_contribution_frequency_chk CHECK (
    contribution_frequency IN ('monthly', 'weekly')
);
