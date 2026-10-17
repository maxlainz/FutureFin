-- 5.0.0 (issue #207, decisión D14 del owner — DATA-CHANGING, firmada el 2026-09-02).
--
-- Con proyecciones INDEPENDIENTES por miembro (D9) una fila sin dueño no entra en ninguna
-- proyección: las filas `owner_user_id IS NULL` (legado anterior a 2026-02-16 o imports de
-- backups muy viejos; ningún camino vivo de la API escribe NULL desde entonces) pasan al
-- **owner más antiguo de la instalación** (`installation_memberships.role = 'owner'`, orden
-- `created_at, user_id`) y la columna queda `NOT NULL` en las CINCO tablas que la tenían
-- nullable: assets, liabilities, budget_entries, planning_flows, allocation_rules. El resto del
-- ledger (transactions y satélites, history_snapshots…) ya era NOT NULL.
--
-- Reglas de asignación (`allocation_rules`) — la única tabla con un invariante entre filas
-- (I1: como mucho UN `remainder` sin tope por scope, y siempre el ÚLTIMO por (priority, id)):
--   1. si el owner ya tiene su sumidero y el scope compartido tenía otro, el compartido se
--      BORRA (dos sumideros en un scope es un estado inválido; el del owner es el que gobierna
--      su proyección desde 4.12.0/#150);
--   2. el resto de reglas compartidas se mueven al owner DETRÁS de sus reglas actuales
--      (`priority` renumerada tras el máximo del owner), en su mismo orden relativo;
--   3. el sumidero del owner se recoloca al final para seguir siendo el último.
-- El resultado respeta I1 por construcción; `commit_with_sink_invariant` lo re-verifica en la
-- primera escritura. El CHANGELOG 5.0.0 lo declara en §Breaking.
--
-- La FK pasa de `ON DELETE SET NULL` a `ON DELETE RESTRICT`: con NOT NULL un SET NULL sería un
-- error igualmente; RESTRICT lo dice de frente. Ningún camino de la API borra usuarios
-- (revocar una membresía conserva sus datos, `handlers/members.rs`).

-- 0. Owner más antiguo por instalación.
CREATE TEMP TABLE first_owner AS
SELECT installation_id, user_id
FROM (
    SELECT installation_id, user_id,
           row_number() OVER (PARTITION BY installation_id ORDER BY created_at ASC, user_id ASC) AS rn
    FROM installation_memberships
    WHERE role = 'owner'
) s
WHERE rn = 1;

-- 1. Sumideros compartidos redundantes: fuera.
DELETE FROM allocation_rules r
USING first_owner fo
WHERE r.installation_id = fo.installation_id
  AND r.owner_user_id IS NULL
  AND r.kind = 'remainder' AND r.cap_kind IS NULL
  AND EXISTS (
      SELECT 1 FROM allocation_rules o
      WHERE o.installation_id = fo.installation_id
        AND o.owner_user_id = fo.user_id
        AND o.kind = 'remainder' AND o.cap_kind IS NULL
  );

-- 2. Reglas compartidas → owner, renumeradas detrás de las suyas (mismo orden relativo).
WITH moved AS (
    SELECT r.id,
           fo.user_id,
           COALESCE((SELECT MAX(o.priority) FROM allocation_rules o
                     WHERE o.installation_id = r.installation_id
                       AND o.owner_user_id = fo.user_id), 0)
             + row_number() OVER (PARTITION BY r.installation_id ORDER BY r.priority ASC, r.id ASC)
             AS new_priority
    FROM allocation_rules r
    JOIN first_owner fo ON fo.installation_id = r.installation_id
    WHERE r.owner_user_id IS NULL
)
UPDATE allocation_rules r
SET owner_user_id = m.user_id,
    priority = m.new_priority
FROM moved m
WHERE r.id = m.id;

-- 3. El sumidero del owner vuelve al final de su cascada.
UPDATE allocation_rules s
SET priority = (SELECT MAX(o.priority) + 1 FROM allocation_rules o
                WHERE o.installation_id = s.installation_id
                  AND o.owner_user_id = s.owner_user_id
                  AND o.id <> s.id)
FROM first_owner fo
WHERE s.installation_id = fo.installation_id
  AND s.owner_user_id = fo.user_id
  AND s.kind = 'remainder' AND s.cap_kind IS NULL
  AND EXISTS (SELECT 1 FROM allocation_rules o
              WHERE o.installation_id = s.installation_id
                AND o.owner_user_id = s.owner_user_id
                AND o.id <> s.id
                AND (o.priority, o.id) > (s.priority, s.id));

-- 4. Las otras cuatro tablas: asignación directa.
UPDATE assets a SET owner_user_id = fo.user_id
FROM first_owner fo WHERE a.installation_id = fo.installation_id AND a.owner_user_id IS NULL;

UPDATE liabilities l SET owner_user_id = fo.user_id
FROM first_owner fo WHERE l.installation_id = fo.installation_id AND l.owner_user_id IS NULL;

UPDATE budget_entries b SET owner_user_id = fo.user_id
FROM first_owner fo WHERE b.installation_id = fo.installation_id AND b.owner_user_id IS NULL;

UPDATE planning_flows p SET owner_user_id = fo.user_id
FROM first_owner fo WHERE p.installation_id = fo.installation_id AND p.owner_user_id IS NULL;

-- 5. Cierre de la clase: NOT NULL + RESTRICT. Si alguna instalación tuviera filas huérfanas sin
-- ningún owner (imposible: el primer usuario es owner y la guardia del último owner impide
-- quedarse sin él), el SET NOT NULL falla EN ALTO y el arranque se detiene sin tocar nada más.
ALTER TABLE assets
    ALTER COLUMN owner_user_id SET NOT NULL,
    DROP CONSTRAINT IF EXISTS assets_owner_user_id_fkey,
    ADD CONSTRAINT assets_owner_user_id_fkey
        FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE RESTRICT;

ALTER TABLE liabilities
    ALTER COLUMN owner_user_id SET NOT NULL,
    DROP CONSTRAINT IF EXISTS liabilities_owner_user_id_fkey,
    ADD CONSTRAINT liabilities_owner_user_id_fkey
        FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE RESTRICT;

ALTER TABLE budget_entries
    ALTER COLUMN owner_user_id SET NOT NULL,
    DROP CONSTRAINT IF EXISTS budget_entries_owner_user_id_fkey,
    ADD CONSTRAINT budget_entries_owner_user_id_fkey
        FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE RESTRICT;

ALTER TABLE planning_flows
    ALTER COLUMN owner_user_id SET NOT NULL,
    DROP CONSTRAINT IF EXISTS planning_flows_owner_user_id_fkey,
    ADD CONSTRAINT planning_flows_owner_user_id_fkey
        FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE RESTRICT;

ALTER TABLE allocation_rules
    ALTER COLUMN owner_user_id SET NOT NULL,
    DROP CONSTRAINT IF EXISTS allocation_rules_owner_user_id_fkey,
    ADD CONSTRAINT allocation_rules_owner_user_id_fkey
        FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE RESTRICT;

DROP TABLE first_owner;
