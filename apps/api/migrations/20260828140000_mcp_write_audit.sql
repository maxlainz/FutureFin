-- Auditoría de las escrituras MCP (Fase 3, issue #84).
--
-- POR QUÉ. Hasta aquí, un token de API podía vaciar el ledger del hogar sin dejar rastro:
-- `delete_transaction` es hard delete, y el único registro persistente de que un token existió
-- es `api_tokens.last_used_at`, con throttle de 60 s (no cuenta llamadas, no dice cuáles). La
-- traza de la Fase 0 (`tracing::info!` en `require_mcp_write`) es mejor que nada pero se va con
-- los logs del contenedor. Esta tabla es el registro que sobrevive.
--
-- QUÉ SE GUARDA — y sobre todo QUÉ NO. Nunca los argumentos de la tool. Dos razones:
--   1. Los argumentos llevan contenido escrito por la persona (conceptos bancarios, notas,
--      nombres de categoría) e importes. Copiarlos aquí crearía un SEGUNDO domicilio para ese
--      contenido, fuera del `.ffbackup` cifrado por usuario, y —lo decisivo— **append-only**:
--      borrar un movimiento «privado» dejaría su concepto vivo en el log para siempre. Un log de
--      auditoría no puede convertir el borrado del usuario en una mentira.
--   2. Un DIGEST de los argumentos tampoco vale: el espacio de entrada es minúsculo (fecha +
--      importe + un concepto de un vocabulario corto), así que un SHA-256 de esos campos es
--      recuperable por fuerza bruta. Un hash de datos de baja entropía no es anonimización.
-- Lo que sí se guarda son identificadores opacos: quién (user), con qué credencial, con qué rol,
-- qué verbo (`tool`, que YA determina la entidad: `delete_transaction` → transacción) y sobre qué
-- filas (`target_ids`). El esquema es tipado a propósito —sin JSONB, sin columnas de texto
-- libre— para que la regla de higiene no dependa de que el siguiente que pase se acuerde de ella:
-- aquí no CABE una frase que haya escrito una persona.
--
-- APPEND-ONLY, con una precisión. La fila se INSERTA antes de que la operación corra (con
-- `outcome = 'attempted'`) y se cierra UNA sola vez cuando termina (`settled_at IS NULL` en el
-- WHERE del UPDATE: una fila cerrada ya no se puede reescribir). No hay otra vía de UPDATE, y la
-- única vía de DELETE es la poda por retención. Un proceso que muera a mitad deja `attempted` +
-- `settled_at IS NULL`, que es exactamente la verdad: «se intentó, no sabemos cómo acabó».
--
-- RETENCIÓN: poda perezosa dentro del propio camino de escritura (mismo patrón que
-- `gc_orphan_clients` en `POST /oauth/register`), NUNCA en un GET (D5). La tabla solo crece
-- cuando hay escrituras, así que podar donde crece es autorregulado: una instalación parada no
-- hace nada. El índice sobre `at` es lo que hace que la poda sea un rango indexado que casi
-- siempre no encuentra nada.
CREATE TABLE mcp_write_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid (),
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Cuál de las dos credenciales del `/mcp` autenticó la llamada.
    credential_kind TEXT NOT NULL,
    -- `api_tokens.id` u `oauth_access_tokens.id`. SIN clave ajena a propósito: es polimórfico, y
    -- además el log tiene que sobrevivir a que la credencial se borre o caduque — si la fila que
    -- se audita se lleva por delante la auditoría, no hay auditoría.
    credential_id UUID NOT NULL,
    -- Rol VIVO en el momento de la llamada. Se guarda porque el rol cambia: sin esto, un log de
    -- hace tres meses se leería con los permisos de hoy.
    role TEXT NOT NULL,
    tool TEXT NOT NULL,
    outcome TEXT NOT NULL,
    -- Solo el CÓDIGO estable del error (`forbidden`, `mcp_write_disabled`…), jamás el mensaje:
    -- los mensajes de `BadRequest` sí pueden llevar datos del usuario.
    error_code TEXT,
    -- Filas sobre las que actuó la operación. UUIDs opacos: identifican qué se tocó sin decir
    -- qué contenía. Vacío mientras la fila no está cerrada.
    target_ids UUID[] NOT NULL DEFAULT '{}',
    settled_at TIMESTAMPTZ,
    CONSTRAINT mcp_write_audit_credential_kind CHECK (
        credential_kind IN ('api_token', 'oauth')
    ),
    -- attempted = el gate dejó pasar y la operación aún no había terminado (o el proceso murió).
    -- ok / failed = desenlace real de la operación. denied = el gate la rechazó.
    CONSTRAINT mcp_write_audit_outcome CHECK (
        outcome IN ('attempted', 'ok', 'failed', 'denied')
    ),
    -- `settled_at IS NULL` significa EXACTAMENTE una cosa: la llamada sigue en vuelo o el
    -- proceso murió sin cerrarla. `denied` nace ya cerrada (el gate ES toda la operación), así
    -- que la consulta «qué quedó sin desenlace» es literalmente `WHERE settled_at IS NULL`.
    CONSTRAINT mcp_write_audit_settled_shape CHECK (
        (settled_at IS NULL) = (outcome = 'attempted')
    )
);

-- Sirve a los dos accesos que existen: la poda por retención (`at < corte`) y leer lo reciente.
CREATE INDEX mcp_write_audit_at_idx ON mcp_write_audit (at);
