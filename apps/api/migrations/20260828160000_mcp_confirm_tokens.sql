-- Confirmación en DOS FASES de las escrituras MCP irreversibles (Fase 3, issue #84).
--
-- POR QUÉ. Desde el issue #3 las tools destructivas piden `confirm: true` y sin él devuelven un
-- preview. Eso se lee como una salvaguarda de dos fases y NO lo es: `confirm` es un booleano del
-- propio esquema, así que el modelo puede escribirlo en la PRIMERA llamada. `confirm: true` sobre
-- una fila jamás previsualizada la borra al instante — el preview era *prompting*, no un control.
-- Un lote de import con 400 movimientos desaparecía sin que nadie hubiera visto el número 400.
--
-- QUÉ ES UN TOKEN. Un secreto opaco (`ffpv_…`) que **solo** se emite dentro de un preview, ligado
-- a tres cosas: la tool, los argumentos normalizados y la HUELLA DE LOS EFECTOS que ese preview
-- enseñó. La confirmación lo exige, y el servidor recalcula los efectos y compara la huella: si
-- entre el preview y el confirm alguien añadió 50 movimientos al lote, el token ya no vale. Eso
-- cierra la ventana que el `confirm` booleano no podía ni ver.
--
-- UN SOLO USO, TTL CORTO — copiado de `oauth_authorization_codes` (misma migración 20260817090000,
-- mismo problema resuelto): `consumed_at` marcado dentro del propio UPDATE que lo valida, así que
-- el consumo es atómico y dos confirmaciones simultáneas no pueden ganar las dos. La diferencia
-- con OAuth es el TTL: allí son 2 minutos porque el que responde es una máquina; aquí hay una
-- PERSONA leyendo un preview en un chat y contestando «sí, bórralo», así que son 10 minutos.
-- Pasado ese plazo el token no se renueva: se vuelve a previsualizar, que además reenseña los
-- números por si han cambiado.
--
-- QUÉ NO SE GUARDA. Ni los argumentos ni los efectos en claro: solo sus SHA-256. A diferencia de
-- `mcp_write_audit` —donde un digest sería recuperable por fuerza bruta porque el espacio de
-- entrada es minúsculo— aquí el hash NO es una medida de privacidad sino de igualdad, y la fila
-- vive 10 minutos y se poda. Aun así se hashea en vez de guardar el JSON: una tabla operativa no
-- tiene por qué contener el concepto de un movimiento ni el nombre de un activo.
--
-- PODA PEREZOSA en el camino de ESCRITURA (nunca en un GET — D5), igual que
-- `oauth/register.rs::gc_orphan_clients` y que las claves de idempotencia: la tabla solo crece
-- cuando se emiten previews, así que podar donde crece es autorregulado.
--
-- NO SE EXPORTA en el `.ffbackup` (misma decisión que `api_tokens`, `oauth_*` y
-- `transaction_idempotency_keys`: artefacto operativo del transporte, no dato del hogar) → el
-- `schema_version` del backup NO cambia.
CREATE TABLE mcp_confirm_tokens (
    -- SHA-256 hex del secreto. El secreto viaja UNA vez, en la respuesta del preview (D14).
    token_hash TEXT PRIMARY KEY,
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    -- El token es de quien previsualizó. Otro miembro no puede confirmar tu borrado con tu token.
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Nombre de la tool que lo emitió: un token de `delete_import` no confirma un `delete_asset`.
    tool TEXT NOT NULL,
    -- SHA-256 de los argumentos NORMALIZADOS del preview (el id del objetivo y las opciones que
    -- cambian lo que se hace). Cambiar el objetivo entre preview y confirm invalida el token.
    args_hash TEXT NOT NULL,
    -- SHA-256 del bloque `effects` que el preview publicó. Es lo que convierte el token en una
    -- salvaguarda real y no en una ceremonia: garantiza que lo que se confirma es lo que se vio.
    effects_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

-- Sirve a los dos accesos: la poda por caducidad y (con el filtro de `consumed_at`) el barrido de
-- tokens vivos. El lookup por token va por la PK.
CREATE INDEX mcp_confirm_tokens_expires_at_idx ON mcp_confirm_tokens (expires_at);
