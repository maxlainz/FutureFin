-- Los recurrentes de un mes existen ⟺ ese mes tiene datos reales (3.9.0).
--
-- ANTES: `POST /materialize` avanzaba un cursor monotónico (`last_materialized_month`) y escribía
-- una instancia por cada mes civil cerrado, hubiera o no movimientos reales en él. Eso producía
-- meses «pseudovacíos» (solo recurrentes) que el promedio del engine tenía que excluir con un
-- predicado especial, y que hacían divergir de forma SISTEMÁTICA los dos contadores de meses con
-- datos (el estricto del promedio y el laxo de la pestaña Movimientos).
--
-- AHORA: una pasada convergente lleva las instancias al estado que definen las reglas — existen
-- exactamente en los meses ACTIVOS (≥1 movimiento real no conciliado, ámbito instalación) desde
-- `origin_month`, y en ningún otro. Un mes sin CSV ni altas manuales queda genuinamente vacío.
--
-- POR QUÉ SE INVIERTE LA DECISIÓN ORIGINAL de 20260708090000 («SIN UNIQUE(rule, ym): a propósito…
-- el cursor es la única fuente de verdad de idempotencia»): al desaparecer el cursor, la
-- EXISTENCIA pasa a ser la fuente de idempotencia, y entonces el índice único es la herramienta
-- correcta — y la única defensa contra dos convergencias concurrentes bajo READ COMMITTED (el
-- `FOR UPDATE` sobre las reglas no re-evalúa el `NOT EXISTS`, que mira `transactions`).
--
-- CONSECUENCIA ACEPTADA (firmada por el owner): borrar a mano una instancia ya no la borra para
-- siempre — se recrea mientras su mes siga activo. Para quitarla de verdad se borra la PLANTILLA.
--
-- DATA-LOSS DELIBERADO, firmado por el owner: el paso (3) borra TODA instancia recurrente alojada
-- en un mes sin movimientos reales, **incluida la del mes de origen de la regla** (el movimiento
-- con el que se dio de alta la recurrencia). Se decidió así porque la exención del origen dejaba
-- meses históricos con instancias sueltas e impedía que los dos contadores de meses convergieran,
-- que es uno de los objetivos del cambio. El ancla `origin_month` conserva igualmente cuándo
-- empezó cada regla, y la convergencia NUNCA recrea lo borrado en un mes que siga inactivo.
-- La exención del mes de origen SÍ existe en la poda de la convergencia continua, donde protege
-- un movimiento recién tecleado cuyo mes aún no tiene otros datos.
--
-- FIRE-NEUTRAL POR CONSTRUCCIÓN: los meses que este paso vacía ya estaban excluidos por completo
-- (numerador y denominador) del promedio que alimenta el engine, así que la proyección, el target
-- FIRE y el runway no cambian ni un decimal. Lo que sí cambia, a propósito, es el promedio de la
-- pestaña Movimientos y el listado visible. El entrypoint escribe un backup pre-migración antes.

-- (1) El cursor monotónico se sustituye por un ANCLA: el mes en que arrancó la regla.
--     Reconstrucción: el mes más antiguo con instancia suya. Si no le queda ninguna (instancias
--     borradas a mano), el cursor es la mejor cota disponible.
--     ORDEN IMPORTANTE: este paso corre ANTES del borrado (3), que destruiría la evidencia.
ALTER TABLE recurring_transaction_rules ADD COLUMN origin_month DATE;

UPDATE recurring_transaction_rules r
SET origin_month = LEAST(
    r.last_materialized_month,
    COALESCE(
        (SELECT MIN(date_trunc('month', t.op_date::timestamp)::date)
           FROM transactions t
          WHERE t.recurring_rule_id = r.id),
        r.last_materialized_month));

ALTER TABLE recurring_transaction_rules
    ALTER COLUMN origin_month SET NOT NULL,
    DROP COLUMN last_materialized_month;

COMMENT ON COLUMN recurring_transaction_rules.origin_month IS
    'Ancla (primer día de mes): mes en que arrancó la regla. La convergencia materializa desde '
    'este mes en adelante, y su poda continua nunca retira la instancia de este mes.';

-- (2) Deduplicado DEFENSIVO antes del índice único. Por construcción del cursor no debería haber
--     duplicados (una instancia por (regla, mes)), pero un .ffbackup restaurado a mano podría
--     traerlos, y una migración que ABORTA en la base de datos de un usuario es el peor resultado
--     posible. Conserva la instancia más antigua de cada par.
DELETE FROM transactions t
USING transactions d
WHERE t.recurring_rule_id IS NOT NULL
  AND t.recurring_rule_id = d.recurring_rule_id
  AND date_trunc('month', t.op_date::timestamp) = date_trunc('month', d.op_date::timestamp)
  AND (t.created_at, t.id) > (d.created_at, d.id);

-- (3) LIMPIEZA HISTÓRICA: instancias recurrentes en meses SIN ningún movimiento real.
--     «Movimiento real» = `recurring_rule_id IS NULL` y no conciliado (una pata de transferencia
--     conciliada es dinero que nunca salió del hogar: no activa el mes).
--     Ámbito INSTALACIÓN, el mismo que activa el mes en el modelo nuevo: con ámbito por owner, un
--     hogar donde A importa CSV y B solo tiene una nómina recurrente perdería TODA la nómina de B.
--     Sin exención del mes de origen — ver la cabecera.
DELETE FROM transactions t
WHERE t.recurring_rule_id IS NOT NULL
  AND NOT EXISTS (
        SELECT 1
          FROM transactions x
         WHERE x.installation_id = t.installation_id
           AND x.op_date >= date_trunc('month', t.op_date::timestamp)::date
           AND x.op_date <  (date_trunc('month', t.op_date::timestamp) + INTERVAL '1 month')::date
           AND x.recurring_rule_id IS NULL
           AND x.transfer_counterpart_id IS NULL);

-- (4) Idempotencia declarativa: como mucho una instancia por (regla, mes).
--     OJO — `date_trunc(text, timestamp)` es IMMUTABLE, pero `date_trunc(text, timestamptz)` es
--     STABLE y NO es indexable. El cast explícito `::timestamp` es OBLIGATORIO: sin él esta
--     migración falla en el deploy con «functions in index expression must be marked IMMUTABLE».
CREATE UNIQUE INDEX transactions_recurring_rule_month_uniq
    ON transactions (recurring_rule_id, date_trunc('month', op_date::timestamp))
    WHERE recurring_rule_id IS NOT NULL;
