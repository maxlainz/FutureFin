ALTER TABLE budget_entries
    ADD COLUMN ends_at_retirement  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN expense_end_date    DATE    NULL;
