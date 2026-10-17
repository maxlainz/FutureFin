-- 5.0.0 (issue #207, decisión D13): la jubilación pasa a ser una ESTRATEGIA POR USUARIO.
-- `users.retirement_profile` guarda el perfil (JSONB, mismo patrón que `installation.fire_settings`:
-- `NULL` = defaults; claves ausentes = defaults; ver `apps/api/src/handlers/retirement_profile.rs`).
--
-- Cuatro ejes que hasta 4.15.x vivían en `installation.fire_settings` — `fire_number_mode`,
-- `fire_number_manual_amount`, `swr_pct`, `horizon_lifespan_age` — pasan al perfil de CADA usuario.
-- Se COPIAN al perfil de todos los usuarios existentes para que el upgrade no mueva un número:
-- cada miembro arranca en la estrategia `asap` (la de hoy) con exactamente los valores que la
-- instalación tenía. Después se retiran del JSONB de la instalación: `FireSettings` ya no los
-- lee, y un backup exportado tras el upgrade no debe llevar dos copias de la misma cifra.
--
-- Idempotente: solo rellena perfiles NULL y solo retira claves si existen.

ALTER TABLE users ADD COLUMN IF NOT EXISTS retirement_profile JSONB;

UPDATE users u
SET retirement_profile = jsonb_strip_nulls(jsonb_build_object(
        'strategy', 'asap',
        'fire_number_mode', i.fire_settings -> 'fire_number_mode',
        'fire_number_manual_amount', i.fire_settings -> 'fire_number_manual_amount',
        'swr_pct', i.fire_settings -> 'swr_pct',
        'horizon_lifespan_age', i.fire_settings -> 'horizon_lifespan_age'
    ))
FROM installation i
WHERE u.retirement_profile IS NULL
  AND i.fire_settings IS NOT NULL;

UPDATE installation
SET fire_settings = fire_settings
    - 'fire_number_mode' - 'fire_number_manual_amount' - 'swr_pct' - 'horizon_lifespan_age'
WHERE fire_settings IS NOT NULL
  AND (fire_settings ? 'fire_number_mode'
    OR fire_settings ? 'fire_number_manual_amount'
    OR fire_settings ? 'swr_pct'
    OR fire_settings ? 'horizon_lifespan_age');
