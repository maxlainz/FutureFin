-- Presupuesto siempre mensual: sin etiqueta ni frecuencia en líneas persistidas.

UPDATE budget_entries
SET amount = ROUND((amount * 52 / 12)::numeric, 4)
WHERE frequency = 'weekly';

DROP INDEX IF EXISTS budget_entries_installation_sort_idx;

ALTER TABLE budget_entries DROP COLUMN IF EXISTS label;
ALTER TABLE budget_entries DROP COLUMN IF EXISTS frequency;

CREATE INDEX budget_entries_installation_sort_idx ON budget_entries (
    installation_id,
    sort_index ASC,
    id ASC
);
