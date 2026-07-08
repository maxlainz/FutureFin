-- Reglas de transacción recurrente: plantillas que materializan movimientos reales (v1.7.0).
--
-- El histórico de gasto (v1.6.0) sólo tenía movimientos importados o metidos a mano de uno en
-- uno. Una regla recurrente es una PLANTILLA per-user (nómina, alquiler, aportación mensual…):
-- guarda el concepto/importe/kind/categoría/enlaces y el día del mes, y un endpoint la
-- "materializa" generando transacciones reales en `transactions` (source='manual'), una por mes
-- civil vencido. No es un input del motor de proyección → materializar NO invalida la cache
-- (mismo contrato no-cache que el resto del módulo handlers/transactions).
--
-- Decisiones de diseño:
--   * `amount` es firmado (negativo = cargo), como en `transactions`; CHECK <> 0.
--   * `category_id` es ON DELETE RESTRICT (igual que `transactions.category_id`): la regla genera
--     transacciones reales con esa categoría, así que una categoría en uso por una regla no debe
--     borrarse sin remap. `linked_asset_id`/`linked_liability_id` son ON DELETE SET NULL (metadata,
--     como en `transactions`): la regla sobrevive al borrado de la fila de ledger.
--   * `last_materialized_month` es un cursor monotónico de idempotencia (siempre el día 1 del mes):
--     el mes más reciente ya materializado. Materializar avanza mes a mes desde ahí hasta el mes
--     civil actual, pero SIN crear fechas futuras: el mes en curso sólo se materializa cuando su
--     día de recurrencia (con clamp de fin de mes) ya ha llegado; nunca retrocede. Al crear una
--     regla se fija al primer día del mes del alta, de modo que la instancia de origen (el
--     movimiento que dispara la recurrencia) no se duplica.
--   * SIN UNIQUE(rule, ym): a propósito. Si el usuario borra una instancia materializada, re-ejecutar
--     materialize NO debe recrearla (el cursor ya pasó ese mes). La ausencia de constraint deja que
--     el cursor sea la única fuente de verdad de idempotencia.
--   * `transactions.recurring_rule_id` es ON DELETE SET NULL: borrar la regla conserva las
--     instancias ya materializadas (se quedan como movimientos manuales sueltos, sin plantilla).

CREATE TABLE recurring_transaction_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    concept TEXT NOT NULL CHECK (char_length(concept) <= 500 AND char_length(trim(concept)) > 0),
    amount NUMERIC(18,4) NOT NULL CHECK (amount <> 0),        -- firmado: negativo = cargo
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'savings')),
    category_id UUID REFERENCES categories (id) ON DELETE RESTRICT,  -- genera transacciones reales
    day_of_month INT NOT NULL CHECK (day_of_month BETWEEN 1 AND 31),
    linked_asset_id UUID REFERENCES assets (id) ON DELETE SET NULL,
    linked_liability_id UUID REFERENCES liabilities (id) ON DELETE SET NULL,
    notes TEXT CHECK (notes IS NULL OR char_length(notes) <= 4000),
    last_materialized_month DATE NOT NULL,   -- cursor monotónico de idempotencia (primer día de mes)
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX recurring_transaction_rules_owner_idx
    ON recurring_transaction_rules (installation_id, owner_user_id);

ALTER TABLE transactions
    ADD COLUMN recurring_rule_id UUID REFERENCES recurring_transaction_rules (id) ON DELETE SET NULL;
CREATE INDEX transactions_recurring_rule_idx ON transactions (recurring_rule_id);
