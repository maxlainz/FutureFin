-- Inflación siempre activa: el target FIRE crece con la inflación para preservar el poder
-- adquisitivo. La columna booleana `projection_includes_inflation` deja de tener sentido —
-- la tasa anual (0 = sin inflación) es ahora el único parámetro.
ALTER TABLE installation DROP COLUMN IF EXISTS projection_includes_inflation;

UPDATE installation
   SET annual_inflation_assumption_percent = 0
 WHERE annual_inflation_assumption_percent IS NULL;

ALTER TABLE installation
    ALTER COLUMN annual_inflation_assumption_percent SET DEFAULT 0,
    ALTER COLUMN annual_inflation_assumption_percent SET NOT NULL;
