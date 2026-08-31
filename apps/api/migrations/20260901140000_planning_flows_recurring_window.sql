-- #148 (4.11.0): «Próximos» gana flujos recurrentes con ventana.
--
-- `amount_basis` declara la BASE del importe (lección de archivo §2.19: no se duplica el campo,
-- se declara su unidad): 'one_off' = expected_amount es un TOTAL en €; 'per_month' = son €/MES
-- durante la ventana [window_start_date, window_end_date]. Las fechas son campos NUEVOS — no se
-- reusa due_date (tri-estado en el PATCH, atado a show_in_chart y a la cota de 100 años del
-- vencimiento puntual).
--
-- Migración puramente aditiva: el DEFAULT reproduce el comportamiento actual y cero filas
-- cambian de significado.
ALTER TABLE planning_flows
    ADD COLUMN amount_basis TEXT NOT NULL DEFAULT 'one_off'
        CHECK (amount_basis IN ('one_off', 'per_month')),
    ADD COLUMN window_start_date DATE NULL,
    ADD COLUMN window_end_date DATE NULL;

-- La forma de cada base: un puntual no lleva ventana; un recurrente no lleva vencimiento y
-- exige inicio (window_end_date NULL = abierto, sin fin — misma convención declarada que
-- liabilities.payment_end_date NULL).
ALTER TABLE planning_flows
    ADD CONSTRAINT planning_flows_basis_shape_chk CHECK (
        (amount_basis = 'one_off'
             AND window_start_date IS NULL AND window_end_date IS NULL)
        OR (amount_basis = 'per_month'
             AND due_date IS NULL AND window_start_date IS NOT NULL)
    ),
    ADD CONSTRAINT planning_flows_window_order_chk CHECK (
        window_end_date IS NULL OR window_start_date IS NULL
        OR window_end_date >= window_start_date
    );
