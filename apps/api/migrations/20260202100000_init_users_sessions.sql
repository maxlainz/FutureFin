CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT username_len CHECK (
        char_length(username) BETWEEN 3 AND 64
    ),
    CONSTRAINT username_chars CHECK (
        username ~ '^[a-zA-Z0-9._-]+$'
    )
);

CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);

CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);
