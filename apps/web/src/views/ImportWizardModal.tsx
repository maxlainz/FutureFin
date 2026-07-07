/**
 * Wizard de import de CSV bancarios (2 pasos, `useReducer`).
 *
 * Paso 1 (select): fuente (Auto/MyInvestor/N26) + archivo `.csv` → `POST /import/preview`.
 * Paso 2 (review): banner-resumen, barra de acciones masivas (filtros + incluir/excluir + asignar
 * kind/categoría en masa), tabla preview (filas dup/transfer/divisa atenuadas y desmarcadas por
 * defecto, selects kind/categoría por fila, editor de vínculos plegable) y footer con resumen
 * dinámico + select de cuenta origen del batch → `POST /import/confirm` con `decisions[]` paralelo.
 *
 * Stateless: el confirm reenvía `file_b64` + `file_sha256` del preview. Perf: filas memoizadas y
 * filtros que no dependen del scroll.
 */

import { memo, useCallback, useEffect, useMemo, useReducer, useState } from "react";
import { apiPost } from "../api/client";
import type {
  AssetApiRow,
  CategoryRow,
  ImportConfirmRequest,
  ImportConfirmResponseApi,
  ImportPreviewRequest,
  ImportPreviewResponseApi,
  ImportPreviewRowApi,
  LiabilityApiRow,
  TransactionImportSourceApi,
  TransactionKindApi,
} from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { LinkIcon } from "../components/icons";
import { formatCurrencyAmount } from "../lib/format";
import { formatDateDmy } from "../lib/dates";
import { readFileAsBase64 } from "../lib/files";
import {
  KIND_LABEL_ES,
  TRANSACTION_KINDS,
  buildConfirmDecisions,
  categoriesForKind,
  initialDraftForRow,
  rowMatchesFilter,
  summarizeDecisions,
  type ImportRowDraft,
  type ImportRowFilter,
} from "../lib/expenses";

const SOURCE_OPTIONS: { id: TransactionImportSourceApi; label: string }[] = [
  { id: "auto", label: "Autodetectar" },
  { id: "myinvestor", label: "MyInvestor" },
  { id: "n26", label: "N26" },
];

const FILTER_OPTIONS: { id: ImportRowFilter; label: string }[] = [
  { id: "all", label: "Todas" },
  { id: "new", label: "Nuevas" },
  { id: "duplicates", label: "Duplicados" },
  { id: "transfers", label: "Transferencias" },
  { id: "uncategorized", label: "Sin categoría" },
];

type State = {
  step: "select" | "review";
  source: TransactionImportSourceApi;
  accountAssetId: string;
  fileName: string;
  fileB64: string | null;
  preview: ImportPreviewResponseApi | null;
  drafts: ImportRowDraft[];
  filter: ImportRowFilter;
  loading: boolean;
  error: string | null;
};

const INITIAL: State = {
  step: "select",
  source: "auto",
  accountAssetId: "",
  fileName: "",
  fileB64: null,
  preview: null,
  drafts: [],
  filter: "all",
  loading: false,
  error: null,
};

type Action =
  | { type: "RESET" }
  | { type: "SET_SOURCE"; source: TransactionImportSourceApi }
  | { type: "SET_FILE"; name: string; b64: string | null }
  | { type: "SET_ACCOUNT"; accountAssetId: string }
  | { type: "PREVIEW_START" }
  | { type: "PREVIEW_OK"; preview: ImportPreviewResponseApi; drafts: ImportRowDraft[] }
  | { type: "FAIL"; message: string }
  | { type: "BACK_TO_SELECT" }
  | { type: "SET_FILTER"; filter: ImportRowFilter }
  | { type: "PATCH_ONE"; index: number; patch: Partial<ImportRowDraft> }
  | { type: "PATCH_MANY"; indices: number[]; patch: Partial<ImportRowDraft> }
  | { type: "CONFIRM_START" };

function applyPatch(d: ImportRowDraft, patch: Partial<ImportRowDraft>): ImportRowDraft {
  const next = { ...d, ...patch };
  const kindChanged = patch.kind !== undefined && patch.kind !== d.kind;
  if (kindChanged && patch.categoryId === undefined) next.categoryId = "";
  if (next.kind === "savings") next.categoryId = "";
  return next;
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "RESET":
      return INITIAL;
    case "SET_SOURCE":
      return { ...state, source: action.source };
    case "SET_FILE":
      return { ...state, fileName: action.name, fileB64: action.b64, error: null };
    case "SET_ACCOUNT":
      return { ...state, accountAssetId: action.accountAssetId };
    case "PREVIEW_START":
      return { ...state, loading: true, error: null };
    case "PREVIEW_OK":
      return {
        ...state,
        loading: false,
        error: null,
        step: "review",
        preview: action.preview,
        drafts: action.drafts,
        filter: "all",
      };
    case "FAIL":
      return { ...state, loading: false, error: action.message };
    case "BACK_TO_SELECT":
      return { ...state, step: "select", preview: null, drafts: [], error: null };
    case "SET_FILTER":
      return { ...state, filter: action.filter };
    case "PATCH_ONE":
      return {
        ...state,
        drafts: state.drafts.map((d, i) =>
          i === action.index ? applyPatch(d, action.patch) : d,
        ),
      };
    case "PATCH_MANY": {
      const set = new Set(action.indices);
      return {
        ...state,
        drafts: state.drafts.map((d, i) =>
          set.has(i) ? applyPatch(d, action.patch) : d,
        ),
      };
    }
    case "CONFIRM_START":
      return { ...state, loading: true, error: null };
    default:
      return state;
  }
}

export function ImportWizardModal({
  open,
  onClose,
  incomeCategories,
  expenseCategories,
  assets,
  liabilities,
  currencyIso,
  onImported,
}: {
  open: boolean;
  onClose: () => void;
  incomeCategories: CategoryRow[];
  expenseCategories: CategoryRow[];
  assets: AssetApiRow[];
  liabilities: LiabilityApiRow[];
  currencyIso: string;
  onImported: (res: ImportConfirmResponseApi) => void;
}) {
  const [state, dispatch] = useReducer(reducer, INITIAL);
  const [expandedLinks, setExpandedLinks] = useState<Set<number>>(new Set());
  const [bulkKind, setBulkKind] = useState<TransactionKindApi>("expense");
  const [bulkCategory, setBulkCategory] = useState<string>("");

  // Reset cada vez que se (re)abre el modal.
  useEffect(() => {
    if (open) {
      dispatch({ type: "RESET" });
      setExpandedLinks(new Set());
      setBulkKind("expense");
      setBulkCategory("");
    }
  }, [open]);

  const rows = useMemo(() => state.preview?.rows ?? [], [state.preview]);

  const visibleIndices = useMemo(() => {
    return rows
      .map((row, i) => ({ row, i }))
      .filter(({ row, i }) => rowMatchesFilter(row, state.drafts[i], state.filter))
      .map(({ i }) => i);
  }, [rows, state.drafts, state.filter]);

  const summary = useMemo(
    () => summarizeDecisions(rows, state.drafts),
    [rows, state.drafts],
  );

  async function runPreview() {
    if (!state.fileB64) {
      dispatch({ type: "FAIL", message: "Selecciona un archivo CSV." });
      return;
    }
    dispatch({ type: "PREVIEW_START" });
    try {
      const body: ImportPreviewRequest = {
        source: state.source,
        file_b64: state.fileB64,
      };
      const preview = await apiPost<ImportPreviewResponseApi>(
        "/v1/transactions/import/preview",
        body,
      );
      if (!preview) throw new Error("Respuesta vacía del servidor.");
      dispatch({
        type: "PREVIEW_OK",
        preview,
        drafts: preview.rows.map(initialDraftForRow),
      });
    } catch (e) {
      dispatch({
        type: "FAIL",
        message: e instanceof Error ? e.message : "No se pudo leer el CSV.",
      });
    }
  }

  async function runConfirm() {
    if (!state.preview || !state.fileB64) return;
    dispatch({ type: "CONFIRM_START" });
    try {
      const body: ImportConfirmRequest = {
        source: state.source,
        file_b64: state.fileB64,
        file_sha256: state.preview.file_sha256,
        decisions: buildConfirmDecisions(state.preview.rows, state.drafts),
        learn_rules: true,
      };
      if (state.accountAssetId) body.account_asset_id = state.accountAssetId;
      if (state.fileName) body.original_filename = state.fileName;
      const res = await apiPost<ImportConfirmResponseApi>(
        "/v1/transactions/import/confirm",
        body,
      );
      if (!res) throw new Error("Respuesta vacía del servidor.");
      onImported(res);
      onClose();
    } catch (e) {
      dispatch({
        type: "FAIL",
        message: e instanceof Error ? e.message : "No se pudo confirmar el import.",
      });
    }
  }

  // Callbacks estables → el `memo` de cada fila del preview se sostiene.
  const patchOne = useCallback(
    (index: number, patch: Partial<ImportRowDraft>) =>
      dispatch({ type: "PATCH_ONE", index, patch }),
    [],
  );

  const toggleLinks = useCallback((index: number) => {
    setExpandedLinks((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }, []);

  function applyBulkCategory() {
    if (!bulkCategory) return;
    const [scope, id] = bulkCategory.split(":");
    dispatch({
      type: "PATCH_MANY",
      indices: visibleIndices,
      patch: {
        kind: scope === "income" ? "income" : "expense",
        categoryId: id,
      },
    });
  }

  const bulkCategoryOptions = useMemo(
    () => [
      ...expenseCategories.map((c) => ({
        value: `expense:${c.id}`,
        label: `Gasto · ${c.name}`,
      })),
      ...incomeCategories.map((c) => ({
        value: `income:${c.id}`,
        label: `Ingreso · ${c.name}`,
      })),
    ],
    [expenseCategories, incomeCategories],
  );

  return (
    <Modal title="Importar CSV" open={open} onClose={onClose} wide>
      <div className="stack import-wizard">
        <ModalFormError message={state.error} />

        {state.step === "select" ? (
          <form
            className="stack"
            onSubmit={(e) => {
              e.preventDefault();
              void runPreview();
            }}
          >
            <p className="muted tight">
              Sube el CSV de tu banco. Detectamos el formato automáticamente
              (MyInvestor o N26); nada se guarda hasta que confirmes.
            </p>
            <div className="segmented" role="group" aria-label="Formato del CSV">
              {SOURCE_OPTIONS.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  className={state.source === s.id ? "active" : ""}
                  aria-pressed={state.source === s.id}
                  onClick={() => dispatch({ type: "SET_SOURCE", source: s.id })}
                >
                  {s.label}
                </button>
              ))}
            </div>
            <label className="field">
              <span>Archivo .csv</span>
              <input
                type="file"
                accept=".csv,text/csv"
                disabled={state.loading}
                onChange={(e) => {
                  const f = e.target.files && e.target.files[0];
                  if (!f) {
                    dispatch({ type: "SET_FILE", name: "", b64: null });
                    return;
                  }
                  void readFileAsBase64(f).then((b64) =>
                    dispatch({ type: "SET_FILE", name: f.name, b64 }),
                  );
                }}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={state.loading || !state.fileB64}
              >
                {state.loading ? "Leyendo…" : "Previsualizar"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={state.loading}
                onClick={onClose}
              >
                Cancelar
              </button>
            </div>
          </form>
        ) : state.preview ? (
          <div className="stack">
            <div className="banner info-banner tight-banner import-summary-banner">
              <strong>{state.preview.source}</strong> · {state.preview.row_count}{" "}
              filas · {state.preview.new_count} nuevas ·{" "}
              {state.preview.already_imported_count} ya importadas ·{" "}
              {state.preview.suggested_transfer_count} transferencias ·{" "}
              {state.preview.precategorized_count} pre-categorizadas
              {state.preview.currency_warning_count > 0
                ? ` · ${state.preview.currency_warning_count} en otra divisa`
                : ""}
            </div>

            <div className="import-bulk-bar">
              <div className="import-filter-pills" role="group" aria-label="Filtrar filas">
                {FILTER_OPTIONS.map((f) => (
                  <button
                    key={f.id}
                    type="button"
                    className={`ff-nav-pill ${state.filter === f.id ? "is-active" : ""}`}
                    aria-current={state.filter === f.id ? "true" : undefined}
                    onClick={() => dispatch({ type: "SET_FILTER", filter: f.id })}
                  >
                    {f.label}
                  </button>
                ))}
              </div>
              <div className="import-bulk-actions">
                <button
                  type="button"
                  className="btn ghost text"
                  onClick={() =>
                    dispatch({
                      type: "PATCH_MANY",
                      indices: visibleIndices,
                      patch: { include: true },
                    })
                  }
                >
                  Incluir visibles
                </button>
                <button
                  type="button"
                  className="btn ghost text"
                  onClick={() =>
                    dispatch({
                      type: "PATCH_MANY",
                      indices: visibleIndices,
                      patch: { include: false },
                    })
                  }
                >
                  Excluir visibles
                </button>
                <label className="field inline-role">
                  <span className="sr-only">Asignar kind a visibles</span>
                  <select
                    value={bulkKind}
                    onChange={(e) =>
                      setBulkKind(e.target.value as TransactionKindApi)
                    }
                  >
                    {TRANSACTION_KINDS.map((k) => (
                      <option key={k} value={k}>
                        {KIND_LABEL_ES[k]}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  className="btn ghost text"
                  onClick={() =>
                    dispatch({
                      type: "PATCH_MANY",
                      indices: visibleIndices,
                      patch: { kind: bulkKind },
                    })
                  }
                >
                  Aplicar kind
                </button>
                <label className="field inline-role">
                  <span className="sr-only">Asignar categoría a visibles</span>
                  <select
                    value={bulkCategory}
                    onChange={(e) => setBulkCategory(e.target.value)}
                  >
                    <option value="">Categoría…</option>
                    {bulkCategoryOptions.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  className="btn ghost text"
                  disabled={!bulkCategory}
                  onClick={applyBulkCategory}
                >
                  Aplicar categoría
                </button>
              </div>
            </div>

            {rows.length === 0 ? (
              <p className="muted bordered-top">Sin filas en el CSV.</p>
            ) : (
              <div className="table-scroll table-scroll--sticky import-preview-scroll">
                <table className="assets-table import-preview-table">
                  <thead>
                    <tr>
                      <th className="import-check-cell">
                        <span className="sr-only">Incluir</span>
                      </th>
                      <th>Fecha</th>
                      <th>Concepto</th>
                      <th className="num">Importe</th>
                      <th>Kind</th>
                      <th>Categoría</th>
                      <th className="import-link-cell">
                        <span className="sr-only">Vínculos</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {visibleIndices.map((i) => (
                      <PreviewRowMemo
                        key={rows[i].index}
                        index={i}
                        row={rows[i]}
                        draft={state.drafts[i]}
                        currencyIso={currencyIso}
                        incomeCategories={incomeCategories}
                        expenseCategories={expenseCategories}
                        assets={assets}
                        liabilities={liabilities}
                        expanded={expandedLinks.has(i)}
                        onPatch={patchOne}
                        onToggleLinks={toggleLinks}
                      />
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            <div className="import-footer">
              <div className="import-footer-summary muted">
                {summary.toImport} a importar · {summary.toSkip} omitidos ·{" "}
                {summary.toDiscard} descartados
              </div>
              <label className="field import-account-select">
                <span>Vincular a cuenta origen (activo)</span>
                <select
                  value={state.accountAssetId}
                  onChange={(e) =>
                    dispatch({ type: "SET_ACCOUNT", accountAssetId: e.target.value })
                  }
                >
                  <option value="">— Sin cuenta —</option>
                  {assets.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn primary"
                  disabled={state.loading || summary.toImport === 0}
                  onClick={() => void runConfirm()}
                >
                  {state.loading
                    ? "Importando…"
                    : `Confirmar (${summary.toImport})`}
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={state.loading}
                  onClick={() => dispatch({ type: "BACK_TO_SELECT" })}
                >
                  Cambiar archivo
                </button>
                <button
                  type="button"
                  className="btn ghost"
                  disabled={state.loading}
                  onClick={onClose}
                >
                  Cancelar
                </button>
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}

type PreviewRowProps = {
  index: number;
  row: ImportPreviewRowApi;
  draft: ImportRowDraft;
  currencyIso: string;
  incomeCategories: CategoryRow[];
  expenseCategories: CategoryRow[];
  assets: AssetApiRow[];
  liabilities: LiabilityApiRow[];
  expanded: boolean;
  onPatch: (index: number, patch: Partial<ImportRowDraft>) => void;
  onToggleLinks: (index: number) => void;
};

const PreviewRowMemo = memo(function PreviewRow({
  index,
  row,
  draft,
  currencyIso,
  incomeCategories,
  expenseCategories,
  assets,
  liabilities,
  expanded,
  onPatch,
  onToggleLinks,
}: PreviewRowProps) {
  const cats = categoriesForKind(draft.kind, incomeCategories, expenseCategories);
  const muted =
    row.status === "already_imported" ||
    row.suggested_transfer ||
    row.currency_warning;
  const amountNum = Number(row.amount);
  const amountClass = amountNum < 0 ? "num-neg" : amountNum > 0 ? "num-pos" : "";
  const statusHints: string[] = [];
  if (row.status === "already_imported") statusHints.push("Duplicado");
  if (row.suggested_transfer) statusHints.push("Transferencia");
  if (row.currency_warning) statusHints.push("Divisa ≠ EUR");

  return (
    <>
      <tr className={muted ? "import-row--muted" : ""}>
        <td className="import-check-cell">
          <input
            type="checkbox"
            checked={draft.include}
            aria-label="Incluir fila"
            onChange={(e) => {
              const patch: Partial<ImportRowDraft> = { include: e.target.checked };
              // Incluir una fila duplicada implica forzar la nueva ocurrencia.
              if (e.target.checked && row.status === "already_imported") {
                patch.force = true;
              }
              onPatch(index, patch);
            }}
          />
        </td>
        <td>{formatDateDmy(row.op_date)}</td>
        <td className="import-concept-cell">
          {row.concept}
          {statusHints.length > 0 ? (
            <span className="import-row-hint"> {statusHints.join(" · ")}</span>
          ) : null}
        </td>
        <td className={`num ${amountClass}`}>
          {formatCurrencyAmount(row.amount, currencyIso)}
        </td>
        <td>
          <select
            value={draft.kind}
            aria-label="Kind"
            onChange={(e) =>
              onPatch(index, { kind: e.target.value as TransactionKindApi })
            }
          >
            {TRANSACTION_KINDS.map((k) => (
              <option key={k} value={k}>
                {KIND_LABEL_ES[k]}
              </option>
            ))}
          </select>
        </td>
        <td>
          <select
            value={draft.categoryId}
            aria-label="Categoría"
            disabled={draft.kind === "savings"}
            onChange={(e) => onPatch(index, { categoryId: e.target.value })}
          >
            <option value="">
              {draft.kind === "savings" ? "—" : "Sin categoría"}
            </option>
            {cats.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </td>
        <td className="import-link-cell">
          <button
            type="button"
            className={`btn ghost icon-btn ${
              draft.linkedAssetId || draft.linkedLiabilityId ? "is-active-link" : ""
            }`}
            aria-label="Vínculos"
            aria-expanded={expanded}
            onClick={() => onToggleLinks(index)}
          >
            <LinkIcon />
          </button>
        </td>
      </tr>
      {expanded ? (
        <tr className="import-links-row">
          <td />
          <td colSpan={6}>
            <div className="import-links-editor">
              <label className="field inline-role">
                <span>Activo destino</span>
                <select
                  value={draft.linkedAssetId}
                  onChange={(e) =>
                    onPatch(index, { linkedAssetId: e.target.value })
                  }
                >
                  <option value="">— Ninguno —</option>
                  {assets.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field inline-role">
                <span>Pasivo vinculado</span>
                <select
                  value={draft.linkedLiabilityId}
                  onChange={(e) =>
                    onPatch(index, { linkedLiabilityId: e.target.value })
                  }
                >
                  <option value="">— Ninguno —</option>
                  {liabilities.map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </td>
        </tr>
      ) : null}
    </>
  );
});
