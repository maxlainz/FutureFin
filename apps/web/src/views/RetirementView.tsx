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
import { MiniProjection } from "../components/charts/MiniProjection";
import { ChartLegend } from "../components/charts/ChartLegend";
import {
  METRIC_DASH,
  formatCurrencyNumber,
  formatPercentAmount,
  parseDisplayDecimal,
} from "../lib/format";
import {
  computeFireAnnualNeedNetEur,
  defaultFireSettingsApi,
  grossUpNetAnnualFire,
  normalizeInstallationFireSettings,
  savingsSourceUsesTransactions,
} from "../lib/fire";
import { type LedgerPersonScope } from "../lib/ledger";
import { settingsSubTabPath } from "../lib/navigation";
import { appUrl } from "../lib/basePath";
import {
  complementaryProjectionTickLabel,
  formatYearsEsFromMonths,
  lastPointIndexAtOrBeforeMonth,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
} from "../lib/projection-chart";

/**
 * Prosa es-ES para `projectionSeries.fire_target_absent_reason` (#119) — los mismos tres
 * literales que `SimKpis.fire_target_absent_reason` en el servidor. Antes la nota solo cubría
 * el caso `swr_pct = 0`, recalculado en cliente; ahora lee el motivo real que ya calculó el
 * servidor, que puede ser cualquiera de los tres.
 */
const FIRE_TARGET_ABSENT_REASON_ES: Record<string, string> = {
  manual_amount_missing: "Falta el importe del objetivo manual: no se calcula fecha de cruce.",
  net_need_not_positive:
    "El gasto neto de jubilación no es positivo: no se calcula fecha de cruce.",
  swr_not_positive: "SWR 0 %: no se calcula fecha de cruce.",
};

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
  /** Aviso cuando el guardado automático se salta un cambio por datos inválidos. */
  const [fireAutosaveIssue, setFireAutosaveIssue] = useState<string | null>(null);
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

  // Fuente del ahorro CONFIGURADA (la de los ajustes, no la efectiva del summary): en los modos
  // con promedio el summary es un input de la KPI, y si se lee de `summary` para decidir si hay
  // que esperarlo, con summary a null nunca se espera. Ese es justo el bucle que dejaba la
  // primera pintada con la base del presupuesto.
  const configuredSavingsUsesTransactions = savingsSourceUsesTransactions(
    installation?.installation.fire_settings?.savings_source,
  );

  // Las tres KPI del panel superior («Patrimonio objetivo», «Primer cruce», «Años hasta el
  // cruce») leen ahora el servidor (#118): no dependen de `retirementBudgetSnapshot` ni de
  // `summary`, así que dejan de esperar a `/v1/summary` en modos B/C (efecto colateral bueno —
  // antes la KPI se calculaba en cliente y SÍ dependía de esos dos).
  const retirementMetricsReady =
    hasMembership && !projectionBusy && !retirementBusy && projectionSeries != null;

  // La vista previa LOCAL del draft sin guardar sí necesita el presupuesto (y, en modos B/C, el
  // summary): es la misma dependencia que antes gobernaba `retirementMetricsReady` entera. Ahora
  // solo gobierna el preview y las tres tarjetas de «Objetivo anual».
  const firePreviewReady =
    hasMembership &&
    !retirementBusy &&
    retirementBudgetSnapshot != null &&
    // En modos B/C la cifra sale del summary: sin él el preview se pintaba primero con la base
    // del presupuesto y daba un salto al llegar (o se quedaba mal para siempre si el summary
    // fallaba). Un número plausible pero equivocado es peor que un guion.
    (!configuredSavingsUsesTransactions || summary != null);

  const installationInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    // Sin suelo a 0 desde 4.9.0 (#146): la preview del objetivo debe decrecer con deflación.
    return n != null && Number.isFinite(n) ? n : 0;
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

  // Vista previa LOCAL del objetivo con los ajustes del draft (sin guardar). `computeFireAnnualNeedNetEur`
  // y `grossUpNetAnnualFire` siguen duplicados en cliente solo para esto: el cruce (mes, fecha,
  // objetivo al cruce) ya no se recalcula aquí — lee siempre del servidor (§2.1/§2.2 del spike).
  const firePreview = useMemo(() => {
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
    if (needAnnual !== null && needAnnual > 0 && swrN !== null && swrN > 0) {
      const grossNoPen = grossUpNetAnnualFire(needAnnual, brackets, taxOn);
      targetNoPen = grossNoPen / (swrN / 100);
      // #142 (4.8.0): el objetivo lleva además el término finito de deuda (Σ cuotas restantes
      // + residuales). El cliente NO puede derivarlo (necesita el calendario completo de cada
      // plan): se SUMA el que publica el servidor. No depende de los ajustes del draft, así que
      // la vista previa sigue siendo exacta al mover SWR/gasto/impuestos.
      const debtComponent =
        projectionSeries?.fire_target_debt_component != null
          ? parseDisplayDecimal(projectionSeries.fire_target_debt_component)
          : null;
      if (debtComponent !== null && debtComponent > 0) {
        targetNoPen += debtComponent;
      }
    }

    return { needAnnual, swrN, targetNoPen };
  }, [
    fireDraft,
    fireExpenseM,
    fireIncomeM,
    retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent,
    projectionSeries?.fire_target_debt_component,
  ]);

  // Ajustes REALMENTE guardados (prop reactiva de la instalación), normalizados igual que el
  // draft — para poder comparar como iguales. Comparar contra `lastSavedFirePayloadRef` (un ref)
  // no re-renderiza cuando el guardado automático completa, así que la tarjeta se quedaría
  // mostrando la vista previa aunque el servidor ya hubiera recalculado (§2.3 del spike).
  const savedFire = useMemo(
    () => normalizeInstallationFireSettings(installation?.installation.fire_settings),
    [installation?.installation.fire_settings],
  );
  const fireDraftDirty = JSON.stringify(fireDraft) !== JSON.stringify(savedFire);

  // Lecturas del servidor — SIEMPRE, nunca recalculadas en cliente: el cruce depende de la
  // simulación mensual completa y el cliente no puede rehacerla (ni debe fingirlo).
  const jubMi =
    typeof projectionSeries?.jubilacion_month_index === "number"
      ? projectionSeries.jubilacion_month_index
      : null;
  const mc = projectionSeries?.months ?? 0;
  const serverTargetToday =
    projectionSeries?.jubilacion_target_net_worth != null
      ? parseDisplayDecimal(projectionSeries.jubilacion_target_net_worth)
      : null;
  const targetAtCrossNominal =
    projectionSeries?.jubilacion_target_net_worth_nominal != null
      ? parseDisplayDecimal(projectionSeries.jubilacion_target_net_worth_nominal)
      : null;
  // «Patrimonio objetivo»: el servidor, salvo que el draft tenga cambios sin guardar — en ese
  // caso la vista previa local (con paréntesis «vista previa · sin guardar») para que la tarjeta
  // siga respondiendo al slider de SWR mientras llega el autosave + refetch.
  const targetToday = fireDraftDirty ? firePreview.targetNoPen : serverTargetToday;
  const targetTodayReady =
    retirementMetricsReady && (fireDraftDirty ? firePreviewReady : true);

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
    // Estos dos `return` salían SIN guardar y **sin decir nada**, mientras el pie del panel
    // seguía prometiendo «Guardado automático». El usuario movía el SWR fuera de rango, veía el
    // mensaje de guardado y se iba con el cambio perdido. Ahora el aviso es visible.
    const swrN = parseDisplayDecimal(fireDraft.swr_pct);
    if (swrN === null || swrN < 0 || swrN > 4) {
      setFireAutosaveIssue(
        "La tasa de retirada segura debe estar entre 0 y 4 %. No se ha guardado.",
      );
      return;
    }
    if (
      fireDraft.fire_number_mode === "manual" &&
      (fireDraft.fire_number_manual_amount == null ||
        String(fireDraft.fire_number_manual_amount).trim() === "")
    ) {
      setFireAutosaveIssue(
        "Has elegido fijar el objetivo a mano pero falta la cifra. No se ha guardado.",
      );
      return;
    }
    setFireAutosaveIssue(null);
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

      {fireAutosaveIssue ? (
        <div className="banner error-banner" role="alert">
          {fireAutosaveIssue}
        </div>
      ) : null}

      {installationInflationPct <= 0 ? (
        <div className="banner info-banner">
          Con la inflación a 0 %, tu objetivo se queda plano en dinero de hoy: la fecha que ves
          puede ser optimista frente a lo que costará vivir entonces.{" "}
          <a
            href={appUrl(settingsSubTabPath("plan"))}
            onClick={(e) => {
              if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                return;
              e.preventDefault();
              // Antes iba a `/ajustes` a secas, y el canonicalizador lo reescribía a la primera
              // sub-pestaña: el aviso hablaba de la inflación y te dejaba en la pantalla de
              // aprobar usuarios.
              navigate(settingsSubTabPath("plan"));
            }}
          >
            Ajustar la inflación
          </a>
          .
        </div>
      ) : null}

      {hasMembership ? (
        <>
          <div className="metric-grid workspace-kpi-strip">
            {/* Los rótulos estaban cruzados: la cifra grande es `targetToday`, euros de HOY, y
                llevaba el rótulo «(con inflación)»; la inflada (target al cruce) iba en el
                paréntesis sin rótulo ninguno, así que la única cifra etiquetada era la que no
                correspondía. */}
            <MetricCard
              label="Patrimonio objetivo (euros de hoy)"
              helpId="retirement.target"
              value={
                targetTodayReady && targetToday !== null && targetToday > 0
                  ? formatCurrencyNumber(targetToday, currencyIso)
                  : METRIC_DASH
              }
              parenthetical={
                !targetTodayReady
                  ? undefined
                  : fireDraftDirty
                    ? "vista previa · sin guardar"
                    : targetAtCrossNominal !== null && targetAtCrossNominal > 0
                      ? `${formatCurrencyNumber(targetAtCrossNominal, currencyIso)} al cruce`
                      : undefined
              }
            />
            <MetricCard
              label="Primer cruce"
              value={
                retirementMetricsReady && jubMi !== null && mc > 0
                  ? `~${projectionXTickLabel(jubMi, mc, {
                      ageUiMode: axisAgeMode,
                      birthDateIso: axisBirth,
                      anchorDateYmd: axisAnchor,
                      calendarTz,
                    })}`
                  : METRIC_DASH
              }
              parenthetical={
                retirementMetricsReady && jubMi !== null && mc > 0
                  ? complementaryProjectionTickLabel(jubMi, mc, axisAgeMode, lblOpts)
                  : ""
              }
            />
            <MetricCard
              label="Años hasta el cruce"
              value={
                retirementMetricsReady && jubMi !== null
                  ? formatYearsEsFromMonths(jubMi)
                  : METRIC_DASH
              }
            />
          </div>
          {retirementMetricsReady && projectionSeries?.fire_target_absent_reason ? (
            <p className="muted tight">
              {FIRE_TARGET_ABSENT_REASON_ES[projectionSeries.fire_target_absent_reason] ??
                "No se calcula fecha de cruce."}
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
              ? // Meses del horizonte, no puntos del array: con `density=hybrid` `pts.length`
                // (~82) no es el número de meses y la etiqueta relativa elegía «m» donde tocaba «a».
                projectionXTickLabel(jubMi, projectionSeries.months, {
                  ageUiMode: axisAgeMode,
                  birthDateIso: axisBirth,
                  anchorDateYmd: axisAnchor,
                  calendarTz,
                })
              : null;
          const hasFire =
            !!projectionSeries.fire_target_series &&
            projectionSeries.fire_target_series.length > 0;
          // `jubMi === 0` es «ya jubilado» — el cruce es HOY, el mes 0. `0` es falsy en JS: un
          // `jubMi ? … : …` aquí reintroduce el bug al revés (#132). Con `alreadyRetired` no
          // recortamos a "cruce + 12": el usuario quiere ver el horizonte completo, no un año.
          const alreadyRetired = jubMi === 0;
          // Si hay jubilación FUTURA, recortamos la serie a jub+12 (un año después del cruce). El
          // eje Y se zoom-ajusta entre NW(hoy) y NW(fin).
          const clampToMonth = jubMi != null && !alreadyRetired ? jubMi + 12 : null;
          // `clampToMonth` es un MES y `lastIdx` una POSICIÓN del array: con `density=hybrid`
          // (0..12, 24, 36…) no son lo mismo, así que el pie del panel enseñaba el patrimonio de
          // un punto que no era el último visible del chart. Misma traducción mes → posición que
          // hace MiniProjection con esta misma prop.
          const lastVisibleIdx =
            clampToMonth != null
              ? lastPointIndexAtOrBeforeMonth(pts, clampToMonth)
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
                    ? `${firstNwLabel} → ${lastVisibleLabel}${clampToMonth != null ? " · cruce + 1 a" : ` · ${horizon} a`}`
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
              <ChartLegend
                size="sm"
                structural={[
                  {
                    key: "nw",
                    label: "Patrimonio neto",
                    color: "var(--proj-nw)",
                    swatch: "line",
                  },
                  ...(hasFire
                    ? ([
                        {
                          key: "fire",
                          label: "Objetivo FIRE",
                          color: "var(--proj-fire)",
                          swatch: "dashed",
                        },
                      ] as const)
                    : []),
                  ...(jubMi != null
                    ? ([
                        {
                          key: "jub",
                          label: `Primer cruce · ${jubLabel ?? ""}`.trim(),
                          color: "var(--ff-accent)",
                          swatch: "line",
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
