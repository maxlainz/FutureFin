/**
 * Wizard de import de CSV bancarios (2 pasos, `useReducer`). Desde 4.13.0 acepta VARIOS archivos
 * a la vez y los procesa en cola, uno tras otro.
 *
 * Paso 1 (select): fuente (Auto/MyInvestor/N26) + archivo(s) `.csv`. Cuenta y formato se aplican
 * a toda la tanda. → `POST /import/preview` del primer archivo.
 * Paso 2 (review), POR ARCHIVO: banner-resumen, barra de acciones masivas (filtros +
 * incluir/excluir + asignar kind/categoría en masa), tabla preview (filas dup/divisa atenuadas y
 * desmarcadas por defecto — las posibles transferencias entran incluidas desde 3.5.0 y solo
 * llevan su hint textual; selects kind/categoría por fila, editor de vínculos plegable) y footer
 * con resumen dinámico → `POST /import/confirm` con `decisions[]` paralelo. Confirmar (u
 * «Omitir archivo») avanza al preview del siguiente; el último cierra el modal.
 *
 * La cola NO es atómica a propósito: cada archivo es su propio preview/confirm y su propia fila
 * de `transaction_imports` (deshacible por separado); cancelar a mitad conserva lo ya confirmado.
 * `onImported` se dispara UNA vez al terminar/cerrar, con el agregado (`summarizeImportBatch`),
 * y solo si hubo al menos un confirm.
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
import { formatDateDm, formatDateDmy } from "../lib/dates";
import { useIsMobile } from "../lib/responsive";
import { readFileAsBase64 } from "../lib/files";
import {
  KIND_LABEL_ES,
  TRANSACTION_KINDS,
  buildConfirmDecisions,
  capitalizeSource,
  categoriesForKind,
  initialDraftForRow,
  rowMatchesFilter,
  summarizeDecisions,
  summarizeImportBatch,
  type ImportBatchSummary,
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

/** Un archivo de la tanda, ya leído a base64 en el select. */
type QueuedFile = { name: string; b64: string };

type State = {
  step: "select" | "review";
  source: TransactionImportSourceApi;
  accountAssetId: string;
  /** Tanda completa elegida en el paso select (1..N archivos). */
  files: QueuedFile[];
  /** Índice del archivo en curso (su preview/review). */
  fileIndex: number;
  /** Preview del archivo en curso; null en review = leyendo el siguiente (o su preview falló). */
  preview: ImportPreviewResponseApi | null;
  drafts: ImportRowDraft[];
  filter: ImportRowFilter;
  loading: boolean;
  error: string | null;
  /** Respuestas de los confirms ya aplicados en esta tanda (para el agregado final). */
  confirmed: ImportConfirmResponseApi[];
};

const INITIAL: State = {
  step: "select",
  source: "auto",
  accountAssetId: "",
  files: [],
  fileIndex: 0,
  preview: null,
  drafts: [],
  filter: "all",
  loading: false,
  error: null,
  confirmed: [],
};

type Action =
  | { type: "RESET" }
  | { type: "SET_SOURCE"; source: TransactionImportSourceApi }
  | { type: "SET_FILES"; files: QueuedFile[] }
  | { type: "SET_ACCOUNT"; accountAssetId: string }
  | { type: "PREVIEW_START"; fileIndex: number }
  | { type: "PREVIEW_OK"; preview: ImportPreviewResponseApi; drafts: ImportRowDraft[] }
  | { type: "FAIL"; message: string }
  | { type: "BACK_TO_SELECT" }
  | { type: "SET_FILTER"; filter: ImportRowFilter }
  | { type: "PATCH_ONE"; index: number; patch: Partial<ImportRowDraft> }
  | { type: "PATCH_MANY"; indices: number[]; patch: Partial<ImportRowDraft> }
  | { type: "CONFIRM_START" }
  | { type: "CONFIRM_OK"; res: ImportConfirmResponseApi };

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
    case "SET_FILES":
      return { ...state, files: action.files, fileIndex: 0, error: null };
    case "SET_ACCOUNT":
      return { ...state, accountAssetId: action.accountAssetId };
    case "PREVIEW_START":
      // Limpia el preview del archivo anterior: en la cola, entre confirm y confirm, la tabla
      // vieja no debe verse bajo el spinner del siguiente.
      return {
        ...state,
        loading: true,
        error: null,
        fileIndex: action.fileIndex,
        preview: null,
        drafts: [],
        filter: "all",
      };
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
    case "CONFIRM_OK":
      // No toca `loading`: o el caller encadena el PREVIEW_START del siguiente archivo, o cierra.
      return { ...state, confirmed: [...state.confirmed, action.res] };
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
  /** Agregado de la tanda (1..N archivos). Solo se llama si hubo al menos un confirm. */
  onImported: (batch: ImportBatchSummary) => void;
}) {
  const [state, dispatch] = useReducer(reducer, INITIAL);
  const [expandedLinks, setExpandedLinks] = useState<Set<number>>(new Set());
  // "" = «Tipo…» (no aplicar kind). El cluster «Asignar a visibles» aplica kind y/o categoría.
  const [bulkKind, setBulkKind] = useState<"" | TransactionKindApi>("");
  const [bulkCategory, setBulkCategory] = useState<string>("");
  const isMobile = useIsMobile();

  // Reset cada vez que se (re)abre el modal.
  useEffect(() => {
    if (open) {
      dispatch({ type: "RESET" });
      setExpandedLinks(new Set());
      setBulkKind("");
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

  const currentFile = state.files[state.fileIndex] ?? null;
  const hasNextFile = state.fileIndex + 1 < state.files.length;

  /** Cierra el modal; si hubo confirms, notifica antes el agregado de la tanda. */
  function finishAndClose(confirmed: ImportConfirmResponseApi[]) {
    if (confirmed.length > 0) onImported(summarizeImportBatch(confirmed));
    onClose();
  }

  const handleClose = () => finishAndClose(state.confirmed);

  async function previewFileAt(fileIndex: number) {
    const file = state.files[fileIndex];
    if (!file) {
      dispatch({ type: "FAIL", message: "Selecciona al menos un archivo CSV." });
      return;
    }
    dispatch({ type: "PREVIEW_START", fileIndex });
    try {
      const body: ImportPreviewRequest = {
        source: state.source,
        file_b64: file.b64,
      };
      if (state.accountAssetId) body.account_asset_id = state.accountAssetId;
      const preview = await apiPost<ImportPreviewResponseApi>(
        "/v1/transactions/import/preview",
        body,
      );
      if (!preview) throw new Error("Respuesta vacía del servidor.");
      // Los estados por-fila del componente son del archivo anterior.
      setExpandedLinks(new Set());
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

  /** Salta el archivo en curso sin importarlo; el último cierra la tanda. */
  function skipCurrentFile() {
    if (hasNextFile) void previewFileAt(state.fileIndex + 1);
    else finishAndClose(state.confirmed);
  }

  async function runConfirm() {
    if (!state.preview || !currentFile) return;
    dispatch({ type: "CONFIRM_START" });
    try {
      const body: ImportConfirmRequest = {
        source: state.source,
        file_b64: currentFile.b64,
        file_sha256: state.preview.file_sha256,
        decisions: buildConfirmDecisions(state.preview.rows, state.drafts),
        learn_rules: true,
      };
      if (state.accountAssetId) body.account_asset_id = state.accountAssetId;
      if (currentFile.name) body.original_filename = currentFile.name;
      const res = await apiPost<ImportConfirmResponseApi>(
        "/v1/transactions/import/confirm",
        body,
      );
      if (!res) throw new Error("Respuesta vacía del servidor.");
      dispatch({ type: "CONFIRM_OK", res });
      if (hasNextFile) {
        void previewFileAt(state.fileIndex + 1);
      } else {
        // `state.confirmed` es el del render del click: añade el res de ESTE confirm a mano.
        finishAndClose([...state.confirmed, res]);
      }
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

  // Cluster «Asignar a visibles»: aplica el kind (si hay uno elegido) y/o la categoría (si hay
  // una elegida) a las filas visibles. La categoría manda sobre el kind (deriva su propio scope).
  function applyBulkToVisible() {
    const patch: Partial<ImportRowDraft> = {};
    if (bulkKind) patch.kind = bulkKind;
    if (bulkCategory) {
      const [scope, id] = bulkCategory.split(":");
      patch.kind = scope === "income" ? "income" : "expense";
      patch.categoryId = id;
    }
    if (Object.keys(patch).length === 0) return;
    dispatch({ type: "PATCH_MANY", indices: visibleIndices, patch });
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
    <Modal title="Importar CSV" open={open} onClose={handleClose} wide>
      <div className="stack import-wizard">
        <ModalFormError message={state.error} />

        {state.step === "select" ? (
          <form
            className="stack"
            onSubmit={(e) => {
              e.preventDefault();
              void previewFileAt(0);
            }}
          >
            <p className="muted tight">
              Sube uno o varios CSV de tu banco. Detectamos el formato automáticamente;
              nada se guarda hasta que confirmes. Si subes varios, los revisarás y
              confirmarás uno a uno.
            </p>
            <label className="field">
              <span>Archivos .csv</span>
              <input
                type="file"
                accept=".csv,text/csv"
                multiple
                disabled={state.loading}
                onChange={(e) => {
                  const list = e.target.files ? Array.from(e.target.files) : [];
                  if (list.length === 0) {
                    dispatch({ type: "SET_FILES", files: [] });
                    return;
                  }
                  void Promise.all(
                    list.map(async (f) => ({
                      name: f.name,
                      b64: await readFileAsBase64(f),
                    })),
                  ).then((files) => dispatch({ type: "SET_FILES", files }));
                }}
              />
            </label>
            {state.files.length > 1 ? (
              <ul className="muted-list">
                {state.files.map((f, i) => (
                  <li key={`${f.name}-${i}`}>{f.name}</li>
                ))}
              </ul>
            ) : null}
            <label className="field">
              <span>Cuenta origen (activo)</span>
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
            <p className="muted tight">
              ¿De qué cuenta es el CSV? Los movimientos se vincularán a ese activo. Si
              subes varios archivos, la cuenta y el formato se aplican a todos.
            </p>
            <details className="import-source-override">
              <summary>
                Formato:{" "}
                {SOURCE_OPTIONS.find((s) => s.id === state.source)?.label ??
                  "Autodetectar"}
              </summary>
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
            </details>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={state.loading || state.files.length === 0}
              >
                {state.loading ? "Leyendo…" : "Previsualizar"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={state.loading}
                onClick={handleClose}
              >
                Cancelar
              </button>
            </div>
          </form>
        ) : state.preview ? (
          <div className="stack">
            {state.files.length > 1 ? (
              <p className="muted tight">
                Archivo {state.fileIndex + 1} de {state.files.length}:{" "}
                <strong>{currentFile?.name}</strong>
              </p>
            ) : null}
            <div className="banner info-banner tight-banner import-summary-banner">
              <strong>{capitalizeSource(state.preview.source)}</strong>
              <span className="import-chips">
                <span className="import-chip">{state.preview.row_count} filas</span>
                <span className="import-chip">{state.preview.new_count} nuevas</span>
                {state.preview.already_imported_count > 0 ? (
                  <span className="import-chip">
                    {state.preview.already_imported_count} duplicadas
                  </span>
                ) : null}
                {state.preview.suggested_transfer_count > 0 ? (
                  <span className="import-chip">
                    {state.preview.suggested_transfer_count} posibles transferencias
                  </span>
                ) : null}
                {state.preview.precategorized_count > 0 ? (
                  <span className="import-chip">
                    {state.preview.precategorized_count} pre-categorizadas
                  </span>
                ) : null}
                {state.preview.currency_warning_count > 0 ? (
                  <span className="import-chip">
                    {state.preview.currency_warning_count} otra divisa
                  </span>
                ) : null}
              </span>
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
                <div
                  className="import-bulk-assign"
                  role="group"
                  aria-label="Asignar a visibles"
                >
                  <label className="field inline-role">
                    <span className="sr-only">Asignar tipo a visibles</span>
                    <select
                      value={bulkKind}
                      onChange={(e) =>
                        setBulkKind(e.target.value as "" | TransactionKindApi)
                      }
                    >
                      <option value="">Tipo…</option>
                      {TRANSACTION_KINDS.map((k) => (
                        <option key={k} value={k}>
                          {KIND_LABEL_ES[k]}
                        </option>
                      ))}
                    </select>
                  </label>
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
                    disabled={!bulkKind && !bulkCategory}
                    onClick={applyBulkToVisible}
                  >
                    Aplicar a visibles
                  </button>
                </div>
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
                      {isMobile ? null : <th>Fecha</th>}
                      <th>Concepto</th>
                      {isMobile ? null : <th className="num">Importe</th>}
                      {isMobile ? null : <th>Tipo</th>}
                      {isMobile ? null : <th>Categoría</th>}
                      {isMobile ? (
                        <th className="row-chevron-cell">
                          <span className="sr-only">Detalles</span>
                        </th>
                      ) : (
                        <th className="import-link-cell">
                          <span className="sr-only">Vínculos</span>
                        </th>
                      )}
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
                        isMobile={isMobile}
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
                {summary.toImport} se importarán ·{" "}
                {summary.toSkip + summary.toDiscard} excluidas
                {summary.toSkip > 0
                  ? ` (${summary.toSkip} duplicadas ya guardadas)`
                  : ""}
              </div>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn primary"
                  disabled={state.loading || summary.toImport === 0}
                  onClick={() => void runConfirm()}
                >
                  {state.loading
                    ? "Importando…"
                    : hasNextFile
                      ? `Confirmar y seguir (${summary.toImport})`
                      : `Confirmar (${summary.toImport})`}
                </button>
                {state.confirmed.length === 0 && state.fileIndex === 0 ? (
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={state.loading}
                    onClick={() => dispatch({ type: "BACK_TO_SELECT" })}
                  >
                    {state.files.length > 1 ? "Cambiar archivos" : "Cambiar archivo"}
                  </button>
                ) : null}
                {state.files.length > 1 ? (
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={state.loading}
                    onClick={skipCurrentFile}
                  >
                    Omitir archivo
                  </button>
                ) : null}
                <button
                  type="button"
                  className="btn ghost"
                  disabled={state.loading}
                  onClick={handleClose}
                >
                  Cancelar
                </button>
              </div>
            </div>
          </div>
        ) : state.loading ? (
          <p className="muted">Leyendo {currentFile?.name ?? "el archivo"}…</p>
        ) : (
          // Preview del archivo en curso falló a mitad de tanda (el error ya está arriba):
          // se puede omitir el archivo, reintentar su preview, o cerrar conservando lo confirmado.
          <div className="asset-form-actions">
            <button type="button" className="btn primary" onClick={skipCurrentFile}>
              Omitir este archivo
            </button>
            <button
              type="button"
              className="btn ghost"
              onClick={() => void previewFileAt(state.fileIndex)}
            >
              Reintentar
            </button>
            <button type="button" className="btn ghost" onClick={handleClose}>
              Cancelar
            </button>
          </div>
        )}
      </div>
    </Modal>
  );
}

type PreviewRowProps = {
  index: number;
  row: ImportPreviewRowApi;
  draft: ImportRowDraft;
  currencyIso: string;
  isMobile: boolean;
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
  isMobile,
  incomeCategories,
  expenseCategories,
  assets,
  liabilities,
  expanded,
  onPatch,
  onToggleLinks,
}: PreviewRowProps) {
  const cats = categoriesForKind(draft.kind, incomeCategories, expenseCategories);
  // Las transferencias sugeridas YA NO se atenúan (3.5.0): entran incluidas por defecto y se
  // sacan del gasto por conciliación, no por descarte. Conservan su hint textual.
  const muted = row.status === "already_imported" || row.currency_warning;
  const amountNum = Number(row.amount);
  const amountClass = amountNum < 0 ? "num-neg" : amountNum > 0 ? "num-pos" : "";
  const statusHints: string[] = [];
  if (row.status === "already_imported") statusHints.push("Duplicado");
  if (row.suggested_transfer) statusHints.push("Transferencia");
  // La divisa del hogar es configurable desde 3.10.0: nombrarla evita decir «≠ EUR» a quien
  // lleva sus cuentas en libras.
  if (row.currency_warning) statusHints.push(`Divisa ≠ ${currencyIso}`);

  // En móvil kind/categoría/vínculos migran a la fila expandible. colSpan
  // dinámico: primera celda vacía (bajo el checkbox) + resto de columnas.
  const detailColSpan = isMobile ? 2 : 6;
  const kindSelect = (
    <select
      value={draft.kind}
      aria-label="Tipo"
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
  );
  const categorySelect = (
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
  );

  // Desktop conserva EXACTAMENTE la expresión original (`""` en filas no muted);
  // solo en móvil se añade `row-tappable`.
  const rowClassName = isMobile
    ? `${muted ? "import-row--muted " : ""}row-tappable`
    : muted
      ? "import-row--muted"
      : "";

  return (
    <>
      <tr
        className={rowClassName}
        role={isMobile ? "button" : undefined}
        tabIndex={isMobile ? 0 : undefined}
        aria-expanded={isMobile ? expanded : undefined}
        onClick={isMobile ? () => onToggleLinks(index) : undefined}
        onKeyDown={
          isMobile
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onToggleLinks(index);
                }
              }
            : undefined
        }
      >
        <td className="import-check-cell">
          <input
            type="checkbox"
            checked={draft.include}
            aria-label="Incluir fila"
            onClick={isMobile ? (e) => e.stopPropagation() : undefined}
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
        {isMobile ? null : <td>{formatDateDmy(row.op_date)}</td>}
        <td className="import-concept-cell">
          {row.concept}
          {isMobile ? (
            <span className="cell-subline">
              {formatDateDm(row.op_date)} ·{" "}
              {formatCurrencyAmount(row.amount, currencyIso)}
              {statusHints.length > 0 ? ` · ${statusHints.join(" · ")}` : ""}
            </span>
          ) : statusHints.length > 0 ? (
            <span className="import-row-hint"> {statusHints.join(" · ")}</span>
          ) : null}
        </td>
        {isMobile ? null : (
          <td className={`num ${amountClass}`}>
            {formatCurrencyAmount(row.amount, currencyIso)}
          </td>
        )}
        {isMobile ? null : <td>{kindSelect}</td>}
        {isMobile ? null : <td>{categorySelect}</td>}
        {isMobile ? (
          <td className="row-chevron-cell">
            <span className="row-chevron" aria-hidden>
              ›
            </span>
          </td>
        ) : (
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
        )}
      </tr>
      {expanded ? (
        <tr className="import-links-row">
          <td />
          <td colSpan={detailColSpan}>
            <div className="import-links-editor">
              {isMobile ? (
                <>
                  <label className="field inline-role">
                    <span>Tipo</span>
                    {kindSelect}
                  </label>
                  <label className="field inline-role">
                    <span>Categoría</span>
                    {categorySelect}
                  </label>
                </>
              ) : null}
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
