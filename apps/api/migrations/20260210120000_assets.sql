-- Asset ledger rows (installation-scoped). Category must reference `categories.scope = 'asset'` (enforced in API).

CREATE TABLE assets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (
        char_length(name) <= 200
        AND char_length(trim(name)) > 0
    ),
    current_value NUMERIC(18, 4) NOT NULL DEFAULT 0,
    purchase_price NUMERIC(18, 4),
    is_liquid BOOLEAN NOT NULL DEFAULT true,
    notes TEXT CHECK (
        notes IS NULL
        OR char_length(notes) <= 4000
    ),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX assets_installation_sort_idx ON assets (
    installation_id,
    sort_index ASC,
    name ASC
);

CREATE INDEX assets_installation_category_idx ON assets (installation_id, category_id);
