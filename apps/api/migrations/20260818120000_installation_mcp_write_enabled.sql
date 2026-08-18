-- Kill-switch de la escritura vía MCP (issue #3): ajuste vivo de la instalación, toggle
-- owner-only en Ajustes → MCP. Default TRUE: la escritura funciona out-of-the-box y quien
-- quiera un conector solo-lectura lo apaga desde la GUI (efecto inmediato por request).
ALTER TABLE installation ADD COLUMN mcp_write_enabled BOOLEAN NOT NULL DEFAULT TRUE;
