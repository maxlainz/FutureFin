-- Identidad delegada a un proxy de confianza (SSO por cabeceras `X-Remote-User-*`, add-on de
-- Home Assistant). Dos cambios, ambos aditivos para las cuentas que ya existen:
--
-- 1. `password_hash` deja de ser NOT NULL. Una cuenta creada por el proxy no tiene contraseña
--    que guardar: la autenticación la hizo el proveedor antes de que la petición llegara aquí.
--    Inventarle una contraseña aleatoria sería peor (una credencial que nadie conoce y que el
--    cambio de contraseña podría rotar), así que la ausencia se modela como ausencia. El login
--    normal la rechaza con `sso_account_no_password`, y `POST /v1/auth/password` también.
-- 2. `external_user_id` guarda la identidad estable del proveedor. Es UNIQUE **parcial**: las
--    cuentas de contraseña la dejan a NULL y un índice UNIQUE normal las haría chocar entre sí
--    en Postgres solo si NULL no fuera distinto de NULL — el WHERE lo deja explícito y además
--    mantiene el índice pequeño.
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

ALTER TABLE users ADD COLUMN external_user_id UUID;

CREATE UNIQUE INDEX users_external_user_id_key ON users (external_user_id)
    WHERE external_user_id IS NOT NULL;

COMMENT ON COLUMN users.password_hash IS
    'Hash Argon2id de la contraseña. NULL = cuenta SSO sin contraseña (creada por un proxy de confianza); el login normal la rechaza con sso_account_no_password.';

COMMENT ON COLUMN users.external_user_id IS
    'Identidad estable del proveedor de confianza (cabecera X-Remote-User-Id de Home Assistant). NULL en las cuentas de usuario+contraseña.';
