-- OAuth 2.1 embebido (v3.1.0): FutureFin es a la vez authorization server y resource
-- server del endpoint MCP (/mcp) — lo que exige el conector de claude.ai web. Mismo
-- contrato de credenciales que api_tokens (D14): solo se persiste el SHA-256 hex del
-- secreto, nada se congela en el token (rol y membership se re-resuelven vivos en cada
-- request), revocación = una fila.
-- Deliberadamente EXCLUIDAS del backup de usuario (.ffbackup): son credenciales de la
-- instalación, no datos financieros. La exclusión es por construcción (export.rs escribe
-- un SELECT por tabla exportada); no añadas estas tablas allí.

-- Clientes registrados por DCR (RFC 7591). El registro es PÚBLICO: la fila por sí sola
-- no da acceso a nada — el gate es el consentimiento del usuario (oauth_grants).
CREATE TABLE oauth_clients (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    client_id TEXT NOT NULL UNIQUE,
    -- NULL para clientes públicos (token_endpoint_auth_method = 'none', el caso de claude.ai).
    client_secret_hash TEXT,
    client_name TEXT NOT NULL,
    client_uri TEXT,
    redirect_uris TEXT[] NOT NULL,
    token_endpoint_auth_method TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    CONSTRAINT oauth_clients_name_len CHECK (char_length(client_name) BETWEEN 1 AND 120),
    CONSTRAINT oauth_clients_redirects_card CHECK (
        cardinality(redirect_uris) BETWEEN 1 AND 5
    ),
    CONSTRAINT oauth_clients_auth_method CHECK (
        token_endpoint_auth_method IN ('none', 'client_secret_basic', 'client_secret_post')
    ),
    CONSTRAINT oauth_clients_secret_presence CHECK (
        (token_endpoint_auth_method = 'none' AND client_secret_hash IS NULL)
        OR (token_endpoint_auth_method <> 'none' AND client_secret_hash IS NOT NULL)
    )
);
-- GC perezoso de registros huérfanos (POST /oauth/register): ordena por antigüedad.
CREATE INDEX oauth_clients_created_at_idx ON oauth_clients (created_at);

-- El consentimiento: UNA fila por (app, usuario). Es la unidad que ve y revoca el panel,
-- y la que muere entera ante reuse-detection de un refresh token o un code.
CREATE TABLE oauth_grants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    client_id UUID NOT NULL REFERENCES oauth_clients (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scope TEXT,
    resource TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    -- 'user_panel' | 'refresh_token_reuse' | 'code_reuse' | 'rfc7009'
    revoked_reason TEXT
);
-- Re-consentir la misma app reutiliza el grant vivo en vez de duplicar la fila del panel.
CREATE UNIQUE INDEX oauth_grants_active_uniq
    ON oauth_grants (client_id, user_id) WHERE revoked_at IS NULL;
CREATE INDEX oauth_grants_user_id_idx ON oauth_grants (user_id);
CREATE INDEX oauth_grants_client_id_idx ON oauth_grants (client_id);

-- Authorization codes: TTL corto (2 min, lo fija Postgres al emitir), un solo uso,
-- PKCE S256 obligatorio. Reuso de un code ⇒ se revoca el grant entero.
CREATE TABLE oauth_authorization_codes (
    code_hash TEXT PRIMARY KEY,
    grant_id UUID NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    resource TEXT,
    scope TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    CONSTRAINT oauth_codes_pkce_s256 CHECK (code_challenge_method = 'S256'),
    CONSTRAINT oauth_codes_challenge_len CHECK (
        char_length(code_challenge) BETWEEN 43 AND 128
    )
);
CREATE INDEX oauth_authorization_codes_grant_id_idx ON oauth_authorization_codes (grant_id);
CREATE INDEX oauth_authorization_codes_expires_at_idx ON oauth_authorization_codes (expires_at);

-- Access tokens opacos (`ffo_`), 1 h. La revocación del grant los mata sin tocarlos:
-- la query de auth hace JOIN con oauth_grants y exige revoked_at IS NULL (una sola fila
-- que actualizar para cortar todo, misma filosofía que borrar una sesión).
CREATE TABLE oauth_access_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    grant_id UUID NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX oauth_access_tokens_grant_id_idx ON oauth_access_tokens (grant_id);
CREATE INDEX oauth_access_tokens_expires_at_idx ON oauth_access_tokens (expires_at);

-- Refresh tokens (`ffr_`) con rotación: cada canje consume el actual y emite uno nuevo con
-- expires_at = now() + 90 días (sliding: 90 días SIN USO). Presentar uno ya consumido es la
-- señal de robo → se revoca el grant entero (OAuth 2.1 §4.3.1).
CREATE TABLE oauth_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    grant_id UUID NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    replaced_by UUID REFERENCES oauth_refresh_tokens (id) ON DELETE SET NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX oauth_refresh_tokens_grant_id_idx ON oauth_refresh_tokens (grant_id);
