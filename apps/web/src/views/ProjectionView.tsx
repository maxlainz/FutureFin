import { useEffect, useMemo, useState } from "react";
import type {
  HistoryCashflowApi,
  HistorySeriesApi,
  InstallationAccess,
  PlanningFlowApiRow,
  ProjectionMilestoneApi,
  ProjectionSeriesApi,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import { Switch } from "../components/Switch";
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
  neededCapitalAtRetirement,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
} from "../lib/projection-chart";
import { useIsMobile } from "../lib/responsive";

import { ProjectionNetWorthChart } from "./ProjectionNetWorthChart";

export function ProjectionView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  projectionSeries,
  historySeries,
  cashflowSeries,
  cashflowDaily,
  onRequestDailyCashflow,
  projectionBusy,
  projectionError,
  userBirthDate,
  calendarTz,
  planningFlows,
  assetOwnerNames,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  historySeries: HistorySeriesApi | null;
  /** Cash-flow histórico (weekly, ventana 24m) para el overlay fino del chart. Opcional. */
  cashflowSeries: HistoryCashflowApi | null;
  /** Detalle diario (ventana 6m), fetcheado lazy vía `onRequestDailyCashflow`. */
  cashflowDaily: HistoryCashflowApi | null;
  onRequestDailyCashflow?: () => void;
  projectionBusy: boolean;
  projectionError: string | null;
  userBirthDate: string | null;
  calendarTz: string;
  planningFlows: PlanningFlowApiRow[];
  /** asset_id → nombre de owner (para desambiguar duplicados en la leyenda, vista hogar;
   *  `null` en el valor = activo sin owner resoluble). */
  assetOwnerNames: Readonly<Record<string, string | null>> | null;
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
  /**
   * El mes que rotula el pseudo-hito «jubilación» de la tira de KPIs (modelo v2, C4).
   *
   * Con `success_threshold` es la FECHA VÁLIDA (`safe_date_month_index`): el mes en que jubilarse
   * cumple tu umbral. Con cualquier otra base manda `jubilacion_month_index`, que es el mes en que
   * esta simulación se jubila de verdad —la edad que pediste—; rotular ahí la fecha válida diría
   * «te jubilas aquí» sobre un mes en el que el plan simulado no hace nada.
   *
   * Con `not_reachable`/`pending` los dos son `null` y el pseudo-hito no existe: no hay fecha que
   * fingir.
   */
  const jubilacionMiNo =
    (projectionSeries?.retirement_date_basis === "success_threshold"
      ? projectionSeries?.safe_date_month_index
      : projectionSeries?.jubilacion_month_index) ?? null;

  // Preferencia PERSISTIDA de «Vista cercana» (la memoria de escritorio)…
  const [focusModeStored, setFocusModeStored] = useState<boolean>(() => {
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
        focusModeStored ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }, [focusModeStored]);

  // …y el override de móvil: 90 años de horizonte en un plot de ~350px son
  // ilegibles, así que en móvil la vista cercana va activada POR DEFECTO. El
  // override es efímero (estado, no storage): el toggle sigue funcionando en
  // móvil pero nunca pisa la preferencia guardada — escritorio mantiene su
  // memoria intacta.
  const isMobile = useIsMobile();
  const [mobileFocusOverride, setMobileFocusOverride] = useState<boolean | null>(
    null,
  );
  const focusMode = isMobile ? mobileFocusOverride ?? true : focusModeStored;
  const setFocusMode = (v: boolean) => {
    if (isMobile) {
      setMobileFocusOverride(v);
    } else {
      setFocusModeStored(v);
    }
  };

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
    // Sin suelo a 0 desde 4.9.0 (#146): una inflación NEGATIVA es válida y el deflactor del
    // chart debe amplificar (> 1) en vez de fingir «sin ajuste».
    return n != null && Number.isFinite(n) ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  /**
   * El tile «Capital necesario hoy» (modelo v2, M9/C4). Reemplaza a «Objetivo al jubilarte»: en v2
   * no hay objetivo — hay el LÍQUIDO que, con tu mezcla de activos, sostiene el plan a tu umbral.
   *
   * **La cifra principal NO se toca con el toggle**: `needed_capital_today` viaja en euros de HOY
   * por contrato, redondeada a cientos hacia arriba, y es la MISMA, al euro, en Jubilación,
   * Resumen y Proyección. Deflactarla la haría discrepar de sus dos gemelas sin que nada fallara.
   */
  const neededToday = useMemo(() => {
    const raw = projectionSeries?.needed_capital_today;
    return raw != null ? parseDisplayDecimal(raw) : null;
  }, [projectionSeries?.needed_capital_today]);

  /** El umbral del perfil, que es el SUJETO de la cifra: «capital necesario» no significa nada sin
   *  decir necesario *para qué*. Eco de la respuesta; sin él el subtítulo se queda en la base. */
  const thresholdOutOfHundred = projectionSeries?.success_threshold_pct ?? null;

  /**
   * La SEGUNDA línea del tile: el capital necesario en la fecha válida, leído de la curva en
   * `safe_date_series_position`. Esta sí sigue el toggle —es un importe de un mes futuro— y por
   * eso la base viaja pegada al importe en vez de re-derivarse del estado del interruptor:
   * «toggle activo con inflación 0» y «toggle apagado» dan el MISMO número con bases distintas.
   *
   * Ausente mientras el nivel 2 calcula la curva (`needed_capital_curve_state: "computing"`): la
   * línea simplemente no se pinta, que es lo correcto — un importe inventado ahí sería una
   * promesa que el sorteo aún no ha hecho.
   */
  const neededAtRetirement = useMemo(
    () =>
      neededCapitalAtRetirement(
        projectionSeries,
        inflationAdjusted,
        projectionInflationPct,
      ),
    [projectionSeries, inflationAdjusted, projectionInflationPct],
  );

  // Con el toggle de inflación activo (y inflación > 0), los hitos se expresan en euros de hoy: el
  // backend ya cruza los mismos umbrales (1M, 2.5M…) sobre el patrimonio deflactado, así que el
  // marcador del chart cae sobre la curva deflactada y la KPI muestra "1M € de hoy hacia ~año".
  // La jubilación no se ve afectada por inflación (su mes de cruce es invariante).
  //
  // El `useMemo` NO es cosmético: era una IIFE, así que cada render del padre devolvía un array
  // nuevo → `focusWindow` (memoizado sobre `milestones`) se recalculaba → su efecto reescribía
  // `viewWindow` y borraba el pan/zoom que el usuario acababa de hacer en el chart grande.
  const nextMilestones: ProjectionMilestoneApi[] = useMemo(() => {
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
  }, [
    inflationAdjusted,
    projectionInflationPct,
    projectionSeries?.milestones,
    projectionSeries?.milestones_real,
    jubilacionMiNo,
  ]);

  return (
    <div className="workspace workspace--projection-fullwidth">
      <div className="workspace-header">
        <div className="projection-header-main">
          <h2 className="workspace-title">Proyección</h2>
          <Switch
            variant="chart"
            label="Vista cercana"
            checked={focusMode}
            onChange={setFocusMode}
            ariaLabel="Acercar la proyección a los próximos hitos"
          />
          <Switch
            variant="chart"
            label="En dinero de hoy"
            checked={inflationAdjusted}
            onChange={setInflationAdjusted}
            ariaLabel="Mostrar la proyección ajustada a inflación (en dinero de hoy)"
          />
        </div>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {projectionError ? (
        <div className="banner error-banner">{projectionError}</div>
      ) : null}

      {/* `!projectionError`: el esqueleto significa «está cargando». Con la serie fallida encima
          del banner de error se quedaba ahí para siempre, prometiendo un chart que no iba a
          llegar. */}
      {hasMembership && !projectionError && (projectionBusy || !projectionSeries) ? (
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
                    label={isJubilacion ? "Capital necesario hoy" : "Hito"}
                    value={
                      isJubilacion
                        ? neededToday !== null
                          ? formatCurrencyNumber(neededToday, currencyIso)
                          : METRIC_DASH
                        : formatProjectionMilestoneCompactLabel(m.target)
                    }
                    helpId={isJubilacion ? "retirement.needed_capital" : undefined}
                    // La tarjeta DECLARA su base en vez de dejar que el lector la adivine del
                    // estado del interruptor, y añade la lectura de la curva en la fecha válida
                    // cuando el nivel 2 ya la ha resuelto: «hoy harían falta X; el día que te
                    // jubiles, Y». Sin importe no se declara base.
                    detail={
                      isJubilacion && neededToday !== null
                        ? [
                            thresholdOutOfHundred != null
                              ? `en euros de hoy · para que aguanten ${thresholdOutOfHundred} de cada 100`
                              : "en euros de hoy",
                            neededAtRetirement.amount !== null
                              ? `al jubilarte: ${formatCurrencyNumber(
                                  neededAtRetirement.amount,
                                  currencyIso,
                                )} ${
                                  neededAtRetirement.basis === "today"
                                    ? "(euros de hoy)"
                                    : "(euros de ese mes)"
                                }`
                              : null,
                          ]
                            .filter((l): l is string => l !== null)
                            .join(" · ")
                        : undefined
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
            history={
              historySeries &&
              historySeries.anchor_date_ymd === projectionSeries.anchor_date_ymd
                ? historySeries
                : null
            }
            cashflow={
              cashflowSeries &&
              cashflowSeries.anchor_date_ymd ===
                projectionSeries.anchor_date_ymd
                ? cashflowSeries
                : null
            }
            cashflowDaily={
              cashflowDaily &&
              cashflowDaily.anchor_date_ymd === projectionSeries.anchor_date_ymd
                ? cashflowDaily
                : null
            }
            onRequestDailyCashflow={onRequestDailyCashflow}
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
            assetOwnerNames={assetOwnerNames}
          />
          {/* S5/#137: model_note viajaba en la respuesta y estaba tipado, pero ningún
              componente lo renderizaba — la confesión de los supuestos del modelo (flujos en
              euros nominales, solo el objetivo se ajusta por inflación) no llegaba a nadie. */}
          {projectionSeries.model_note ? (
            <details className="projection-model-note">
              <summary>Supuestos del modelo</summary>
              <p>{projectionSeries.model_note}</p>
            </details>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}
