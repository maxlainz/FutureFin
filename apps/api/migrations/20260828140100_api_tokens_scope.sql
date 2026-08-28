-- Scope de la credencial: un token de API puede ser de SOLO LECTURA (Fase 3, issue #84).
--
-- Hasta aquí la única forma de emitir una credencial que no escribiera era degradar a la persona
-- a `viewer` — que además le quita la escritura en la web. Un token para «que Claude me analice
-- los gastos» podía ejecutar las 31 tools de escritura sobre el hogar entero.
--
-- El scope solo RESTA: nunca concede nada que el rol vivo del usuario no conceda ya. El orden de
-- las puertas en `require_mcp_write` es rol → scope → kill-switch de la instalación.
--
-- SEGURIDAD SOBRE UNA BASE CON DATOS. `ADD COLUMN … NOT NULL DEFAULT` no reescribe la tabla en
-- PostgreSQL 11+ (el default vive en el catálogo), y el default `read_write` es EXACTAMENTE el
-- comportamiento anterior: todos los tokens ya emitidos siguen funcionando igual, byte a byte.
-- El CHECK se valida contra las filas existentes, que todas llevan `read_write` — pasa.
ALTER TABLE api_tokens
ADD COLUMN scope TEXT NOT NULL DEFAULT 'read_write';

ALTER TABLE api_tokens
ADD CONSTRAINT api_tokens_scope_valid CHECK (scope IN ('read_write', 'read_only'));
