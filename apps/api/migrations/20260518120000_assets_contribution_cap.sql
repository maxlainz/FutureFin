-- Per-asset contribution cap: optional limit for automatic contributions.
-- Two columns (must be both NULL or both NOT NULL).
--   contribution_cap_kind: 'amount' (fixed €) | 'months_expense' (N × monthly expense incl. debt service)
--   contribution_cap_value: NUMERIC(18,4), >= 0 when set

ALTER TABLE assets
    ADD COLUMN IF NOT EXISTS contribution_cap_kind TEXT NULL,
    ADD COLUMN IF NOT EXISTS contribution_cap_value NUMERIC(18, 4) NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'assets_contribution_cap_kind_chk'
    ) THEN
        ALTER TABLE assets
            ADD CONSTRAINT assets_contribution_cap_kind_chk
            CHECK (contribution_cap_kind IS NULL
                   OR contribution_cap_kind IN ('amount', 'months_expense'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'assets_contribution_cap_pair_chk'
    ) THEN
        ALTER TABLE assets
            ADD CONSTRAINT assets_contribution_cap_pair_chk
            CHECK ((contribution_cap_kind IS NULL AND contribution_cap_value IS NULL)
                OR (contribution_cap_kind IS NOT NULL
                    AND contribution_cap_value IS NOT NULL
                    AND contribution_cap_value >= 0));
    END IF;
END $$;
