-- 4.12.1 (#176) — el sumidero pasa a ser ESTRUCTURAL: se reactivan los deshabilitados.
--
-- Hasta 4.12.0 deshabilitar la regla «resto» era legal (el engine la saltaba y el sobrante caía
-- a `surplus_cash`). Con `surplus_cash` muerto, un sumidero apagado dejaría el sobrante de ese
-- scope FUERA del balance — dinero desapareciendo en silencio tras el upgrade, el modo de fallo
-- que esta casa persigue. La retro-siembra de 4.12.0 (`20260901150000`) no alcanzó estos scopes
-- (tenían sumidero, aunque apagado) y la pre-guardia nueva solo caza la transición
-- habilitado→deshabilitado: a quien ya lo tenía apagado le llega ESTA migración.
--
-- El espejo del import de backups vive en apps/api/src/handlers/backup_user/import.rs
-- (cross-referencia obligada: un .ffbackup viejo puede traer el sumidero deshabilitado).
UPDATE allocation_rules
SET enabled = true
WHERE kind = 'remainder'
  AND cap_kind IS NULL
  AND enabled = false;
