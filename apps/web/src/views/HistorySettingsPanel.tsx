/**
 * Ajustes → Histórico: backfill y gestión de snapshots de patrimonio (v1.5.0).
 *
 * Panel autoservicio (dato de uso ocasional): hace fetch al montar y en cada cambio de filtro
 * (año + tipo). Solo trabaja con snapshots del propio usuario (`GET/POST/PUT/DELETE
 * /v1/history/snapshots`), Decimal-as-string. `canEdit === false` → tabla de solo lectura sin
 * controles. Toda mutación exitosa refresca la lista y llama `onHistoryMutated` (que en App
 * recarga la serie histórica del chart).
 *
 * Prefill (backfill assistido): al crear un snapshot el grid se autocompleta con
 * `GET /v1/history/snapshots/prefill?kind&date` (interpolación entre snapshots vecinos), un
 * `DraftItem` por ítem sugerido reusando su `item_id` para preservar el enlace entre snapshots.
 * En crear, cambiar fecha/tipo re-sugiere en silencio mientras el usuario no haya tocado el grid
 * (`formDirty`); si lo tocó, aparece «Recalcular sugerencias». En editar, «Añadir items que
 * faltan» hace append de los ítems cuyo `item_id` no esté ya en el grid, sin tocar las filas
 * existentes. El fallo de red nunca bloquea: se cae a una fila en blanco y el modal sigue usable.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { apiDelete, apiGet, apiPost, apiPut } from "../api/client";
import type {
  HistoryPrefillItemApi,
  HistoryPrefillResponseApi,
  HistorySnapshotApi,
  HistorySnapshotKindApi,
} from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { PlusIcon, RowEditIcon, RowTrashIcon, XIcon } from "../components/icons";
import {
  formatCurrencyAmount,
  formatEditableDecimalString,
  parseDisplayDecimal,
} from "../lib/format";
import { formatDateDmy, parseYmdComponents, todayYmdInTimeZone } from "../lib/dates";

const KIND_LABEL: Record<HistorySnapshotKindApi, string> = {
  asset: "Activos",
  liability: "Pasivos",
};

const KIND_FILTERS: HistorySnapshotKindApi[] = ["asset", "liability"];

/** Cuántos años atrás ofrece el selector de año (año actual … año actual − 15). */
const YEARS_BACK = 15;

/** Fila editable del grid de ítems del modal. `itemId` presente = ítem existente (se reenvía
 *  para preservar el enlace entre snapshots en el PUT). `key` es solo para React.
 *  `existedHint` es solo display (sugerencia de prefill de un ítem que no existía en esa fecha):
 *  NO afecta a `buildItemsPayload` — una fila con valor "0" se envía igualmente. */
type DraftItem = {
  key: string;
  itemId?: string;
  label: string;
  value: string;
  aprPercent: string;
  paymentAmount: string;
  paymentFrequency: "" | "monthly" | "weekly";
  existedHint?: boolean;
};

/** Cuerpo de un ítem enviado a la API (términos solo en pasivos). */
type ItemPayload = {
  item_id?: string;
  label: string;
  value: string;
  apr_percent?: string;
  payment_amount?: string;
  payment_frequency?: "monthly" | "weekly";
};

export function HistorySettingsPanel({
  canEdit,
  currencyIso,
  calendarTz,
  onHistoryMutated,
}: {
  canEdit: boolean;
  currencyIso: string;
  /** Zona horaria (IANA) de la instalación: define «hoy» igual que el servidor valida los
   *  snapshots (installation_naive_today). Con tz vacía/ inválida `todayYmdInTimeZone` cae a UTC. */
  calendarTz: string;
  onHistoryMutated: () => void;
}) {
  // «Hoy» en el calendario de la instalación, no UTC: evita 400 snapshot_date_in_future cuando la
  // tz va por detrás de UTC, o un installation-today no seleccionable cuando va por delante.
  const today = todayYmdInTimeZone(calendarTz);
  const todayComponents = parseYmdComponents(today);
  const currentYear = todayComponents ? todayComponents.y : new Date().getFullYear();

  const yearOptions = useMemo(() => {
    const out: number[] = [];
    for (let y = currentYear; y >= currentYear - YEARS_BACK; y -= 1) out.push(y);
    return out;
  }, [currentYear]);

  const [year, setYear] = useState<number>(currentYear);
  const [kind, setKind] = useState<HistorySnapshotKindApi>("asset");
  const [snapshots, setSnapshots] = useState<HistorySnapshotApi[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Modal añadir/editar.
  const [modalOpen, setModalOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formKind, setFormKind] = useState<HistorySnapshotKindApi>("asset");
  const [formDate, setFormDate] = useState<string>(today);
  const [formItems, setFormItems] = useState<DraftItem[]>([]);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Prefill (sugerencias de backfill). `formDirty` = el usuario tocó el grid en modo crear →
  // deja de re-sugerir en silencio y ofrece «Recalcular». Un ref espejo lo hace legible desde
  // los callbacks async sin re-crearlos. `prefillReqSeq` descarta respuestas fuera de orden.
  const [formDirty, setFormDirty] = useState(false);
  const formDirtyRef = useRef(false);
  const prefillReqSeq = useRef(0);
  const [prefillLoading, setPrefillLoading] = useState(false);
  const [prefillError, setPrefillError] = useState(false);
  const [appendLoading, setAppendLoading] = useState(false);
  const [appendNote, setAppendNote] = useState<string | null>(null);

  // Modal borrar.
  const [deleteTarget, setDeleteTarget] = useState<HistorySnapshotApi | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const keySeq = useRef(0);
  const blankItem = useCallback((): DraftItem => {
    keySeq.current += 1;
    return {
      key: `new-${keySeq.current}`,
      label: "",
      value: "",
      aprPercent: "",
      paymentAmount: "",
      paymentFrequency: "",
    };
  }, []);

  /** Marca el grid como editado por el usuario (solo relevante en modo crear). */
  const markDirty = useCallback(() => {
    formDirtyRef.current = true;
    setFormDirty(true);
  }, []);

  /** Mapea un ítem de prefill a fila del grid, conservando `item_id` (enlace entre snapshots). */
  const mapPrefillItem = useCallback((item: HistoryPrefillItemApi): DraftItem => {
    keySeq.current += 1;
    return {
      key: `pf-${keySeq.current}`,
      itemId: item.item_id,
      label: item.label,
      value: formatEditableDecimalString(item.value),
      aprPercent: formatEditableDecimalString(item.apr_percent),
      paymentAmount: formatEditableDecimalString(item.payment_amount),
      paymentFrequency: item.payment_frequency ?? "",
      existedHint: item.existed === false,
    };
  }, []);

  const fetchSnapshots = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiGet<HistorySnapshotApi[]>(
        `/v1/history/snapshots?year=${year}&kind=${kind}`,
      );
      setSnapshots(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudo cargar el histórico.");
      setSnapshots([]);
    } finally {
      setLoading(false);
    }
  }, [year, kind]);

  useEffect(() => {
    void fetchSnapshots();
  }, [fetchSnapshots]);

  /** Deja el estado de prefill en su punto de partida (al abrir cualquiera de los dos modos). */
  const resetPrefillState = useCallback(() => {
    formDirtyRef.current = false;
    setFormDirty(false);
    prefillReqSeq.current += 1; // invalida cualquier respuesta en vuelo
    setPrefillLoading(false);
    setPrefillError(false);
    setAppendLoading(false);
    setAppendNote(null);
  }, []);

  const openNew = useCallback(() => {
    setEditingId(null);
    setFormKind(kind);
    setFormDate(today);
    // Fila en blanco como base usable de inmediato; el efecto de prefill la sustituye con las
    // sugerencias cuando llegan (o la conserva si el fetch falla).
    setFormItems([blankItem()]);
    setFormError(null);
    resetPrefillState();
    setModalOpen(true);
  }, [kind, today, blankItem, resetPrefillState]);

  const openEdit = useCallback(
    (s: HistorySnapshotApi) => {
      setEditingId(s.id);
      setFormKind(s.kind);
      setFormDate(s.snapshot_date_ymd);
      setFormItems(
        s.items.map((it) => ({
          key: it.item_id,
          itemId: it.item_id,
          label: it.label,
          value: formatEditableDecimalString(it.value),
          aprPercent: formatEditableDecimalString(it.apr_percent),
          paymentAmount: formatEditableDecimalString(it.payment_amount),
          paymentFrequency: it.payment_frequency ?? "",
        })),
      );
      setFormError(null);
      resetPrefillState();
      setModalOpen(true);
    },
    [resetPrefillState],
  );

  const closeModal = useCallback(() => {
    if (saving) return;
    setModalOpen(false);
  }, [saving]);

  const addItem = useCallback(() => {
    markDirty();
    setFormItems((prev) => [...prev, blankItem()]);
  }, [blankItem, markDirty]);

  const removeItem = useCallback(
    (key: string) => {
      markDirty();
      setFormItems((prev) => prev.filter((x) => x.key !== key));
    },
    [markDirty],
  );

  const updateItem = useCallback(
    (key: string, patch: Partial<DraftItem>) => {
      markDirty();
      setFormItems((prev) => prev.map((x) => (x.key === key ? { ...x, ...patch } : x)));
    },
    [markDirty],
  );

  /** Valida y normaliza (coma→punto) el grid de ítems. Descarta filas completamente vacías. */
  const buildItemsPayload = useCallback(
    (
      items: DraftItem[],
      forKind: HistorySnapshotKindApi,
    ): { ok: true; items: ItemPayload[] } | { ok: false; error: string } => {
      const out: ItemPayload[] = [];
      for (const it of items) {
        const label = it.label.trim();
        const rawValue = it.value.trim();
        if (label === "" && rawValue === "") continue; // fila vacía → ignorar
        if (label === "") return { ok: false, error: "Cada ítem necesita un nombre." };
        const parsedValue = parseDisplayDecimal(rawValue);
        if (parsedValue === null || parsedValue < 0) {
          return { ok: false, error: "Cada ítem necesita un valor válido ≥ 0." };
        }
        const payload: ItemPayload = { label, value: rawValue.replace(",", ".") };
        if (it.itemId) payload.item_id = it.itemId;
        if (forKind === "liability") {
          const apr = it.aprPercent.trim();
          if (apr !== "") {
            const a = parseDisplayDecimal(apr);
            if (a === null || a < 0) return { ok: false, error: "TAE inválida." };
            payload.apr_percent = apr.replace(",", ".");
          }
          const pay = it.paymentAmount.trim();
          if (pay !== "") {
            const p = parseDisplayDecimal(pay);
            if (p === null || p < 0) return { ok: false, error: "Cuota inválida." };
            payload.payment_amount = pay.replace(",", ".");
          }
          if (it.paymentFrequency) payload.payment_frequency = it.paymentFrequency;
        }
        out.push(payload);
      }
      return { ok: true, items: out };
    },
    [],
  );

  /** Pide sugerencias para (kind, date) y repuebla el grid (modo crear). `force` = el usuario
   *  pidió recalcular explícitamente y sobreescribe aunque haya tocado el grid; sin `force` se
   *  respeta un grid ya editado. Fallo de red → nota discreta, se conserva el grid (fila blanca). */
  const runCreatePrefill = useCallback(
    async (kindArg: HistorySnapshotKindApi, dateArg: string, force: boolean) => {
      const reqId = prefillReqSeq.current + 1;
      prefillReqSeq.current = reqId;
      setPrefillLoading(true);
      setPrefillError(false);
      try {
        const data = await apiGet<HistoryPrefillResponseApi>(
          `/v1/history/snapshots/prefill?kind=${kindArg}&date=${encodeURIComponent(dateArg)}`,
        );
        if (reqId !== prefillReqSeq.current) return; // respuesta obsoleta
        if (!force && formDirtyRef.current) return; // se editó a mitad de vuelo → conservar grid
        const rows = data.items.map(mapPrefillItem);
        setFormItems(rows.length > 0 ? rows : [blankItem()]);
      } catch {
        if (reqId !== prefillReqSeq.current) return;
        setPrefillError(true);
      } finally {
        if (reqId === prefillReqSeq.current) setPrefillLoading(false);
      }
    },
    [blankItem, mapPrefillItem],
  );

  // Modo crear: al abrir y en cada cambio de fecha/tipo, re-sugerir en silencio mientras el
  // usuario no haya tocado el grid. Editar (editingId !== null) no auto-sugiere.
  useEffect(() => {
    if (!modalOpen || editingId !== null) return;
    if (formDirtyRef.current) return;
    if (!formDate.trim()) return;
    void runCreatePrefill(formKind, formDate, false);
  }, [modalOpen, editingId, formKind, formDate, runCreatePrefill]);

  /** «Recalcular sugerencias»: descarta las ediciones y reescribe el grid con el prefill. */
  const recalcSuggestions = useCallback(() => {
    formDirtyRef.current = false;
    setFormDirty(false);
    void runCreatePrefill(formKind, formDate, true);
  }, [formKind, formDate, runCreatePrefill]);

  /** «Añadir items que faltan» (modo editar): añade solo los ítems cuyo `item_id` no esté ya en
   *  el grid. Nunca toca las filas existentes. Sin novedades → nota «Nada que añadir.». */
  const appendMissing = useCallback(async () => {
    setAppendLoading(true);
    setAppendNote(null);
    try {
      const data = await apiGet<HistoryPrefillResponseApi>(
        `/v1/history/snapshots/prefill?kind=${formKind}&date=${encodeURIComponent(formDate)}`,
      );
      const present = new Set(
        formItems.map((x) => x.itemId).filter((id): id is string => Boolean(id)),
      );
      const additions = data.items
        .filter((it) => !present.has(it.item_id))
        .map(mapPrefillItem);
      if (additions.length === 0) {
        setAppendNote("Nada que añadir.");
      } else {
        setFormItems((prev) => [...prev, ...additions]);
      }
    } catch {
      setAppendNote("No se pudieron cargar sugerencias.");
    } finally {
      setAppendLoading(false);
    }
  }, [formKind, formDate, formItems, mapPrefillItem]);

  // Auto-descartar la nota de «Añadir items que faltan» a los pocos segundos.
  useEffect(() => {
    if (!appendNote) return;
    const t = window.setTimeout(() => setAppendNote(null), 4000);
    return () => window.clearTimeout(t);
  }, [appendNote]);

  const submitForm = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      setFormError(null);
      const date = formDate.trim();
      if (!date) {
        setFormError("Indica una fecha.");
        return;
      }
      const built = buildItemsPayload(formItems, formKind);
      if (!built.ok) {
        setFormError(built.error);
        return;
      }
      setSaving(true);
      try {
        if (editingId) {
          await apiPut(`/v1/history/snapshots/${editingId}`, {
            snapshot_date: date,
            items: built.items,
          });
        } else {
          await apiPost(`/v1/history/snapshots`, {
            kind: formKind,
            snapshot_date: date,
            items: built.items,
          });
        }
        setModalOpen(false);
        await fetchSnapshots();
        onHistoryMutated();
      } catch (err) {
        setFormError(
          err instanceof Error ? err.message : "No se pudo guardar el snapshot.",
        );
      } finally {
        setSaving(false);
      }
    },
    [
      formDate,
      formItems,
      formKind,
      editingId,
      buildItemsPayload,
      fetchSnapshots,
      onHistoryMutated,
    ],
  );

  const confirmDelete = useCallback(async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await apiDelete(`/v1/history/snapshots/${deleteTarget.id}`);
      setDeleteTarget(null);
      await fetchSnapshots();
      onHistoryMutated();
    } catch (err) {
      setDeleteError(err instanceof Error ? err.message : "No se pudo eliminar.");
    } finally {
      setDeleting(false);
    }
  }, [deleteTarget, fetchSnapshots, onHistoryMutated]);

  const showTerms = formKind === "liability";

  return (
    <section className="panel">
      <div className="panel-head-row">
        <h3 className="panel-title">Snapshots</h3>
        {canEdit ? (
          <button
            type="button"
            className="btn primary snapshot-add-btn"
            onClick={openNew}
          >
            <PlusIcon />
            Añadir snapshot
          </button>
        ) : null}
      </div>

      <div className="category-toolbar bordered-top">
        <label className="field inline-role">
          <span className="sr-only">Año</span>
          <select
            value={year}
            onChange={(e) => setYear(Number(e.target.value))}
            aria-label="Filtrar por año"
          >
            {yearOptions.map((y) => (
              <option key={y} value={y}>
                {y}
              </option>
            ))}
          </select>
        </label>
        <div
          className="history-kind-filter"
          role="group"
          aria-label="Tipo de snapshot"
        >
          {KIND_FILTERS.map((k) => (
            <button
              key={k}
              type="button"
              className={`ff-nav-pill ${kind === k ? "is-active" : ""}`}
              aria-current={kind === k ? "true" : undefined}
              onClick={() => setKind(k)}
            >
              {KIND_LABEL[k]}
            </button>
          ))}
        </div>
      </div>

      {!canEdit ? <p className="muted bordered-top">Solo lectura.</p> : null}

      {error ? (
        <p className="error compact bordered-top">{error}</p>
      ) : loading ? (
        <p className="muted bordered-top">Cargando…</p>
      ) : snapshots.length === 0 ? (
        <p className="muted bordered-top">Sin datos.</p>
      ) : (
        <div className="table-scroll bordered-top">
          <table className="assets-table">
            <thead>
              <tr>
                <th>Fecha</th>
                <th className="num">Total</th>
                <th className="num">Ítems</th>
                {canEdit ? (
                  <th className="asset-actions-cell">
                    <span className="sr-only">Acciones</span>
                  </th>
                ) : null}
              </tr>
            </thead>
            <tbody>
              {snapshots.map((s) => (
                <tr key={s.id}>
                  <td>{formatDateDmy(s.snapshot_date_ymd)}</td>
                  <td className="num">{formatCurrencyAmount(s.total, currencyIso)}</td>
                  <td className="num">{s.items.length}</td>
                  {canEdit ? (
                    <td className="asset-actions-cell">
                      <div className="budget-row-actions">
                        <button
                          type="button"
                          className="btn ghost icon-btn"
                          aria-label="Editar snapshot"
                          onClick={() => openEdit(s)}
                        >
                          <RowEditIcon />
                        </button>
                        <button
                          type="button"
                          className="btn ghost danger icon-btn"
                          aria-label="Eliminar snapshot"
                          onClick={() => {
                            setDeleteError(null);
                            setDeleteTarget(s);
                          }}
                        >
                          <RowTrashIcon />
                        </button>
                      </div>
                    </td>
                  ) : null}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {canEdit ? (
        <>
          <Modal
            title={editingId ? "Editar snapshot" : "Nuevo snapshot"}
            open={modalOpen}
            onClose={closeModal}
            wide
          >
            <form className="asset-form stack" onSubmit={submitForm}>
              <ModalFormError message={formError} />
              <div className="segmented" role="group" aria-label="Tipo de snapshot">
                {KIND_FILTERS.map((k) => (
                  <button
                    key={k}
                    type="button"
                    className={formKind === k ? "active" : ""}
                    aria-pressed={formKind === k}
                    disabled={editingId !== null}
                    onClick={() => setFormKind(k)}
                  >
                    {KIND_LABEL[k]}
                  </button>
                ))}
              </div>
              <label className="field">
                <span>Fecha</span>
                <input
                  type="date"
                  value={formDate}
                  max={today}
                  onChange={(e) => setFormDate(e.target.value)}
                  required
                />
              </label>

              {editingId === null ? (
                prefillLoading || prefillError || formDirty ? (
                  <div className="snapshot-prefill-bar">
                    {prefillLoading ? (
                      <span className="muted snapshot-prefill-note">Cargando sugerencias…</span>
                    ) : null}
                    {prefillError ? (
                      <span className="muted snapshot-prefill-note">
                        No se pudieron cargar sugerencias.
                      </span>
                    ) : null}
                    {formDirty && !prefillLoading ? (
                      <button
                        type="button"
                        className="btn ghost text snapshot-prefill-action"
                        onClick={recalcSuggestions}
                      >
                        Recalcular sugerencias
                      </button>
                    ) : null}
                  </div>
                ) : null
              ) : (
                <div className="snapshot-prefill-bar">
                  <button
                    type="button"
                    className="btn ghost text snapshot-prefill-action"
                    disabled={appendLoading}
                    onClick={() => void appendMissing()}
                  >
                    {appendLoading ? "Buscando…" : "Añadir items que faltan"}
                  </button>
                  {appendNote ? (
                    <span className="muted snapshot-prefill-note">{appendNote}</span>
                  ) : null}
                </div>
              )}

              <div className="snapshot-items">
                {formItems.length === 0 ? (
                  <p className="muted tight">Sin ítems. Añade al menos uno.</p>
                ) : (
                  formItems.map((it) => (
                    <div key={it.key} className="snapshot-item-row">
                      <label className="field snapshot-item-name">
                        <span>Nombre</span>
                        <input
                          value={it.label}
                          onChange={(e) => updateItem(it.key, { label: e.target.value })}
                          maxLength={200}
                          autoComplete="off"
                        />
                        {it.existedHint ? (
                          <span className="snapshot-item-existed-hint">
                            No existía en esta fecha
                          </span>
                        ) : null}
                      </label>
                      <label className="field">
                        <span>Valor</span>
                        <input
                          value={it.value}
                          onChange={(e) => updateItem(it.key, { value: e.target.value })}
                          inputMode="decimal"
                          autoComplete="off"
                        />
                      </label>
                      {showTerms ? (
                        <>
                          <label className="field">
                            <span>TAE %</span>
                            <input
                              value={it.aprPercent}
                              onChange={(e) =>
                                updateItem(it.key, { aprPercent: e.target.value })
                              }
                              inputMode="decimal"
                              placeholder="—"
                              autoComplete="off"
                            />
                          </label>
                          <label className="field">
                            <span>Cuota</span>
                            <input
                              value={it.paymentAmount}
                              onChange={(e) =>
                                updateItem(it.key, { paymentAmount: e.target.value })
                              }
                              inputMode="decimal"
                              placeholder="—"
                              autoComplete="off"
                            />
                          </label>
                          <label className="field">
                            <span>Frecuencia</span>
                            <select
                              value={it.paymentFrequency}
                              onChange={(e) =>
                                updateItem(it.key, {
                                  paymentFrequency: e.target
                                    .value as DraftItem["paymentFrequency"],
                                })
                              }
                            >
                              <option value="">—</option>
                              <option value="monthly">Mensual</option>
                              <option value="weekly">Semanal</option>
                            </select>
                          </label>
                        </>
                      ) : null}
                      <button
                        type="button"
                        className="btn ghost danger icon-btn snapshot-item-remove"
                        aria-label="Quitar ítem"
                        onClick={() => removeItem(it.key)}
                      >
                        <XIcon />
                      </button>
                    </div>
                  ))
                )}
              </div>

              <button type="button" className="btn ghost" onClick={addItem}>
                Añadir ítem
              </button>

              <div className="asset-form-actions">
                <button type="submit" className="btn primary" disabled={saving}>
                  {saving ? "Guardando…" : "Guardar"}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={saving}
                  onClick={closeModal}
                >
                  Cancelar
                </button>
              </div>
            </form>
          </Modal>

          <Modal
            title="Eliminar snapshot"
            open={deleteTarget !== null}
            onClose={() => {
              if (!deleting) setDeleteTarget(null);
            }}
          >
            {deleteTarget ? (
              <div className="stack">
                <ModalFormError message={deleteError} />
                <p className="muted tight">
                  ¿Eliminar el snapshot del{" "}
                  <strong>{formatDateDmy(deleteTarget.snapshot_date_ymd)}</strong>? Esta
                  acción no se puede deshacer.
                </p>
                <div className="asset-form-actions">
                  <button
                    type="button"
                    className="btn ghost danger"
                    disabled={deleting}
                    onClick={() => void confirmDelete()}
                  >
                    {deleting ? "Eliminando…" : "Eliminar"}
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={deleting}
                    onClick={() => setDeleteTarget(null)}
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            ) : (
              <p className="muted tight">Nada seleccionado.</p>
            )}
          </Modal>
        </>
      ) : null}
    </section>
  );
}
