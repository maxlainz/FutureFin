import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type {
  BudgetSnapshotApi,
  FireSettingsApi,
  InstallationAccess,
  ProjectionSeriesApi,
  SummaryResponse,
  UserResponse,
} from "../api/types";
import { HelpPopover } from "../components/HelpPopover";
import { HELP_TEXTS } from "../lib/helpTexts";
import { MetricCard } from "../components/MetricCard";
import {
  MiniProjection,
  MiniProjectionLegend,
} from "../components/charts/MiniProjection";
import {
  METRIC_DASH,
  formatCurrencyNumber,
  formatPercentAmount,
  parseDisplayDecimal,
} from "../lib/format";
import {
  computeFireAnnualNeedNetEur,
  defaultFireSettingsApi,
  findFirstMonthNetWorthAtLeastInflated,
  grossUpNetAnnualFire,
  normalizeInstallationFireSettings,
  savingsSourceUsesTransactions,
} from "../lib/fire";
import { type LedgerPersonScope } from "../lib/ledger";
import { TAB_PATH } from "../lib/navigation";
import {
  complementaryProjectionTickLabel,
  formatYearsEsFromMonths,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
} from "../lib/projection-chart";

export function RetirementView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  projectionSeries,
  projectionBusy,
  retirementBudgetSnapshot,
  summary,
  retirementBusy,
  retirementError,
  user,
  calendarTz,
  canEditFire,
  onSaveFire,
  navigate,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  projectionBusy: boolean;
  retirementBudgetSnapshot: BudgetSnapshotApi | null;
  /** Solo se consume en modo B (promedio): los equivalentes efectivos del ahorro real. */
  summary: SummaryResponse | null;
  retirementBusy: boolean;
  retirementError: string | null;
  user: UserResponse | null;
  calendarTz: string;
  canEditFire: boolean;
  onSaveFire: (fs: FireSettingsApi) => Promise<void>;
  navigate: (path: string, replace?: boolean) => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const [fireDraft, setFireDraft] = useState<FireSettingsApi>(() =>
    defaultFireSettingsApi(),
  );
  const lastSavedFirePayloadRef = useRef<string>("");
  const fireSaveTimerRef = useRef(0);
  const fireSaveSeqRef = useRef(0);

  useEffect(() => {
    setFireDraft(
      normalizeInstallationFireSettings(
        installation?.installation.fire_settings,
      ),
    );
    const serverFs = normalizeInstallationFireSettings(
      installation?.installation.fire_settings,
    );
    lastSavedFirePayloadRef.current = JSON.stringify(serverFs);
    // Re-inicializa el draft solo al cambiar de instalación; NO en cada cambio de
    // fire_settings, que clobbearía ediciones en curso (este draft autosalva).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installation?.installation.id]);

  const axisAgeMode = projectionSeries
    ? resolveProjectionAxisAgeMode(projectionSeries, installation)
    : "dates";
  const axisBirth = (() => {
    const fromApi = projectionSeries?.viewer_birth_date?.trim();
    const fromUser = user?.birth_date?.trim();
    const pick =
      fromApi && fromApi.length > 0
        ? fromApi
        : fromUser && fromUser.length > 0
          ? fromUser
          : null;
    return pick;
  })();
  const axisAnchor = projectionSeries?.anchor_date_ymd?.trim() || null;

  const retirementMetricsReady =
    hasMembership &&
    !projectionBusy &&
    !retirementBusy &&
    projectionSeries != null &&
    retirementBudgetSnapshot != null;

  const installationInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    return n != null && n > 0 ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  // Fuente EFECTIVA del ahorro (tras el fallback del servidor). En los modos con promedio
  // (B `transactions_avg` y C `budget_income_real_expense`) el preview del target NO recalcula
  // el promedio en cliente: consume los equivalentes ya calculados por el servidor en
  // /v1/summary (expense_regular = expense_avg_sin_cuotas; income = presupuesto en C / promedio
  // en B), de modo que el preview coincide con el target del servidor. `income_retirement`
  // sigue del presupuesto (la fase post-jubilación no cambia de base). Si el servidor cayó a
  // `budget` (0 meses con datos) o el summary aún no está cargado, se usan los equivalentes del
  // presupuesto (idéntico a modo A).
  const savingsAvgActive = savingsSourceUsesTransactions(
    summary?.financial_health.savings_source,
  );
  const fireExpenseM = savingsAvgActive
    ? summary?.financial_health.expense_regular_monthly_equivalent
    : retirementBudgetSnapshot?.totals.expense_retirement_monthly_equivalent;
  const fireIncomeM = savingsAvgActive
    ? summary?.financial_health.income_monthly_equivalent
    : retirementBudgetSnapshot?.totals.income_monthly_equivalent;

  const fireKpis = useMemo(() => {
    const expenseM = fireExpenseM;
    const incomeM = fireIncomeM;
    const incomeRetM =
      retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent;
    const needAnnual = computeFireAnnualNeedNetEur(
      fireDraft,
      expenseM,
      incomeM,
      incomeRetM,
    );
    const swrN = parseDisplayDecimal(fireDraft.swr_pct);
    const brackets = fireDraft.tax_brackets;
    const taxOn = fireDraft.taxes_enabled;

    let targetNoPen: number | null = null;
    let grossNoPen: number | null = null;

    if (needAnnual !== null && needAnnual > 0 && swrN !== null && swrN > 0) {
      grossNoPen = grossUpNetAnnualFire(needAnnual, brackets, taxOn);
      targetNoPen = grossNoPen / (swrN / 100);
    }

    const pts = projectionSeries?.points ?? [];
    const mc = projectionSeries?.months ?? 0;

    let miNo: number | null = null;
    if (targetNoPen !== null && targetNoPen > 0) {
      miNo = findFirstMonthNetWorthAtLeastInflated(
        pts,
        targetNoPen,
        installationInflationPct,
      );
    }

    let targetAtCross: number | null = null;
    if (
      targetNoPen !== null &&
      targetNoPen > 0 &&
      miNo !== null &&
      installationInflationPct > 0
    ) {
      targetAtCross =
        targetNoPen * Math.pow(1 + installationInflationPct / 100, miNo / 12);
    }

    return {
      needAnnual,
      swrN,
      targetNoPen,
      targetAtCross,
      miNo,
      mc,
    };
  }, [
    fireDraft,
    installationInflationPct,
    fireExpenseM,
    fireIncomeM,
    retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent,
    projectionSeries?.points,
    projectionSeries?.months,
  ]);

  const renderRetirementAmount = useCallback(
    (annual: number, monthly: number): ReactNode => (
      <>
        {formatCurrencyNumber(annual, currencyIso)}{" "}
        <span className="retirement-mode-monthly">
          ({formatCurrencyNumber(monthly, currencyIso)}/mes)
        </span>
      </>
    ),
    [currencyIso],
  );

  const retirementObjectiveManualAnnualDisplay = useMemo<ReactNode>(() => {
    const m = parseDisplayDecimal(
      String(fireDraft.fire_number_manual_amount ?? ""),
    );
    if (!(m !== null && m > 0)) return METRIC_DASH;
    return renderRetirementAmount(m, m / 12);
  }, [fireDraft.fire_number_manual_amount, renderRetirementAmount]);

  const retirementObjectiveExpenseAnnualDisplay = useMemo<ReactNode>(() => {
    const baseM = parseDisplayDecimal(String(fireExpenseM ?? ""));
    if (!(baseM !== null && baseM >= 0)) return METRIC_DASH;
    return renderRetirementAmount(baseM * 12, baseM);
  }, [fireExpenseM, renderRetirementAmount]);

  const retirementObjectiveIncomeAnnualDisplay = useMemo<ReactNode>(() => {
    const incM = parseDisplayDecimal(String(fireIncomeM ?? ""));
    if (!(incM !== null && incM >= 0)) return METRIC_DASH;
    return renderRetirementAmount(incM * 12, incM);
  }, [fireIncomeM, renderRetirementAmount]);

  const skipFireAutosaveRef = useRef(true);

  useEffect(() => {
    skipFireAutosaveRef.current = true;
  }, [installation?.installation.id]);

  const runFireSave = useCallback(() => {
    if (!hasMembership || !canEditFire) return;
    const swrN = parseDisplayDecimal(fireDraft.swr_pct);
    if (swrN === null || swrN < 0 || swrN > 4) {
      return;
    }
    if (
      fireDraft.fire_number_mode === "manual" &&
      (fireDraft.fire_number_manual_amount == null ||
        String(fireDraft.fire_number_manual_amount).trim() === "")
    ) {
      return;
    }
    const payloadJson = JSON.stringify(fireDraft);
    if (payloadJson === lastSavedFirePayloadRef.current) return;
    const seq = ++fireSaveSeqRef.current;
    void onSaveFire(fireDraft)
      .then(() => {
        if (seq !== fireSaveSeqRef.current) return;
        lastSavedFirePayloadRef.current = payloadJson;
      })
      .catch(() => {});
  }, [fireDraft, hasMembership, canEditFire, onSaveFire]);

  const queueFireSave = useCallback(
    (delayMs: number) => {
      window.clearTimeout(fireSaveTimerRef.current);
      fireSaveTimerRef.current = window.setTimeout(() => {
        fireSaveTimerRef.current = 0;
        runFireSave();
      }, delayMs);
    },
    [runFireSave],
  );

  useEffect(() => {
    if (!hasMembership || !canEditFire) return;
    if (skipFireAutosaveRef.current) {
      skipFireAutosaveRef.current = false;
      return;
    }
    queueFireSave(420);
    return () => {
      window.clearTimeout(fireSaveTimerRef.current);
    };
  }, [fireDraft, hasMembership, canEditFire, queueFireSave]);

  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState !== "hidden") return;
      window.clearTimeout(fireSaveTimerRef.current);
      runFireSave();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [runFireSave]);

  const lblOpts = {
    birthDateIso: axisBirth,
    anchorDateYmd: axisAnchor,
    calendarTz,
  };

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Jubilación</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {retirementError ? (
        <div className="banner error-banner">{retirementError}</div>
      ) : null}

      {installationInflationPct <= 0 ? (
        <div className="banner info-banner">
          Inflación a 0%: el target FIRE queda plano en euros de hoy. La fecha objetivo puede ser optimista respecto a tu poder adquisitivo real.{" "}
          <a
            href={TAB_PATH.settings}
            onClick={(e) => {
              if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                return;
              e.preventDefault();
              navigate(TAB_PATH.settings);
            }}
          >
            Ajustes
          </a>
          .
        </div>
      ) : null}

      {hasMembership ? (
        <>
          <div className="metric-grid workspace-kpi-strip">
            <MetricCard
              label="Patrimonio objetivo (Actual + Inf. Adj.)"
              value={
                retirementMetricsReady &&
                fireKpis.targetNoPen !== null &&
                fireKpis.targetNoPen > 0
                  ? formatCurrencyNumber(fireKpis.targetNoPen, currencyIso)
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.targetAtCross !== null &&
                fireKpis.targetAtCross > 0
                  ? formatCurrencyNumber(fireKpis.targetAtCross, currencyIso)
                  : undefined
              }
            />
            <MetricCard
              label="Primer cruce"
              value={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0
                  ? `~${projectionXTickLabel(fireKpis.miNo, fireKpis.mc, {
                      ageUiMode: axisAgeMode,
                      birthDateIso: axisBirth,
                      anchorDateYmd: axisAnchor,
                      calendarTz,
                    })}`
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0
                  ? complementaryProjectionTickLabel(
                      fireKpis.miNo,
                      fireKpis.mc,
                      axisAgeMode,
                      lblOpts,
                    )
                  : ""
              }
            />
            <MetricCard
              label="Años hasta el cruce"
              value={
                retirementMetricsReady && fireKpis.miNo !== null
                  ? formatYearsEsFromMonths(fireKpis.miNo)
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady &&
                fireKpis.miNo !== null &&
                fireKpis.mc > 0 &&
                fireKpis.miNo > fireKpis.mc
                  ? "Fuera del horizonte de la proyección actual."
                  : ""
              }
            />
          </div>
          {retirementMetricsReady &&
          fireKpis.swrN !== null &&
          fireKpis.swrN <= 0 ? (
            <p className="muted tight">
              SWR 0 %: no se calcula fecha de cruce.
            </p>
          ) : null}
        </>
      ) : null}

      {!canEditFire ? (
        <p className="muted tight">
          Solo el propietario puede editar esta configuración.
        </p>
      ) : null}

      {hasMembership &&
      projectionSeries &&
      projectionSeries.points.length > 0 ? (
        (() => {
          const pts = projectionSeries.points;
          const horizon = projectionSeries.horizon_years;
          const lastIdx = pts.length - 1;
          const firstNwLabel = pts[0]
            ? formatCurrencyNumber(pts[0].net_worth, currencyIso)
            : null;
          const lastNwLabel = pts[lastIdx]
            ? formatCurrencyNumber(pts[lastIdx].net_worth, currencyIso)
            : null;
          const jubMi =
            typeof projectionSeries.jubilacion_month_index === "number"
              ? projectionSeries.jubilacion_month_index
              : null;
          const jubLabel =
            jubMi != null
              ? projectionXTickLabel(jubMi, pts.length, {
                  ageUiMode: axisAgeMode,
                  birthDateIso: axisBirth,
                  anchorDateYmd: axisAnchor,
                  calendarTz,
                })
              : null;
          const hasFire =
            !!projectionSeries.fire_target_series &&
            projectionSeries.fire_target_series.length > 0;
          // Si hay jubilación, recortamos la serie a jub+12 (un año después
          // del cruce). El eje Y se zoom-ajusta entre NW(hoy) y NW(fin).
          const clampToMonth = jubMi != null ? jubMi + 12 : null;
          const lastVisibleIdx =
            clampToMonth != null
              ? Math.min(clampToMonth, lastIdx)
              : lastIdx;
          const lastVisibleLabel = pts[lastVisibleIdx]
            ? formatCurrencyNumber(pts[lastVisibleIdx].net_worth, currencyIso)
            : lastNwLabel;
          return (
            <section className="panel">
              <div className="panel-head-row">
                <h3 className="panel-title">Patrimonio vs. objetivo FIRE</h3>
                <span
                  className="muted"
                  style={{
                    fontSize: "0.78rem",
                    fontVariantNumeric: "tabular-nums",
                  }}
                >
                  {firstNwLabel && lastVisibleLabel
                    ? `${firstNwLabel} → ${lastVisibleLabel}${jubMi != null ? " · cruce + 1 a" : ` · ${horizon} a`}`
                    : `${horizon} a`}
                </span>
              </div>
              <MiniProjection
                series={projectionSeries}
                height={240}
                showFire={hasFire}
                showJub={true}
                showAreas={false}
                zoomY
                clampToMonth={clampToMonth}
                xAxis={{
                  ageUiMode: axisAgeMode,
                  birthDateIso: axisBirth,
                  anchorDateYmd: axisAnchor,
                  calendarTz,
                }}
              />
              <MiniProjectionLegend
                items={[
                  { label: "Patrimonio neto", color: "var(--proj-nw)" },
                  ...(hasFire
                    ? ([
                        {
                          label: "Target FIRE",
                          color: "var(--proj-fire)",
                          dash: true,
                        },
                      ] as const)
                    : []),
                  ...(jubMi != null
                    ? ([
                        {
                          label: `Primer cruce · ${jubLabel ?? ""}`.trim(),
                          color: "var(--ff-accent)",
                        },
                      ] as const)
                    : []),
                ]}
              />
              {!hasFire ? (
                <p
                  className="muted tight"
                  style={{ marginTop: "0.6rem", fontSize: "0.78rem" }}
                >
                  Configura el objetivo FIRE más abajo para ver la línea de
                  target sobre el gráfico.
                </p>
              ) : jubMi == null ? (
                <p
                  className="muted tight"
                  style={{ marginTop: "0.6rem", fontSize: "0.78rem" }}
                >
                  Sin cruce en el horizonte ({horizon} a). Aumenta el horizonte
                  o ajusta el aporte mensual.
                </p>
              ) : null}
            </section>
          );
        })()
      ) : hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Patrimonio vs. objetivo FIRE</h3>
          <div className="ff-chart-skeleton ff-chart-skeleton--mini" aria-hidden style={{ minHeight: 240 }} />
        </section>
      ) : null}

      <section className="panel">
        <h3 className="panel-title">Objetivo anual <span className="muted">(en dinero de hoy)</span></h3>
        <div className="stack bordered-top retirement-config-stack">
          <fieldset disabled={!canEditFire} className="stack retirement-config-stack">
            <div className="retirement-mode-grid" role="radiogroup" aria-label="Modo objetivo anual">
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "manual" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "manual"}
                  onChange={() =>
                    setFireDraft((p) => ({ ...p, fire_number_mode: "manual" }))
                  }
                />
                <span className="retirement-mode-name">Manual</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveManualAnnualDisplay}
                </span>
              </label>
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "annual_expense" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "annual_expense"}
                  onChange={() =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_mode: "annual_expense",
                    }))
                  }
                />
                <span className="retirement-mode-name">Gasto actual</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveExpenseAnnualDisplay}
                </span>
              </label>
              <label
                className={`retirement-mode-card ${
                  fireDraft.fire_number_mode === "current_income" ? "is-active" : ""
                }`}
              >
                <input
                  type="radio"
                  name="fire_mode"
                  className="sr-only"
                  checked={fireDraft.fire_number_mode === "current_income"}
                  onChange={() =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_mode: "current_income",
                    }))
                  }
                />
                <span className="retirement-mode-name">Ingresos actuales</span>
                <span className="retirement-mode-sub retirement-mode-amount">
                  {retirementObjectiveIncomeAnnualDisplay}
                </span>
              </label>
            </div>

            {fireDraft.fire_number_mode === "manual" ? (
              <label className="field">
                <span>Gasto anual neto objetivo</span>
                <input
                  inputMode="decimal"
                  value={fireDraft.fire_number_manual_amount ?? ""}
                  onChange={(e) =>
                    setFireDraft((p) => ({
                      ...p,
                      fire_number_manual_amount:
                        e.target.value.trim() === ""
                          ? null
                          : e.target.value.replace(",", "."),
                    }))
                  }
                  onBlur={() => queueFireSave(0)}
                />
              </label>
            ) : null}
          </fieldset>
        </div>
      </section>

      <section className="panel">
        <h3 className="panel-title">Retirada</h3>
        <div className="stack bordered-top retirement-config-stack">
          <fieldset disabled={!canEditFire} className="stack retirement-config-stack">
            <label className="field">
              <span className="label-with-help">
                Retirada anual (SWR)
                <HelpPopover
                  title={HELP_TEXTS["settings.swr"].title}
                  body={HELP_TEXTS["settings.swr"].body}
                />
              </span>
              <input
                type="range"
                min={0}
                max={40}
                step={1}
                value={Math.round(
                  (parseDisplayDecimal(fireDraft.swr_pct) ?? 0) * 10,
                )}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  setFireDraft((p) => ({
                    ...p,
                    swr_pct: String(v / 10),
                  }));
                }}
                onBlur={() => queueFireSave(0)}
              />
              <span className="muted tight">
                {formatPercentAmount(fireDraft.swr_pct)}
              </span>
            </label>
          </fieldset>
        </div>
      </section>

      {hasMembership &&
      !projectionBusy &&
      !retirementBusy &&
      (!projectionSeries || !retirementBudgetSnapshot) ? (
        <div className="banner info-banner">Sin datos.</div>
      ) : null}
    </div>
  );
}
