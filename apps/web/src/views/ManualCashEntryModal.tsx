/**
 * Alta manual de efectivo: grid multifila (`DraftItem[]`, patrón de HistorySettingsPanel) →
 * `POST /v1/transactions/batch`.
 *
 * El usuario teclea una MAGNITUD; el signo lo fija el kind (ingreso → +, gasto/inversión → −, que
 * es la convención firmada del backend: negativo = cargo). savings no admite categoría —el resto
 * la lleva SIEMPRE, preseleccionada a la por defecto—; el vínculo opcional es contextual
 * (savings → activo destino, gasto → pasivo).
 */

import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { apiPost } from "../api/client";
import type {
  AssetApiRow,
  BatchCreateTransactionsRequest,
  CategoryRow,
  CreateTransactionRequest,
  LiabilityApiRow,
  TransactionKindApi,
} from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { XIcon } from "../components/icons";
import { parseDisplayDecimal } from "../lib/format";
import {
  KIND_LABEL_ES,
  TRANSACTION_KINDS,
  categoriesForKind,
} from "../lib/expenses";

type CashDraft = {
  key: string;
  date: string;
  concept: string;
  amount: string;
  kind: TransactionKindApi;
  categoryId: string;
  linkedAssetId: string;
  linkedLiabilityId: string;
  /** «Repetir cada mes»: al confirmar, el item lleva `recurrence: {}`. */
  repeatMonthly: boolean;
};

export function ManualCashEntryModal({
  open,
  onClose,
  incomeCategories,
  expenseCategories,
  assets,
  liabilities,
  defaultDate,
  onSaved,
}: {
  open: boolean;
  onClose: () => void;
  incomeCategories: CategoryRow[];
  expenseCategories: CategoryRow[];
  assets: AssetApiRow[];
  liabilities: LiabilityApiRow[];
  defaultDate: string;
  onSaved: (count: number) => void;
}) {
  const keySeq = useRef(0);
  /**
   * Categoría POR DEFECTO del kind (4.15.0): la que recibe todo ingreso/gasto sin clasificar.
   * `""` para savings (no admite categoría) y también si el hogar todavía no tiene ninguna
   * marcada — ahí el `<select>` enseña su placeholder y el servidor resuelve por su cuenta.
   */
  const fallbackFor = useCallback(
    (kind: TransactionKindApi): string =>
      categoriesForKind(kind, incomeCategories, expenseCategories).find(
        (c) => c.is_fallback,
      )?.id ?? "",
    [incomeCategories, expenseCategories],
  );

  const blankRow = useCallback(
    (): CashDraft => {
      keySeq.current += 1;
      return {
        key: `cash-${keySeq.current}`,
        date: defaultDate,
        concept: "",
        amount: "",
        kind: "expense",
        // La fila nace clasificada: en ingreso/gasto «sin categoría» dejó de ser un estado
        // guardable, así que arrancar en blanco solo produciría un error al enviar.
        categoryId: fallbackFor("expense"),
        linkedAssetId: "",
        linkedLiabilityId: "",
        repeatMonthly: false,
      };
    },
    [defaultDate, fallbackFor],
  );

  const [rows, setRows] = useState<CashDraft[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      keySeq.current = 0;
      setRows([blankRow()]);
      setError(null);
      setSaving(false);
    }
  }, [open, blankRow]);

  const updateRow = useCallback(
    (key: string, patch: Partial<CashDraft>) => {
      setRows((prev) =>
        prev.map((r) => {
          if (r.key !== key) return r;
          const next = { ...r, ...patch };
          if (patch.kind !== undefined && patch.kind !== r.kind) {
            // Cambiar de ámbito invalida la categoría anterior: cae en la por defecto del
            // nuevo (vacía en savings, que no admite ninguna).
            next.categoryId = fallbackFor(next.kind);
            next.linkedAssetId = "";
            next.linkedLiabilityId = "";
          }
          if (next.kind === "savings") next.categoryId = "";
          return next;
        }),
      );
    },
    [fallbackFor],
  );

  const addRow = useCallback(() => setRows((p) => [...p, blankRow()]), [blankRow]);
  const removeRow = useCallback(
    (key: string) => setRows((p) => p.filter((r) => r.key !== key)),
    [],
  );

  function buildPayload():
    | { ok: true; items: CreateTransactionRequest[] }
    | { ok: false; error: string } {
    const items: CreateTransactionRequest[] = [];
    for (const r of rows) {
      const concept = r.concept.trim();
      const rawAmount = r.amount.trim();
      if (concept === "" && rawAmount === "") continue; // fila vacía → ignorar
      if (!r.date.trim()) return { ok: false, error: "Cada fila necesita una fecha." };
      if (concept === "") return { ok: false, error: "Cada movimiento necesita un concepto." };
      const parsed = parseDisplayDecimal(rawAmount);
      if (parsed === null || parsed <= 0) {
        return { ok: false, error: "Cada importe debe ser un número mayor que 0." };
      }
      const magnitude = rawAmount.replace(",", ".").replace(/^[+-]/, "");
      const signed = r.kind === "income" ? magnitude : `-${magnitude}`;
      const item: CreateTransactionRequest = {
        op_date: r.date,
        concept,
        amount: signed,
        kind: r.kind,
      };
      if (r.kind !== "savings" && r.categoryId) item.category_id = r.categoryId;
      if (r.kind === "savings" && r.linkedAssetId) item.linked_asset_id = r.linkedAssetId;
      if (r.kind === "expense" && r.linkedLiabilityId) {
        item.linked_liability_id = r.linkedLiabilityId;
      }
      if (r.repeatMonthly) item.recurrence = {};
      items.push(item);
    }
    if (items.length === 0) {
      return { ok: false, error: "Añade al menos un movimiento." };
    }
    return { ok: true, items };
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    const built = buildPayload();
    if (!built.ok) {
      setError(built.error);
      return;
    }
    setSaving(true);
    try {
      const body: BatchCreateTransactionsRequest = { transactions: built.items };
      await apiPost("/v1/transactions/batch", body);
      onSaved(built.items.length);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "No se pudo guardar.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal title="Añadir efectivo" open={open} onClose={onClose} wide>
      <form className="asset-form stack" onSubmit={submit}>
        <ModalFormError message={error} />
        <p className="muted tight">
          Escribe el importe en positivo; el signo lo pone el tipo (ingreso suma,
          gasto e inversión restan).
        </p>

        <div className="snapshot-items">
          {rows.length === 0 ? (
            <p className="muted tight">Sin filas. Añade al menos una.</p>
          ) : (
            rows.map((r) => {
              const cats = categoriesForKind(
                r.kind,
                incomeCategories,
                expenseCategories,
              );
              return (
                <div key={r.key} className="cash-entry-row">
                  <label className="field cash-entry-date">
                    <span>Fecha</span>
                    <input
                      type="date"
                      value={r.date}
                      max={defaultDate}
                      onChange={(e) => updateRow(r.key, { date: e.target.value })}
                    />
                  </label>
                  <label className="field cash-entry-concept">
                    <span>Concepto</span>
                    <input
                      value={r.concept}
                      maxLength={500}
                      autoComplete="off"
                      onChange={(e) => updateRow(r.key, { concept: e.target.value })}
                    />
                  </label>
                  <label className="field cash-entry-amount">
                    <span>Importe</span>
                    <input
                      value={r.amount}
                      inputMode="decimal"
                      autoComplete="off"
                      placeholder="0"
                      onChange={(e) => updateRow(r.key, { amount: e.target.value })}
                    />
                  </label>
                  <label className="field">
                    <span>Tipo</span>
                    <select
                      value={r.kind}
                      onChange={(e) =>
                        updateRow(r.key, {
                          kind: e.target.value as TransactionKindApi,
                        })
                      }
                    >
                      {TRANSACTION_KINDS.map((k) => (
                        <option key={k} value={k}>
                          {KIND_LABEL_ES[k]}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="field">
                    <span>Categoría</span>
                    <select
                      value={r.categoryId}
                      disabled={r.kind === "savings"}
                      onChange={(e) =>
                        updateRow(r.key, { categoryId: e.target.value })
                      }
                    >
                      {/* Sin opción «Sin categoría» en ingreso/gasto (4.15.0). El hueco solo
                          aparece —deshabilitado— si no hay categorías de ese ámbito. */}
                      {r.kind === "savings" ? (
                        <option value="">—</option>
                      ) : r.categoryId === "" ? (
                        <option value="" disabled>
                          Elige categoría
                        </option>
                      ) : null}
                      {cats.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  {r.kind === "savings" ? (
                    <label className="field">
                      <span>Activo destino</span>
                      <select
                        value={r.linkedAssetId}
                        onChange={(e) =>
                          updateRow(r.key, { linkedAssetId: e.target.value })
                        }
                      >
                        <option value="">—</option>
                        {assets.map((a) => (
                          <option key={a.id} value={a.id}>
                            {a.name}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : r.kind === "expense" ? (
                    <label className="field">
                      <span>Pasivo</span>
                      <select
                        value={r.linkedLiabilityId}
                        onChange={(e) =>
                          updateRow(r.key, { linkedLiabilityId: e.target.value })
                        }
                      >
                        <option value="">—</option>
                        {liabilities.map((l) => (
                          <option key={l.id} value={l.id}>
                            {l.label}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}
                  <label className="cash-entry-repeat">
                    <input
                      type="checkbox"
                      checked={r.repeatMonthly}
                      onChange={(e) =>
                        updateRow(r.key, { repeatMonthly: e.target.checked })
                      }
                    />
                    Repetir cada mes
                  </label>
                  <button
                    type="button"
                    className="btn ghost danger icon-btn snapshot-item-remove"
                    aria-label="Quitar movimiento"
                    onClick={() => removeRow(r.key)}
                  >
                    <XIcon />
                  </button>
                </div>
              );
            })
          )}
        </div>

        <button type="button" className="btn ghost" onClick={addRow}>
          Añadir fila
        </button>

        <div className="asset-form-actions">
          <button type="submit" className="btn primary" disabled={saving}>
            {saving ? "Guardando…" : "Guardar"}
          </button>
          <button
            type="button"
            className="btn ghost"
            disabled={saving}
            onClick={onClose}
          >
            Cancelar
          </button>
        </div>
      </form>
    </Modal>
  );
}
