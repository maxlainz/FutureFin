-- Conciliación de transferencias internas (3.5.0).
--
-- El bug que corrige: las filas marcadas `suggested_transfer` por la heurística del preview se
-- DESCARTABAN por defecto en el confirm (el default vivía en el frontend, `initialDraftForRow`)
-- → nunca se insertaban, no dejaban huella, y volvían a ofrecerse como `new` en cada re-import.
-- Una transferencia que ERA gasto real (alquiler pagado por transferencia, envío a un tercero)
-- desaparecía del gasto en silencio.
--
-- El modelo correcto: nada se descarta. Un movimiento solo deja de contar como gasto/ingreso
-- cuando se CONCILIA con su contrapartida — la otra pata del mismo traspaso, normalmente
-- importada de otro extracto. Conciliado ⇒ sigue visible en Movimientos, excluido de todos los
-- agregados de flujo (predicado `transfer_counterpart_id IS NULL`).
--
-- Decisiones de diseño (ver CHANGELOG 3.5.0):
--   * `transfer_counterpart_id` es self-FK ON DELETE SET NULL: borrar una pata desconcilia la
--     otra sin código (la superviviente reaparece en el gasto). Se rechazó `transfer_group_id`
--     precisamente porque dejaría un «grupo de uno» cuya pata seguiría oculta — el bug de
--     partida con otro nombre. OJO: el SET NULL no limpia `transfer_reconciled_at/source`; la
--     fuente de verdad de «conciliada» es SIEMPRE `transfer_counterpart_id IS NOT NULL`.
--   * El índice UNIQUE parcial hace la relación INYECTIVA (nadie es contrapartida de dos). La
--     simetría (A→B ∧ B→A) la garantiza `handlers/transactions/reconcile.rs`, que escribe las
--     dos patas siempre en la misma transacción.
--   * `transfer_match_rejections` es la memoria del auto-matcher: un par desconciliado A MANO
--     no debe resucitar en el siguiente pase (que corre sobre TODA la BD del owner, N veces).
--     Par canónico `transaction_a_id < transaction_b_id` → un par = una fila. LIMITACIÓN
--     documentada: deshacer un import y re-importarlo da UUIDs nuevos → el rechazo cascadea y
--     el par vuelve a ser candidato (se acepta como «empezar de cero»).
--   * El índice parcial de matching está keyed por importe y fecha porque el pase busca
--     importes exactamente opuestos (misma divisa, mismo owner, |Δop_date| ≤ 5 días) entre
--     filas AÚN no conciliadas.

ALTER TABLE transactions
    ADD COLUMN transfer_counterpart_id UUID
        REFERENCES transactions (id) ON DELETE SET NULL,
    ADD COLUMN transfer_reconciled_at TIMESTAMPTZ,
    ADD COLUMN transfer_reconciled_source TEXT
        CHECK (transfer_reconciled_source IS NULL
               OR transfer_reconciled_source IN ('auto', 'manual')),
    ADD CONSTRAINT transactions_transfer_not_self
        CHECK (transfer_counterpart_id IS NULL OR transfer_counterpart_id <> id);

-- Inyectividad: como mucho una fila apunta a una contrapartida dada.
CREATE UNIQUE INDEX transactions_transfer_counterpart_uniq
    ON transactions (transfer_counterpart_id)
    WHERE transfer_counterpart_id IS NOT NULL;

-- Índice del auto-matcher: solo filas sin conciliar, por importe (join `a.amount = -b.amount`)
-- y fecha (ventana ±5 días).
CREATE INDEX transactions_transfer_match_idx
    ON transactions (installation_id, owner_user_id, amount, op_date)
    WHERE transfer_counterpart_id IS NULL;

CREATE TABLE transfer_match_rejections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    transaction_a_id UUID NOT NULL REFERENCES transactions (id) ON DELETE CASCADE,
    transaction_b_id UUID NOT NULL REFERENCES transactions (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Orden canónico del par (UUID tiene orden total en Postgres) → un par = una fila.
    CONSTRAINT transfer_match_rejections_ordered
        CHECK (transaction_a_id < transaction_b_id),
    CONSTRAINT transfer_match_rejections_unique
        UNIQUE (transaction_a_id, transaction_b_id)
);
CREATE INDEX transfer_match_rejections_owner_idx
    ON transfer_match_rejections (installation_id, owner_user_id);
