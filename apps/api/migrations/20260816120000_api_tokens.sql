-- Tokens de API por usuario (Bearer) para el servidor MCP embebido y futuros clientes
-- programáticos. El secreto NUNCA se persiste: solo su SHA-256 (hex). `token_prefix`
-- guarda los primeros caracteres visibles ("ffp_XXXXXXXX") para que la UI identifique
-- el token sin exponerlo. Revocación soft (`revoked_at`) para conservar auditoría.
-- Deliberadamente EXCLUIDA del backup de usuario (.ffbackup): son credenciales de la
-- instalación, no datos financieros — un restore no debe resucitar secretos.
CREATE TABLE api_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    CONSTRAINT api_tokens_label_len CHECK (char_length(label) BETWEEN 1 AND 64)
);

CREATE INDEX api_tokens_user_id_idx ON api_tokens (user_id);
