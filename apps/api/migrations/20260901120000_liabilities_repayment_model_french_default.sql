-- Ola 3 (#144, sign-off del owner 2026-08-31): el catálogo de amortización dice la verdad.
--
-- 1) El default de la columna pasa a 'french' — el sistema francés ES el préstamo español; el
--    default histórico ('fixed_payments', cuota íntegra a principal) describe un producto que
--    no existe y era el que recibía cualquier alta sin tocar el desplegable.
ALTER TABLE liabilities ALTER COLUMN repayment_model SET DEFAULT 'french';

-- 2) DATA-CHANGING (firmado): las filas que YA declaraban TIN > 0 y cuota mensual pero se
--    simulaban sin intereses pasan a 'french'. Su proyección empieza a cobrar intereses:
--    una hipoteca 200.000 € / 1.000 €/mes / TIN 3 % pasa de extinguirse en el mes 200 con
--    0 € de intereses a extinguirse en el 278 con ≈78.000 € — el número honesto.
--    Es el MISMO predicado que la validación nueva de la API: una sola regla, dos sitios.
UPDATE liabilities
   SET repayment_model = 'french'
 WHERE repayment_model = 'fixed_payments'
   AND apr_percent > 0
   AND payment_frequency = 'monthly'
   AND payment_amount > 0;

-- 3) El residuo (firmado): 'fixed_payments' con TIN que NO puede ser francés (cuota semanal o
--    sin plan de pago). El TIN se anula — el engine SIEMPRE lo ignoró en este modelo (número
--    inmóvil que solo confundía) y conservarlo dejaría la fila ineditable bajo la validación
--    nueva (apr_forbidden_for_model).
UPDATE liabilities
   SET apr_percent = NULL
 WHERE repayment_model = 'fixed_payments'
   AND apr_percent > 0;

-- 4) La cuota mínima revolving real: porcentaje del saldo con suelo en euros. NULLables — solo
--    'revolving' las usa; el acoplamiento modelo⇔mínimos vive en la validación de escritura
--    (un CHECK acoplado rompería el import de backups v10 con revolvings antiguas).
ALTER TABLE liabilities
    ADD COLUMN min_payment_pct NUMERIC(8,4)
        CHECK (min_payment_pct IS NULL OR (min_payment_pct >= 0 AND min_payment_pct <= 100)),
    ADD COLUMN min_payment_eur NUMERIC(18,4)
        CHECK (min_payment_eur IS NULL OR min_payment_eur >= 0);

-- 5) Backfill BIT-IDÉNTICO de las revolving existentes: pct = 0 y suelo = la cuota declarada
--    ⇒ max(0·saldo, cuota) = cuota — exactamente la recurrencia que 4.6.0 les aplicaba.
UPDATE liabilities
   SET min_payment_pct = 0, min_payment_eur = payment_amount
 WHERE repayment_model = 'revolving' AND payment_amount IS NOT NULL;
