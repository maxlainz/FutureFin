-- 4.15.0 — Categoría por defecto («fallback») por scope + categoría OBLIGATORIA en
-- ingresos/gastos.
--
-- **DATA-CHANGING** — firmada por el owner (2026-09-02). Rellena `transactions.category_id` y
-- `recurring_transaction_rules.category_id` donde estaban a NULL, y después lo blinda con dos
-- CHECK. **No hay vuelta atrás por SQL**: soltar los CHECK NO devuelve los NULL, porque el
-- estado «sin categoría» deja de existir en cuanto se escribe la categoría por defecto. La vía
-- de vuelta es el dump `pre-migration-*.sql.gz` que el entrypoint escribe en el volumen ANTES
-- de aplicar migraciones nuevas.
--
-- **Idempotente en su parte de datos** (los pasos 2-5 se pueden reejecutar sin efecto), pero
-- **NO en los `ADD CONSTRAINT`** de los pasos 6-7: PostgreSQL 16 no tiene
-- `ADD CONSTRAINT IF NOT EXISTS` y eso es lo correcto — sqlx no reaplica una migración ya
-- registrada, y una réplica manual debe fallar alto en vez de tragarse la diferencia.
--
-- Por qué el CHECK de `transactions` deja fuera a `kind IS NULL`: la columna es NULLABLE desde
-- `20260707120000_transactions_and_rules`, y una fila SIN clase es «sin clasificar», no «un
-- gasto sin categoría». Obligarle una categoría sería inventarse la clase. En
-- `recurring_transaction_rules` el `kind` es NOT NULL, así que ahí el CHECK no necesita la
-- salvedad.

-- 1. La marca. Una sola por instalación y scope (índice único PARCIAL: las no marcadas no
--    compiten), y solo tiene sentido en las clases que llevan categoría.
ALTER TABLE categories
    ADD COLUMN IF NOT EXISTS is_fallback BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE categories
    ADD CONSTRAINT categories_fallback_scope_check
    CHECK (NOT is_fallback OR scope IN ('income', 'expense'));

CREATE UNIQUE INDEX IF NOT EXISTS categories_one_fallback_per_scope_idx
    ON categories (installation_id, scope)
    WHERE is_fallback;

-- 2. Adoptar la categoría sembrada que ya hacía de cajón, por NOMBRE («Otros gastos» /
--    «Otros ingresos» de `seed_default_categories`). Renombrada o borrada por el usuario, no
--    casa y el paso 3 crea una nueva: nunca se pisa una designación previa.
UPDATE categories AS c
   SET is_fallback = true
 WHERE c.scope IN ('income', 'expense')
   AND c.name = CASE c.scope WHEN 'expense' THEN 'Otros gastos' ELSE 'Otros ingresos' END
   AND NOT EXISTS (
       SELECT 1 FROM categories AS f
        WHERE f.installation_id = c.installation_id
          AND f.scope = c.scope
          AND f.is_fallback
   );

-- 3. Crear la que falte, por instalación y scope, al final del orden de su scope.
INSERT INTO categories (installation_id, scope, name, sort_index, is_fallback)
SELECT i.id,
       v.scope,
       v.name,
       COALESCE(
           (SELECT MAX(c2.sort_index) FROM categories AS c2
             WHERE c2.installation_id = i.id AND c2.scope = v.scope),
           -1
       ) + 1,
       true
  FROM installation AS i
  CROSS JOIN (VALUES ('income', 'Otros ingresos'), ('expense', 'Otros gastos')) AS v(scope, name)
 WHERE NOT EXISTS (
     SELECT 1 FROM categories AS f
      WHERE f.installation_id = i.id AND f.scope = v.scope AND f.is_fallback
 )
ON CONFLICT (installation_id, scope, name) DO NOTHING;

-- 4. Backfill de los movimientos. `updated_at` NO se toca a propósito: para el usuario el
--    movimiento no ha cambiado (ni importe, ni fecha, ni concepto), y mover la marca de tiempo
--    de miles de filas por una migración enmascararía sus ediciones reales.
UPDATE transactions AS t
   SET category_id = f.id
  FROM categories AS f
 WHERE t.category_id IS NULL
   AND t.kind IN ('income', 'expense')
   AND f.installation_id = t.installation_id
   AND f.scope = t.kind
   AND f.is_fallback;

-- 5. Mismo backfill en las plantillas recurrentes (su `kind` es NOT NULL).
UPDATE recurring_transaction_rules AS r
   SET category_id = f.id
  FROM categories AS f
 WHERE r.category_id IS NULL
   AND r.kind IN ('income', 'expense')
   AND f.installation_id = r.installation_id
   AND f.scope = r.kind
   AND f.is_fallback;

-- 6-7. El blindaje. A partir de aquí `category_id IS NULL` implica «sin clasificar»
--      (`kind IS NULL`) o «inversión» (`kind = 'savings'`), nunca «gasto/ingreso sin
--      categorizar»: ese estado deja de ser representable.
ALTER TABLE transactions
    ADD CONSTRAINT transactions_category_required_check
    CHECK (kind IS NULL OR kind = 'savings' OR category_id IS NOT NULL);

ALTER TABLE recurring_transaction_rules
    ADD CONSTRAINT recurring_rules_category_required_check
    CHECK (kind = 'savings' OR category_id IS NOT NULL);
