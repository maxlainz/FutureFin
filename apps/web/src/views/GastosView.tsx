/**
 * Pestaña «Gastos» — histórico de gasto mensual (v1.6.0). Vista AUTÓNOMA (patrón
 * HistorySettingsPanel): hace sus propios fetch al montar y en cada cambio de scope/mes/ventana;
 * recibe props finas de App. Sus mutaciones nunca tocan la cache de proyección (las transacciones
 * no son inputs del engine); `onCashflowMutated` avisa a App para refrescar el overlay del chart.
 *
 * Estructura: KPIs → toolbar (mes + ventana + acciones) → Comparativa (tabla + barras + cash-flow)
 * → Movimientos (tabla con edición inline optimista + modal de edición completa + borrado).
 */

import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { apiDelete, apiGet, apiPatch } from "../api/client";
import type {
  AssetApiRow,
  CategoryComparisonLineApi,
  CategoryRow,
  HistoryCashflowApi,
  InstallationAccess,
  LiabilityApiRow,
  PatchTransactionRequest,
  TransactionApi,
  TransactionKindApi,
  TransactionMonthApi,
  TransactionsSummaryApi,
  UserResponse,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import {
  ChevronIcon,
  ChevronLeftIcon,
  LinkIcon,
  PlusIcon,
  RowEditIcon,
  RowTrashIcon,
  UploadIcon,
} from "../components/icons";
import {
  CategoryComparisonBars,
  MonthlyCashflowBars,
  type ComparisonBarRow,
} from "../components/charts/CategoryComparisonBars";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyOrDash,
  formatPercentDisplay,
  parseDisplayDecimal,
} from "../lib/format";
import { formatDateDm, formatDateDmy, todayYmdInTimeZone } from "../lib/dates";
import { ledgerViewQs, type LedgerPersonScope } from "../lib/ledger";
import { useIsMobile } from "../lib/responsive";
import {
  KIND_LABEL_ES,
  TRANSACTION_KINDS,
  adjacentMonthInList,
  categoriesForKind,
  defaultSelectedMonth,
  deltaToneClass,
  formatDeltaCurrency,
  monthLabelEs,
  parseMonth,
} from "../lib/expenses";
import { ImportWizardModal } from "./ImportWizardModal";
import { ManualCashEntryModal } from "./ManualCashEntryModal";

const AVG_WINDOWS = [3, 6, 12];

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
  const [avgMonths, setAvgMonths] = useState<number>(6);

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
  const [editTarget, setEditTarget] = useState<TransactionApi | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TransactionApi | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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
        `/v1/transactions/summary?year=${c.y}&month=${c.m}&avg_months=${avgMonths}${viewAmp}`,
      );
      setSummary(sum);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudo cargar la comparativa.");
      setSummary(null);
    }
  }, [selectedMonth, avgMonths, viewAmp]);

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

  // ---- Derived KPIs ------------------------------------------------------

  const totals = summary?.totals ?? null;
  const isPartial = summary?.is_partial ?? false;

  const savingsRate = useMemo(() => {
    if (!totals) return null;
    const inc = parseDisplayDecimal(totals.income_actual) ?? 0;
    const sav = parseDisplayDecimal(totals.savings_actual) ?? 0;
    if (inc <= 0) return null;
    return (sav / inc) * 100;
  }, [totals]);

  const savingsRateAvg = useMemo(() => {
    if (!totals) return null;
    const inc = parseDisplayDecimal(totals.income_avg) ?? 0;
    const sav = parseDisplayDecimal(totals.savings_avg) ?? 0;
    if (inc <= 0) return null;
    return (sav / inc) * 100;
  }, [totals]);

  const comparisonBarRows: ComparisonBarRow[] = useMemo(() => {
    if (!summary) return [];
    return summary.expense_categories
      .map((l) => ({
        key: l.category_id ?? "uncategorized",
        label: l.category_name,
        actual: parseDisplayDecimal(l.actual) ?? 0,
        budget: parseDisplayDecimal(l.budget) ?? 0,
        avg: parseDisplayDecimal(l.avg) ?? 0,
      }))
      .filter((r) => r.actual > 0 || r.budget > 0 || r.avg > 0);
  }, [summary]);

  const noCategories =
    incomeCategories.length === 0 && expenseCategories.length === 0;

  // ---- Render helpers ----------------------------------------------------

  const monthBadgePartial =
    selectedMonth !== null &&
    months.find((m) => m.month === selectedMonth)?.is_complete === false;

  function renderComparisonTable(
    lines: CategoryComparisonLineApi[],
    kind: "expense" | "income",
    includeDerived: boolean,
  ) {
    if (lines.length === 0 && !includeDerived) {
      return <p className="muted bordered-top">Sin datos.</p>;
    }
    return (
      <div className="table-scroll bordered-top">
        <table className="assets-table exp-comparison-table">
          <thead>
            <tr>
              <th>Categoría</th>
              <th className="num">Real</th>
              {isMobile ? null : <th className="num">Budget</th>}
              <th className="num">Δ</th>
              {isMobile ? null : <th className="num">Promedio {avgMonths}m</th>}
            </tr>
          </thead>
          <tbody>
            {lines.map((l) => {
              const dNum = parseDisplayDecimal(l.delta_vs_budget) ?? 0;
              const toneClass = isPartial ? "muted" : deltaToneClass(dNum, kind);
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
                        Budget {formatCurrencyAmount(l.budget, currencyIso)} · Prom{" "}
                        {avgMonths}m {formatCurrencyAmount(l.avg, currencyIso)}
                      </span>
                    ) : null}
                  </td>
                  <td className="num">{formatCurrencyAmount(l.actual, currencyIso)}</td>
                  {isMobile ? null : (
                    <td className="num">{formatCurrencyAmount(l.budget, currencyIso)}</td>
                  )}
                  <td className={`num ${toneClass}`}>
                    {formatDeltaCurrency(dNum, currencyIso)}
                  </td>
                  {isMobile ? null : (
                    <td className="num">{formatCurrencyAmount(l.avg, currencyIso)}</td>
                  )}
                </tr>
              );
            })}
            {includeDerived && summary ? (
              <tr className="exp-derived-row">
                <td>
                  {summary.derived_debt_line.label}
                  {isMobile ? (
                    <span className="cell-subline">
                      Budget{" "}
                      {formatCurrencyAmount(
                        summary.derived_debt_line.budget,
                        currencyIso,
                      )}{" "}
                      · Prom {avgMonths}m {METRIC_DASH}
                    </span>
                  ) : null}
                </td>
                <td className="num">{METRIC_DASH}</td>
                {isMobile ? null : (
                  <td className="num">
                    {formatCurrencyAmount(summary.derived_debt_line.budget, currencyIso)}
                  </td>
                )}
                <td className="num">{METRIC_DASH}</td>
                {isMobile ? null : <td className="num">{METRIC_DASH}</td>}
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    );
  }

  // ---- Render ------------------------------------------------------------

  return (
    <div className="workspace expenses-page">
      <div className="workspace-header">
        <h2 className="workspace-title">Gastos</h2>
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
              label="Gastos"
              value={
                totals ? formatCurrencyOrDash(totals.expense_actual, currencyIso) : METRIC_DASH
              }
              parenthetical={
                totals ? `media ${formatCurrencyAmount(totals.expense_avg, currencyIso)}` : undefined
              }
            />
            <MetricCard
              label="Ingresos"
              value={
                totals ? formatCurrencyOrDash(totals.income_actual, currencyIso) : METRIC_DASH
              }
              parenthetical={
                totals ? `media ${formatCurrencyAmount(totals.income_avg, currencyIso)}` : undefined
              }
            />
            <MetricCard
              label="Ahorro/Inversión"
              value={
                totals ? formatCurrencyOrDash(totals.savings_actual, currencyIso) : METRIC_DASH
              }
              parenthetical={
                totals ? `media ${formatCurrencyAmount(totals.savings_avg, currencyIso)}` : undefined
              }
              tone="accent-2"
            />
            <MetricCard
              label="Tasa de ahorro"
              value={savingsRate !== null ? formatPercentDisplay(savingsRate) : METRIC_DASH}
              parenthetical={
                savingsRateAvg !== null
                  ? `media ${formatPercentDisplay(savingsRateAvg)}`
                  : "vs media"
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
                  key={w}
                  type="button"
                  className={`ff-nav-pill ${avgMonths === w ? "is-active" : ""}`}
                  aria-current={avgMonths === w ? "true" : undefined}
                  onClick={() => setAvgMonths(w)}
                >
                  {w}m
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
                          true,
                        )}
                      </div>
                      <div className="exp-comparison-col">
                        <h4 className="subsection-title">Ingresos</h4>
                        {renderComparisonTable(
                          summary.income_categories,
                          "income",
                          false,
                        )}
                      </div>
                    </div>
                    <CategoryComparisonBars
                      rows={comparisonBarRows}
                      currencyIso={currencyIso}
                      avgMonths={avgMonths}
                    />
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
                  <div className="table-scroll table-scroll--sticky bordered-top">
                    <table className="assets-table exp-movements-table">
                      <thead>
                        <tr>
                          <th>Fecha</th>
                          <th>Concepto</th>
                          {isMobile ? null : <th>Categoría</th>}
                          {isMobile ? null : <th>Tipo</th>}
                          <th className="num">Importe</th>
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
                        {transactions.map((t) => {
                          const kind = t.kind ?? "expense";
                          const amountNum = parseDisplayDecimal(t.amount) ?? 0;
                          const amountClass =
                            amountNum < 0 ? "num-neg" : amountNum > 0 ? "num-pos" : "";
                          const cats = categoriesForKind(
                            kind,
                            incomeCategories,
                            expenseCategories,
                          );
                          const hasLink = !!(t.linked_asset_id || t.linked_liability_id);
                          const rowTappable = isMobile && canEdit;
                          const openEdit = () => {
                            setRowError(null);
                            setEditTarget(t);
                          };
                          const categoryLabel =
                            t.category_name ??
                            (kind === "savings" ? "—" : "Sin categoría");
                          return (
                            <tr
                              key={t.id}
                              className={rowTappable ? "row-tappable" : undefined}
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
                              <td>
                                {isMobile
                                  ? formatDateDm(t.op_date)
                                  : formatDateDmy(t.op_date)}
                              </td>
                              <td className="exp-concept-cell">
                                {t.concept}
                                {isMobile ? (
                                  <span className="cell-subline">
                                    {categoryLabel} · {KIND_LABEL_ES[kind]}
                                    {t.import_id ? null : " · efectivo"}
                                    {hasLink ? (
                                      <>
                                        {" · "}
                                        <LinkIcon />
                                      </>
                                    ) : null}
                                  </span>
                                ) : t.import_id ? null : (
                                  <span className="exp-source-tag"> efectivo</span>
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
                        })}
                      </tbody>
                    </table>
                  </div>
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
              setNotice(
                `Import: ${res.imported} importados, ${res.skipped_already_imported} omitidos, ${res.discarded} descartados.`,
              );
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
          <EditTransactionModal
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
  }, [target]);

  if (!target) {
    return <Modal title="Editar movimiento" open={false} onClose={onClose} children={null} />;
  }

  const isManual = !target.import_id;
  const cats = categoriesForKind(kind, incomeCategories, expenseCategories);

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
    if (isManual) {
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
      patch.amount = amount.trim().replace(",", ".");
    }
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
            Movimiento importado: fecha, concepto e importe no se pueden editar.
          </p>
        ) : null}
        <div className="asset-form-grid">
          <label className="field">
            <span>Fecha</span>
            <input
              type="date"
              value={opDate}
              disabled={!isManual}
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
              disabled={!isManual}
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
            disabled={!isManual}
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
