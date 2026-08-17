-- Reglas recurrentes con resolución MENSUAL (3.2.0): se elimina el día configurable.
-- Desde este cambio la instancia del mes M se fecha en el último día de M y solo se materializa
-- con M ya cerrado (servidor en M+1), así el mes en curso no muestra movimientos sintéticos.
-- Data-loss deliberado y firmado por el owner (CHANGELOG 3.2.0): se pierde la configuración
-- por-regla del día; las instancias ya materializadas conservan su op_date histórico.
ALTER TABLE recurring_transaction_rules DROP COLUMN day_of_month;
