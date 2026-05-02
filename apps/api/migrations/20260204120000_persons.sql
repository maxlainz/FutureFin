CREATE TABLE persons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    household_id UUID NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    birth_date DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now (),
    CONSTRAINT persons_display_name_len CHECK (
        char_length(display_name) BETWEEN 1 AND 128
    )
);

CREATE INDEX persons_household_id_idx ON persons (household_id);

CREATE UNIQUE INDEX persons_one_primary_per_household ON persons (household_id)
WHERE
    is_primary;
