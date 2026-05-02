-- Household persons (FIRE ages, primary member); reintroduced after legacy drop — see PARITY_CHECKLIST Settings.

CREATE TABLE persons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    display_name TEXT NOT NULL CHECK (
        char_length(trim(display_name)) > 0
        AND char_length(display_name) <= 128
    ),
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    birth_date DATE,
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX persons_installation_sort_idx ON persons (
    installation_id,
    sort_index ASC,
    display_name ASC
);

CREATE UNIQUE INDEX persons_one_primary_per_installation ON persons (installation_id)
WHERE
    is_primary;
