-- Histórico de gasto mensual: transacciones datadas + reglas de categorización (v1.6.0).
--
-- FutureFin modelaba flujos recurrentes (`budget_entries`) y snapshots de patrimonio; no
-- existía ninguna transacción datada. Estas tres tablas guardan el histórico REAL de gasto
-- (importado de CSV bancarios o metido a mano como efectivo), su categorización y las reglas
-- aprendidas al confirmar un import.
--
--   - transaction_imports    → cabecera de un lote de import (un CSV). Borrarla deshace el import.
--   - transactions           → un movimiento datado, firmado (negativo = cargo). Per-user.
--   - categorization_rules   → reglas que PRE-asignan kind+categoría en el preview del import.
--
-- Decisiones de diseño (ver plan v1.6.0):
--   * `source` es TEXT sin CHECK de valores: un banco nuevo no debe exigir migración (se valida
--     en Rust). En `categorization_rules.source`, NULL = regla agnóstica (cualquier banco).
--   * Las transacciones NO son inputs del motor de proyección → sus mutaciones nunca invalidan
--     la cache de proyección (contrato del módulo handlers/transactions, test de regresión).
--   * `linked_asset_id` / `linked_liability_id` son ON DELETE SET NULL: el movimiento sobrevive
--     al borrado de la fila de ledger (contraste deliberado con `source_item_id` de history, que
--     no tiene FK). `category_id` es ON DELETE RESTRICT (categoría en uso no se borra sin remap).
--   * `import_id` ON DELETE CASCADE con NULL para movimientos manuales (efectivo).
--   * Dedup por huella: UNIQUE (installation, owner, fingerprint, fingerprint_ordinal). La huella
--     se computa en Rust (source · op_date ISO · importe canónico 4dp · concepto normalizado);
--     el ordinal distingue ocurrencias repetidas del mismo movimiento en el mismo archivo.

CREATE TABLE transaction_imports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    source TEXT NOT NULL,                                   -- validado en Rust (myinvestor|n26|manual|…)
    account_asset_id UUID REFERENCES assets (id) ON DELETE SET NULL,  -- cuenta origen (metadata)
    original_filename TEXT CHECK (original_filename IS NULL OR char_length(original_filename) <= 300),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX transaction_imports_owner_created_idx
    ON transaction_imports (installation_id, owner_user_id, created_at DESC);

CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    import_id UUID REFERENCES transaction_imports (id) ON DELETE CASCADE,  -- NULL = manual (efectivo)
    source TEXT NOT NULL,
    op_date DATE NOT NULL,                                  -- fecha de operación/booking (referencia)
    value_date DATE,
    concept TEXT NOT NULL CHECK (char_length(concept) <= 500 AND char_length(trim(concept)) > 0),
    amount NUMERIC(18,4) NOT NULL,                          -- firmado: negativo = cargo
    currency CHAR(3) NOT NULL DEFAULT 'EUR' CHECK (currency = 'EUR'),
    kind TEXT CHECK (kind IS NULL OR kind IN ('expense', 'income', 'savings')),
    category_id UUID REFERENCES categories (id) ON DELETE RESTRICT,  -- nullable ("Sin categoría")
    fingerprint TEXT NOT NULL,
    fingerprint_ordinal INT NOT NULL DEFAULT 0,
    linked_asset_id UUID REFERENCES assets (id) ON DELETE SET NULL,        -- destino savings (metadata)
    linked_liability_id UUID REFERENCES liabilities (id) ON DELETE SET NULL, -- cuota de préstamo (metadata)
    notes TEXT CHECK (notes IS NULL OR char_length(notes) <= 4000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT transactions_unique_fingerprint
        UNIQUE (installation_id, owner_user_id, fingerprint, fingerprint_ordinal)
);
CREATE INDEX transactions_installation_op_date_idx ON transactions (installation_id, op_date);
CREATE INDEX transactions_owner_op_date_idx ON transactions (installation_id, owner_user_id, op_date);
CREATE INDEX transactions_installation_category_idx ON transactions (installation_id, category_id);
CREATE INDEX transactions_import_idx ON transactions (import_id);

CREATE TABLE categorization_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    installation_id UUID NOT NULL REFERENCES installation (id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    match_kind TEXT NOT NULL DEFAULT 'substring'
        CHECK (match_kind IN ('substring', 'prefix', 'exact')),
    pattern TEXT NOT NULL CHECK (char_length(pattern) <= 500 AND char_length(trim(pattern)) > 0),
    source TEXT,                                            -- NULL = cualquier banco (agnóstica)
    assign_kind TEXT CHECK (assign_kind IS NULL OR assign_kind IN ('expense', 'income', 'savings')),
    assign_category_id UUID REFERENCES categories (id) ON DELETE SET NULL,  -- regla degradada no bloquea borrado
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT categorization_rules_unique UNIQUE (installation_id, owner_user_id, source, pattern)
);
CREATE INDEX categorization_rules_owner_idx ON categorization_rules (installation_id, owner_user_id);
