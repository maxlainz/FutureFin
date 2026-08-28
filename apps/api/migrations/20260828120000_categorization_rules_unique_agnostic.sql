-- Reglas de categorización agnósticas de banco (`source IS NULL`): cerrar el agujero de unicidad.
--
-- PROBLEMA. `categorization_rules_unique UNIQUE (installation_id, owner_user_id, source, pattern)`
-- (migración 20260707120000) no atrapa las reglas agnósticas: en SQL `NULL <> NULL`, así que dos
-- filas con `source IS NULL` y el MISMO patrón nunca colisionan. Y las agnósticas son justo las que
-- se crean por defecto, porque `source` es opcional: dos llamadas idénticas a
-- `POST /v1/transactions/rules` (o a la tool MCP `create_categorization_rule`) devolvían 200 las dos
-- veces y dejaban dos reglas con ids distintos, pese a que la descripción de la tool promete
-- «Duplicado (source, pattern) → resource conflict». Un agente que reintenta tras un timeout
-- envenena la categorización de todos los imports futuros, y las reglas contradictorias «ganan por
-- precedencia, no por acierto».
--
-- QUÉ HACE ESTA MIGRACIÓN. Deduplica lo que ya exista y crea un índice único PARCIAL para la mitad
-- que la constraint no cubre. No toca `categorization_rules_unique` (sigue cubriendo `source`
-- no-NULL), así que el `ON CONFLICT ON CONSTRAINT` de `learn_rule` —que siempre inserta con un
-- `source` concreto— queda intacto.
--
-- POR QUÉ ES SEGURA SOBRE UNA BASE QUE YA TIENE DUPLICADOS. `CREATE UNIQUE INDEX` falla si quedan
-- duplicados, y una migración que falla deja el contenedor sin arrancar: hay que deduplicar ANTES,
-- en la misma migración, y hacerlo sin cambiar ningún número. La clave del índice incluye
-- `match_kind` precisamente para que el borrado sea **demostrablemente inocuo**:
--
--   * La precedencia de `match_rule` (apps/api/src/handlers/transactions/rules.rs) ordena por
--     `(source específica, rank(match_kind), longitud del patrón, updated_at)`. Dos filas con el
--     mismo `(installation, owner, pattern, match_kind)` y `source IS NULL` empatan en los tres
--     primeros componentes: solo puede ganar la de `updated_at` mayor. Las demás **no pueden ganar
--     ningún matching, para ningún concepto, nunca**. Borrarlas no cambia la categorización de
--     nada: ni de lo ya importado (esto no reescribe transacciones) ni de lo que se importe después.
--   * Se conserva exactamente la que gana hoy: `MAX(updated_at)`, desempatando por `id` — el mismo
--     criterio determinista que `match_rule` aplica con `key > best_key` (estricto: el primero en
--     empate gana, y aquí ese orden lo fija el desempate por id).
--   * NO se tocan las filas que difieren en `match_kind` (p. ej. `exact` y `substring` sobre el
--     mismo patrón). Esas SÍ matchean conjuntos distintos de conceptos: borrar una cambiaría qué
--     se categoriza, y eso sería una migración con pérdida de datos, que en este repo exige firma
--     explícita del owner. Se quedan, y el índice las tolera a propósito.
--
-- Consecuencia deliberada: el índice es más ESTRECHO que la validación de la aplicación, que
-- rechaza con 409 `rule_duplicate` cualquier `(source efectivo, pattern)` repetido —sin mirar
-- `match_kind`— para cumplir literalmente lo que promete el contrato. Las combinaciones antiguas
-- que difieren en `match_kind` quedan por tanto grandfathered: siguen existiendo y funcionando,
-- pero no se pueden volver a crear.

DELETE FROM categorization_rules r
USING categorization_rules keep
WHERE r.source IS NULL
  AND keep.source IS NULL
  AND r.installation_id = keep.installation_id
  AND r.owner_user_id = keep.owner_user_id
  AND r.pattern = keep.pattern
  AND r.match_kind = keep.match_kind
  AND (keep.updated_at, keep.id) > (r.updated_at, r.id);

CREATE UNIQUE INDEX categorization_rules_unique_agnostic
    ON categorization_rules (installation_id, owner_user_id, pattern, match_kind)
    WHERE source IS NULL;
