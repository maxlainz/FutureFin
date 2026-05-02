-- Recurring budget lines (income / expense categories only); liability-derived lines are computed on read.

CREATE TABLE budget_entries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    label TEXT CHECK (
        label IS NULL
        OR (
            char_length(label) <= 200
            AND char_length(trim(label)) > 0
        )
    ),
    amount NUMERIC(18, 4) NOT NULL,
    frequency TEXT NOT NULL DEFAULT 'monthly' CHECK (frequency IN ('monthly', 'weekly')),
    notes TEXT CHECK (
        notes IS NULL
        OR char_length(notes) <= 4000
    ),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX budget_entries_installation_sort_idx ON budget_entries (
    installation_id,
    sort_index ASC,
    label ASC
);

CREATE INDEX budget_entries_installation_category_idx ON budget_entries (
    installation_id,
    category_id
);
