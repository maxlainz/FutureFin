-- Allocation rules: cascada de reglas que distribuyen el sobrante mensual
-- (income − expense − debt_service) entre activos. Reemplaza los campos
-- monthly_contribution_fixed/contribution_remainder_weight/contribution_cap_*
-- que vivían en `assets`.
--
-- Cada regla:
--   - target_asset_id  → destino
--   - priority         → orden ascendente de aplicación
--   - kind             → 'fixed' | 'percent' | 'remainder'
--   - amount           → € (fixed) | 0..100 (percent) | NULL (remainder)
--   - cap_kind/value   → tope opcional sobre el current_value del destino

CREATE TABLE IF NOT EXISTS allocation_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation(id) ON DELETE CASCADE,
    owner_user_id UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    target_asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    priority INTEGER NOT NULL,
    kind TEXT NOT NULL,
    amount NUMERIC(18, 4) NULL,
    cap_kind TEXT NULL,
    cap_value NUMERIC(18, 4) NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    notes TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT alloc_rules_kind_chk
        CHECK (kind IN ('fixed', 'percent', 'remainder')),
    CONSTRAINT alloc_rules_amount_chk
        CHECK (
            (kind = 'remainder' AND amount IS NULL)
            OR (kind = 'fixed'     AND amount IS NOT NULL AND amount >= 0)
            OR (kind = 'percent'   AND amount IS NOT NULL AND amount >= 0 AND amount <= 100)
        ),
    CONSTRAINT alloc_rules_cap_kind_chk
        CHECK (cap_kind IS NULL
               OR cap_kind IN ('amount', 'months_expense', 'income_multiple')),
    CONSTRAINT alloc_rules_cap_pair_chk
        CHECK (
            (cap_kind IS NULL AND cap_value IS NULL)
            OR (cap_kind IS NOT NULL AND cap_value IS NOT NULL AND cap_value >= 0)
        )
);

CREATE INDEX IF NOT EXISTS idx_alloc_rules_inst_priority
    ON allocation_rules(installation_id, priority);
CREATE INDEX IF NOT EXISTS idx_alloc_rules_owner
    ON allocation_rules(installation_id, owner_user_id);
CREATE INDEX IF NOT EXISTS idx_alloc_rules_target
    ON allocation_rules(target_asset_id);
