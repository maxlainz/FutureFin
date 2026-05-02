CREATE TABLE households (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    name TEXT NOT NULL,
    base_currency VARCHAR(3) NOT NULL,
    projection_includes_inflation BOOLEAN NOT NULL DEFAULT FALSE,
    projection_target_age SMALLINT,
    show_age_mode TEXT NOT NULL DEFAULT 'dates',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT households_name_len CHECK (char_length(name) BETWEEN 1 AND 128),
    CONSTRAINT households_currency_fmt CHECK (base_currency ~ '^[A-Z]{3}$'),
    CONSTRAINT households_target_age CHECK (
        projection_target_age IS NULL OR projection_target_age BETWEEN 65 AND 105
    ),
    CONSTRAINT households_show_age_mode CHECK (show_age_mode IN ('dates', 'ages'))
);

CREATE TABLE household_memberships (
    household_id UUID NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member', 'viewer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (household_id, user_id)
);

CREATE INDEX household_memberships_user_id_idx ON household_memberships (user_id);

CREATE INDEX household_memberships_household_id_idx ON household_memberships (household_id);
