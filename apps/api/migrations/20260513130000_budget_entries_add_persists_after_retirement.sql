ALTER TABLE budget_entries
    ADD COLUMN persists_after_retirement BOOLEAN NOT NULL DEFAULT FALSE;
