-- Liability ledger; category must be scope `liability` (API-enforced).
-- Rows with payment_end_date before today are purged on list when the caller may write (parity Mac startup purge).

CREATE TABLE liabilities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    label TEXT NOT NULL CHECK (
        char_length(label) <= 200
        AND char_length(trim(label)) > 0
    ),
    type_tag TEXT CHECK (
        type_tag IS NULL
        OR char_length(type_tag) <= 120
    ),
    principal NUMERIC(18, 4) NOT NULL DEFAULT 0,
    apr_percent NUMERIC(8, 4),
    payment_amount NUMERIC(18, 4),
    payment_frequency TEXT CHECK (
        payment_frequency IS NULL
        OR payment_frequency IN ('monthly', 'weekly')
    ),
    payment_end_date DATE,
    notes TEXT CHECK (
        notes IS NULL
        OR char_length(notes) <= 4000
    ),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX liabilities_installation_sort_idx ON liabilities (
    installation_id,
    sort_index ASC,
    label ASC
);

CREATE INDEX liabilities_installation_category_idx ON liabilities (
    installation_id,
    category_id
);
