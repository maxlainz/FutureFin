-- Modelo de amortización del pasivo: cómo cobra el engine de proyección la cuota mensual.
-- Opt-in puro. `fixed_payments` reproduce EXACTAMENTE el modelo anterior a 4.2.0 (la cuota va
-- íntegra a principal, el pasivo no devenga intereses), así que los pasivos ya existentes
-- quedan todos en él por el DEFAULT y NADIE ve moverse un número al actualizar. Solo cuando el
-- usuario elige otro modelo (y configura el TIN) la proyección empieza a cobrar intereses.
--
-- Los cuatro valores: 'fixed_payments' (histórico, sin intereses), 'french' (sistema francés:
-- interés sobre el saldo de apertura y cuota a fin de mes), 'interest_only' (solo intereses:
-- el principal no se mueve) y 'revolving' (misma recurrencia que el francés en 4.2.0).
-- NOT NULL + CHECK en vez de un ENUM de Postgres: añadir un modelo nuevo es un ALTER del CHECK,
-- sin las servidumbres de tipo que arrastra un ENUM en migraciones y en el backup por usuario.
ALTER TABLE liabilities
    ADD COLUMN repayment_model TEXT NOT NULL DEFAULT 'fixed_payments';

ALTER TABLE liabilities
    ADD CONSTRAINT liabilities_repayment_model_chk
    CHECK (repayment_model IN ('fixed_payments', 'french', 'interest_only', 'revolving'));
