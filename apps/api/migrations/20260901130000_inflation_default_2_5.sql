-- #146 (Ola 5, 4.9.0): el default de la inflación asumida pasa de 0 a 2,5 % — SOLO para
-- instalaciones NUEVAS (INSERT sin valor explícito). El 0 % era el valor MÁS optimista del rango
-- y el único 2,5 vivía como sugerencia del asistente saltable: una instalación creada por API o
-- por MCP nacía asumiendo inflación cero y su objetivo FIRE quedaba plano (−64 % en el escenario
-- tipo respecto del objetivo con el 2,5 % del BCE).
--
-- Las filas EXISTENTES no se tocan: cambiar el supuesto almacenado de una instalación viva
-- movería su objetivo y su fecha de cruce sin que el usuario haya decidido nada. Decisión del
-- owner (2026-08-30, issue #146): default 2,5 en nuevas + rango [−2, 50] (el rango vive en la
-- validación de escritura, no en un CHECK — coherente con el resto de cotas de la tabla).
ALTER TABLE installation
    ALTER COLUMN annual_inflation_assumption_percent SET DEFAULT 2.5;
