-- Ola 3 (#129): el modelo de amortización viaja al snapshot.
--
-- NULL = snapshot anterior a 4.7.0: la foto no sabe su modelo ⇒ ley LINEAL (la cuerda) en la
-- interpolación histórica. Honestidad del trade-off: hasta 4.6.0 esos snapshots se RENDERIZABAN
-- con la curva francesa universal — para un pasivo genuinamente francés la curva vieja era la
-- correcta y la cuerda pierde forma interior (~300 €/50 k€, extremos exactos); para el default
-- mayoritario (`fixed_payments`) esa curva era el bug de #129. Sin el modelo no hay forma de
-- distinguirlos, y la cuerda es la ley menos comprometida. No se backfilla desde `liabilities`:
-- el modelo de HOY no es el que tenía el pasivo cuando se hizo la foto.
ALTER TABLE history_snapshot_items ADD COLUMN repayment_model TEXT
    CHECK (repayment_model IS NULL
           OR repayment_model IN ('fixed_payments','french','interest_only','revolving'));
