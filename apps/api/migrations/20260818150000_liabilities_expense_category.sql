-- Categoría de gasto de la cuota: el pasivo publica su cuota en el presupuesto bajo esta
-- categoría (panel «Derivado de pasivos» + lado Budget de la comparativa de Movimientos).
-- Nullable: los pasivos existentes quedan sin asignar (comportamiento previo intacto) y el
-- usuario la asigna vía PATCH/formulario; la API la exige solo al CREAR pasivos nuevos.
-- ON DELETE SET NULL (mismo trato que categorization_rules.assign_category_id): borrar la
-- categoría degrada la atribución sin bloquear el borrado; el remap de categorías la actualiza.
ALTER TABLE liabilities
    ADD COLUMN expense_category_id UUID REFERENCES categories(id) ON DELETE SET NULL;
