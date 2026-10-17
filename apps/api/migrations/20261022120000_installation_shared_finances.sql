-- 6.0.0 (RFC Planes, D-16): «finanzas compartidas», interruptor de la instalación, owner-only.
-- Es una cuestión de PERMISOS y visualización, no de propiedad: con `true`, cualquier miembro con
-- rol de escritura puede editar, borrar y reordenar cualquier fila del ledger del hogar, y la
-- fila conserva su `owner_user_id`. Con `false` (default) rige D23 tal cual: solo el dueño de la
-- fila la cambia (403 `not_row_owner`). Columna propia y no JSONB, como `mcp_write_enabled`: es un
-- permiso vivo que se lee en cada escritura, no un supuesto de simulación.
ALTER TABLE installation ADD COLUMN shared_finances BOOLEAN NOT NULL DEFAULT false;
