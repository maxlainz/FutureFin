-- 5.0.0 (issue #207, P3 Monte Carlo): la volatilidad es un atributo del ACTIVO, como su
-- rentabilidad esperada. Desviación típica ANUAL de los retornos en puntos porcentuales
-- (17 = 17 %/año). `NULL` o 0 = activo determinista (cuenta corriente, depósito): el camino
-- Decimal del engine la ignora siempre; solo el crate estocástico la lee (D11/D12).
-- Cota de API [0, 100]; el CHECK de columna queda laxo a propósito (importar backups viejos).

ALTER TABLE assets
    ADD COLUMN IF NOT EXISTS annual_volatility_percent NUMERIC(8, 4) NULL
        CONSTRAINT assets_annual_volatility_nonneg CHECK (annual_volatility_percent >= 0);
