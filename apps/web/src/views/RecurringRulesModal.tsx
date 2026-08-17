/**
 * Reglas de movimientos recurrentes (modal). Lista `GET /v1/transactions/recurring` y permite
 * detener (borrar) cada regla. Detener una regla NO borra los movimientos ya materializados; solo
 * evita que se generen los futuros. Patrón de ManualCashEntryModal: fetch al abrir; toda la lógica
 * de presentación vive aquí (nada en lib/, que es puro).
 */

import { useCallback, useEffect, useState } from "react";
import { apiDelete, apiGet } from "../api/client";
import type { RecurringRuleApi } from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { formatCurrencyAmount, parseDisplayDecimal } from "../lib/format";
import { KIND_LABEL_ES } from "../lib/expenses";

export function RecurringRulesModal({
  open,
  onClose,
  currencyIso,
  onChanged,
}: {
  open: boolean;
  onClose: () => void;
  currencyIso: string;
  /** Se llama tras detener una regla (por si la vista quiere refrescar). */
  onChanged: () => void;
}) {
  const [rules, setRules] = useState<RecurringRuleApi[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await apiGet<RecurringRuleApi[]>("/v1/transactions/recurring");
      setRules(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudieron cargar las reglas.");
      setRules([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) void load();
  }, [open, load]);

  const stopRule = useCallback(
    async (id: string) => {
      setDeletingId(id);
      setError(null);
      try {
        await apiDelete(`/v1/transactions/recurring/${id}`);
        setRules((rs) => rs.filter((r) => r.id !== id));
        onChanged();
      } catch (e) {
        setError(e instanceof Error ? e.message : "No se pudo detener la regla.");
      } finally {
        setDeletingId(null);
      }
    },
    [onChanged],
  );

  return (
    <Modal title="Movimientos recurrentes" open={open} onClose={onClose} wide>
      <div className="stack recurring-rules">
        <ModalFormError message={error} />
        <p className="muted tight">
          Cada mes se crean automáticamente estos movimientos. Detener una regla no
          borra los movimientos ya creados.
        </p>
        {loading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : rules.length === 0 ? (
          <p className="muted bordered-top">Sin reglas recurrentes.</p>
        ) : (
          <ul className="recurring-list bordered-top">
            {rules.map((r) => {
              const amountNum = parseDisplayDecimal(r.amount) ?? 0;
              const sign = r.kind === "income" ? "+" : "−";
              const amountLabel = `${sign}${formatCurrencyAmount(
                String(Math.abs(amountNum)),
                currencyIso,
              )}`;
              const categoryLabel =
                r.kind === "savings"
                  ? KIND_LABEL_ES.savings
                  : r.category_name ?? "Sin categoría";
              return (
                <li className="recurring-item" key={r.id}>
                  <div className="recurring-item-main">
                    <span className="recurring-item-concept">{r.concept}</span>
                    <span className="recurring-item-meta muted">
                      {categoryLabel} · a mes cerrado
                    </span>
                  </div>
                  <span className="recurring-item-amount num">{amountLabel}</span>
                  <button
                    type="button"
                    className="btn ghost danger"
                    disabled={deletingId === r.id}
                    onClick={() => void stopRule(r.id)}
                  >
                    {deletingId === r.id ? "Deteniendo…" : "Detener"}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <div className="asset-form-actions">
          <button type="button" className="btn ghost" onClick={onClose}>
            Cerrar
          </button>
        </div>
      </div>
    </Modal>
  );
}
