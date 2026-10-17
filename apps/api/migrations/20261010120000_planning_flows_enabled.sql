-- 5.0.0: cada flujo de la pestaña «Simulación» (antes «Próximos») lleva un interruptor.
-- `enabled = false` ⇒ el flujo NO entra en la proyección ni en ningún KPI (totales `upcoming_*`
-- del Resumen, tramo `planning` de la resolución de reparto, eventos del chart), pero la fila
-- sigue existiendo, se lista y se puede volver a encender. Los flujos existentes quedan
-- encendidos: el default reproduce exactamente el comportamiento anterior.
ALTER TABLE planning_flows ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT true;
