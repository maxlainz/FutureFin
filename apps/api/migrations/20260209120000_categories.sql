-- Categories per installation (asset / liability / income / expense scopes).
-- No server-side seeding; clients create categories as needed.

CREATE TABLE categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (
        scope IN (
            'asset',
            'liability',
            'income',
            'expense'
        )
    ),
    name TEXT NOT NULL CHECK (
        char_length(name) <= 200
        AND char_length(trim(name)) > 0
    ),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (installation_id, scope, name)
);

CREATE INDEX categories_installation_scope_idx ON categories (installation_id, scope);

CREATE INDEX categories_installation_sort_idx ON categories (
    installation_id,
    scope,
    sort_index ASC,
    name ASC
);
