/**
 * Jubilación — **rediseño UX U1b** (5.0.0, issue #207, decisiones U1–U12 y S1–S11).
 *
 * **Tercera vuelta de UX (V1–V7, feedback F2 y F5–F10 del owner)**: la página tiene ahora DOS
 * bloques y ningún acordeón, en este orden y sin excepciones:
 *
 *  1. **Cabecera**: el título y UN solo indicador de guardado (S6). Antes había seis pies
 *     «Guardado automático.», uno por panel, que podían contradecirse entre sí.
 *  2. **«Tu plan»** (configuración): una TARJETA POR TEMA —Estrategia · Edades · Pensión · Gasto
 *     en jubilación · Retirada · Horizonte—, cada una con su frase de qué hace, y **solo los
 *     campos que la estrategia elegida usa**. La tabla U2 vive en `lib/plan-fields.ts` y aquí no
 *     se re-decide nada; lo que V3 cambió es su eje de agrupación, no una sola condición.
 *  3. **«Resultado»**: la FRASE del plan (`lib/plan-sentence.ts`), el rojo cuando lo hay, como
 *     mucho tres tarjetas (`buildRetirementTilesV2`), **un solo gráfico** —con eje Y, etiquetas
 *     de borde y la banda COLOREADA por la probabilidad de agotar el capital (V2/V5)—, el riesgo
 *     en compacto y un «Detalle del cálculo» plegado con todo lo de segundo orden.
 *
 * Lo que se fue en esta vuelta, y por qué no vuelve sin deshacer una decisión del owner: el
 * **banner de alta** (F5 — con estrategia elegida, «Elige tu estrategia» es un cartel que sobra,
 * y su flag de `localStorage` nunca miró el perfil), el **acordeón «Avanzado»** con la línea
 * «Supuestos» de cabecera (F10 — la línea existía para enunciar lo que el acordeón escondía; sin
 * acordeón no hay nada escondido), la **tabla «agotar a los 65/70/…»** (F7/V5 — el color de la
 * banda lo dice con más resolución, y el total acumulado bajó a «Detalle del cálculo») y los dos
 * campos de la tarjeta «Riesgo» (V6/V7 — el colchón se deriva del tope de tu regla de ahorro y el
 * umbral de éxito es fijo al 100 %).
 *
 * Cuatro invariantes que este archivo no puede romper:
 *
 *  - **Un solo porcentaje de retirada** (U4). El slider es `swr_pct` y el formulario **jamás**
 *    manda `withdrawal_rule.pct` ni `start_pct`: el servidor los hereda del SWR y publica de
 *    dónde salieron (`pct_source`). Dos porcentajes obligaban a explicar cuál mandaba, y la
 *    respuesta honesta era «depende de la pantalla».
 *  - **Todo por MES** (`month_index`), nunca por posición de `points[]`: con `density=hybrid` la
 *    posición 13 es el mes 24.
 *  - **`null` no es cero**: una tarjeta que la estrategia no responde no se pinta con guion.
 *  - **En Hogar no hay plan** (D9/U10): el agregado no tiene estrategia propia, así que se
 *    enseñan las frases por miembro y nada más — ni tarjetas, ni chart, ni formulario.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import type {
  BudgetSnapshotApi,
  InstallationAccess,
  PensionPlanApi,
  ProjectionBandsApi,
  ProjectionSeriesApi,
  RetirementProfileApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
  SummaryResponse,
  UserResponse,
  WithdrawalRuleKindApi,
} from "../api/types";
import { HelpPopover } from "../components/HelpPopover";
import { Switch } from "../components/Switch";
import { HELP_TEXTS } from "../lib/helpTexts";
import { MetricCard } from "../components/MetricCard";
import { MiniProjection } from "../components/charts/MiniProjection";
import { ChartLegend } from "../components/charts/ChartLegend";
import {
  formatCurrencyNumber,
  formatPercentAmount,
  formatPercentDisplay,
  parseDisplayDecimal,
} from "../lib/format";
import { savingsSourceUsesTransactions } from "../lib/fire";
import {
  BRIDGE_DISCOUNT_BASIS_LABEL,
  HORIZON_LIFESPAN_AGE_OPTIONS,
  RETIREMENT_STRATEGIES,
  RETIREMENT_STRATEGY_BLURB,
  RETIREMENT_STRATEGY_LABEL,
  WITHDRAWAL_RULE_KIND_LABEL,
  buildRetirementProfilePatch,
  defaultRetirementProfileApi,
  effectiveTargetBasis,
  isEmptyRetirementProfilePatch,
  newPartialRetirementDraft,
  newPensionPlanDraft,
  normalizeRetirementProfile,
  retirementProfileIssue,
  targetBasisSource,
  withdrawalPctSource,
} from "../lib/retirementProfile";
import { messageForError } from "../lib/errorMessages";
import {
  buildRetirementNotices,
  buildRetirementTilesV2,
  retirementDetailRows,
} from "../lib/retirement-tiles";
import { planSentence } from "../lib/plan-sentence";
import { householdPlanLines } from "../lib/household-plan-lines";
import {
  planCardGroups,
  planFieldsContextFromProfile,
  requiredPlanFields,
  type PlanFieldDescriptor,
  type PlanFieldId,
} from "../lib/plan-fields";
import {
  PLAN_CARD_COPY,
  PLAN_FIELD_HELP,
  fractionFromPercent,
  missingRequiredPlanFields,
  percentFromFraction,
  saveIndicatorLabel,
  withdrawalPctNote,
} from "../lib/retirement-form";
import { buildRetirementChartMarkers } from "../lib/retirement-chart";
import {
  buildRiskExtraRows,
  cashBufferLine,
  formatSuccessPercent,
  riskFootnote,
  showsNoVolatilityNotice,
  successParenthetical,
  successVerdictTone,
} from "../lib/risk-bands";
import {
  RISK_AMBER_AT,
  RISK_RED_AT,
  depletionProbabilityAtMonth,
  riskColorForProbability,
  riskGradientStops,
} from "../lib/risk-gradient";
import { type LedgerPersonScope } from "../lib/ledger";
import { TAB_PATH, settingsSubTabPath } from "../lib/navigation";
import { appUrl } from "../lib/basePath";
import {
  PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
  deflationFactorAt,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
} from "../lib/projection-chart";

/**
 * Prosa es-ES para `projectionSeries.fire_target_absent_reason` (#119) — los mismos tres
 * literales que `SimKpis.fire_target_absent_reason` en el servidor.
 */
const FIRE_TARGET_ABSENT_REASON_ES: Record<string, string> = {
  manual_amount_missing: "Falta el importe del objetivo manual: no se calcula fecha de cruce.",
  net_need_not_positive:
    "El gasto neto de jubilación no es positivo: no se calcula fecha de cruce.",
  swr_not_positive: "SWR 0 %: no se calcula fecha de cruce.",
};

/** Un decimal tecleado por el usuario, listo para el wire: coma española → punto. */
function typedDecimal(raw: string): string {
  return raw.replace(",", ".");
}

/** Un decimal opcional: vacío = «no hay valor», que en el perfil es `null`, no `0`. */
function typedDecimalOrNull(raw: string): string | null {
  const t = raw.trim();
  return t === "" ? null : typedDecimal(raw);
}

/** Marca de campo obligatorio sin rellenar (U2). No es un error del servidor: es el dato que la
 *  estrategia elegida necesita para poder simularse como se ha pedido. */
function RequiredHint() {
  return <small className="retirement-required-hint">obligatorio · sin guardar</small>;
}

export function RetirementView({
  installation,
  installationBusy,
  hasMembership,
  projectionSeries,
  projectionBusy,
  projectionBands,
  projectionBandsBusy,
  projectionBandsError,
  retirementBudgetSnapshot,
  summary,
  retirementBusy,
  retirementError,
  retirementProfile,
  retirementProfileBusy,
  retirementProfileError,
  retirementProfileSaving,
  user,
  calendarTz,
  scopeReadOnly,
  onSaveRetirementProfile,
  onSelectMineScope,
  navigate,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  /** Se recibe pero NO se lee: el candado de la vista es `scopeReadOnly`, derivado de él en
   *  `App.tsx`. Repetir aquí la regla `household ⇒ solo lectura` es cómo se abre una segunda
   *  fuente de verdad para el mismo ámbito. */
  ledgerPersonScope: LedgerPersonScope;
  projectionSeries: ProjectionSeriesApi | null;
  projectionBusy: boolean;
  /** Bandas de Monte Carlo (5.0.0, D28). `null` = aún no han llegado, o la vista es Hogar. */
  projectionBands: ProjectionBandsApi | null;
  projectionBandsBusy: boolean;
  projectionBandsError: string | null;
  retirementBudgetSnapshot: BudgetSnapshotApi | null;
  /** Solo se consume en modo B (promedio): los equivalentes efectivos del ahorro real. */
  summary: SummaryResponse | null;
  retirementBusy: boolean;
  retirementError: string | null;
  /** Perfil de jubilación del usuario de la sesión (5.0.0, D13). `null` = aún no ha llegado. */
  retirementProfile: RetirementProfileApi | null;
  retirementProfileBusy: boolean;
  retirementProfileError: string | null;
  retirementProfileSaving: boolean;
  user: UserResponse | null;
  calendarTz: string;
  /** Vista Hogar (D9/D32): agregado de solo lectura — el plan se edita desde la vista «Yo». */
  scopeReadOnly: boolean;
  /** Guarda un PATCH mínimo y devuelve el perfil YA resuelto por el servidor. */
  onSaveRetirementProfile: (
    patch: RetirementProfilePatchApi,
  ) => Promise<RetirementProfileApi | null>;
  /** Vuelve a la vista «Yo» (U10): el único camino desde el agregado del hogar a tu plan. */
  onSelectMineScope?: () => void;
  navigate: (path: string, replace?: boolean) => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";

  /**
   * El plan de jubilación es dato PERSONAL: lo edita cualquier rol, `viewer` incluido. Lo único
   * que lo bloquea es la vista Hogar, que es un agregado de N personas y no tiene un perfil al
   * que atribuir el cambio.
   */
  const canEditProfile = hasMembership && !scopeReadOnly;

  // ── Borrador del perfil y su autoguardado ─────────────────────────────────────────────────
  const [profileDraft, setProfileDraft] = useState<RetirementProfileApi>(() =>
    defaultRetirementProfileApi(),
  );
  const syncedProfileRef = useRef<RetirementProfileApi>(defaultRetirementProfileApi());
  const [profileIssue, setProfileIssue] = useState<string | null>(null);
  const profileSaveTimerRef = useRef(0);
  const profileSaveSeqRef = useRef(0);
  const skipProfileAutosaveRef = useRef(true);
  const profileInitializedRef = useRef<RetirementProfileApi | null>(null);
  /** Instante del último guardado con éxito — la mitad viva del indicador único (S6). */
  const [savedAtMs, setSavedAtMs] = useState<number | null>(null);

  useEffect(() => {
    if (!retirementProfile) {
      profileInitializedRef.current = null;
      return;
    }
    if (profileInitializedRef.current !== null) return;
    profileInitializedRef.current = retirementProfile;
    const p = normalizeRetirementProfile(retirementProfile);
    setProfileDraft(p);
    syncedProfileRef.current = p;
    skipProfileAutosaveRef.current = true;
  }, [retirementProfile]);

  const savedProfile = useMemo(
    () => normalizeRetirementProfile(retirementProfile),
    [retirementProfile],
  );
  /** Hay cambios sin guardar: la cabecera del Resultado lo dice en vez de fingir que las cifras
   *  del servidor ya incluyen lo que se acaba de teclear. */
  const profileDirty =
    retirementProfile != null &&
    !isEmptyRetirementProfilePatch(
      buildRetirementProfilePatch(savedProfile, profileDraft),
    );

  const birthDate = user?.birth_date?.trim() || null;
  const hasBirthDate = birthDate != null && birthDate !== "";

  /** El contexto de la tabla U2: qué campos existen con ESTE perfil. Una sola derivación para
   *  el formulario, la línea de supuestos y la guarda de obligatorios. */
  const fieldCtx = useMemo(
    () => planFieldsContextFromProfile(profileDraft, hasBirthDate),
    [profileDraft, hasBirthDate],
  );
  const requiredIds = useMemo(
    () => requiredPlanFields(profileDraft.strategy, fieldCtx),
    [profileDraft.strategy, fieldCtx],
  );
  const missingIds = useMemo(
    () =>
      missingRequiredPlanFields({
        profile: profileDraft,
        required: requiredIds,
        birthDate,
      }),
    [profileDraft, requiredIds, birthDate],
  );
  const missingSet = useMemo(() => new Set(missingIds), [missingIds]);
  /** Referencia estable para el efecto de autosave: sin ella, un array nuevo por render
   *  reiniciaría el debounce en cada repintado ajeno. */
  const blockedBySomeRequired = missingIds.length > 0;

  const runProfileSave = useCallback(() => {
    if (!canEditProfile) return;
    const patch = buildRetirementProfilePatch(syncedProfileRef.current, profileDraft);
    if (isEmptyRetirementProfilePatch(patch)) {
      setProfileIssue(null);
      return;
    }
    // U2 — la guarda nueva: una estrategia a la que le falta un dato NO se guarda. El servidor
    // aceptaría el PATCH y degradaría el plan en silencio (una edad objetivo ausente se simula
    // como «Cuanto antes»), que es exactamente lo que no puede pasar sin que se vea.
    if (blockedBySomeRequired) {
      setProfileIssue(null);
      return;
    }
    // La guarda de validez habla con los MISMOS códigos que el servidor, así que la frase sale
    // del catálogo único.
    const issue = retirementProfileIssue(profileDraft);
    if (issue) {
      setProfileIssue(messageForError(issue, null));
      return;
    }
    setProfileIssue(null);
    const seq = ++profileSaveSeqRef.current;
    void onSaveRetirementProfile(patch)
      .then((saved) => {
        if (seq !== profileSaveSeqRef.current || !saved) return;
        syncedProfileRef.current = saved;
        setSavedAtMs(Date.now());
        // `saved.target_basis` es la elección ALMACENADA (`null` = derivada): resincronizar el
        // borrador con ella lo mantiene alineado sin convertir en elección explícita lo que
        // sigue siendo una derivación.
        setProfileDraft((d) =>
          d.target_basis === saved.target_basis
            ? d
            : { ...d, target_basis: saved.target_basis },
        );
      })
      .catch(() => {
        // El banner lo pinta App.tsx. Aquí solo hay que NO marcar como guardado.
      });
  }, [profileDraft, canEditProfile, blockedBySomeRequired, onSaveRetirementProfile]);

  const queueProfileSave = useCallback(
    (delayMs: number) => {
      window.clearTimeout(profileSaveTimerRef.current);
      profileSaveTimerRef.current = window.setTimeout(() => {
        profileSaveTimerRef.current = 0;
        runProfileSave();
      }, delayMs);
    },
    [runProfileSave],
  );

  useEffect(() => {
    if (!canEditProfile) return;
    if (skipProfileAutosaveRef.current) {
      skipProfileAutosaveRef.current = false;
      return;
    }
    // Sin nada que guardar no se arma el temporizador: este efecto se re-ejecuta en CADA render
    // y sin la salida temprana un flujo de renders ajenos reiniciaría el debounce sin fin.
    if (
      isEmptyRetirementProfilePatch(
        buildRetirementProfilePatch(syncedProfileRef.current, profileDraft),
      )
    ) {
      return;
    }
    queueProfileSave(420);
    return () => {
      window.clearTimeout(profileSaveTimerRef.current);
    };
  }, [profileDraft, canEditProfile, queueProfileSave]);

  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState !== "hidden") return;
      window.clearTimeout(profileSaveTimerRef.current);
      runProfileSave();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [runProfileSave]);

  /** Reloj del «hace N s». Solo late cuando hay algo que envejecer. */
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (savedAtMs == null) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 5000);
    return () => window.clearInterval(id);
  }, [savedAtMs]);

  const saveState = saveIndicatorLabel({
    saving: retirementProfileSaving,
    savedAtMs,
    nowMs,
    error: retirementProfileError != null,
    blocked: blockedBySomeRequired && profileDirty,
  });

  /** Atajo para editar un campo del borrador (todo el formulario autosalva). */
  const patchDraft = useCallback(
    (fn: (p: RetirementProfileApi) => RetirementProfileApi) => {
      setProfileDraft((prev) => fn(prev));
    },
    [],
  );

  // ── S1 · la fecha de nacimiento se puede fijar aquí mismo ─────────────────────────────────
  //
  // Su sitio natural es «Tu cuenta», pero tres de las cinco estrategias no se pueden simular sin
  // ella: mandar al usuario a otra pestaña a mitad de la elección es donde se abandona el plan.
  // El PATCH del perfil acepta `birth_date` (misma columna que `PATCH /v1/auth/me`), así que se
  // guarda por el mismo camino y no hay una segunda vía de escritura.
  const [birthDraft, setBirthDraft] = useState("");
  const saveBirthDate = useCallback(
    (value: string) => {
      const t = value.trim();
      if (!/^\d{4}-\d{2}-\d{2}$/.test(t)) return;
      void onSaveRetirementProfile({ birth_date: t })
        .then(() => setSavedAtMs(Date.now()))
        .catch(() => {
          /* el banner lo pinta App.tsx */
        });
    },
    [onSaveRetirementProfile],
  );

  // ── Ejes y rotuladores ────────────────────────────────────────────────────────────────────
  const axisAgeMode = projectionSeries
    ? resolveProjectionAxisAgeMode(projectionSeries, installation)
    : "dates";
  const axisBirth =
    projectionSeries?.viewer_birth_date?.trim() || birthDate || null;
  const axisAnchor = projectionSeries?.anchor_date_ymd?.trim() || null;
  const mc = projectionSeries?.months ?? 0;

  /** Mes de la rejilla → etiqueta del eje. Lo consumen la frase, las tarjetas, el detalle y las
   *  líneas del hogar: una sola definición para que las cuatro digan lo mismo. */
  const monthLabel = useCallback(
    (mi: number) =>
      projectionXTickLabel(mi, mc > 0 ? mc : 1, {
        ageUiMode: axisAgeMode,
        birthDateIso: axisBirth,
        anchorDateYmd: axisAnchor,
        calendarTz,
      }),
    [mc, axisAgeMode, axisBirth, axisAnchor, calendarTz],
  );

  const configuredSavingsUsesTransactions = savingsSourceUsesTransactions(
    installation?.installation.fire_settings?.savings_source,
  );
  const retirementMetricsReady =
    hasMembership && !projectionBusy && !retirementBusy && projectionSeries != null;

  const installationInflationPct = useMemo(() => {
    const raw = installation?.installation.annual_inflation_assumption_percent;
    if (raw == null) return 0;
    const n = parseDisplayDecimal(String(raw));
    return n != null && Number.isFinite(n) ? n : 0;
  }, [installation?.installation.annual_inflation_assumption_percent]);

  // Fuente EFECTIVA del ahorro (tras el fallback del servidor): en los modos con promedio la
  // cifra derivada del objetivo sale del summary ya calculado, no de un promedio recalculado
  // aquí — así la línea de «Gasto en jubilación» coincide con lo que el servidor simuló.
  const savingsAvgActive = savingsSourceUsesTransactions(
    summary?.financial_health.savings_source,
  );
  const fireExpenseM = savingsAvgActive
    ? summary?.financial_health.expense_regular_monthly_equivalent
    : retirementBudgetSnapshot?.totals.expense_retirement_monthly_equivalent;
  const fireIncomeM = savingsAvgActive
    ? summary?.financial_health.income_monthly_equivalent
    : retirementBudgetSnapshot?.totals.income_monthly_equivalent;
  const spendBaseReady =
    hasMembership &&
    !retirementBusy &&
    retirementBudgetSnapshot != null &&
    (!configuredSavingsUsesTransactions || summary != null);

  // ── Resultado: frase, tarjetas y avisos ───────────────────────────────────────────────────
  const basis = effectiveTargetBasis(profileDraft);
  const basisSource = targetBasisSource(profileDraft);
  const rule = profileDraft.withdrawal_rule;
  const pension = profileDraft.pension;
  const partial = profileDraft.partial_retirement;
  const rulePctNote = withdrawalPctNote({
    rule,
    swrPct: profileDraft.swr_pct,
    pctSource: withdrawalPctSource(rule),
  });

  const sentence = useMemo(
    () =>
      planSentence({
        series: retirementMetricsReady ? projectionSeries : null,
        targetRetirementAge: savedProfile.target_retirement_age ?? null,
        monthLabel,
        ageMode: axisAgeMode,
      }),
    [
      retirementMetricsReady,
      projectionSeries,
      savedProfile.target_retirement_age,
      monthLabel,
      axisAgeMode,
    ],
  );

  /** Las tarjetas se leen del perfil GUARDADO, no del borrador: la base y la edad del borrador
   *  nombrarían un plan que la respuesta del servidor no simuló. */
  const tilesInput = useMemo(
    () => ({
      series: retirementMetricsReady ? projectionSeries : null,
      currencyIso,
      monthLabel,
      targetRetirementAge: savedProfile.target_retirement_age ?? null,
      targetBasis: retirementProfile ? effectiveTargetBasis(savedProfile) : null,
      pensionStartAge: savedProfile.pension?.starts_at_age ?? null,
    }),
    [
      retirementMetricsReady,
      projectionSeries,
      currencyIso,
      monthLabel,
      savedProfile,
      retirementProfile,
    ],
  );
  const tiles = useMemo(() => buildRetirementTilesV2(tilesInput), [tilesInput]);
  const detailRows = useMemo(() => retirementDetailRows(tilesInput), [tilesInput]);
  /** El rojo de D17 va ARRIBA, con el resto de banners; los demás avisos bajan al «Detalle»
   *  dentro de `retirementDetailRows`, así que aquí solo se filtran los que suben. */
  const dangerNotices = useMemo(
    () =>
      buildRetirementNotices(
        retirementMetricsReady ? projectionSeries : null,
        savedProfile.target_retirement_age ?? null,
      ).filter((n) => n.tone === "danger"),
    [retirementMetricsReady, projectionSeries, savedProfile.target_retirement_age],
  );

  // ── El chart único (U5) ───────────────────────────────────────────────────────────────────
  //
  // El toggle «En dinero de hoy» comparte llave de localStorage con el de Proyección a
  // propósito: es la MISMA pregunta («¿en euros de qué año leo esto?») y dos respuestas en dos
  // pestañas de la misma app es cómo se acaban comparando dos cifras que no son comparables.
  const [inflationAdjusted, setInflationAdjusted] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    try {
      const v = window.localStorage.getItem(PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY);
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

  /** La banda se enseña POR DEFECTO cuando hay escenarios: el plan determinista es una de las
   *  lecturas posibles, no la única, y esconder la dispersión tras un clic la convierte en una
   *  curiosidad opcional. Se puede apagar para leer la curva sola. */
  const [showBand, setShowBand] = useState(true);

  /**
   * Tasa del deflactor: la de la RESPUESTA (la misma con la que el servidor construyó
   * `net_worth_real`), y solo cae a la de la instalación con un backend antiguo.
   */
  const deflationPct = useMemo(() => {
    const raw = projectionSeries?.deflation_annual_inflation_percent;
    const parsed = raw != null ? Number(raw) : Number.NaN;
    return Number.isFinite(parsed) ? parsed : installationInflationPct;
  }, [projectionSeries?.deflation_annual_inflation_percent, installationInflationPct]);

  /** UN solo deflactor para patrimonio, objetivo y banda. Deflactar solo unos los separaría y
   *  el abanico dejaría de contener a la línea que dice contener. */
  const chartDeflator = useMemo(() => {
    const pct = inflationAdjusted ? deflationPct : 0;
    return (mi: number) => deflationFactorAt(mi, pct);
  }, [inflationAdjusted, deflationPct]);

  /** Puntos de banda en euros NOMINALES: la deflactación la aplica el chart, una sola vez. */
  const bandPoints = useMemo(() => {
    if (!projectionBands) return null;
    return projectionBands.points.map((p) => ({
      month: p.month_index,
      p10: p.net_worth_p10,
      p90: p.net_worth_p90,
    }));
  }, [projectionBands]);

  const chartMarkers = useMemo(() => {
    const pts = projectionSeries?.points;
    if (!pts || pts.length === 0) return [];
    return buildRetirementChartMarkers(projectionSeries, {
      startMonth: pts[0]!.month_index,
      endMonth: pts[pts.length - 1]!.month_index,
    });
  }, [projectionSeries]);

  /**
   * El color de la banda (V2/V5). Los extremos son los MISMOS que los de `chartMarkers` —el
   * primer y el último mes de la serie cargada— porque son los que el chart usa para repartir su
   * eje X: el `<linearGradient>` va en `userSpaceOnUse` entre esos dos meses, y con otros el
   * mapeo mes→color se desplazaría sin que nada fallara.
   *
   * Sin volatilidad declarada NO se colorea: las tres bandas son la línea determinista y teñir de
   * verde una banda de ancho cero diría «ningún escenario falla» sobre un sorteo que no existe.
   */
  const gradientStops = useMemo(() => {
    const pts = projectionSeries?.points;
    if (!showBand || !pts || pts.length === 0) return [];
    if (!projectionBands || projectionBands.any_volatility_declared === false) return [];
    return riskGradientStops({
      points: projectionBands.depletion_probability_by_age,
      monthStart: pts[0]!.month_index,
      monthEnd: pts[pts.length - 1]!.month_index,
    });
  }, [showBand, projectionSeries, projectionBands]);

  /** Rótulo del hover. Sale de `depletionProbabilityAtMonth`, **la misma función que colorea**:
   *  un tooltip alimentado por otro cálculo podría contradecir al tinte y nadie lo notaría. */
  const chartHoverLabel = useMemo(() => {
    if (gradientStops.length < 2 || !projectionBands) return null;
    const points = projectionBands.depletion_probability_by_age;
    return (mi: number): string | null => {
      const p = depletionProbabilityAtMonth(points, mi);
      if (p == null) return null;
      return `${monthLabel(mi)} · ${formatPercentDisplay(p * 100)} de los escenarios ya se han quedado sin capital`;
    };
  }, [gradientStops, projectionBands, monthLabel]);

  /** Qué decir del colchón de caja, que desde V6 se DERIVA del tope de tu regla de ahorro. */
  const bufferLine = useMemo(
    () => cashBufferLine(projectionBands, currencyIso),
    [projectionBands, currencyIso],
  );

  // ── Riesgo compacto ───────────────────────────────────────────────────────────────────────
  const riskExtraRows = useMemo(
    () => buildRiskExtraRows({ bands: projectionBands, currencyIso, monthLabel }),
    [projectionBands, currencyIso, monthLabel],
  );

  // ── Hogar (U10): frases por miembro y nada más ────────────────────────────────────────────
  const memberLines = useMemo(
    () => householdPlanLines(projectionSeries?.members, monthLabel),
    [projectionSeries?.members, monthLabel],
  );

  // ── Editores compartidos ──────────────────────────────────────────────────────────────────
  const selectStrategy = useCallback(
    (s: RetirementStrategyApi) => {
      patchDraft((p) => {
        const next: RetirementProfileApi = { ...p, strategy: s };
        if (s === "partial" && next.partial_retirement == null) {
          next.partial_retirement = newPartialRetirementDraft();
        }
        if (s === "pension_bridge" && next.pension == null) {
          next.pension = newPensionPlanDraft();
        }
        return next;
      });
    },
    [patchDraft],
  );

  const setPension = useCallback(
    (fn: (p: PensionPlanApi) => PensionPlanApi) => {
      patchDraft((p) => (p.pension ? { ...p, pension: fn(p.pension) } : p));
    },
    [patchDraft],
  );

  const intFieldValue = (v: number | null) => (v == null ? "" : String(v));
  const readIntField = (raw: string): number | null | undefined => {
    const t = raw.trim();
    if (t === "") return null;
    const n = Number(t);
    // `undefined` = «no es un entero, ignora la pulsación»: el patrón de la casa para no dejar
    // un número a medio teclear dentro del borrador que autosalva.
    if (!Number.isInteger(n) || n < 0 || n > 200) return undefined;
    return n;
  };

  const fieldHelp = (id: PlanFieldId): ReactNode => {
    const entry = PLAN_FIELD_HELP[id];
    if (!entry) return null;
    const t = HELP_TEXTS[entry.helpId];
    return <HelpPopover title={t.title} body={t.body} />;
  };

  // ═══════════════════════════════════════════════════════════════════════════════════════════
  // UN solo renderer de campo, por ID, en el orden que dicta `planFields`
  // ------------------------------------------------------------------------------------------
  // Hasta V3 había dos —`renderPlanField` y `renderAdvancedField`— porque había dos GRUPOS y dos
  // sitios donde pintarlos. Con las tarjetas por tema el grupo desapareció y con él la razón de
  // los dos switches: un mismo id no puede tener dos editores, y tener dos funciones que podían
  // divergir era una invitación a que un campo se pintara distinto según dónde cayera.
  // ═══════════════════════════════════════════════════════════════════════════════════════════
  const renderField = (f: PlanFieldDescriptor): ReactNode => {
    const missing = missingSet.has(f.id);
    switch (f.id) {
      case "birth_date":
        return (
          <label className="field" key={f.id}>
            <span>{f.label}</span>
            <input
              type="date"
              value={birthDraft}
              onChange={(e) => {
                setBirthDraft(e.target.value);
                saveBirthDate(e.target.value);
              }}
            />
            {missing ? (
              <RequiredHint />
            ) : (
              <small className="muted">
                Se guarda en tu cuenta: convierte las edades del plan en meses concretos.
              </small>
            )}
          </label>
        );

      case "target_retirement_age":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {f.required ? null : <span className="muted"> (opcional)</span>}
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              value={intFieldValue(profileDraft.target_retirement_age)}
              placeholder={f.required ? "p. ej. 60" : "—"}
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined) return;
                patchDraft((p) => ({ ...p, target_retirement_age: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? <RequiredHint /> : null}
          </label>
        );

      // La media jornada se pinta ENTERA en su primer campo: edad e ingreso son un solo dato
      // partido en dos y separarlos en dos filas obligaba a leer la fase dos veces.
      case "partial_start_age":
        return (
          <div className="field-row" key={f.id}>
            <label className="field">
              <span className="label-with-help">
                {f.label}
                {fieldHelp(f.id)}
              </span>
              <input
                inputMode="numeric"
                value={partial ? String(partial.starts_at_age) : ""}
                onChange={(e) => {
                  const v = readIntField(e.target.value);
                  if (v === undefined || v === null) return;
                  patchDraft((p) =>
                    p.partial_retirement
                      ? {
                          ...p,
                          partial_retirement: { ...p.partial_retirement, starts_at_age: v },
                        }
                      : p,
                  );
                }}
                onBlur={() => queueProfileSave(0)}
              />
              {missing ? <RequiredHint /> : null}
            </label>
            <label className="field">
              <span>Ingreso mensual en media jornada</span>
              <input
                inputMode="decimal"
                placeholder="0"
                value={partial?.income_monthly_today ?? ""}
                onChange={(e) =>
                  patchDraft((p) =>
                    p.partial_retirement
                      ? {
                          ...p,
                          partial_retirement: {
                            ...p.partial_retirement,
                            income_monthly_today: typedDecimal(e.target.value),
                          },
                        }
                      : p,
                  )
                }
                onBlur={() => queueProfileSave(0)}
              />
              <small className="muted">En euros de hoy. Vacío = año sabático.</small>
            </label>
          </div>
        );
      case "partial_income":
        return null; // se pinta con `partial_start_age`

      // La pensión se pinta entera en su primer campo: la casilla y sus dos cifras son un bloque.
      case "pension_amount":
        return (
          <div className="stack" key={f.id}>
            <label className="field checkbox-field">
              <input
                type="checkbox"
                checked={pension != null}
                // El puente ES la pensión: quitarla dejaría la estrategia sin su dato y el
                // servidor rechazaría el PATCH.
                disabled={profileDraft.strategy === "pension_bridge" && pension != null}
                onChange={(e) =>
                  patchDraft((p) => ({
                    ...p,
                    pension: e.target.checked ? newPensionPlanDraft() : null,
                  }))
                }
              />
              <span className="label-with-help">
                Cuento con una pensión
                {fieldHelp(f.id)}
              </span>
            </label>
            {pension ? (
              <div className="field-row">
                <label className="field">
                  <span>Pensión mensual (euros de hoy)</span>
                  <input
                    inputMode="decimal"
                    placeholder="p. ej. 1200"
                    value={pension.monthly_amount_today}
                    onChange={(e) =>
                      setPension((p) => ({
                        ...p,
                        monthly_amount_today: typedDecimal(e.target.value),
                      }))
                    }
                    onBlur={() => queueProfileSave(0)}
                  />
                  {missing ? <RequiredHint /> : null}
                </label>
                <label className="field">
                  <span>Edad a la que empieza</span>
                  <input
                    inputMode="numeric"
                    value={String(pension.starts_at_age)}
                    onChange={(e) => {
                      const v = readIntField(e.target.value);
                      if (v === undefined || v === null) return;
                      setPension((p) => ({ ...p, starts_at_age: v }));
                    }}
                    onBlur={() => queueProfileSave(0)}
                  />
                </label>
              </div>
            ) : profileDraft.strategy === "pension_bridge" ? (
              <small className="muted">
                «Puente hasta la pensión» la necesita: el objetivo se dimensiona con los años que
                van de tu jubilación a la primera paga.
              </small>
            ) : null}
          </div>
        );
      case "pension_start_age":
        return null; // se pinta con `pension_amount`

      case "fire_number_mode":
        return (
          <div className="field" key={f.id}>
            {/* Sin rótulo propio: el `<h4>` de la tarjeta ya dice «Gasto en jubilación», y
                repetirlo dos líneas más abajo es el ruido que V3 vino a quitar. El
                `aria-label` del radiogroup se queda: ahí sí hace falta el nombre. */}
            <div
              className="retirement-mode-grid"
              role="radiogroup"
              aria-label="Gasto en jubilación"
            >
              {(
                [
                  ["annual_expense", "Gasto actual", "tus partidas de jubilación"],
                  ["current_income", "Ingresos actuales", "mantener tu nivel de vida"],
                  ["manual", "Manual", "una cifra que decides tú"],
                ] as const
              ).map(([mode, name, blurb]) => (
                <label
                  key={mode}
                  className={`retirement-mode-card ${
                    profileDraft.fire_number_mode === mode ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="fire_mode"
                    className="sr-only"
                    checked={profileDraft.fire_number_mode === mode}
                    onChange={() => patchDraft((p) => ({ ...p, fire_number_mode: mode }))}
                  />
                  <span className="retirement-mode-name">{name}</span>
                  <span className="retirement-mode-sub">{blurb}</span>
                </label>
              ))}
            </div>
            {profileDraft.fire_number_mode === "manual" ? null : (
              <p className="retirement-derived-line">{derivedSpendLine()}</p>
            )}
          </div>
        );

      case "fire_number_manual_amount":
        return (
          <label className="field" key={f.id}>
            <span>{f.label} · gasto anual neto</span>
            <input
              inputMode="decimal"
              placeholder="p. ej. 24000"
              value={profileDraft.fire_number_manual_amount ?? ""}
              onChange={(e) =>
                patchDraft((p) => ({
                  ...p,
                  fire_number_manual_amount: typedDecimalOrNull(e.target.value),
                }))
              }
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? <RequiredHint /> : null}
          </label>
        );

      // ── Supuestos con default: retirada, base del objetivo, pensión fina y horizonte ────
      //
      // Antes vivían en el acordeón «Avanzado»; desde V3 caen en la tarjeta de su tema
      // (`lib/plan-fields.ts`) y comparten switch con el resto. Ni uno solo cambió de editor.
      case "swr_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <input
              type="range"
              min={0}
              max={40}
              step={1}
              value={Math.round((parseDisplayDecimal(profileDraft.swr_pct) ?? 0) * 10)}
              onChange={(e) => {
                const v = Number(e.target.value);
                patchDraft((p) => ({ ...p, swr_pct: String(v / 10) }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            <span className="retirement-slider-value">
              {formatPercentAmount(profileDraft.swr_pct)}
            </span>
            <small className="muted">
              Dimensiona tu objetivo y es el que retira la regla de abajo.
            </small>
          </label>
        );

      case "withdrawal_rule_kind":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <select
              value={rule.kind}
              onChange={(e) => {
                const kind = e.target.value as WithdrawalRuleKindApi;
                patchDraft((p) => ({
                  ...p,
                  withdrawal_rule: { ...p.withdrawal_rule, kind },
                }));
              }}
            >
              {(
                ["fixed_real", "percent_of_balance", "hybrid", "guardrails"] as const
              ).map((k) => (
                <option key={k} value={k}>
                  {WITHDRAWAL_RULE_KIND_LABEL[k]}
                </option>
              ))}
            </select>
            {/* Un porcentaje fijado por API o MCP no se edita aquí (U4: la pantalla tiene uno
                solo y es el SWR), pero callarlo dejaría al usuario moviendo un slider que su
                regla ignora. La frase la decide `withdrawalPctNote`, con su test. */}
            {rulePctNote ? <small className="muted">{rulePctNote}</small> : null}
          </label>
        );

      case "hybrid_end_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (%)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="decimal"
              placeholder="p. ej. 3"
              value={rule.end_pct ?? ""}
              onChange={(e) =>
                patchDraft((p) => ({
                  ...p,
                  withdrawal_rule: {
                    ...p.withdrawal_rule,
                    end_pct: typedDecimalOrNull(e.target.value),
                  },
                }))
              }
              onBlur={() => queueProfileSave(0)}
            />
            <small className="muted">
              El suelo del latch: tiene que quedar por debajo de tu tasa de retirada.
            </small>
          </label>
        );

      case "guardrails_band_pct":
      case "guardrails_adjust_pct": {
        const isBand = f.id === "guardrails_band_pct";
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (%)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="decimal"
              placeholder={isBand ? "p. ej. 20" : "p. ej. 10"}
              value={(isBand ? rule.band_pct : rule.adjust_pct) ?? ""}
              onChange={(e) =>
                patchDraft((p) => ({
                  ...p,
                  withdrawal_rule: {
                    ...p.withdrawal_rule,
                    ...(isBand
                      ? { band_pct: typedDecimalOrNull(e.target.value) }
                      : { adjust_pct: typedDecimalOrNull(e.target.value) }),
                  },
                }))
              }
              onBlur={() => queueProfileSave(0)}
            />
          </label>
        );
      }

      case "spend_mode":
        return (
          <div className="field" key={f.id}>
            <div
              className="retirement-radio-stack"
              role="radiogroup"
              aria-label="Cómo se aplica la regla"
            >
              <span className="label-with-help field-label-text">
                {f.label}
                {fieldHelp(f.id)}
              </span>
              {(
                [
                  ["ceiling", "Techo: retiro como mucho la regla"],
                  ["rule_is_spend", "La regla es mi gasto"],
                ] as const
              ).map(([mode, text]) => (
                <label className="field checkbox-field" key={mode}>
                  <input
                    type="radio"
                    name="spend_mode"
                    checked={rule.spend_mode === mode}
                    onChange={() =>
                      patchDraft((p) => ({
                        ...p,
                        withdrawal_rule: { ...p.withdrawal_rule, spend_mode: mode },
                      }))
                    }
                  />
                  <span>{text}</span>
                </label>
              ))}
            </div>
          </div>
        );

      case "target_basis":
        return (
          <div className="field" key={f.id}>
            <div
              className="retirement-radio-stack"
              role="radiogroup"
              aria-label="Base del objetivo"
            >
              <span className="label-with-help field-label-text">
                {f.label}
                {fieldHelp(f.id)}
                {/* La opción marcada puede no ser una elección: mientras nadie la fija, la
                    deriva el servidor. Decirlo evita que se lea como una decisión tomada. */}
                {basisSource === "derived" ? <span className="muted"> (derivada)</span> : null}
              </span>
              <label className="field checkbox-field">
                <input
                  type="radio"
                  name="target_basis"
                  checked={basis === "perpetuity"}
                  onChange={() => patchDraft((p) => ({ ...p, target_basis: "perpetuity" }))}
                />
                <span>Renta perpetua (ignora la pensión)</span>
              </label>
              <label className="field checkbox-field">
                <input
                  type="radio"
                  name="target_basis"
                  checked={basis === "bridge_to_pension"}
                  onChange={() =>
                    patchDraft((p) => ({ ...p, target_basis: "bridge_to_pension" }))
                  }
                />
                <span>Puente hasta la pensión</span>
              </label>
            </div>
            {/* FUERA del `radiogroup`: un botón enfocable entre radios rompe la navegación con
                flechas. Sin esta salida, fijar la base a mano es irreversible desde la UI. */}
            {basisSource === "stored" ? (
              <button
                type="button"
                className="btn ghost text retirement-basis-reset"
                onClick={() => patchDraft((p) => ({ ...p, target_basis: null }))}
              >
                Volver a la derivada
              </button>
            ) : null}
          </div>
        );

      case "bridge_discount_basis":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <select
              value={profileDraft.bridge_discount_basis}
              onChange={(e) =>
                patchDraft((p) => ({
                  ...p,
                  bridge_discount_basis:
                    e.target.value === "swr"
                      ? "swr"
                      : e.target.value === "none"
                        ? "none"
                        : "expected_return",
                }))
              }
            >
              {(["expected_return", "swr", "none"] as const).map((b) => (
                <option key={b} value={b}>
                  {BRIDGE_DISCOUNT_BASIS_LABEL[b]}
                </option>
              ))}
            </select>
          </label>
        );

      case "pension_indexed":
        return (
          <label className="field checkbox-field" key={f.id}>
            <input
              type="checkbox"
              checked={pension?.indexed ?? true}
              onChange={(e) => setPension((p) => ({ ...p, indexed: e.target.checked }))}
            />
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
          </label>
        );

      case "pension_fraction_while_partial":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (%)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="decimal"
              placeholder="0"
              value={percentFromFraction(pension?.fraction_while_partial)}
              onChange={(e) =>
                setPension((p) => ({
                  ...p,
                  // S3 — la pantalla habla en PORCENTAJE y la API en fracción. Antes el campo
                  // pedía «0 a 1» y quien escribía 40 declaraba cobrar 40 veces su pensión.
                  fraction_while_partial: fractionFromPercent(e.target.value),
                }))
              }
              onBlur={() => queueProfileSave(0)}
            />
            <small className="muted">
              0 % = no cobras nada de pensión mientras dure la media jornada.
            </small>
          </label>
        );

      case "partial_expense_basis":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <select
              value={partial?.expense_basis ?? "retirement"}
              onChange={(e) =>
                patchDraft((p) =>
                  p.partial_retirement
                    ? {
                        ...p,
                        partial_retirement: {
                          ...p.partial_retirement,
                          expense_basis:
                            e.target.value === "regular" ? "regular" : "retirement",
                        },
                      }
                    : p,
                )
              }
            >
              <option value="retirement">El de jubilación</option>
              <option value="regular">El regular de hoy</option>
            </select>
          </label>
        );

      case "horizon_lifespan_age":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <select
              value={String(profileDraft.horizon_lifespan_age)}
              onChange={(e) => {
                const n = Number(e.target.value);
                if (!Number.isInteger(n)) return;
                patchDraft((p) => ({ ...p, horizon_lifespan_age: n }));
              }}
            >
              {HORIZON_LIFESPAN_AGE_OPTIONS.map((edad) => (
                <option key={edad} value={String(edad)}>
                  {edad} años
                </option>
              ))}
            </select>
          </label>
        );

      default:
        return null;
    }
  };

  /** «1.250 €/mes · 15.000 €/año · del presupuesto» — la cifra que el modo elegido DERIVA, con
   *  su procedencia pegada. Sin la procedencia, dos hogares con el mismo número creen estar
   *  mirando lo mismo cuando uno lee su presupuesto y el otro su histórico real. */
  function derivedSpendLine(): string {
    if (!spendBaseReady) return "Calculando la base…";
    const usingIncome = profileDraft.fire_number_mode === "current_income";
    const monthly = parseDisplayDecimal(String((usingIncome ? fireIncomeM : fireExpenseM) ?? ""));
    if (monthly == null || !Number.isFinite(monthly)) return "Sin base declarada todavía.";
    const source = usingIncome
      ? savingsAvgActive
        ? "promedio de tus ingresos reales"
        : "de tus ingresos del presupuesto"
      : savingsAvgActive
        ? "promedio de tus gastos reales"
        : "de tus partidas de jubilación del presupuesto";
    return `${formatCurrencyNumber(monthly, currencyIso)}/mes · ${formatCurrencyNumber(
      monthly * 12,
      currencyIso,
    )}/año · ${source}`;
  }

  const editingPlan = canEditProfile && retirementProfile != null;
  /** Las tarjetas a pintar, ya sin vacías y en `PLAN_CARD_ORDER` (V3). Una sola lista: el
   *  formulario dejó de tener dos mitades el día que dejó de tener un acordeón. */
  const cardGroups = useMemo(
    () => (editingPlan ? planCardGroups(fieldCtx) : []),
    [editingPlan, fieldCtx],
  );

  const chartReady =
    hasMembership && projectionSeries != null && projectionSeries.points.length > 0;

  return (
    <div className="workspace">
      {/* ── 1 · Cabecera: título + UN indicador de guardado (S6) ────────────────────────── */}
      <div className="workspace-header retirement-header">
        <h2 className="workspace-title">Jubilación</h2>
        {installationBusy ? (
          <p className="workspace-sub">Cargando…</p>
        ) : !hasMembership ? (
          <p className="workspace-sub">Sin acceso hasta aprobación.</p>
        ) : canEditProfile && retirementProfile ? (
          <span
            className={`retirement-save-state${
              saveState.tone === "danger" ? " retirement-save-state--danger" : ""
            }`}
            role="status"
            aria-live="polite"
          >
            {saveState.text}
          </span>
        ) : null}
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {retirementError ? (
        <div className="banner error-banner">{retirementError}</div>
      ) : null}
      {retirementProfileError ? (
        <div className="banner error-banner">{retirementProfileError}</div>
      ) : null}
      {profileIssue ? (
        <div className="banner error-banner" role="alert">
          {profileIssue}
        </div>
      ) : null}

      {!hasMembership ? null : scopeReadOnly ? (
        /* ── Hogar (U10): números agregados y UNA frase por persona ────────────────────────
           El hogar no tiene plan propio —`strategy` viaja `null` y todo el bloque de solves va
           vacío—, así que lo único honesto por miembro es su hito. Una rejilla de tarjetas
           invitaba justo a lo contrario: comparar el «ahorro necesario» de dos personas con
           edades objetivo distintas, que no es una comparación. */
        <section className="panel">
          <h3 className="panel-title">Planes del hogar</h3>
          <p className="muted tight">
            Vista agregada · solo lectura. El plan de jubilación es de cada persona: el hogar no
            tiene una estrategia propia.
          </p>
          {memberLines.length > 0 ? (
            <ul className="household-plan-lines bordered-top">
              {memberLines.map((l) => (
                <li key={l.userId}>{l.text}</li>
              ))}
            </ul>
          ) : (
            <p className="muted tight bordered-top">
              {projectionBusy ? "Cargando…" : "Sin datos."}
            </p>
          )}
          {onSelectMineScope ? (
            <button
              type="button"
              className="btn ghost text retirement-scope-link"
              onClick={onSelectMineScope}
            >
              Cambia a «Yo» para editar tu plan
            </button>
          ) : null}
        </section>
      ) : (
        <>
          {/* ── 2 · «Tu plan»: una tarjeta por tema, todo a la vista (V3) ───────────────────
              Sin banner de alta (F5: con una estrategia ya elegida, «Elige tu estrategia» es un
              cartel que sobra — y el flag de `localStorage` que lo descartaba nunca miraba el
              perfil, así que reaparecía en cada navegador nuevo) y sin acordeón «Avanzado» (F10:
              «un cajón de sastre mal explicado»). Cada tarjeta abre con una frase de QUÉ hace y
              qué implica tocarla; los campos son exactamente los mismos de la tabla U2. */}
          <section className="panel">
            <h3 className="panel-title">Tu plan</h3>

            {retirementProfile == null ? (
              <p className="muted tight bordered-top">
                {retirementProfileBusy ? "Cargando…" : "Sin datos."}
              </p>
            ) : (
              <div className="retirement-card-grid bordered-top">
                {cardGroups.map(({ card, fields }) => (
                  <section
                    key={card}
                    className={`retirement-card${
                      card === "strategy" || card === "spending"
                        ? " retirement-card--wide"
                        : ""
                    }`}
                  >
                    <h4
                      className={`panel-title${
                        card === "strategy" ? " label-with-help" : ""
                      }`}
                    >
                      {PLAN_CARD_COPY[card].title}
                      {/* La ayuda de la estrategia colgaba del `<h3>` del panel; su sitio es el
                          título de la tarjeta que gobierna. Misma clave, mismo texto. */}
                      {card === "strategy" ? (
                        <HelpPopover
                          title={HELP_TEXTS["retirement.strategy"].title}
                          body={HELP_TEXTS["retirement.strategy"].body}
                        />
                      ) : null}
                    </h4>
                    <p className="retirement-card-blurb">{PLAN_CARD_COPY[card].blurb}</p>
                    <div className="stack retirement-config-stack">
                      {card === "strategy" ? (
                        <div
                          className="retirement-mode-grid retirement-strategy-grid"
                          role="radiogroup"
                          aria-label="Estrategia de jubilación"
                        >
                          {RETIREMENT_STRATEGIES.map((st) => (
                            <label
                              key={st}
                              className={`retirement-mode-card ${
                                profileDraft.strategy === st ? "is-active" : ""
                              }`}
                            >
                              <input
                                type="radio"
                                name="retirement_strategy"
                                className="sr-only"
                                checked={profileDraft.strategy === st}
                                onChange={() => selectStrategy(st)}
                              />
                              <span className="retirement-mode-name">
                                {RETIREMENT_STRATEGY_LABEL[st]}
                              </span>
                              <span className="retirement-mode-sub">
                                {RETIREMENT_STRATEGY_BLURB[st]}
                              </span>
                            </label>
                          ))}
                        </div>
                      ) : (
                        fields.map((f) => renderField(f))
                      )}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </section>

          {/* ── 3 · «Resultado» ───────────────────────────────────────────────────────────── */}
          <section className="panel">
            <div className="panel-head-row">
              <h3 className="panel-title">Resultado</h3>
              <HelpPopover
                title={HELP_TEXTS["retirement.plan_sentence"].title}
                body={HELP_TEXTS["retirement.plan_sentence"].body}
              />
            </div>

            {/* U7 — la cabecera de resultados es una FRASE, no tres tarjetas que el usuario
                tenga que volver a juntar en su cabeza. */}
            <p className={`retirement-sentence retirement-sentence--${sentence.tone}`}>
              {retirementMetricsReady ? sentence.text : "Calculando tu plan…"}
            </p>
            {profileDirty ? (
              <p className="muted tight">
                Tus cambios aún no están en estas cifras: se recalculan al guardar.
              </p>
            ) : null}

            {/* D17 — el rojo GRANDE. No es un error (la simulación existe y la fecha no se
                mueve): dice que te jubilarás POR DEBAJO de tu objetivo, y va antes de las cifras
                porque cambia cómo se leen todas. */}
            {dangerNotices.map((n) => (
              <div key={n.code} className="banner error-banner" role="status">
                {n.text}
              </div>
            ))}

            {retirementMetricsReady && projectionSeries?.fire_target_absent_reason ? (
              <p className="muted tight">
                {FIRE_TARGET_ABSENT_REASON_ES[projectionSeries.fire_target_absent_reason] ??
                  "No se calcula fecha de cruce."}
              </p>
            ) : null}

            {installationInflationPct <= 0 ? (
              <div className="banner info-banner">
                Con la inflación a 0 %, tu objetivo se queda plano en dinero de hoy: la fecha que
                ves puede ser optimista frente a lo que costará vivir entonces.{" "}
                <a
                  href={appUrl(settingsSubTabPath("plan"))}
                  onClick={(e) => {
                    if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                      return;
                    e.preventDefault();
                    navigate(settingsSubTabPath("plan"));
                  }}
                >
                  Ajustar la inflación
                </a>
                .
              </div>
            ) : null}

            {/* U7 — como mucho TRES tarjetas, una cifra por tarjeta y el subtítulo COMPLETO: la
                base de la cifra vive ahí, y media base es peor que ninguna. */}
            {tiles.length > 0 ? (
              <div className="metric-grid retirement-tiles-grid">
                {tiles.map((t) => (
                  <MetricCard
                    key={t.key}
                    label={t.label}
                    helpId={t.helpId}
                    value={t.value}
                    parenthetical={t.subtitle}
                    tone={t.tone === "danger" ? "danger" : "default"}
                  />
                ))}
              </div>
            ) : null}

            {/* U5 — UN gráfico: patrimonio, objetivo, banda de escenarios y los hitos del plan,
                todos sobre el mismo eje y hasta el horizonte. Antes eran dos charts con ejes X
                distintos que el usuario tenía que emparejar a ojo. */}
            {chartReady ? (
              <div className="retirement-chart-block bordered-top">
                <div className="retirement-chart-toolbar">
                  <Switch
                    variant="chart"
                    label="En dinero de hoy"
                    checked={inflationAdjusted}
                    onChange={setInflationAdjusted}
                    ariaLabel="Leer el gráfico en euros de hoy"
                  />
                  {bandPoints && bandPoints.length > 1 ? (
                    <Switch
                      variant="chart"
                      label="Banda 10–90 %"
                      checked={showBand}
                      onChange={setShowBand}
                      ariaLabel="Mostrar la banda de escenarios con volatilidad"
                    />
                  ) : null}
                </div>
                <MiniProjection
                  series={projectionSeries}
                  height={260}
                  showFire={
                    !!projectionSeries?.fire_target_series &&
                    projectionSeries.fire_target_series.length > 0
                  }
                  /* El hito de jubilación lo dibujan las MARCAS (con su rótulo): dejar también
                     `showJub` pintaría dos líneas verticales en el mismo mes. */
                  showJub={false}
                  showPhases
                  showAreas={false}
                  zoomY
                  band={showBand ? bandPoints : null}
                  markers={chartMarkers}
                  deflator={chartDeflator}
                  xAxis={{
                    ageUiMode: axisAgeMode,
                    birthDateIso: axisBirth,
                    anchorDateYmd: axisAnchor,
                    calendarTz,
                  }}
                  /* V2 — el eje Y con importes: sin él no había forma de saber si la banda
                     valía 200.000 € o dos millones. Los valores ya vienen deflactados, así que
                     «En dinero de hoy» mueve el eje entero. */
                  yAxis={{ currencyIso }}
                  bandGradient={gradientStops}
                  bandEdgeLabels={{ p10: "pesimista (p10)", p90: "optimista (p90)" }}
                  hoverLabel={chartHoverLabel}
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
                    ...(projectionSeries?.fire_target_series &&
                    projectionSeries.fire_target_series.length > 0
                      ? ([
                          {
                            key: "fire",
                            label: "Objetivo FIRE",
                            color: "var(--proj-fire)",
                            swatch: "dashed",
                          },
                        ] as const)
                      : []),
                    /* Con degradado, la banda sale de la leyenda: su entrada tendría que
                       enseñar UN color y la banda ya no tiene uno. Lo que la explica es la
                       ESCALA de debajo, que no es una serie y por eso no es un ítem de
                       `ChartLegend`. */
                    ...(showBand &&
                    bandPoints &&
                    bandPoints.length > 1 &&
                    gradientStops.length < 2
                      ? ([
                          {
                            key: "band",
                            label: "Banda 10–90 %",
                            color: "var(--ff-accent)",
                            swatch: "area",
                          },
                        ] as const)
                      : []),
                    ...(chartMarkers.length > 0
                      ? ([
                          {
                            key: "marks",
                            label: "Hitos del plan",
                            color: "var(--ff-accent)",
                            swatch: "line",
                          },
                        ] as const)
                      : []),
                  ]}
                />
                {/* La ESCALA del color (V5). No es un ítem de `ChartLegend` a propósito: una
                    leyenda nombra SERIES, y esto es una escala continua — meterla ahí la haría
                    parecer una cuarta línea del gráfico. */}
                {gradientStops.length > 1 ? (
                  <p className="retirement-risk-scale">
                    <span className="label-with-help">
                      <strong>Banda 10–90 %</strong>
                      <HelpPopover
                        title={HELP_TEXTS["retirement.depletion_by_age"].title}
                        body={HELP_TEXTS["retirement.depletion_by_age"].body}
                      />
                    </span>{" "}
                    · el color dice qué parte de los escenarios se ha quedado ya sin capital a esa
                    edad:{" "}
                    {(
                      [
                        [0, "ninguno"],
                        [RISK_AMBER_AT, "5 %"],
                        [RISK_RED_AT, "10 % o más"],
                      ] as const
                    ).map(([p, label]) => (
                      <span key={label} className="retirement-risk-scale-step">
                        <span
                          className="retirement-risk-scale-swatch"
                          /* Color por custom property, el mismo patrón que `ChartLegend` usa
                             para su `--ff-legend-color`: el valor es un token (o una mezcla de
                             dos) y tiene que resolver por tema, así que no puede vivir en una
                             clase fija. */
                          style={
                            {
                              "--ff-risk-swatch": riskColorForProbability(p),
                            } as CSSProperties
                          }
                          aria-hidden
                        />
                        {label}
                      </span>
                    ))}
                  </p>
                ) : null}
              </div>
            ) : hasMembership ? (
              <div
                className="ff-chart-skeleton ff-chart-skeleton--mini bordered-top"
                aria-hidden
                style={{ minHeight: 260 }}
              />
            ) : null}

            {/* ── Riesgo, en compacto ───────────────────────────────────────────────────── */}
            <div className="retirement-risk-block bordered-top">
              <h4 className="panel-title">Riesgo</h4>
              {projectionBandsError ? (
                <div className="banner error-banner">{projectionBandsError}</div>
              ) : !projectionBands ? (
                projectionBandsBusy ? (
                  <p className="muted tight">Sorteando escenarios…</p>
                ) : (
                  <p className="muted tight">Aún no hay escenarios que mostrar.</p>
                )
              ) : showsNoVolatilityNotice(projectionBands) ? (
                /* Sin σ declarada las tres bandas SON la línea, y un abanico plano se lee como
                   certeza — la lectura más cara posible de esta pantalla. */
                <div className="banner info-banner">
                  Sin volatilidad declarada: la banda es la línea. Añade la volatilidad anual a
                  tus activos.{" "}
                  <a
                    href={appUrl(TAB_PATH.assets)}
                    onClick={(e) => {
                      if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                        return;
                      e.preventDefault();
                      navigate(TAB_PATH.assets);
                    }}
                  >
                    Ir a Activos
                  </a>
                  .
                </div>
              ) : (
                <>
                  <div className="metric-grid summary-success-grid">
                    <MetricCard
                      label="Éxito del plan"
                      helpId="retirement.success"
                      value={formatSuccessPercent(projectionBands.success_probability)}
                      parenthetical={successParenthetical(
                        projectionBands.success_probability,
                        projectionBands.paths,
                      )}
                      /* El veredicto lo decide el SERVIDOR (verde SOLO al 100 %, ámbar hasta
                         diez puntos por debajo, V7): aquí solo se traduce a la piel que la app
                         ya habla. Recalcularlo aquí es cómo el tile y el chart acaban contando
                         dos éxitos distintos del mismo plan. */
                      tone={(() => {
                        const t = successVerdictTone(projectionBands.success_verdict);
                        return t === "danger" ? "danger" : t === "warn" ? "warn" : "default";
                      })()}
                    />
                  </div>
                  {/* V6 — el colchón de caja ya no se PREGUNTA: se deriva del tope de tu regla
                      de ahorro y aquí se informa de dónde sale. Un valor derivado se rotula
                      como derivado, y con su salida cuando alguien lo ha fijado por API. */}
                  {bufferLine ? (
                    <p className="retirement-buffer-line">
                      <span className="label-with-help">
                        {bufferLine.text}
                        <HelpPopover
                          title={HELP_TEXTS["retirement.cash_buffer"].title}
                          body={HELP_TEXTS["retirement.cash_buffer"].body}
                        />
                      </span>
                      {bufferLine.linksToAllocationRules ? (
                        <a
                          href={appUrl(TAB_PATH.assets)}
                          onClick={(e) => {
                            if (e.button !== 0 || e.metaKey || e.altKey || e.ctrlKey || e.shiftKey)
                              return;
                            e.preventDefault();
                            navigate(TAB_PATH.assets);
                          }}
                        >
                          Cambiar en Reglas de ahorro
                        </a>
                      ) : null}
                      {bufferLine.canResetToDerived && canEditProfile ? (
                        // Tri-estado del PATCH: `null` SUELTA el override y devuelve la
                        // derivación. Sin esta salida, un colchón puesto por API sería
                        // irreversible desde la pantalla.
                        <button
                          type="button"
                          className="btn ghost text"
                          onClick={() =>
                            void onSaveRetirementProfile({ cash_buffer_months: null })
                          }
                        >
                          Volver al tope de tu regla
                        </button>
                      ) : null}
                    </p>
                  ) : null}
                </>
              )}
            </div>

            {/* ── «Detalle del cálculo» ─────────────────────────────────────────────────────
                No es un cajón de sastre: son las lecturas de SEGUNDO orden —las que matizan una
                cifra de arriba en vez de responder una pregunta propia— más los avisos. Que
                estén plegadas no las hace opcionales; que estén fuera de la cabecera es lo que
                permite leer la cabecera de un vistazo. */}
            {detailRows.length > 0 || riskExtraRows.length > 0 || projectionBands ? (
              <details className="retirement-detail bordered-top">
                <summary className="details-trigger">Detalle del cálculo</summary>
                <div className="risk-extra-rows">
                  {detailRows
                    .filter((r) => r.tone !== "danger")
                    .map((r) => (
                      <div key={r.key} className="risk-extra-row">
                        <div className="risk-extra-head">
                          <span
                            className={
                              r.key === "liquid_crossing"
                                ? "label-with-help risk-extra-label"
                                : "risk-extra-label"
                            }
                          >
                            {r.label}
                            {r.key === "liquid_crossing" ? (
                              <HelpPopover
                                title={HELP_TEXTS["retirement.crossing_reading"].title}
                                body={HELP_TEXTS["retirement.crossing_reading"].body}
                              />
                            ) : null}
                          </span>
                          <span className="risk-extra-value">{r.value}</span>
                        </div>
                      </div>
                    ))}
                  {riskExtraRows.map((r) => (
                    <div key={r.key} className="risk-extra-row">
                      <div className="risk-extra-head">
                        {/* La ayuda cuelga del RÓTULO, no del bloque: estas filas miden cosas
                            distintas y una sola ayuda arriba explicaría la que el usuario no
                            está mirando. */}
                        <span
                          className={
                            r.helpId ? "label-with-help risk-extra-label" : "risk-extra-label"
                          }
                        >
                          {r.label}
                          {r.helpId ? (
                            <HelpPopover
                              title={HELP_TEXTS[r.helpId].title}
                              body={HELP_TEXTS[r.helpId].body}
                            />
                          ) : null}
                        </span>
                        <span className="risk-extra-value">{r.value}</span>
                      </div>
                      {r.detail ? (
                        <span className="risk-extra-detail">{r.detail}</span>
                      ) : null}
                    </div>
                  ))}
                  {projectionBands ? (
                    <div className="risk-extra-row">
                      <div className="risk-extra-head">
                        <span className="label-with-help risk-extra-label">
                          Cómo leer la banda
                          <HelpPopover
                            title={HELP_TEXTS["retirement.bands"].title}
                            body={HELP_TEXTS["retirement.bands"].body}
                          />
                        </span>
                      </div>
                      <span className="risk-extra-detail">
                        Bandas puntuales: cada mes se ordena por separado, así que el borde de la
                        banda no es un futuro concreto.
                      </span>
                    </div>
                  ) : null}
                </div>
                {projectionBands ? (
                  <p className="risk-footnote">{riskFootnote(projectionBands)}</p>
                ) : null}
              </details>
            ) : null}
          </section>

        </>
      )}

      {hasMembership &&
      !projectionBusy &&
      !retirementBusy &&
      (!projectionSeries || !retirementBudgetSnapshot) ? (
        <div className="banner info-banner">Sin datos.</div>
      ) : null}
    </div>
  );
}
