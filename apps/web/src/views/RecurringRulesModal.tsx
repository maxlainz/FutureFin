/**
 * Reglas de movimientos recurrentes (modal). Lista `GET /v1/transactions/recurring` y permite
 * editar (5.0.0) y detener (borrar) cada regla. Detener una regla NO borra los movimientos ya
 * materializados; solo evita que se generen los futuros.
 *
 * Editar sigue la semántica de una app de calendario: al guardar se pregunta el ALCANCE —
 * «Todos» (la plantilla y todos sus movimientos ya creados, `apply_to: "all"`), «Solo futuros»
 * (solo la plantilla, `apply_to: "future"`) o «Cancelar». El PATCH lleva SOLO los campos que el
 * usuario ha cambiado: con «Todos» el servidor reescribe exactamente esos campos en cada
 * movimiento, así que mandar el formulario entero pisaría retoques manuales que nadie pidió.
 *
 * Patrón de ManualCashEntryModal: fetch al abrir. El diff borrador→PATCH es lógica pura y vive en
 * `lib/recurring-patch.ts` (con su test); aquí solo queda la presentación.
 */

import { useCallback, useEffect, useState, type FormEvent } from "react";
import { apiDelete, apiGet, apiPatch } from "../api/client";
import type {
  CategoryRow,
  PatchRecurringRuleResponse,
  RecurringApplyTo,
  RecurringRuleApi,
  TransactionKindApi,
} from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { formatCurrencyAmount, parseDisplayDecimal } from "../lib/format";
import { KIND_LABEL_ES, TRANSACTION_KINDS, categoriesForKind } from "../lib/expenses";
import {
  buildRecurringPatch,
  draftFromRule,
  recurringPatchBody,
  type RecurringPatchFields,
  type RuleDraft,
} from "../lib/recurring-patch";

export function RecurringRulesModal({
  open,
  onClose,
  currencyIso,
  onChanged,
  canEdit = true,
  incomeCategories = [],
  expenseCategories = [],
}: {
  open: boolean;
  onClose: () => void;
  currencyIso: string;
  /** Se llama tras detener o editar una regla (Movimientos recarga el mes y el summary). */
  onChanged: () => void;
  /** Rol con escritura. Un viewer ve la lista sin botones de editar ni detener. */
  canEdit?: boolean;
  incomeCategories?: CategoryRow[];
  expenseCategories?: CategoryRow[];
}) {
  const [rules, setRules] = useState<RecurringRuleApi[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [editing, setEditing] = useState<RecurringRuleApi | null>(null);
  const [draft, setDraft] = useState<RuleDraft | null>(null);
  /** Cuerpo pendiente de confirmar alcance: con él presente se enseña el diálogo. */
  const [pending, setPending] = useState<RecurringPatchFields | null>(null);
  const [saving, setSaving] = useState(false);

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
    if (open) {
      setEditing(null);
      setDraft(null);
      setPending(null);
      void load();
    }
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

  const startEdit = useCallback((r: RecurringRuleApi) => {
    setError(null);
    setPending(null);
    setEditing(r);
    setDraft(draftFromRule(r));
  }, []);

  const cancelEdit = useCallback(() => {
    setEditing(null);
    setDraft(null);
    setPending(null);
    setError(null);
  }, []);

  const fallbackFor = useCallback(
    (kind: TransactionKindApi): string =>
      categoriesForKind(kind, incomeCategories, expenseCategories).find((c) => c.is_fallback)
        ?.id ?? "",
    [incomeCategories, expenseCategories],
  );

  const updateDraft = useCallback(
    (patch: Partial<RuleDraft>) => {
      setDraft((prev) => {
        if (!prev) return prev;
        const next = { ...prev, ...patch };
        if (patch.kind !== undefined && patch.kind !== prev.kind) {
          // Cambiar de ámbito invalida la categoría anterior: cae en la por defecto del nuevo.
          next.categoryId = fallbackFor(next.kind);
        }
        if (next.kind === "savings") next.categoryId = "";
        return next;
      });
    },
    [fallbackFor],
  );

  function submitEdit(e: FormEvent) {
    e.preventDefault();
    if (!editing || !draft) return;
    setError(null);
    const built = buildRecurringPatch(editing, draft);
    if (built === null) {
      setError("No has cambiado nada. Modifica algún dato antes de guardar.");
      return;
    }
    if ("error" in built) {
      setError(built.error);
      return;
    }
    setPending(built);
  }

  async function applyWithScope(applyTo: RecurringApplyTo) {
    if (!editing || !pending) return;
    setSaving(true);
    setError(null);
    try {
      await apiPatch<PatchRecurringRuleResponse>(
        `/v1/transactions/recurring/${editing.id}`,
        recurringPatchBody(applyTo, pending),
      );
      setEditing(null);
      setDraft(null);
      setPending(null);
      await load();
      onChanged();
    } catch (err) {
      setPending(null);
      setError(err instanceof Error ? err.message : "No se pudo guardar la regla.");
    } finally {
      setSaving(false);
    }
  }

  const cats = draft
    ? categoriesForKind(draft.kind, incomeCategories, expenseCategories)
    : [];

  return (
    <Modal title="Movimientos recurrentes" open={open} onClose={onClose} wide>
      <div className="stack recurring-rules">
        <ModalFormError message={error} />
        {editing && draft ? (
          <form className="asset-form stack" onSubmit={submitEdit}>
            <p className="muted tight">
              Escribe el importe en positivo; el signo lo pone el tipo (ingreso suma,
              gasto e inversión restan).
            </p>
            <div className="cash-entry-row">
              <label className="field cash-entry-concept">
                <span>Concepto</span>
                <input
                  value={draft.concept}
                  maxLength={500}
                  autoComplete="off"
                  disabled={pending !== null}
                  onChange={(e) => updateDraft({ concept: e.target.value })}
                />
              </label>
              <label className="field cash-entry-amount">
                <span>Importe</span>
                <input
                  value={draft.amount}
                  inputMode="decimal"
                  autoComplete="off"
                  placeholder="0"
                  disabled={pending !== null}
                  onChange={(e) => updateDraft({ amount: e.target.value })}
                />
              </label>
              <label className="field">
                <span>Tipo</span>
                <select
                  value={draft.kind}
                  disabled={pending !== null}
                  onChange={(e) => updateDraft({ kind: e.target.value as TransactionKindApi })}
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
                  value={draft.categoryId}
                  disabled={pending !== null || draft.kind === "savings"}
                  onChange={(e) => updateDraft({ categoryId: e.target.value })}
                >
                  {draft.kind === "savings" ? (
                    <option value="">—</option>
                  ) : draft.categoryId === "" ? (
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
            </div>
            <label className="field">
              <span>Notas</span>
              <input
                value={draft.notes}
                maxLength={4000}
                autoComplete="off"
                disabled={pending !== null}
                onChange={(e) => updateDraft({ notes: e.target.value })}
              />
            </label>

            {pending ? (
              <div className="stack bordered-top" role="alertdialog" aria-label="Alcance del cambio">
                <p className="tight">
                  ¿Aplicar a todos los movimientos de esta recurrencia o solo a los futuros?
                </p>
                <div className="asset-form-actions">
                  <button
                    type="button"
                    className="btn primary"
                    disabled={saving}
                    onClick={() => void applyWithScope("all")}
                  >
                    Todos
                  </button>
                  <button
                    type="button"
                    className="btn primary"
                    disabled={saving}
                    onClick={() => void applyWithScope("future")}
                  >
                    Solo futuros
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={saving}
                    onClick={() => setPending(null)}
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            ) : (
              <div className="asset-form-actions">
                <button type="submit" className="btn primary">
                  Guardar
                </button>
                <button type="button" className="btn ghost" onClick={cancelEdit}>
                  Volver
                </button>
              </div>
            )}
          </form>
        ) : (
          <>
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
                      {canEdit ? (
                        <>
                          <button
                            type="button"
                            className="btn ghost"
                            disabled={deletingId === r.id}
                            onClick={() => startEdit(r)}
                          >
                            Editar
                          </button>
                          <button
                            type="button"
                            className="btn ghost danger"
                            disabled={deletingId === r.id}
                            onClick={() => void stopRule(r.id)}
                          >
                            {deletingId === r.id ? "Deteniendo…" : "Detener"}
                          </button>
                        </>
                      ) : null}
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
          </>
        )}
      </div>
    </Modal>
  );
}
