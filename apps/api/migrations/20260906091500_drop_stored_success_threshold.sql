-- 5.0.0 · modelo de jubilación v2 (decisiones M2/M4/M6/C3/C7 del owner): limpieza del JSONB
-- `users.retirement_profile`.
--
-- POR QUÉ EXISTE, la clave por clave:
--
-- * `success_threshold_pct`. Entre la decisión V7 y v2 el campo se «aceptaba y se ignoraba»: la
--   SPA lo seguía mandando y el servidor lo descartaba… pero los perfiles escritos ANTES de V7 se
--   quedaron con un 95 almacenado que nadie leía. En v2 la clave vuelve a ser LOAD-BEARING (es la
--   restricción que decide la fecha de jubilación, C3) con default 95. Dejar ahí el 95 muerto
--   convertiría un valor que nadie eligió en una elección explícita del usuario: idéntico hoy,
--   pero congelado — el día que el default se mueva, esos perfiles no se moverían con él y nadie
--   sabría por qué. Borrarla hace que MANDE EL DEFAULT y que quien quiera otro umbral lo escriba.
--
-- * `target_basis`, `bridge_discount_basis`, `cash_buffer_months`. Retiradas del modelo: la
--   pensión es un flujo de caja y no descuenta ningún objetivo (M4), y el colchón de caja
--   desapareció como mecanismo (M6). El código ya no las lee —`RetirementProfile` no lleva
--   `deny_unknown_fields`, así que un perfil que las conserve carga igual y las ignora—, pero un
--   JSONB con claves que ningún lector entiende es una trampa para el siguiente que lo abra: el
--   `.ffbackup` las exportaría, un `psql` las enseñaría y alguien las creería vivas.
--
-- Es DATA-CHANGING sobre datos de usuario y va aprobada en el plan del modelo v2. Lo que borra no
-- es recuperable, y no hace falta que lo sea: ninguna de las cuatro claves tiene consumidor.
--
-- Sin DDL: la columna no cambia de tipo ni de nulabilidad. Idempotente: el `WHERE` solo toca las
-- filas que aún llevan alguna de las cuatro, y volver a correrla no encuentra ninguna.

UPDATE users
SET retirement_profile = retirement_profile
    - 'success_threshold_pct'
    - 'target_basis'
    - 'bridge_discount_basis'
    - 'cash_buffer_months'
WHERE retirement_profile IS NOT NULL
  AND (retirement_profile ? 'success_threshold_pct'
    OR retirement_profile ? 'target_basis'
    OR retirement_profile ? 'bridge_discount_basis'
    OR retirement_profile ? 'cash_buffer_months');
