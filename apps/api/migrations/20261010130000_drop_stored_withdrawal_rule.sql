-- 5.0.0 · decisión «solo SWR» del owner (2026-10-10): limpieza del JSONB
-- `users.retirement_profile`.
--
-- POR QUÉ EXISTE:
--
-- * `withdrawal_rule`. 5.0.0-dev añadió al perfil una regla de retirada (`fixed_real`,
--   `percent_of_balance`, `hybrid`, `guardrails`) con un modo de gasto (`ceiling`,
--   `rule_is_spend`) y sus porcentajes. Se retiró entera antes de publicar: el modelo de retirada
--   es ÚNICAMENTE la puerta de tasa inicial —en el mes en que el hogar se jubila, su gasto
--   ordinario anual neto de rentas y pensión tiene que caber en `swr_pct` % del líquido, o en
--   `bridge_max_pct` si el puente aplica— y, jubilado, se vende cada mes lo que el gasto indexado
--   necesita (la antigua `fixed_real`, el drenaje de 4.15.x). Lo medido que llevó a retirarla: el
--   modo de gasto no movía la fecha, ni la tasa inicial, ni el capital necesario; el % de la regla
--   era siempre el SWR salvo por API; y con ese % heredado el fallo de la regla contenía al de la
--   tasa inicial en el mes de jubilación sin mirar el puente, así que lo neutralizaba.
--
-- El código ya no la lee —`RetirementProfile` no lleva `deny_unknown_fields`, así que un perfil
-- que la conserve carga igual y la ignora—, pero un JSONB con una clave que ningún lector entiende
-- es una trampa para el siguiente que lo abra: el `.ffbackup` la exportaría, un `psql` la
-- enseñaría y alguien la creería viva.
--
-- Es DATA-CHANGING sobre datos de usuario y va aprobada con la decisión del owner. Lo que borra
-- no es recuperable, y no hace falta que lo sea: la clave no tiene consumidor, y ninguna versión
-- publicada la escribió nunca (4.15.x no tiene perfil de jubilación).
--
-- Sin DDL: la columna no cambia de tipo ni de nulabilidad. Idempotente: el `WHERE` solo toca las
-- filas que aún la llevan, y volver a correrla no encuentra ninguna.

UPDATE users
SET retirement_profile = retirement_profile - 'withdrawal_rule'
WHERE retirement_profile IS NOT NULL
  AND retirement_profile ? 'withdrawal_rule';
