-- 6.0.0 F2-1 — «Planes»: tablas del plan, sus bloques y sus cambios (aditiva).
--
-- Plan de implementación: `.claude/rfc/planes-6.0-f2.md` §Migraciones 1; diseño: `.claude/rfc/planes-6.0.md`.
-- En F2-1 NADA lee estas tablas desde la proyección (el cableado es F2-4): ninguna cifra se mueve.
--
-- Propiedad (D-16/D21): `owner_user_id` NULL = plan de HOGAR. Solo existe en el esquema hasta F3:
-- la API rechaza crearlos o escribirlos (`household_plans_not_available`). `ON DELETE CASCADE` y no
-- `SET NULL` como en las tablas del ledger: un plan huérfano se convertiría en un plan de hogar que
-- nadie creó.

CREATE TABLE plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (
        char_length(trim(name)) > 0
        AND char_length(name) <= 200
    ),
    is_active BOOLEAN NOT NULL DEFAULT false,
    -- `PlanAssumptions` (handlers/plans/validate.rs). Las claves ausentes toman su default al leer.
    assumptions JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(assumptions) = 'object'),
    -- Sube en TODA escritura del plan, de sus bloques o de sus cambios (y al activarlo o
    -- desactivarlo). F2-4 lo usa como sello de cache: `(plan_id, revision)`.
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision >= 1),
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX plans_installation_owner_idx ON plans (installation_id, owner_user_id);

-- Exactamente un plan activo como mucho por persona…
CREATE UNIQUE INDEX plans_one_active_per_owner ON plans (installation_id, owner_user_id)
WHERE is_active AND owner_user_id IS NOT NULL;

-- …y uno como mucho de hogar.
CREATE UNIQUE INDEX plans_one_active_household ON plans (installation_id)
WHERE is_active AND owner_user_id IS NULL;

CREATE TABLE plan_blocks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id UUID NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (
        char_length(trim(name)) > 0
        AND char_length(name) <= 200
    ),
    enabled BOOLEAN NOT NULL DEFAULT true,
    sort_index INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Destino de la FK compuesta de `plan_changes`: un cambio no puede colgar de un bloque de
    -- OTRO plan.
    UNIQUE (id, plan_id)
);

CREATE INDEX plan_blocks_plan_idx ON plan_blocks (plan_id, sort_index);

CREATE TABLE plan_changes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Denormalizado a propósito: hace expresables como FK «el bloque es de este plan» y «el
    -- activo que se vende entró en este plan».
    plan_id UUID NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    block_id UUID NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('income', 'expense', 'asset_in', 'asset_sale', 'loan')),
    start_date DATE NOT NULL,
    -- Inclusivo. NULL = sin fin (solo mensuales).
    end_date DATE NULL,
    -- Euros de hoy (o % con `amount_unit = 'pct'`). Con signo en ingreso/gasto (sube/baja);
    -- positivo en el resto. NULL solo en `asset_sale` = vender el activo entero.
    amount NUMERIC(18, 4) NULL,
    amount_unit TEXT NOT NULL DEFAULT 'eur' CHECK (amount_unit IN ('eur', 'pct')),
    cadence TEXT NOT NULL DEFAULT 'monthly' CHECK (cadence IN ('monthly', 'one_off')),
    keeps_purchasing_power BOOLEAN NOT NULL DEFAULT true,
    -- Activo REAL que se vende. `SET NULL`: borrar el activo deja la venta sin sujeto (la API lo
    -- publica tal cual; F2-4 decide cómo la trata el motor). Por eso el CHECK de abajo es «como
    -- mucho uno», no «exactamente uno»: la API exige exactamente uno al escribir.
    asset_id UUID NULL REFERENCES assets (id) ON DELETE SET NULL,
    -- `asset_in` del MISMO plan que se vende.
    target_change_id UUID NULL,
    -- `PlanChangeParams`, validado por tipo en la API (handlers/plans/validate.rs).
    params JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(params) = 'object'),
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, plan_id),
    FOREIGN KEY (block_id, plan_id) REFERENCES plan_blocks (id, plan_id) ON DELETE CASCADE,
    FOREIGN KEY (target_change_id, plan_id) REFERENCES plan_changes (id, plan_id)
        ON DELETE SET NULL (target_change_id),
    CONSTRAINT plan_changes_end_after_start CHECK (end_date IS NULL OR end_date >= start_date),
    CONSTRAINT plan_changes_one_off_has_no_end CHECK (cadence = 'monthly' OR end_date IS NULL),
    -- «pct solo mensual».
    CONSTRAINT plan_changes_pct_only_monthly CHECK (amount_unit = 'eur' OR cadence = 'monthly'),
    CONSTRAINT plan_changes_flow_shape CHECK (
        kind NOT IN ('income', 'expense')
        OR (amount IS NOT NULL AND amount <> 0)
    ),
    CONSTRAINT plan_changes_asset_in_shape CHECK (
        kind <> 'asset_in'
        OR (
            amount IS NOT NULL AND amount > 0
            AND amount_unit = 'eur'
            AND cadence = 'one_off'
        )
    ),
    CONSTRAINT plan_changes_asset_sale_shape CHECK (
        kind <> 'asset_sale'
        OR (
            NOT (asset_id IS NOT NULL AND target_change_id IS NOT NULL)
            AND (amount IS NULL OR amount > 0)
            AND (amount IS NOT NULL OR (cadence = 'one_off' AND amount_unit = 'eur'))
        )
    ),
    -- «loan siempre nominal».
    CONSTRAINT plan_changes_loan_shape CHECK (
        kind <> 'loan'
        OR (
            amount IS NOT NULL AND amount > 0
            AND amount_unit = 'eur'
            AND cadence = 'one_off'
            AND NOT keeps_purchasing_power
        )
    ),
    CONSTRAINT plan_changes_subject_only_on_sales CHECK (
        kind = 'asset_sale'
        OR (asset_id IS NULL AND target_change_id IS NULL)
    ),
    CONSTRAINT plan_changes_not_self_target CHECK (target_change_id IS NULL OR target_change_id <> id)
);

CREATE INDEX plan_changes_plan_idx ON plan_changes (plan_id, start_date);
CREATE INDEX plan_changes_block_idx ON plan_changes (block_id);
CREATE INDEX plan_changes_asset_idx ON plan_changes (asset_id) WHERE asset_id IS NOT NULL;
CREATE INDEX plan_changes_target_idx ON plan_changes (target_change_id)
WHERE target_change_id IS NOT NULL;
