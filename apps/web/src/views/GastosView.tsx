/**
 * Pestaña «Gastos» — histórico de gasto mensual (v1.6.0). Vista AUTÓNOMA (patrón
 * HistorySettingsPanel): hace sus propios fetch al montar y en cada cambio de scope/mes/ventana;
 * recibe props finas de App. Sus mutaciones nunca tocan la cache de proyección (las transacciones
 * no son inputs del engine); `onCashflowMutated` avisa a App para refrescar el overlay del chart.
 *
 * Estructura: KPIs → toolbar (mes + ventana + acciones) → Comparativa
 * (tabla + barras + cash-flow) → Movimientos (tabla con edición inline optimista + modal de
 * edición completa + borrado). Las conciliadas (traspasos internos) se listan atenuadas con tag
 * «conciliada» y se desconcilian desde el modal; el servidor ya las excluye de los totales.
 */

import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { apiDelete, apiDeleteJson, apiGet, apiPatch, apiPost } from "../api/client";
import type {
  AssetApiRow,
  CategoryComparisonLineApi,
  CategoryRow,
  HistoryCashflowApi,
  InstallationAccess,
  LiabilityApiRow,
  PatchTransactionRequest,
  ReconcilePairResponseApi,
  RecurringMaterializeResponse,
  TransactionApi,
  TransactionKindApi,
  TransactionMonthApi,
  TransactionsSummaryApi,
  UserResponse,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronIcon,
  ChevronLeftIcon,
  EqualsIcon,
  LinkIcon,
  PlusIcon,
  RefreshIcon,
  RowEditIcon,
  RowTrashIcon,
  UploadIcon,
} from "../components/icons";
import { MonthlyCashflowBars } from "../components/charts/CategoryComparisonBars";
import {
  formatCurrencyAmount,
  formatPercentDisplay,
  METRIC_DASH,
  parseDisplayDecimal,
  toApiDecimalString,
} from "../lib/format";
import { formatDateDm, formatDateDmy, todayYmdInTimeZone } from "../lib/dates";
import { ledgerViewQs, type LedgerPersonScope } from "../lib/ledger";
import { useIsMobile } from "../lib/responsive";
import {
  AVG_WINDOWS,
  KIND_LABEL_ES,
  TRANSACTION_KINDS,
  adjacentMonthInList,
  avgBasisDetail,
  avgUnavailableDetail,
  avgWindowLabel,
  categoriesForKind,
  defaultSelectedMonth,
  formatDeltaCurrency,
  groupTransactionsByCategory,
  isReconciled,
  kpiBudgetTrend,
  monthLabelEs,
  naturalSortDir,
  parseMonth,
  significanceThreshold,
  significantDeltaTone,
  sortTransactionGroups,
  sortTransactions,
  transactionMatchesQuery,
  trendArrow,
  type TxnSortDir,
  type TxnSortKey,
} from "../lib/expenses";
import { ImportWizardModal } from "./ImportWizardModal";
import { ManualCashEntryModal } from "./ManualCashEntryModal";
import { RecurringRulesModal } from "./RecurringRulesModal";

/** Glifo de dirección de tendencia (compartido por la tabla y las KPIs). `null`/`undefined` → sin
 * glifo (no hay datos ≠ sin cambio). */
function trendGlyph(
  direction: "up" | "down" | "flat" | null | undefined,
): ReactNode {
  switch (direction) {
    case "up":
      return <ArrowUpIcon />;
    case "down":
      return <ArrowDownIcon />;
    case "flat":
      return <EqualsIcon />;
    default:
      return null;
  }
}

export function GastosView({
  installation,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  onCashflowMutated,
}: {
  installation: InstallationAccess | null;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  /** App lo pasa; reservado para futuras comprobaciones de titular. */
  user: UserResponse | null;
  /** Avisa a App para re-fetch del overlay de cash-flow del chart grande. */
  onCashflowMutated?: () => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const calendarTz = installation?.installation.calendar_tz?.trim() || "UTC";
  const today = todayYmdInTimeZone(calendarTz);
  const isMobile = useIsMobile();
  const viewSuffix = ledgerViewQs(ledgerPersonScope);
  const viewAmp = ledgerPersonScope === "mine" ? "&view=mine" : "";

  const [incomeCategories, setIncomeCategories] = useState<CategoryRow[]>([]);
  const [expenseCategories, setExpenseCategories] = useState<CategoryRow[]>([]);
  const [assets, setAssets] = useState<AssetApiRow[]>([]);
  const [liabilities, setLiabilities] = useState<LiabilityApiRow[]>([]);
  const [months, setMonths] = useState<TransactionMonthApi[]>([]);
  const [selectedMonth, setSelectedMonth] = useState<string | null>(null);
  const [avgWindow, setAvgWindow] = useState<string>("6");

  const [summary, setSummary] = useState<TransactionsSummaryApi | null>(null);
  const [transactions, setTransactions] = useState<TransactionApi[]>([]);
  const [cashflow, setCashflow] = useState<HistoryCashflowApi | null>(null);

  const [bootstrapLoading, setBootstrapLoading] = useState(true);
  const [monthLoading, setMonthLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  const [importOpen, setImportOpen] = useState(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [recurringOpen, setRecurringOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<TransactionApi | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TransactionApi | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [deletingRecurring, setDeletingRecurring] = useState(false);
  /** Materialización de recurrentes: una sola vez por montaje de la vista. */
  const materializedOnce = useRef(false);

  // Controles de la tabla de movimientos (estado local, filtrado/orden en vivo, sin fetch).
  const [movementQuery, setMovementQuery] = useState("");
  const [grouped, setGrouped] = useState(true);
  const [sortKey, setSortKey] = useState<TxnSortKey>("date");
  const [sortDir, setSortDir] = useState<TxnSortDir>("desc");

  const toggleSort = useCallback(
    (key: TxnSortKey) => {
      if (sortKey === key) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"));
      } else {
        setSortKey(key);
        setSortDir(naturalSortDir(key));
      }
    },
    [sortKey],
  );

  // ---- Loaders -----------------------------------------------------------

  const loadBootstrap = useCallback(async () => {
    if (!hasMembership) {
      setBootstrapLoading(false);
      return;
    }
    setBootstrapLoading(true);
    setError(null);
    try {
      const [inc, exp, ast, lia, mo] = await Promise.all([
        apiGet<CategoryRow[]>(`/v1/categories?scope=income`),
        apiGet<CategoryRow[]>(`/v1/categories?scope=expense`),
        apiGet<AssetApiRow[]>(`/v1/assets${viewSuffix}`),
        apiGet<LiabilityApiRow[]>(`/v1/liabilities${viewSuffix}`),
        apiGet<TransactionMonthApi[]>(`/v1/transactions/months${viewSuffix}`),
      ]);
      setIncomeCategories(inc);
      setExpenseCategories(exp);
      setAssets(ast);
      setLiabilities(lia);
      setMonths(mo);
      setSelectedMonth((prev) =>
        prev && mo.some((m) => m.month === prev) ? prev : defaultSelectedMonth(mo),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudieron cargar los datos.");
    } finally {
      setBootstrapLoading(false);
    }
  }, [hasMembership, viewSuffix]);

  const reloadMonths = useCallback(async () => {
    try {
      const mo = await apiGet<TransactionMonthApi[]>(
        `/v1/transactions/months${viewSuffix}`,
      );
      setMonths(mo);
      setSelectedMonth((prev) =>
        prev && mo.some((m) => m.month === prev) ? prev : defaultSelectedMonth(mo),
      );
    } catch {
      /* silencioso: el error principal ya se muestra en otros loaders */
    }
  }, [viewSuffix]);

  const loadSummary = useCallback(async () => {
    if (!selectedMonth) {
      setSummary(null);
      return;
    }
    const c = parseMonth(selectedMonth);
    if (!c) return;
    try {
      const sum = await apiGet<TransactionsSummaryApi>(
        `/v1/transactions/summary?year=${c.y}&month=${c.m}&avg_window=${avgWindow}${viewAmp}`,
      );
      setSummary(sum);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudo cargar la comparativa.");
      setSummary(null);
    }
  }, [selectedMonth, avgWindow, viewAmp]);

  const loadTransactions = useCallback(async () => {
    if (!selectedMonth) {
      setTransactions([]);
      return;
    }
    try {
      const t = await apiGet<TransactionApi[]>(
        `/v1/transactions?month=${selectedMonth}${viewAmp}`,
      );
      setTransactions(t);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudieron cargar los movimientos.");
      setTransactions([]);
    }
  }, [selectedMonth, viewAmp]);

  const loadCashflow = useCallback(async () => {
    try {
      const cf = await apiGet<HistoryCashflowApi>(
        `/v1/history/cashflow?window_months=24${viewAmp}`,
      );
      setCashflow(cf);
    } catch {
      setCashflow(null); // degradación silenciosa (sección de barras → "Sin datos.")
    }
  }, [viewAmp]);

  useEffect(() => {
    void loadBootstrap();
  }, [loadBootstrap]);

  useEffect(() => {
    if (!hasMembership) return;
    setMonthLoading(true);
    void Promise.all([loadSummary(), loadTransactions()]).finally(() =>
      setMonthLoading(false),
    );
  }, [hasMembership, loadSummary, loadTransactions, refreshKey]);

  useEffect(() => {
    if (!hasMembership) return;
    void loadCashflow();
  }, [hasMembership, loadCashflow, refreshKey]);

  useEffect(() => {
    if (!notice) return;
    const t = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(t);
  }, [notice]);

  // ---- Mutations ---------------------------------------------------------

  /** Refresco completo tras alta/import/borrado (puede cambiar los meses disponibles). */
  const handleMutated = useCallback(async () => {
    await reloadMonths();
    setRefreshKey((k) => k + 1);
    void loadCashflow();
    onCashflowMutated?.();
  }, [reloadMonths, loadCashflow, onCashflowMutated]);

  /** Refresco de derivados tras edición inline (la fila ya está actualizada en local). */
  const refreshDerived = useCallback(() => {
    void loadSummary();
    void loadCashflow();
    onCashflowMutated?.();
  }, [loadSummary, loadCashflow, onCashflowMutated]);

  const categoryFitsKind = useCallback(
    (categoryId: string, kind: TransactionKindApi): boolean => {
      const list = categoriesForKind(kind, incomeCategories, expenseCategories);
      return list.some((c) => c.id === categoryId);
    },
    [incomeCategories, expenseCategories],
  );

  const patchTransaction = useCallback(
    async (
      id: string,
      patch: PatchTransactionRequest,
      optimistic: (t: TransactionApi) => TransactionApi,
    ) => {
      setRowError(null);
      setTransactions((ts) => ts.map((t) => (t.id === id ? optimistic(t) : t)));
      try {
        const updated = await apiPatch<TransactionApi>(
          `/v1/transactions/${id}`,
          patch,
        );
        if (updated) {
          setTransactions((ts) => ts.map((t) => (t.id === id ? updated : t)));
        }
        refreshDerived();
      } catch (e) {
        setRowError(e instanceof Error ? e.message : "No se pudo actualizar.");
        void loadTransactions(); // revert a la verdad del servidor
      }
    },
    [refreshDerived, loadTransactions],
  );

  const onInlineKind = useCallback(
    (row: TransactionApi, newKind: TransactionKindApi) => {
      if (newKind === row.kind) return;
      const patch: PatchTransactionRequest = { kind: newKind };
      const clears =
        newKind === "savings" ||
        (!!row.category_id && !categoryFitsKind(row.category_id, newKind));
      if (clears) patch.clear_category = true;
      void patchTransaction(row.id, patch, (t) => ({
        ...t,
        kind: newKind,
        category_id: clears ? undefined : t.category_id,
        category_name: clears ? undefined : t.category_name,
      }));
    },
    [categoryFitsKind, patchTransaction],
  );

  const onInlineCategory = useCallback(
    (row: TransactionApi, newCategoryId: string) => {
      const patch: PatchTransactionRequest = newCategoryId
        ? { category_id: newCategoryId }
        : { clear_category: true };
      const name = newCategoryId
        ? categoriesForKind(row.kind ?? "expense", incomeCategories, expenseCategories).find(
            (c) => c.id === newCategoryId,
          )?.name
        : undefined;
      void patchTransaction(row.id, patch, (t) => ({
        ...t,
        category_id: newCategoryId || undefined,
        category_name: name,
      }));
    },
    [incomeCategories, expenseCategories, patchTransaction],
  );

  const confirmDelete = useCallback(async () => {
    if (!deleteTarget) return;
    try {
      await apiDelete(`/v1/transactions/${deleteTarget.id}`);
      setDeleteTarget(null);
      setNotice("Movimiento eliminado.");
      await handleMutated();
    } catch (e) {
      setRowError(e instanceof Error ? e.message : "No se pudo eliminar.");
      setDeleteTarget(null);
    }
  }, [deleteTarget, handleMutated]);

  /** Borra la regla recurrente ANTES del movimiento, luego el movimiento (secuencial). */
  const confirmDeleteAndStopRecurring = useCallback(async () => {
    if (!deleteTarget?.recurring_rule_id) return;
    setDeletingRecurring(true);
    setRowError(null);
    try {
      await apiDelete(
        `/v1/transactions/recurring/${deleteTarget.recurring_rule_id}`,
      );
      await apiDelete(`/v1/transactions/${deleteTarget.id}`);
      setDeleteTarget(null);
      setNotice("Movimiento eliminado y repetición detenida.");
      await handleMutated();
    } catch (e) {
      setRowError(e instanceof Error ? e.message : "No se pudo eliminar.");
      setDeleteTarget(null);
    } finally {
      setDeletingRecurring(false);
    }
  }, [deleteTarget, handleMutated]);

  // Materializa las reglas recurrentes pendientes una sola vez por montaje. Silencioso:
  // si falla, el pasado/presente queda igual. Refresca solo si generó algún movimiento.
  useEffect(() => {
    if (!hasMembership || !canEdit || materializedOnce.current) return;
    materializedOnce.current = true;
    void (async () => {
      try {
        const res = await apiPost<RecurringMaterializeResponse>(
          "/v1/transactions/recurring/materialize",
          {},
        );
        if (res && res.materialized > 0) await handleMutated();
      } catch {
        /* silencioso */
      }
    })();
  }, [hasMembership, canEdit, handleMutated]);

  // ---- Derived KPIs ------------------------------------------------------

  const totals = summary?.totals ?? null;
  const isPartial = summary?.is_partial ?? false;
  // El denominador del promedio son los meses REALES, no todos los que tienen algo: un mes cuyo
  // único contenido son instancias recurrentes no promedia (y por eso no diluye la media).
  const avgMonths = summary?.avg_months ?? 0;
  const hasAvg = avgMonths > 0;
  // Qué meses produjeron la media, o por qué no hay. La ventana pedida («6m») no es lo mismo que
  // los meses promediados, así que la tarjeta lo dice en vez de dejarlo suponer.
  // Va al slot `detail` (segundo), no al `parenthetical`: en gasto e ingreso el primero lo ocupa
  // el trend, que tiene prioridad en MetricCard. Con `detail` en las cuatro, la base sale siempre
  // a la misma altura de la fila.
  const avgBasisText = summary
    ? (avgBasisDetail(summary.avg_basis) ??
      avgUnavailableDetail(summary.avg_unavailable_reason))
    : undefined;
  const threshold = summary ? significanceThreshold(summary.totals) : 0;
  const avgLabel = avgWindowLabel(avgWindow);

  const savingsRateAvg = useMemo(() => {
    if (!totals) return null;
    const inc = parseDisplayDecimal(totals.income_avg) ?? 0;
    const sav = parseDisplayDecimal(totals.savings_avg) ?? 0;
    if (inc <= 0) return null;
    return (sav / inc) * 100;
  }, [totals]);

  const noCategories =
    incomeCategories.length === 0 && expenseCategories.length === 0;

  // ---- Movimientos: búsqueda / orden / agrupación (todo en cliente) -------

  /** id → nombre de la categoría viva (para las cabeceras de grupo). */
  const categoryNameById = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of [...incomeCategories, ...expenseCategories]) m.set(c.id, c.name);
    return m;
  }, [incomeCategories, expenseCategories]);

  const visibleRows = useMemo(
    () =>
      transactions.filter((t) =>
        transactionMatchesQuery(t.concept, t.category_name ?? null, movementQuery),
      ),
    [transactions, movementQuery],
  );

  const sortedRows = useMemo(
    () => sortTransactions(visibleRows, sortKey, sortDir),
    [visibleRows, sortKey, sortDir],
  );

  // El orden de los GRUPOS es fijo (kind → |subtotal| desc); la clave activa solo
  // ordena las filas dentro de cada grupo.
  const movementGroups = useMemo(() => {
    const gs = groupTransactionsByCategory(visibleRows, categoryNameById);
    return sortTransactionGroups(gs).map((g) => ({
      ...g,
      rows: sortTransactions(g.rows, sortKey, sortDir),
    }));
  }, [visibleRows, categoryNameById, sortKey, sortDir]);

  /** Nº de columnas visible de la tabla de movimientos (para el colspan del grupo). */
  const movementCols = isMobile ? 3 : 6 + (canEdit ? 1 : 0);

  // ---- Render helpers ----------------------------------------------------

  const monthBadgePartial =
    selectedMonth !== null &&
    months.find((m) => m.month === selectedMonth)?.is_complete === false;

  /**
   * Flecha de tendencia «real vs promedio» para la celda Real (visible también en móvil). El slot
   * se renderiza SIEMPRE con ancho fijo (aunque no haya glifo) para no desalinear las cifras de la
   * columna Real — mismo principio que el paren-slot de MetricCard. Con promedio pero desviación
   * bajo el umbral → «=» atenuado (`flat`); sin promedio → slot vacío (sin datos ≠ sin cambio).
   */
  function renderTrend(deltaVsAvg: number, kind: "expense" | "income") {
    const arrow = trendArrow(deltaVsAvg, kind, threshold, hasAvg);
    return (
      <span
        className={`exp-trend-slot ${arrow.direction === "flat" ? "muted" : arrow.tone}`}
        title={arrow.direction ? "Real vs promedio" : undefined}
        aria-hidden={arrow.direction ? undefined : true}
      >
        {trendGlyph(arrow.direction)}
      </span>
    );
  }

  /**
   * Tendencia «promedio vs presupuesto» bajo la cifra principal de un KPI (solo Gastos e Ingresos).
   * Ocupa el slot reservado del paréntesis de MetricCard. `undefined` (sin tendencia → slot vacío) si
   * no hay promedio o no hay presupuesto contra el que comparar. La flecha y la cifra delta comparten
   * tono (num-pos/num-neg); «vs presupuesto» hereda muted.
   */
  function renderKpiTrend(
    avg: string,
    budget: string,
    kind: "expense" | "income",
  ): ReactNode {
    const t = kpiBudgetTrend(
      parseDisplayDecimal(avg) ?? 0,
      parseDisplayDecimal(budget) ?? 0,
      kind,
      threshold,
      hasAvg,
    );
    if (!t) return undefined;
    const colorClass = t.direction === "flat" ? "" : t.tone;
    // Orden fijo: flecha + delta (nunca truncados) y «vs presupuesto» al final con ellipsis si
    // la tarjeta es estrecha, para no romper la altura de una línea de la banda KPI.
    return (
      <span className="metric-trend">
        <span className={`metric-trend-arrow ${colorClass}`} aria-hidden>
          {trendGlyph(t.direction)}
        </span>
        <span className={`metric-trend-delta ${colorClass}`}>
          {formatDeltaCurrency(t.delta, currencyIso)}
        </span>
        <span className="metric-trend-label">vs presupuesto</span>
      </span>
    );
  }

  function renderComparisonTable(
    lines: CategoryComparisonLineApi[],
    kind: "expense" | "income",
    total: { actual: string; budget: string; avg: string },
    threshold: number,
    hasAvg: boolean,
  ) {
    if (lines.length === 0) {
      return <p className="muted bordered-top">Sin datos.</p>;
    }
    const avgLabel = avgWindowLabel(avgWindow);
    const totalActual = parseDisplayDecimal(total.actual) ?? 0;
    const totalBudget = parseDisplayDecimal(total.budget) ?? 0;
    const totalAvg = parseDisplayDecimal(total.avg) ?? 0;
    const totalDeltaBudget = totalActual - totalBudget;
    const totalDeltaAvg = totalActual - totalAvg;
    const totalTone = isPartial
      ? "muted"
      : significantDeltaTone(totalDeltaBudget, kind, threshold);
    return (
      <div className="table-scroll bordered-top">
        <table className="assets-table exp-comparison-table">
          <thead>
            <tr>
              <th>Categoría</th>
              <th className="num">Real</th>
              {isMobile ? null : <th className="num">Presupuesto</th>}
              <th className="num">Δ</th>
              {isMobile ? null : <th className="num">Promedio {avgLabel}</th>}
            </tr>
          </thead>
          <tbody>
            {lines.map((l) => {
              const dNum = parseDisplayDecimal(l.delta_vs_budget) ?? 0;
              const dAvg = parseDisplayDecimal(l.delta_vs_avg) ?? 0;
              const toneClass = isPartial
                ? "muted"
                : significantDeltaTone(dNum, kind, threshold);
              return (
                <tr key={l.category_id ?? "uncategorized"}>
                  <td>
                    {l.category_id ? (
                      l.category_name
                    ) : (
                      <span className="exp-uncategorized">
                        <span className="exp-cat-dot" aria-hidden /> {l.category_name}
                      </span>
                    )}
                    {isMobile ? (
                      <span className="cell-subline">
                        Presup. {formatCurrencyAmount(l.budget, currencyIso)} · Prom{" "}
                        {avgLabel}{" "}
                        {hasAvg
                          ? formatCurrencyAmount(l.avg, currencyIso)
                          : METRIC_DASH}
                      </span>
                    ) : null}
                  </td>
                  <td className="num">
                    {formatCurrencyAmount(l.actual, currencyIso)}
                    {renderTrend(dAvg, kind)}
                  </td>
                  {isMobile ? null : (
                    <td className="num">{formatCurrencyAmount(l.budget, currencyIso)}</td>
                  )}
                  <td className={`num ${toneClass}`}>
                    {formatDeltaCurrency(dNum, currencyIso)}
                  </td>
                  {isMobile ? null : (
                    <td className="num">
                      {hasAvg ? formatCurrencyAmount(l.avg, currencyIso) : METRIC_DASH}
                    </td>
                  )}
                </tr>
              );
            })}
            <tr className="exp-total-row">
              <td>
                Total
                {isMobile ? (
                  <span className="cell-subline">
                    Presup. {formatCurrencyAmount(total.budget, currencyIso)} · Prom{" "}
                    {avgLabel}{" "}
                    {hasAvg
                      ? formatCurrencyAmount(total.avg, currencyIso)
                      : METRIC_DASH}
                  </span>
                ) : null}
              </td>
              <td className="num">
                {formatCurrencyAmount(total.actual, currencyIso)}
                {renderTrend(totalDeltaAvg, kind)}
              </td>
              {isMobile ? null : (
                <td className="num">{formatCurrencyAmount(total.budget, currencyIso)}</td>
              )}
              <td className={`num ${totalTone}`}>
                {formatDeltaCurrency(totalDeltaBudget, currencyIso)}
              </td>
              {isMobile ? null : (
                <td className="num">
                  {hasAvg ? formatCurrencyAmount(total.avg, currencyIso) : METRIC_DASH}
                </td>
              )}
            </tr>
          </tbody>
        </table>
      </div>
    );
  }

  /** Cabecera ordenable de la tabla de movimientos (`<th>` con `aria-sort` + botón e indicador). */
  function renderSortHeader(label: string, key: TxnSortKey, numeric = false) {
    const active = sortKey === key;
    return (
      <th
        className={numeric ? "num" : undefined}
        aria-sort={active ? (sortDir === "asc" ? "ascending" : "descending") : "none"}
      >
        <button
          type="button"
          className={`exp-sort-btn ${active ? "is-active" : ""}`}
          onClick={() => toggleSort(key)}
        >
          <span>{label}</span>
          {active ? sortDir === "asc" ? <ArrowUpIcon /> : <ArrowDownIcon /> : null}
        </button>
      </th>
    );
  }

  /** Una fila de la tabla de movimientos (compartida por el modo agrupado y el plano). */
  function renderMovementRow(t: TransactionApi): ReactNode {
    const kind = t.kind ?? "expense";
    const amountNum = parseDisplayDecimal(t.amount) ?? 0;
    const amountClass = amountNum < 0 ? "num-neg" : amountNum > 0 ? "num-pos" : "";
    const cats = categoriesForKind(kind, incomeCategories, expenseCategories);
    const hasLink = !!(t.linked_asset_id || t.linked_liability_id);
    const reconciled = isReconciled(t);
    const reconciledTitle = reconciled
      ? `Conciliada con «${t.transfer_counterpart_concept ?? "—"}» (${
          t.transfer_counterpart_op_date
            ? formatDateDmy(t.transfer_counterpart_op_date)
            : "—"
        })`
      : undefined;
    const rowTappable = isMobile && canEdit;
    const openEdit = () => {
      setRowError(null);
      setEditTarget(t);
    };
    const categoryLabel =
      t.category_name ?? (kind === "savings" ? "—" : "Sin categoría");
    return (
      <tr
        key={t.id}
        className={
          [rowTappable ? "row-tappable" : "", reconciled ? "exp-row-reconciled" : ""]
            .filter(Boolean)
            .join(" ") || undefined
        }
        role={rowTappable ? "button" : undefined}
        tabIndex={rowTappable ? 0 : undefined}
        onClick={rowTappable ? openEdit : undefined}
        onKeyDown={
          rowTappable
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  openEdit();
                }
              }
            : undefined
        }
      >
        <td>{isMobile ? formatDateDm(t.op_date) : formatDateDmy(t.op_date)}</td>
        <td className="exp-concept-cell">
          {t.concept}
          {isMobile ? (
            <span className="cell-subline">
              {categoryLabel} · {KIND_LABEL_ES[kind]}
              {t.import_id ? null : " · efectivo"}
              {t.recurring_rule_id ? " · recurrente" : null}
              {reconciled ? " · conciliada" : null}
              {hasLink ? (
                <>
                  {" · "}
                  <LinkIcon />
                </>
              ) : null}
            </span>
          ) : (
            <>
              {t.import_id ? null : (
                <span className="exp-source-tag"> efectivo</span>
              )}
              {t.recurring_rule_id ? (
                <span className="exp-source-tag exp-recurring-tag">
                  {" "}
                  <RefreshIcon /> recurrente
                </span>
              ) : null}
              {reconciled ? (
                <span
                  className="exp-source-tag exp-reconciled-tag"
                  title={reconciledTitle}
                >
                  {" "}
                  <LinkIcon /> conciliada
                </span>
              ) : null}
            </>
          )}
        </td>
        {isMobile ? null : (
          <td>
            <span className="exp-cat-edit">
              {kind !== "savings" && !t.category_id ? (
                <span className="exp-cat-dot" aria-hidden />
              ) : null}
              <select
                className="exp-inline-select"
                value={t.category_id ?? ""}
                disabled={!canEdit || kind === "savings"}
                aria-label="Categoría"
                onChange={(e) => onInlineCategory(t, e.target.value)}
              >
                <option value="">
                  {kind === "savings" ? "—" : "Sin categoría"}
                </option>
                {cats.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </span>
          </td>
        )}
        {isMobile ? null : (
          <td>
            <select
              className={`exp-inline-select exp-kind-select ${
                kind === "savings" ? "exp-kind-select--savings" : ""
              }`}
              value={kind}
              disabled={!canEdit}
              aria-label="Tipo"
              onChange={(e) =>
                onInlineKind(t, e.target.value as TransactionKindApi)
              }
            >
              {TRANSACTION_KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL_ES[k]}
                </option>
              ))}
            </select>
          </td>
        )}
        <td className={`num ${amountClass}`}>
          {formatCurrencyAmount(t.amount, currencyIso)}
          {rowTappable ? (
            <span className="row-chevron" aria-hidden>
              ›
            </span>
          ) : null}
        </td>
        {isMobile ? null : (
          <td className="exp-link-col">
            {hasLink ? (
              <span className="exp-link-indicator" title="Con vínculo">
                <LinkIcon />
              </span>
            ) : null}
          </td>
        )}
        {!isMobile && canEdit ? (
          <td className="asset-actions-cell">
            <div className="budget-row-actions">
              <button
                type="button"
                className="btn ghost icon-btn"
                aria-label="Editar movimiento"
                onClick={() => {
                  setRowError(null);
                  setEditTarget(t);
                }}
              >
                <RowEditIcon />
              </button>
              <button
                type="button"
                className="btn ghost danger icon-btn"
                aria-label="Eliminar movimiento"
                onClick={() => {
                  setRowError(null);
                  setDeleteTarget(t);
                }}
              >
                <RowTrashIcon />
              </button>
            </div>
          </td>
        ) : null}
      </tr>
    );
  }

  // ---- Render ------------------------------------------------------------

  return (
    <div className="workspace expenses-page">
      <div className="workspace-header">
        <h2 className="workspace-title">Movimientos</h2>
        <p className="workspace-sub">
          {bootstrapLoading
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : selectedMonth
                ? monthLabelEs(selectedMonth)
                : "Histórico de gasto mensual"}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · solo tus movimientos
        </div>
      ) : null}

      {!hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership && noCategories && !bootstrapLoading ? (
        <div className="banner info-banner">
          Crea categorías de <strong>Ingreso/Gasto</strong> en{" "}
          <strong>Ajustes → Categorías</strong> para clasificar tus movimientos.
        </div>
      ) : null}

      {error ? <div className="banner error-banner">{error}</div> : null}
      {rowError ? <div className="banner error-banner">{rowError}</div> : null}
      {notice ? <div className="banner info-banner tight-banner">{notice}</div> : null}

      {hasMembership ? (
        <>
          {/* KPIs */}
          <div
            className="metric-grid workspace-kpi-strip"
            aria-label="Resumen del mes"
          >
            <MetricCard
              label={`Gasto promedio (${avgLabel})`}
              helpId="expenses.expense_avg"
              detail={avgBasisText}
              value={
                totals && hasAvg
                  ? formatCurrencyAmount(totals.expense_avg, currencyIso)
                  : METRIC_DASH
              }
              trend={
                totals
                  ? renderKpiTrend(totals.expense_avg, totals.expense_budget, "expense")
                  : undefined
              }
            />
            <MetricCard
              label={`Ingreso promedio (${avgLabel})`}
              helpId="expenses.income_avg"
              detail={avgBasisText}
              value={
                totals && hasAvg
                  ? formatCurrencyAmount(totals.income_avg, currencyIso)
                  : METRIC_DASH
              }
              trend={
                totals
                  ? renderKpiTrend(totals.income_avg, totals.income_budget, "income")
                  : undefined
              }
            />
            <MetricCard
              label={`Traspasado a ahorro (${avgLabel})`}
              helpId="expenses.savings_transferred"
              detail={avgBasisText}
              value={
                totals && hasAvg
                  ? formatCurrencyAmount(totals.savings_avg, currencyIso)
                  : METRIC_DASH
              }
              tone="accent-2"
            />
            <MetricCard
              label={`% traspasado (${avgLabel})`}
              helpId="expenses.transferred_rate"
              detail={avgBasisText}
              value={
                savingsRateAvg !== null
                  ? formatPercentDisplay(savingsRateAvg)
                  : METRIC_DASH
              }
              tone="accent"
            />
          </div>

          {/* Toolbar */}
          <div className="category-toolbar bordered-top expenses-toolbar">
            <div className="expenses-month-nav">
              <button
                type="button"
                className="btn ghost icon-btn"
                aria-label="Mes anterior"
                disabled={
                  !selectedMonth ||
                  adjacentMonthInList(
                    months.map((m) => m.month),
                    selectedMonth,
                    "older",
                  ) === null
                }
                onClick={() => {
                  if (!selectedMonth) return;
                  const prev = adjacentMonthInList(
                    months.map((m) => m.month),
                    selectedMonth,
                    "older",
                  );
                  if (prev) setSelectedMonth(prev);
                }}
              >
                <ChevronLeftIcon />
              </button>
              <label className="field inline-role">
                <span className="sr-only">Mes</span>
                <select
                  value={selectedMonth ?? ""}
                  disabled={months.length === 0}
                  onChange={(e) => setSelectedMonth(e.target.value || null)}
                >
                  {months.length === 0 ? <option value="">Sin meses</option> : null}
                  {months.map((m) => (
                    <option key={m.month} value={m.month}>
                      {monthLabelEs(m.month)}
                      {m.is_complete ? "" : " · parcial"}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                className="btn ghost icon-btn"
                aria-label="Mes siguiente"
                disabled={
                  !selectedMonth ||
                  adjacentMonthInList(
                    months.map((m) => m.month),
                    selectedMonth,
                    "newer",
                  ) === null
                }
                onClick={() => {
                  if (!selectedMonth) return;
                  const next = adjacentMonthInList(
                    months.map((m) => m.month),
                    selectedMonth,
                    "newer",
                  );
                  if (next) setSelectedMonth(next);
                }}
              >
                <ChevronIcon />
              </button>
              {monthBadgePartial ? <span className="chip exp-partial-chip">parcial</span> : null}
            </div>

            <div className="expenses-window-pills" role="group" aria-label="Ventana del promedio">
              {AVG_WINDOWS.map((w) => (
                <button
                  key={w.id}
                  type="button"
                  className={`ff-nav-pill ${avgWindow === w.id ? "is-active" : ""}`}
                  aria-current={avgWindow === w.id ? "true" : undefined}
                  onClick={() => setAvgWindow(w.id)}
                >
                  {w.pill}
                </button>
              ))}
            </div>

            {canEdit ? (
              <div className="expenses-toolbar-actions">
                <button
                  type="button"
                  className="btn primary expenses-action-btn"
                  onClick={() => setImportOpen(true)}
                >
                  <UploadIcon />
                  Importar CSV
                </button>
                <button
                  type="button"
                  className="btn ghost expenses-action-btn"
                  onClick={() => setManualOpen(true)}
                >
                  <PlusIcon />
                  Añadir efectivo
                </button>
                <button
                  type="button"
                  className="btn ghost expenses-action-btn"
                  onClick={() => setRecurringOpen(true)}
                >
                  <RefreshIcon />
                  Recurrentes
                </button>
              </div>
            ) : null}
          </div>

          {/* First-use empty state */}
          {!bootstrapLoading && months.length === 0 ? (
            <section className="panel">
              <h3 className="panel-title">Sin movimientos</h3>
              {canEdit ? (
                <p className="muted bordered-top">
                  Importa el CSV de tu banco o añade efectivo a mano para empezar a
                  ver tu histórico de gasto.
                </p>
              ) : (
                <p className="muted bordered-top">Sin datos.</p>
              )}
            </section>
          ) : (
            <>
              {/* Comparativa */}
              <section className="panel">
                <h3 className="panel-title">Comparativa</h3>
                {monthLoading ? (
                  <p className="muted bordered-top">Cargando…</p>
                ) : !summary ? (
                  <p className="muted bordered-top">Sin datos.</p>
                ) : (
                  <>
                    {isPartial ? (
                      <p className="muted tight exp-partial-note">
                        Mes en curso: las comparativas son provisionales.
                      </p>
                    ) : null}
                    <div className="exp-comparison-grid">
                      <div className="exp-comparison-col">
                        <h4 className="subsection-title">Gastos</h4>
                        {renderComparisonTable(
                          summary.expense_categories,
                          "expense",
                          {
                            actual: summary.totals.expense_actual,
                            budget: summary.totals.expense_budget,
                            avg: summary.totals.expense_avg,
                          },
                          threshold,
                          hasAvg,
                        )}
                      </div>
                      <div className="exp-comparison-col">
                        <h4 className="subsection-title">Ingresos</h4>
                        {renderComparisonTable(
                          summary.income_categories,
                          "income",
                          {
                            actual: summary.totals.income_actual,
                            budget: summary.totals.income_budget,
                            avg: summary.totals.income_avg,
                          },
                          threshold,
                          hasAvg,
                        )}
                      </div>
                    </div>
                  </>
                )}
              </section>

              {/* Cash-flow mensual */}
              <section className="panel">
                <h3 className="panel-title">Cash-flow mensual</h3>
                {cashflow && cashflow.months.length > 0 ? (
                  <MonthlyCashflowBars
                    // Móvil: solo los últimos 12 meses. 24 columnas a ~13px son
                    // ilegibles en phone; 12 ≈ 27px/columna y sin scroll-X (regla
                    // de oro). Escritorio conserva la ventana completa.
                    months={
                      isMobile
                        ? [...cashflow.months]
                            .sort((a, b) => a.month_index - b.month_index)
                            .slice(-12)
                        : cashflow.months
                    }
                    currencyIso={currencyIso}
                  />
                ) : (
                  <p className="muted bordered-top">Sin datos.</p>
                )}
              </section>

              {/* Movimientos */}
              <section className="panel">
                <h3 className="panel-title">Movimientos</h3>
                {monthLoading ? (
                  <p className="muted bordered-top">Cargando…</p>
                ) : transactions.length === 0 ? (
                  <p className="muted bordered-top">Sin movimientos este mes.</p>
                ) : (
                  <>
                    {/* Controles: búsqueda + agrupación (estado local, en vivo) */}
                    <div className="exp-movements-controls bordered-top">
                      <label className="field exp-search-field">
                        <span className="sr-only">Buscar movimientos</span>
                        <input
                          type="search"
                          value={movementQuery}
                          placeholder="Buscar…"
                          aria-label="Buscar movimientos"
                          onChange={(e) => setMovementQuery(e.target.value)}
                        />
                      </label>
                      <div
                        className="exp-movements-group-toggle"
                        role="group"
                        aria-label="Agrupación"
                      >
                        <button
                          type="button"
                          className={`ff-nav-pill ${grouped ? "is-active" : ""}`}
                          aria-pressed={grouped}
                          onClick={() => setGrouped((g) => !g)}
                        >
                          Por categoría
                        </button>
                      </div>
                    </div>
                    {visibleRows.length === 0 ? (
                      <p className="muted exp-movements-empty">Sin resultados.</p>
                    ) : (
                      <div className="table-scroll">
                        <table className="assets-table exp-movements-table">
                          <thead>
                            <tr>
                              {renderSortHeader("Fecha", "date")}
                              {renderSortHeader("Concepto", "concept")}
                              {isMobile ? null : <th>Categoría</th>}
                              {isMobile ? null : <th>Tipo</th>}
                              {renderSortHeader("Importe", "amount", true)}
                              {isMobile ? null : (
                                <th className="exp-link-col">
                                  <span className="sr-only">Vínculo</span>
                                </th>
                              )}
                              {!isMobile && canEdit ? (
                                <th className="asset-actions-cell">
                                  <span className="sr-only">Acciones</span>
                                </th>
                              ) : null}
                            </tr>
                          </thead>
                          <tbody>
                            {grouped
                              ? movementGroups.map((g) => (
                                  <Fragment key={g.key}>
                                    <tr className="exp-group-row">
                                      <td colSpan={movementCols}>
                                        <div className="exp-group-head">
                                          <span className="exp-group-name">
                                            {g.label}{" "}
                                            <span className="exp-group-count">
                                              ({g.rows.length})
                                            </span>
                                          </span>
                                          <span
                                            className={`num ${
                                              g.subtotal < 0
                                                ? "num-neg"
                                                : g.subtotal > 0
                                                  ? "num-pos"
                                                  : ""
                                            }`}
                                          >
                                            {formatCurrencyAmount(
                                              String(g.subtotal),
                                              currencyIso,
                                            )}
                                          </span>
                                        </div>
                                      </td>
                                    </tr>
                                    {g.rows.map(renderMovementRow)}
                                  </Fragment>
                                ))
                              : sortedRows.map(renderMovementRow)}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </>
                )}
              </section>
            </>
          )}
        </>
      ) : null}

      {/* Modales */}
      {canEdit ? (
        <>
          <ImportWizardModal
            open={importOpen}
            onClose={() => setImportOpen(false)}
            incomeCategories={incomeCategories}
            expenseCategories={expenseCategories}
            assets={assets}
            liabilities={liabilities}
            currencyIso={currencyIso}
            onImported={(res) => {
              // El aviso post-confirm vive aquí (el wizard se cierra al confirmar): añade los
              // pares auto-conciliados solo cuando los hay.
              const parts = [
                `${res.imported} importados`,
                `${res.skipped_already_imported + res.discarded} excluidos`,
              ];
              if (res.reconciled_pairs > 0) {
                parts.push(
                  `${res.reconciled_pairs} par${res.reconciled_pairs === 1 ? "" : "es"} conciliado${
                    res.reconciled_pairs === 1 ? "" : "s"
                  } automáticamente`,
                );
              }
              setNotice(parts.join(" · "));
              void handleMutated();
            }}
          />
          <ManualCashEntryModal
            open={manualOpen}
            onClose={() => setManualOpen(false)}
            incomeCategories={incomeCategories}
            expenseCategories={expenseCategories}
            assets={assets}
            liabilities={liabilities}
            defaultDate={today}
            onSaved={(count) => {
              setNotice(`${count} movimiento${count === 1 ? "" : "s"} añadido${count === 1 ? "" : "s"}.`);
              void handleMutated();
            }}
          />
          <RecurringRulesModal
            open={recurringOpen}
            onClose={() => setRecurringOpen(false)}
            currencyIso={currencyIso}
            onChanged={() => void handleMutated()}
          />
          <EditTransactionModal
            today={today}
            target={editTarget}
            onClose={() => setEditTarget(null)}
            isMobile={isMobile}
            onRequestDelete={(t) => {
              setEditTarget(null);
              setRowError(null);
              setDeleteTarget(t);
            }}
            incomeCategories={incomeCategories}
            expenseCategories={expenseCategories}
            assets={assets}
            liabilities={liabilities}
            categoryFitsKind={categoryFitsKind}
            onSaved={(updated) => {
              setTransactions((ts) => ts.map((t) => (t.id === updated.id ? updated : t)));
              setEditTarget(null);
              setNotice("Movimiento actualizado.");
              refreshDerived();
            }}
            onUnreconciled={() => {
              setEditTarget(null);
              setNotice("Movimiento desconciliado.");
              void handleMutated();
            }}
          />
          <Modal
            title="Eliminar movimiento"
            open={deleteTarget !== null}
            onClose={() => setDeleteTarget(null)}
          >
            {deleteTarget ? (
              <div className="stack">
                <p className="muted tight">
                  ¿Eliminar <strong>{deleteTarget.concept}</strong> del{" "}
                  <strong>{formatDateDmy(deleteTarget.op_date)}</strong>? Esta acción
                  no se puede deshacer.
                </p>
                {deleteTarget.recurring_rule_id ? (
                  <>
                    <p className="muted tight">Este movimiento es recurrente.</p>
                    <div className="asset-form-actions">
                      <button
                        type="button"
                        className="btn ghost danger"
                        disabled={deletingRecurring}
                        onClick={() => void confirmDelete()}
                      >
                        Eliminar solo este
                      </button>
                      <button
                        type="button"
                        className="btn ghost danger"
                        disabled={deletingRecurring}
                        onClick={() => void confirmDeleteAndStopRecurring()}
                      >
                        Eliminar y detener repetición
                      </button>
                      <button
                        type="button"
                        className="btn ghost"
                        disabled={deletingRecurring}
                        onClick={() => setDeleteTarget(null)}
                      >
                        Cancelar
                      </button>
                    </div>
                  </>
                ) : (
                  <div className="asset-form-actions">
                    <button
                      type="button"
                      className="btn ghost danger"
                      onClick={() => void confirmDelete()}
                    >
                      Eliminar
                    </button>
                    <button
                      type="button"
                      className="btn ghost"
                      onClick={() => setDeleteTarget(null)}
                    >
                      Cancelar
                    </button>
                  </div>
                )}
              </div>
            ) : null}
          </Modal>
        </>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Modal de edición completa (vínculos + notas; en manuales también fecha/concepto/importe)
// ---------------------------------------------------------------------------

function EditTransactionModal({
  target,
  onClose,
  isMobile,
  onRequestDelete,
  incomeCategories,
  expenseCategories,
  assets,
  liabilities,
  categoryFitsKind,
  onSaved,
  onUnreconciled,
  today,
}: {
  target: TransactionApi | null;
  onClose: () => void;
  /** En móvil desaparece la columna Acciones → el borrado se ofrece aquí. */
  isMobile: boolean;
  /** Cierra este modal y abre el de confirmación de borrado (deleteTarget). */
  onRequestDelete: (t: TransactionApi) => void;
  incomeCategories: CategoryRow[];
  expenseCategories: CategoryRow[];
  assets: AssetApiRow[];
  liabilities: LiabilityApiRow[];
  categoryFitsKind: (categoryId: string, kind: TransactionKindApi) => boolean;
  onSaved: (updated: TransactionApi) => void;
  /** Par roto: la vista cierra el modal y recarga (ambas patas vuelven a los totales). */
  onUnreconciled: (pair: ReconcilePairResponseApi | null) => void;
  /** Hoy en la zona horaria del hogar: tope del selector de fecha. El modal de ALTA ya lo tenía
   *  y éste no, así que se podía mover un movimiento al futuro editándolo. La API lo rechaza
   *  desde 4.0.0 (`op_date_in_future`); el tope evita el viaje de ida y vuelta. */
  today: string;
}) {
  const [opDate, setOpDate] = useState("");
  const [valueDate, setValueDate] = useState("");
  const [concept, setConcept] = useState("");
  const [amount, setAmount] = useState("");
  const [kind, setKind] = useState<TransactionKindApi>("expense");
  const [categoryId, setCategoryId] = useState("");
  const [linkedAssetId, setLinkedAssetId] = useState("");
  const [linkedLiabilityId, setLinkedLiabilityId] = useState("");
  const [notes, setNotes] = useState("");
  const [saving, setSaving] = useState(false);
  const [unreconciling, setUnreconciling] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (!target) return;
    setOpDate(target.op_date);
    setValueDate(target.value_date ?? "");
    setConcept(target.concept);
    setAmount(target.amount);
    setKind(target.kind ?? "expense");
    setCategoryId(target.category_id ?? "");
    setLinkedAssetId(target.linked_asset_id ?? "");
    setLinkedLiabilityId(target.linked_liability_id ?? "");
    setNotes(target.notes ?? "");
    setFormError(null);
    setSaving(false);
    setUnreconciling(false);
  }, [target]);

  if (!target) {
    return <Modal title="Editar movimiento" open={false} onClose={onClose} children={null} />;
  }

  const isManual = !target.import_id;
  const reconciled = isReconciled(target);
  const cats = categoriesForKind(kind, incomeCategories, expenseCategories);

  /** Rompe el par (`DELETE /reconcile`): ambas patas vuelven a contar en los totales y el pase
   *  automático no las re-empareja (el backend persiste el rechazo). */
  async function unreconcile() {
    if (!target) return;
    setFormError(null);
    setUnreconciling(true);
    try {
      const pair = await apiDeleteJson<ReconcilePairResponseApi>(
        `/v1/transactions/${target.id}/reconcile`,
      );
      onUnreconciled(pair);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "No se pudo desconciliar.");
    } finally {
      setUnreconciling(false);
    }
  }

  function changeKind(newKind: TransactionKindApi) {
    setKind(newKind);
    if (newKind === "savings" || (categoryId && !categoryFitsKind(categoryId, newKind))) {
      setCategoryId("");
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!target) return;
    setFormError(null);
    const patch: PatchTransactionRequest = { kind };
    // Fecha/concepto/importe son editables también en importadas: el backend conserva la huella
    // original del CSV, así que reubicar una nómina o corregir un importe no rompe el dedup.
    if (!opDate.trim()) {
      setFormError("Indica una fecha.");
      return;
    }
    if (!concept.trim()) {
      setFormError("Indica un concepto.");
      return;
    }
    const parsed = parseDisplayDecimal(amount);
    if (parsed === null || parsed === 0) {
      setFormError("Indica un importe distinto de 0.");
      return;
    }
    patch.op_date = opDate;
    patch.concept = concept.trim();
    patch.amount = toApiDecimalString(amount);
    if (kind === "savings" || !categoryId) patch.clear_category = true;
    else patch.category_id = categoryId;
    if (valueDate.trim()) patch.value_date = valueDate;
    else patch.clear_value_date = true;
    if (linkedAssetId) patch.linked_asset_id = linkedAssetId;
    else patch.clear_linked_asset = true;
    if (linkedLiabilityId) patch.linked_liability_id = linkedLiabilityId;
    else patch.clear_linked_liability = true;
    if (notes.trim()) patch.notes = notes.trim();
    else patch.clear_notes = true;

    setSaving(true);
    try {
      const updated = await apiPatch<TransactionApi>(
        `/v1/transactions/${target.id}`,
        patch,
      );
      if (updated) onSaved(updated);
      else onClose();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "No se pudo guardar.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal title="Editar movimiento" open={target !== null} onClose={onClose} wide>
      <form className="asset-form stack" onSubmit={submit}>
        <ModalFormError message={formError} />
        {!isManual ? (
          <p className="muted tight">
            Movimiento importado: editarlo no afecta a la detección de duplicados
            del CSV (la huella original se conserva).
          </p>
        ) : null}
        {reconciled ? (
          <div className="banner info-banner tight-banner exp-reconciled-note">
            <span>
              Conciliada con «{target.transfer_counterpart_concept ?? "—"}» (
              {target.transfer_counterpart_op_date
                ? formatDateDmy(target.transfer_counterpart_op_date)
                : "—"}
              ). Fuera de los totales.
            </span>
            <button
              type="button"
              className="btn ghost text"
              disabled={saving || unreconciling}
              onClick={() => void unreconcile()}
            >
              {unreconciling ? "Desconciliando…" : "Desconciliar"}
            </button>
          </div>
        ) : null}
        <div className="asset-form-grid">
          <label className="field">
            <span>Fecha</span>
            <input
              type="date"
              value={opDate}
              max={today}
              onChange={(e) => setOpDate(e.target.value)}
            />
          </label>
          <label className="field">
            <span>Fecha valor (opc.)</span>
            <input
              type="date"
              value={valueDate}
              onChange={(e) => setValueDate(e.target.value)}
            />
          </label>
          <label className="field">
            <span>Importe</span>
            <input
              value={amount}
              inputMode="decimal"
              autoComplete="off"
              onChange={(e) => setAmount(e.target.value)}
            />
          </label>
        </div>
        <label className="field">
          <span>Concepto</span>
          <input
            value={concept}
            maxLength={500}
            autoComplete="off"
            onChange={(e) => setConcept(e.target.value)}
          />
        </label>
        <div className="asset-form-grid">
          <label className="field">
            <span>Tipo</span>
            <select
              value={kind}
              onChange={(e) => changeKind(e.target.value as TransactionKindApi)}
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
              value={categoryId}
              disabled={kind === "savings"}
              onChange={(e) => setCategoryId(e.target.value)}
            >
              <option value="">{kind === "savings" ? "—" : "Sin categoría"}</option>
              {cats.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="asset-form-grid">
          <label className="field">
            <span>Activo vinculado (opc.)</span>
            <select
              value={linkedAssetId}
              onChange={(e) => setLinkedAssetId(e.target.value)}
            >
              <option value="">— Ninguno —</option>
              {assets.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Pasivo vinculado (opc.)</span>
            <select
              value={linkedLiabilityId}
              onChange={(e) => setLinkedLiabilityId(e.target.value)}
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
        <label className="field">
          <span>Notas (opc.)</span>
          <textarea
            value={notes}
            rows={2}
            maxLength={4000}
            onChange={(e) => setNotes(e.target.value)}
          />
        </label>
        <div className="asset-form-actions">
          <button type="submit" className="btn primary" disabled={saving}>
            {saving ? "Guardando…" : "Guardar cambios"}
          </button>
          <button type="button" className="btn ghost" disabled={saving} onClick={onClose}>
            Cancelar
          </button>
        </div>
        {isMobile ? (
          <div className="modal-mobile-delete-row">
            <button
              type="button"
              className="btn ghost danger"
              disabled={saving}
              onClick={() => onRequestDelete(target)}
            >
              Eliminar movimiento
            </button>
          </div>
        ) : null}
      </form>
    </Modal>
  );
}
