-- Claves de idempotencia para el alta manual de movimientos (`POST /v1/transactions`).
--
-- El bug que cierra: `next_fingerprint_ordinal` resuelve el ordinal con `MAX(ordinal)+1`, así que
-- un reintento tras un timeout de red inserta una SEGUNDA fila y responde 201. El llamante no
-- tiene forma de distinguir «no llegó» de «llegó y se perdió la respuesta», y en los modos que
-- usan transacciones (`savings_source ∈ {transactions_avg, budget_income_real_expense}`) un gasto
-- duplicado infla el promedio ponderado 12m → mueve el ahorro mensual del motor y retrasa la
-- fecha de jubilación proyectada, en silencio.
--
-- Decisiones de diseño (todas deliberadas; ver el doc del módulo
-- `handlers/transactions/idempotency.rs`):
--
--   * **Opt-in**: sin `idempotency_key` en el cuerpo NO se toca esta tabla y el comportamiento es
--     bit a bit el de siempre — reenviar el mismo movimiento crea otro movimiento. Los duplicados
--     legítimos existen (dos cafés de 1,80 € el mismo día): para eso está el `fingerprint_ordinal`,
--     y cambiar el default rompería un contrato documentado en la propia tool MCP.
--   * **Ámbito `(installation, owner)`**: la clave la elige el cliente, así que dos miembros pueden
--     elegir la misma. Con ámbito de instalación, la de Bob «reproduciría» el movimiento de Alice y
--     le devolvería una fila que no es suya — una fuga entre miembros, no una colisión benigna.
--     El ámbito coincide además con el del `fingerprint` y con el de todo lo per-user del módulo.
--   * **Tabla aparte, no una columna en `transactions`**: guarda el hash del cuerpo (para detectar
--     el reintento con cuerpo distinto), tiene caducidad propia y no toca una tabla que exporta el
--     `.ffbackup`. Esta tabla NO se exporta (misma decisión que `api_tokens` y `oauth_*`: es un
--     artefacto operativo del transporte, no dato del hogar) → el `schema_version` del backup NO
--     cambia.
--   * **Caducidad + poda perezosa**: `created_at` con índice, y el propio POST borra lo caducado
--     (precedente `oauth/register.rs::gc_orphan_clients`; la poda vive en un POST y nunca en un GET
--     — D5, reads never mutate). Una clave protege contra el reintento de una petición EN VUELO:
--     la ventana útil son segundos. La retención es de 24 h, tres órdenes de magnitud de margen.
--     Caducar solo retira la PROTECCIÓN; nunca borra un movimiento.
--   * **`ON DELETE CASCADE` hacia `transactions`**: si el movimiento se borra, la clave deja de
--     apuntar a nada y desaparece con él. Reintentar después vuelve a crear — correcto: borrar es
--     una intención posterior y explícita del usuario, no un reintento.

CREATE TABLE transaction_idempotency_keys (
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Cadena elegida por el cliente (UUID, ULID, lo que sea). Se normaliza con trim y se acota
    -- en el handler; aquí solo se acota la longitud para que un cliente no use la tabla de saco.
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 200),
    -- SHA-256 del cuerpo YA VALIDADO (valores normalizados), para distinguir un reintento honesto
    -- de una reutilización de clave con otro contenido.
    request_hash TEXT NOT NULL,
    transaction_id UUID NOT NULL REFERENCES transactions (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (installation_id, owner_user_id, idempotency_key)
);

-- Poda perezosa por antigüedad: la barre el propio POST antes de mirar la clave.
CREATE INDEX transaction_idempotency_keys_created_idx
    ON transaction_idempotency_keys (created_at);
