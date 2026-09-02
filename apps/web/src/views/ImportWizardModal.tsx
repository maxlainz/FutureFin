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
 * CATEGORÍA OBLIGATORIA (4.15.0): el preview llega con la categoría por defecto ya puesta en las
 * filas de ingreso/gasto que ninguna regla clasificó, y el select ya no ofrece «Sin categoría».
 * Confirmar queda bloqueado si alguna fila incluida se quedara sin ella — el guarda de verdad es
 * el servidor (`category_required`), esto es el cinturón. Una categoría marcada como «por
 * defecto» NO se propaga por automatch ni se aprende como regla (`categorySource`).
 *
 * Stateless: el confirm reenvía `file_b64` + `file_sha256` del preview. Perf: filas memoizadas y
 * filtros que no dependen del scroll.
 *
 * AUTOMATCH EN VIVO: cada asignación del usuario (fila o «Aplicar a visibles») entra en un
 * acumulador concepto→asignación de toda la SESIÓN y, tras un debounce, se repite el preview del
 * archivo en curso mandándolo en `pending_assignments`. Es el SERVIDOR quien recalcula las
 * sugerencias con su motor de reglas (patrón derivado + precedencia completa): aquí no se duplica
 * ni una línea de matching. Al volver, las filas que el usuario tocó quedan intactas y el resto
 * adopta lo recalculado — clasificar un «CAFE 365» reclasifica los otros catorce.
 */

import { memo, useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { apiPost } from "../api/client";
import type {
  AssetApiRow,
  CategoryRow,
  ImportConfirmRequest,
  ImportConfirmResponseApi,
  ImportPendingAssignmentApi,
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
  buildPendingAssignments,
  capitalizeSource,
  categoriesForKind,
  draftsEqual,
  initialDraftForRow,
  isRefundRow,
  mergePendingAssignments,
  mergeRepreview,
  rowMatchesFilter,
  summarizeDecisions,
  summarizeImportBatch,
  type ImportBatchSummary,
  type ImportRowDraft,
  type ImportRowFilter,
} from "../lib/expenses";

/**
 * Espera tras la ÚLTIMA asignación del usuario antes de repetir el preview. Suficiente para que
 * un «kind + categoría» seguidos (dos `onChange`) viajen en una sola petición, y corto como para
 * que la propagación se lea como inmediata.
 */
const REPREVIEW_DEBOUNCE_MS = 400;

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
  // «Sin clasificar», no «Sin categoría» (4.15.0): desde que el preview pre-rellena la categoría
  // por defecto, el filtro solo puede pescar filas que nadie ha llegado a clasificar.
  { id: "uncategorized", label: "Sin clasificar" },
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
  /** Paralelo a `drafts`: filas que el usuario tocó a mano. Un re-preview NUNCA las pisa. */
  touched: boolean[];
  filter: ImportRowFilter;
  loading: boolean;
  error: string | null;
  /** Respuestas de los confirms ya aplicados en esta tanda (para el agregado final). */
  confirmed: ImportConfirmResponseApi[];
  /**
   * Acumulador de asignaciones de la SESIÓN del wizard (concepto → asignación), no del archivo:
   * sobrevive a los saltos de la cola multi-CSV, así que lo clasificado en el archivo 1
   * pre-categoriza el 2 sin esperar al aprendizaje del confirm. Orden = recencia.
   */
  assignments: Map<string, ImportPendingAssignmentApi>;
  /** Se incrementa SOLO cuando `assignments` cambia: es lo que dispara el re-preview. */
  assignmentsVersion: number;
  /** Re-preview en vuelo. Deliberadamente NO es `loading`: la tabla sigue viva y editable. */
  repreviewing: boolean;
  /** Filas no-tocadas que movió el último re-preview (aviso «aplicada a N filas similares»). */
  automatchApplied: number | null;
};

const INITIAL: State = {
  step: "select",
  source: "auto",
  accountAssetId: "",
  files: [],
  fileIndex: 0,
  preview: null,
  drafts: [],
  touched: [],
  filter: "all",
  loading: false,
  error: null,
  confirmed: [],
  assignments: new Map(),
  assignmentsVersion: 0,
  repreviewing: false,
  automatchApplied: null,
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
  | { type: "REPREVIEW_START" }
  | { type: "REPREVIEW_OK"; preview: ImportPreviewResponseApi; fileIndex: number }
  | { type: "REPREVIEW_DONE" }
  | { type: "CONFIRM_START" }
  | { type: "CONFIRM_OK"; res: ImportConfirmResponseApi };

function applyPatch(d: ImportRowDraft, patch: Partial<ImportRowDraft>): ImportRowDraft {
  const next = { ...d, ...patch };
  const kindChanged = patch.kind !== undefined && patch.kind !== d.kind;
  if (kindChanged && patch.categoryId === undefined) next.categoryId = "";
  if (next.kind === "savings") next.categoryId = "";
  // Toda categoría que venga de un patch la ha elegido la persona: deja de ser sugerencia y
  // vuelve a alimentar el automatch aunque resulte ser la de por defecto (elegirla a mano SÍ es
  // una decisión). Y sin categoría no hay procedencia que declarar.
  if (patch.categoryId !== undefined && patch.categoryId !== "") {
    next.categorySource = "user";
  }
  if (next.categoryId === "") next.categorySource = null;
  return next;
}

/**
 * Registra en el acumulador lo que el usuario acaba de asignar en `indices`. Solo bumpea
 * `assignmentsVersion` —y por tanto solo relanza el preview— si el Map cambió de verdad: las
 * asignaciones que no pasan el gate (un gasto que se queda sin categoría) no tienen nada que
 * enseñarle al servidor.
 */
function recordAssignments(
  state: State,
  indices: number[],
  drafts: ImportRowDraft[],
): State {
  const rows = state.preview?.rows ?? [];
  const assignments = mergePendingAssignments(
    state.assignments,
    indices.map((i) => ({ concept: rows[i]?.concept ?? "", draft: drafts[i] })),
  );
  if (assignments === state.assignments) return state;
  return {
    ...state,
    assignments,
    assignmentsVersion: state.assignmentsVersion + 1,
  };
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
      // vieja no debe verse bajo el spinner del siguiente. `assignments` NO se limpia: es de la
      // sesión, no del archivo.
      return {
        ...state,
        loading: true,
        error: null,
        fileIndex: action.fileIndex,
        preview: null,
        drafts: [],
        touched: [],
        filter: "all",
        repreviewing: false,
        automatchApplied: null,
      };
    case "PREVIEW_OK":
      return {
        ...state,
        loading: false,
        error: null,
        step: "review",
        preview: action.preview,
        drafts: action.drafts,
        touched: action.drafts.map(() => false),
        filter: "all",
      };
    case "FAIL":
      return { ...state, loading: false, repreviewing: false, error: action.message };
    case "BACK_TO_SELECT":
      return {
        ...state,
        step: "select",
        preview: null,
        drafts: [],
        touched: [],
        error: null,
        repreviewing: false,
        automatchApplied: null,
      };
    case "SET_FILTER":
      return { ...state, filter: action.filter };
    case "PATCH_ONE": {
      const prev = state.drafts[action.index];
      if (!prev) return state;
      const next = applyPatch(prev, action.patch);
      // Un patch que no cambia nada no «toca» la fila: si lo hiciera, el automatch dejaría de
      // propagarse a filas que el usuario jamás miró.
      if (draftsEqual(prev, next)) return state;
      const drafts = state.drafts.map((d, i) => (i === action.index ? next : d));
      return recordAssignments(
        {
          ...state,
          drafts,
          touched: state.touched.map((t, i) => (i === action.index ? true : t)),
          automatchApplied: null,
        },
        [action.index],
        drafts,
      );
    }
    case "PATCH_MANY": {
      const set = new Set(action.indices);
      const changed: number[] = [];
      const drafts = state.drafts.map((d, i) => {
        if (!set.has(i)) return d;
        const next = applyPatch(d, action.patch);
        if (draftsEqual(d, next)) return d;
        changed.push(i);
        return next;
      });
      if (changed.length === 0) return state;
      const changedSet = new Set(changed);
      return recordAssignments(
        {
          ...state,
          drafts,
          touched: state.touched.map((t, i) => t || changedSet.has(i)),
          automatchApplied: null,
        },
        changed,
        drafts,
      );
    }
    case "REPREVIEW_START":
      return { ...state, repreviewing: true };
    case "REPREVIEW_DONE":
      return { ...state, repreviewing: false };
    case "REPREVIEW_OK": {
      // La respuesta puede llegar cuando la cola ya avanzó de archivo: el `file_sha256` y el
      // índice son la prueba de que este preview es el del archivo que hay en pantalla.
      const prev = state.preview;
      if (
        !prev ||
        action.fileIndex !== state.fileIndex ||
        action.preview.file_sha256 !== prev.file_sha256
      ) {
        return { ...state, repreviewing: false };
      }
      const merged = mergeRepreview(
        state.drafts,
        state.touched,
        action.preview.rows,
        prev.rows,
      );
      if (!merged) return { ...state, repreviewing: false };
      return {
        ...state,
        repreviewing: false,
        preview: action.preview,
        drafts: merged.drafts,
        automatchApplied: merged.changed,
      };
    }
    case "CONFIRM_START":
      return { ...state, loading: true, error: null, repreviewing: false };
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

  /**
   * Espejo del estado para el re-preview diferido. El debounce corre 400 ms DESPUÉS del render
   * que lo programó, así que leer de una clausura daría el estado de entonces; y meter el estado
   * en las dependencias del efecto lo relanzaría con cada respuesta —el re-preview reemplaza
   * `preview`— en un bucle infinito. Se declara ANTES del efecto del debounce para que React lo
   * refresque primero en el mismo commit.
   */
  const latest = useRef(state);
  useEffect(() => {
    latest.current = state;
  });

  const repreviewTimer = useRef<number | null>(null);
  /** Sello del re-preview en vuelo: una respuesta con sello viejo se descarta (el `fetch` no se
   *  aborta, pero su resultado no llega nunca a la tabla). */
  const repreviewSeq = useRef(0);

  /** Cancela el re-preview pendiente Y el que esté en vuelo. */
  const cancelRepreview = useCallback(() => {
    if (repreviewTimer.current !== null) {
      window.clearTimeout(repreviewTimer.current);
      repreviewTimer.current = null;
    }
    repreviewSeq.current += 1;
  }, []);

  // Reset cada vez que se (re)abre el modal.
  useEffect(() => {
    if (open) {
      cancelRepreview();
      dispatch({ type: "RESET" });
      setExpandedLinks(new Set());
      setBulkKind("");
      setBulkCategory("");
    }
  }, [open, cancelRepreview]);

  /**
   * Repite el preview del archivo EN CURSO con las asignaciones de la sesión, para que el
   * servidor —con su motor de reglas real, no una copia en TS— recalcule las sugerencias. Las
   * filas que el usuario tocó no se tocan; las demás adoptan lo recalculado.
   *
   * Es una comodidad, no una operación del import: si falla, se traga el error (la tabla vigente
   * sigue siendo válida y el banner del wizard está reservado para los fallos de preview/confirm).
   */
  const runRepreview = useCallback(async () => {
    const s = latest.current;
    const file = s.files[s.fileIndex];
    if (s.step !== "review" || s.loading || !file || !s.preview) return;
    const seq = ++repreviewSeq.current;
    const fileIndex = s.fileIndex;
    dispatch({ type: "REPREVIEW_START" });
    try {
      const body: ImportPreviewRequest = { source: s.source, file_b64: file.b64 };
      if (s.accountAssetId) body.account_asset_id = s.accountAssetId;
      const pending = buildPendingAssignments(s.assignments);
      if (pending.length > 0) body.pending_assignments = pending;
      const preview = await apiPost<ImportPreviewResponseApi>(
        "/v1/transactions/import/preview",
        body,
      );
      if (seq !== repreviewSeq.current) return;
      if (!preview) {
        dispatch({ type: "REPREVIEW_DONE" });
        return;
      }
      dispatch({ type: "REPREVIEW_OK", preview, fileIndex });
    } catch {
      if (seq !== repreviewSeq.current) return;
      dispatch({ type: "REPREVIEW_DONE" });
    }
  }, []);

  // Debounce: cada asignación nueva reprograma el re-preview y anula el anterior.
  useEffect(() => {
    if (state.assignmentsVersion === 0) return;
    if (repreviewTimer.current !== null) window.clearTimeout(repreviewTimer.current);
    const id = window.setTimeout(() => {
      repreviewTimer.current = null;
      void runRepreview();
    }, REPREVIEW_DEBOUNCE_MS);
    repreviewTimer.current = id;
    return () => {
      window.clearTimeout(id);
      if (repreviewTimer.current === id) repreviewTimer.current = null;
    };
  }, [state.assignmentsVersion, runRepreview]);

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
    cancelRepreview();
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
    cancelRepreview();
    dispatch({ type: "PREVIEW_START", fileIndex });
    try {
      const body: ImportPreviewRequest = {
        source: state.source,
        file_b64: file.b64,
      };
      if (state.accountAssetId) body.account_asset_id = state.accountAssetId;
      // También en el preview INICIAL de cada archivo: lo clasificado en los anteriores de la
      // cola llega pre-categorizado sin esperar a que el confirm consolide las reglas.
      const pending = buildPendingAssignments(state.assignments);
      if (pending.length > 0) body.pending_assignments = pending;
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
    // Un re-preview a medio vuelo reescribiría los drafts DESPUÉS de leerlos para las decisiones.
    cancelRepreview();
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

  /**
   * Aviso del automatch. Vive en un slot de altura reservada (`.import-automatch-note` siempre se
   * renderiza, con un espacio duro cuando está vacío): la tabla no puede saltar cada vez que el
   * servidor propaga una categoría.
   */
  const automatchNote = state.repreviewing
    ? "Aplicando categorías…"
    : state.automatchApplied && state.automatchApplied > 0
      ? `Categoría aplicada a ${state.automatchApplied} ${
          state.automatchApplied === 1 ? "fila similar" : "filas similares"
        }.`
      : "";

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

            <p className="import-automatch-note" aria-live="polite">
              {automatchNote === "" ? " " : automatchNote}
            </p>

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
                <div>
                  {summary.toImport} se importarán ·{" "}
                  {summary.toSkip + summary.toDiscard} excluidas
                  {summary.toSkip > 0
                    ? ` (${summary.toSkip} duplicadas ya guardadas)`
                    : ""}
                  {summary.missingCategory > 0 ? (
                    <>
                      {" · "}
                      <strong>
                        {summary.missingCategory}{" "}
                        {summary.missingCategory === 1 ? "fila" : "filas"} sin categoría
                      </strong>
                    </>
                  ) : null}
                </div>
                <div>Otros gastos/ingresos no se aprende como regla.</div>
              </div>
              <div className="asset-form-actions">
                <button
                  type="button"
                  className="btn primary"
                  // El cinturón, no la guarda: el servidor rechaza el confirm con una fila de
                  // ingreso/gasto sin categoría (`category_required`). Bloquear aquí evita el
                  // viaje y señala en el footer cuántas faltan.
                  disabled={
                    state.loading ||
                    summary.toImport === 0 ||
                    summary.missingCategory > 0
                  }
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
  // Se mira el DRAFT, no la sugerencia del servidor: si el usuario mueve la fila a Ingreso, la
  // señal de devolución desaparece en el acto.
  const refund = isRefundRow(draft.kind, row.amount);
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
      {/* Sin opción «Sin categoría» en ingreso/gasto (4.15.0): el servidor rechaza el confirm
          de una fila así, y el preview ya llega con la categoría por defecto puesta. El hueco
          solo aparece —deshabilitado— si de verdad no hay ninguna, para que el select no mienta
          preseleccionando la primera de la lista. */}
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
          {refund ? (
            <>
              {" "}
              <span
                className="ff-refund-tag"
                title="Gasto con importe positivo: una devolución. Resta del gasto de su categoría."
              >
                Devolución
              </span>
            </>
          ) : null}
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
