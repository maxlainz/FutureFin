-- Estado del asistente de primera vez (3.10.0).
--
-- Hasta ahora una instalación recién creada no tenía forma de saber si su dueño ya había pasado
-- por la configuración inicial: el primer usuario aterrizaba en un Resumen en blanco, iba a
-- Activos y se encontraba una pantalla SIN botón de añadir, porque el «+» se ocultaba cuando no
-- había ninguna categoría — y no había ninguna, porque el servidor nunca sembró categorías.
--
-- Aditiva y sin pérdida: NULL = nunca se completó. Las instalaciones que ya existen se marcan
-- como completadas, porque su dueño ya configuró el hogar a mano y enseñarle ahora un asistente
-- de bienvenida sería absurdo.
ALTER TABLE installation
    ADD COLUMN onboarding_completed_at TIMESTAMPTZ;

UPDATE installation SET onboarding_completed_at = now();
