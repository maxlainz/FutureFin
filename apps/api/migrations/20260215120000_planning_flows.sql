-- Upcoming / planned cash flows (Planning tab parity); category must be income or expense.

CREATE TABLE planning_flows (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (
        char_length(trim(title)) > 0
        AND char_length(title) <= 200
    ),
    expected_amount NUMERIC(18, 4) NOT NULL CHECK (expected_amount > 0),
    due_date DATE NULL,
    notes TEXT CHECK (
        notes IS NULL
        OR char_length(notes) <= 4000
    ),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX planning_flows_installation_sort_idx ON planning_flows (
    installation_id,
    sort_index ASC,
    title ASC
);

CREATE INDEX planning_flows_installation_category_idx ON planning_flows (
    installation_id,
    category_id
);
