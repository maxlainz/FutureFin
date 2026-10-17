-- 6.0.0 «Planes» · D-7: se elimina el modelo de jubilación (perfil, estrategias, SWR, número
-- FIRE, umbral de éxito, solves y la pestaña Jubilación).
--
-- **No se migra nada** (decisión del owner, RFC `planes-6.0.md` D-7): los perfiles existentes se
-- borran con la columna. La pérdida de datos es aceptada y se declara en el CHANGELOG de 6.0.0.
--
-- Copias de seguridad: el `.ffbackup` sigue en el schema 13. El export escribe
-- `retirement_profile: null` y el import ignora el campo (y los cuatro ejes legados de un
-- fichero ≤ 12), así que un fichero 5.0.x sigue siendo legible.
ALTER TABLE users DROP COLUMN IF EXISTS retirement_profile;
