import { useEffect, useMemo, useState } from "react";
import type {
  InstallationAccess,
  PlanningFlowApiRow,
  ProjectionMilestoneApi,
  ProjectionSeriesApi,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import {
  METRIC_DASH,
  formatCurrencyNumber,
  formatPercentAmount,
  parseDisplayDecimal,
} from "../lib/format";
import {
  formatProjectionMilestoneCompactLabel,
  type LedgerPersonScope,
} from "../lib/ledger";
import {
  PROJECTION_FOCUS_STORAGE_KEY,
  PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
} from "../lib/projection-chart";

import { ProjectionNetWorthChart } from "./ProjectionNetWorthChart";

export function ProjectionView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  projectionSeries,
  projectionBusy,
  projectionError,
  userBirthDate,
  calendarTz,
  planningFlows,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  projectionBusy: boolean;
  projectionError: string | null;
  userBirthDate: string | null;
  calendarTz: string;
  planningFlows: PlanningFlowApiRow[];
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const inflationPctRaw =
    installation?.installation.annual_inflation_assumption_percent;
  const inflationPctDisplay =
    inflationPctRaw != null && String(inflationPctRaw).trim() !== ""
      ? formatPercentAmount(String(inflationPctRaw))
      : null;

  const axisAgeMode = projectionSeries
    ? resolveProjectionAxisAgeMode(projectionSeries, installation)
    : "dates";
  const axisBirth = (() => {
    const fromApi = projectionSeries?.viewer_birth_date?.trim();
    const fromUser = userBirthDate?.trim();
    const pick =
      fromApi && fromApi.length > 0
        ? fromApi
        : fromUser && fromUser.length > 0
          ? fromUser
          : null;
    return pick;
  })();
  const axisAnchor = projectionSeries?.anchor_date_ymd?.trim() || null;
  const jubilacionMiNo = projectionSeries?.jubilacion_month_index ?? null;
  const jubilacionTargetNoPen =
    projectionSeries?.jubilacion_target_net_worth != null
      ? parseDisplayDecimal(projectionSeries.jubilacion_target_net_worth)
      : null;

  const [focusMode, setFocusMode] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    try {
      return window.localStorage.getItem(PROJECTION_FOCUS_STORAGE_KEY) === "1";
    } catch {
      return false;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(
        PROJECTION_FOCUS_STORAGE_KEY,
        focusMode ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }, [focusMode]);

  const [inflationAdjusted, setInflationAdjusted] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    try {
      const v = window.localStorage.getItem(
        PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
      );
      return v == null ? true : v === "1";
    } catch {
      return true;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(
        PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
        inflationAdjusted ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }, [inflationAdjusted]);

  const projectionInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    return n != null && n > 0 ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  // Con el toggle de inflación activo (y inflación > 0), los hitos se expresan en euros de hoy: el
  // backend ya cruza los mismos umbrales (1M, 2.5M…) sobre el patrimonio deflactado, así que el
  // marcador del chart cae sobre la curva deflactada y la KPI muestra "1M € de hoy hacia ~año".
  // La jubilación no se ve afectada por inflación (su mes de cruce es invariante).
  const nextMilestones: ProjectionMilestoneApi[] = (() => {
    const useRealMilestones =
      inflationAdjusted &&
      projectionInflationPct > 0 &&
      (projectionSeries?.milestones_real?.length ?? 0) > 0;
    const base =
      (useRealMilestones
        ? projectionSeries?.milestones_real
        : projectionSeries?.milestones) ?? [];
    if (jubilacionMiNo !== null) {
      return [
        ...base,
        {
          target: "jubilacion",
          reached_month_index: jubilacionMiNo,
          reached_date_ymd: "",
        },
      ];
    }
    return base;
  })();

  return (
    <div className="workspace workspace--projection-fullwidth">
      <div className="workspace-header">
        <div className="projection-header-main">
          <h2 className="workspace-title">Proyección</h2>
          <label className="projection-focus-toggle">
            <span className="projection-focus-toggle-label">Focus</span>
            <input
              type="checkbox"
              role="switch"
              checked={focusMode}
              onChange={(e) => setFocusMode(e.target.checked)}
              aria-label="Activar focus en la proyección"
            />
            <span className="projection-focus-toggle-track" aria-hidden="true">
              <span className="projection-focus-toggle-thumb" />
            </span>
          </label>
          <label className="projection-focus-toggle">
            <span className="projection-focus-toggle-label">
              Inflation Adjusted
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={inflationAdjusted}
              onChange={(e) => setInflationAdjusted(e.target.checked)}
              aria-label="Mostrar la proyección ajustada a inflación (en dinero de hoy)"
            />
            <span className="projection-focus-toggle-track" aria-hidden="true">
              <span className="projection-focus-toggle-thumb" />
            </span>
          </label>
        </div>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {projectionError ? (
        <div className="banner error-banner">{projectionError}</div>
      ) : null}

      {hasMembership && (projectionBusy || !projectionSeries) ? (
        <section className="panel">
          <h3 className="panel-title">Trayectoria proyectada</h3>
          <div className="ff-chart-skeleton" aria-hidden />
        </section>
      ) : null}

      {hasMembership && !projectionBusy && projectionSeries ? (
        <section className="panel">
          <h3 className="panel-title">Trayectoria proyectada</h3>
          {nextMilestones.length > 0 ? (
            <div className="metric-grid workspace-kpi-strip">
              {nextMilestones.map((m) => {
                const isJubilacion = m.target === "jubilacion";
                return (
                  <MetricCard
                    key={`${m.target}-${m.reached_month_index}`}
                    label={isJubilacion ? "Jubilación" : "Milestone"}
                    value={
                      isJubilacion
                        ? jubilacionTargetNoPen !== null
                          ? formatCurrencyNumber(
                              jubilacionTargetNoPen,
                              currencyIso,
                            )
                          : METRIC_DASH
                        : formatProjectionMilestoneCompactLabel(m.target)
                    }
                    parenthetical={`~${projectionXTickLabel(
                      m.reached_month_index,
                      projectionSeries.months,
                      {
                        ageUiMode: axisAgeMode,
                        birthDateIso: axisBirth,
                        anchorDateYmd: axisAnchor,
                        calendarTz,
                      },
                    )}`}
                  />
                );
              })}
            </div>
          ) : null}
          <ProjectionNetWorthChart
            series={projectionSeries}
            milestones={nextMilestones}
            focusMode={focusMode}
            inflationAdjusted={inflationAdjusted}
            installationInflationPct={projectionInflationPct}
            currencyIso={currencyIso}
            ledgerPersonScope={ledgerPersonScope}
            inflationPctDisplay={inflationPctDisplay}
            ageUiMode={axisAgeMode}
            userBirthDate={axisBirth}
            anchorDateYmd={axisAnchor}
            calendarTz={calendarTz}
            planningFlows={planningFlows}
          />
        </section>
      ) : null}
    </div>
  );
}
