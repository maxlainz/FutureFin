-- Ola 3 (#129): el modelo de amortización viaja al snapshot.
--
-- NULL = snapshot anterior a 4.7.0: se reinterpreta como `fixed_payments` (que era el default
-- de la columna cuando se capturó) ⇒ ley LINEAL en la interpolación histórica. No se backfilla
-- desde `liabilities`: el modelo de HOY no es el que tenía el pasivo cuando se hizo la foto.
ALTER TABLE history_snapshot_items ADD COLUMN repayment_model TEXT
    CHECK (repayment_model IS NULL
           OR repayment_model IN ('fixed_payments','french','interest_only','revolving'));
