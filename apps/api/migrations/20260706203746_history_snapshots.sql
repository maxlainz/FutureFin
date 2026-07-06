-- Perspectiva histórica: snapshots manuales de patrimonio (v1.5.0).
--
-- FutureFin solo modela presente y futuro (`assets.current_value` /
-- `liabilities.principal` son escalares mutables sin historial). Estas dos
-- tablas guardan snapshots manuales per-user de los que el servidor
-- interpola la serie histórica de net worth, unida a la proyección en un
-- único chart temporal.
--
--   - history_snapshots       → cabecera: una fila por (instalación, usuario,
--                               kind, día civil). Upsert por día.
--   - history_snapshot_items  → un valor Decimal por asset/liability copiado
--                               del ledger; totales derivados, nunca almacenados.
--
-- owner_user_id CASCADE (no SET NULL): un snapshot sin dueño no significa nada
-- y el export per-user queda en un WHERE. source_item_id es la clave de
-- interpolación/series: captura = id del asset/liability; backfill = UUID del
-- cliente (enlaza el mismo item entre snapshots) o generado por el servidor y
-- devuelto. NO es FK: la copia sobrevive al borrado de la fila de ledger.

CREATE TABLE history_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('asset', 'liability')),
    snapshot_date DATE NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('capture', 'backfill')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT history_snapshots_unique_per_day
        UNIQUE (installation_id, owner_user_id, kind, snapshot_date)
);
CREATE INDEX history_snapshots_installation_date_idx
    ON history_snapshots (installation_id, snapshot_date);

CREATE TABLE history_snapshot_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    snapshot_id UUID NOT NULL REFERENCES history_snapshots (id) ON DELETE CASCADE,
    source_item_id UUID NOT NULL,          -- id de ledger en captura, o clave generada en backfill; NO es FK (la copia sobrevive borrados)
    label TEXT NOT NULL CHECK (char_length(label) <= 200 AND char_length(trim(label)) > 0),
    value NUMERIC(18,4) NOT NULL CHECK (value >= 0),
    apr_percent NUMERIC(8,4) CHECK (apr_percent IS NULL OR apr_percent >= 0),
    payment_amount NUMERIC(18,4) CHECK (payment_amount IS NULL OR payment_amount >= 0),
    payment_frequency TEXT CHECK (payment_frequency IS NULL OR payment_frequency IN ('monthly','weekly')),
    CONSTRAINT history_snapshot_items_unique_item UNIQUE (snapshot_id, source_item_id)
);
