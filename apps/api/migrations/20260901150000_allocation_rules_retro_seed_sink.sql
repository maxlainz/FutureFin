-- #150/#178 (4.12.0) — RETRO-SIEMBRA del sumidero. DATA-CHANGING, ordenada por el owner el
-- 2026-08-31 («quiero eliminar surplus_cash; si no existe la regla, que se cree
-- automáticamente») — revierte el «sin retro-siembra» de la auditoría, decisión del mismo owner.
--
-- Todo scope (instalación × owner, el scope NULL incluido — las reglas admiten owner NULL) con
-- ≥ 1 activo y CERO reglas `remainder` sin tope gana su sumidero. Sin `created_at` en `assets`
-- no existe «el primer activo que se creó»: aplica el criterio del owner — el activo LÍQUIDO de
-- menor rentabilidad esperada (NULL cuenta como 0; una negativa ordena antes), empate al de
-- mayor saldo; sin líquidos, los mismos criterios sobre todos. Prioridad: al final del scope
-- (la invariante del sumidero exige que sea el último por (priority, id)).
--
-- La MISMA regla de selección corre en Rust tras un import de backup pre-#150
-- (apps/api/src/handlers/backup_user/import.rs — cross-referencia obligada: si tocas un criterio
-- aquí, tócalo allí).
INSERT INTO allocation_rules (
    installation_id, owner_user_id, target_asset_id, priority,
    kind, amount, cap_kind, cap_value, enabled, notes
)
SELECT
    s.installation_id,
    s.owner_user_id,
    s.target_asset_id,
    COALESCE((
        SELECT MAX(r.priority) FROM allocation_rules r
        WHERE r.installation_id = s.installation_id
          AND r.owner_user_id IS NOT DISTINCT FROM s.owner_user_id
    ), 0) + 1,
    'remainder', NULL, NULL, NULL, true,
    'Regla «resto» sembrada automáticamente (4.12.0): el sobrante mensual deja de quedarse en caja al 0 %.'
FROM (
    SELECT DISTINCT ON (a.installation_id, a.owner_user_id)
           a.installation_id, a.owner_user_id, a.id AS target_asset_id
    FROM assets a
    ORDER BY a.installation_id, a.owner_user_id,
             a.is_liquid DESC,
             COALESCE(a.expected_annual_return_percent, 0) ASC,
             a.current_value DESC,
             a.id ASC
) s
WHERE NOT EXISTS (
    SELECT 1 FROM allocation_rules r
    WHERE r.installation_id = s.installation_id
      AND r.owner_user_id IS NOT DISTINCT FROM s.owner_user_id
      AND r.kind = 'remainder'
      AND r.cap_kind IS NULL
);
