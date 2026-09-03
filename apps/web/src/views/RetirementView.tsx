/**
 * Jubilación — **rediseño UX U1b** (5.0.0, issue #207, decisiones U1–U12 y S1–S11).
 *
 * La página tiene ahora TRES bloques y un acordeón, en este orden y sin excepciones:
 *
 *  1. **Cabecera**: el título y UN solo indicador de guardado (S6). Antes había seis pies
 *     «Guardado automático.», uno por panel, que podían contradecirse entre sí.
 *  2. **«Tu plan»** (configuración): el aviso de alta, las cinco tarjetas de estrategia y **solo
 *     los campos que la estrategia elegida usa** — la tabla U2 vive en `lib/plan-fields.ts` y
 *     aquí no se re-decide nada. Un campo visible anuncia que la simulación lo va a mirar; en
 *     cuatro de las cinco estrategias eso era mentira para media pantalla.
 *  3. **«Resultado»**: la FRASE del plan (`lib/plan-sentence.ts`), el rojo cuando lo hay, como
 *     mucho tres tarjetas (`buildRetirementTilesV2`), **un solo gráfico** con el objetivo, la
 *     banda de escenarios y los hitos (U5), el riesgo en compacto y un «Detalle del cálculo»
 *     plegado con todo lo de segundo orden.
 *  4. **«Avanzado»**, plegado, cuya cabecera ES la línea de supuestos (U12): nada se fuerza en
 *     silencio. Esconder un campo (U2) sin enunciar su valor sería justo eso.
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
  parseDisplayDecimal,
} from "../lib/format";
import { savingsSourceUsesTransactions } from "../lib/fire";
import {
  BRIDGE_DISCOUNT_BASIS_LABEL,
  HORIZON_LIFESPAN_AGE_OPTIONS,
  MAX_CASH_BUFFER_MONTHS,
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
import { assumptionsLine } from "../lib/assumptions-line";
import {
  advancedGroupFields,
  planFieldsContextFromProfile,
  planGroupFields,
  requiredPlanFields,
  type PlanFieldDescriptorAdvanced,
  type PlanFieldDescriptorPlan,
  type PlanFieldId,
  type PlanFieldSection,
} from "../lib/plan-fields";
import {
  ADVANCED_SECTION_LABEL,
  PLAN_FIELD_HELP,
  fractionFromPercent,
  missingRequiredPlanFields,
  percentFromFraction,
  saveIndicatorLabel,
  withdrawalPctNote,
} from "../lib/retirement-form";
import { buildRetirementChartMarkers } from "../lib/retirement-chart";
import {
  buildDepletionRows,
  buildRiskExtraRows,
  formatSuccessScenarios,
  formatSuccessThreshold,
  riskFootnote,
  showsNoVolatilityNotice,
  successVerdictTone,
} from "../lib/risk-bands";
import { type LedgerPersonScope } from "../lib/ledger";
import {
  persistRetirementIntroDismissed,
  readRetirementIntroDismissed,
} from "../lib/retirement-intro";
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
   * Aviso de alta (D33), una sola vez por navegador. Vive en estado local además de en
   * `localStorage` para que el descarte sea inmediato aunque el almacenamiento esté bloqueado.
   */
  const [introDismissed, setIntroDismissed] = useState<boolean>(() =>
    readRetirementIntroDismissed(),
  );

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

  // ── Riesgo compacto ───────────────────────────────────────────────────────────────────────
  const depletionRows = useMemo(
    () =>
      buildDepletionRows(
        projectionBands?.depletion_probability_by_age,
        projectionBands?.months,
      ),
    [projectionBands?.depletion_probability_by_age, projectionBands?.months],
  );
  const riskExtraRows = useMemo(
    () => buildRiskExtraRows({ bands: projectionBands, currencyIso, monthLabel }),
    [projectionBands, currencyIso, monthLabel],
  );

  // ── Hogar (U10): frases por miembro y nada más ────────────────────────────────────────────
  const memberLines = useMemo(
    () => householdPlanLines(projectionSeries?.members, monthLabel),
    [projectionSeries?.members, monthLabel],
  );

  // ── «Avanzado» ────────────────────────────────────────────────────────────────────────────
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const advancedRef = useRef<HTMLDetailsElement>(null);
  const openAdvanced = useCallback(() => {
    setAdvancedOpen(true);
    // El `scrollIntoView` va en el siguiente frame: con el `<details>` todavía cerrado, el
    // navegador mediría la altura plegada y dejaría la sección a media pantalla.
    window.requestAnimationFrame(() =>
      advancedRef.current?.scrollIntoView({ behavior: "smooth", block: "start" }),
    );
  }, []);

  const assumptions = useMemo(
    () =>
      assumptionsLine(profileDraft, {
        hasBirthDate,
        targetBasisSource: basisSource,
      }),
    [profileDraft, hasBirthDate, basisSource],
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
  // Campos del grupo «plan» — renderizados POR ID, en el orden que dicta `planGroupFields`
  // ═══════════════════════════════════════════════════════════════════════════════════════════
  const renderPlanField = (f: PlanFieldDescriptorPlan): ReactNode => {
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
            <span className="field-label-text">Gasto en jubilación</span>
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

  // ═══════════════════════════════════════════════════════════════════════════════════════════
  // Campos del grupo «avanzado»
  // ═══════════════════════════════════════════════════════════════════════════════════════════
  const renderAdvancedField = (f: PlanFieldDescriptorAdvanced): ReactNode => {
    switch (f.id) {
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
              Es el ÚNICO porcentaje de retirada: dimensiona tu objetivo y es el que retira la
              regla de abajo.
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

      case "cash_buffer_months":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (meses)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              placeholder="—"
              value={intFieldValue(profileDraft.cash_buffer_months)}
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined) return;
                if (v !== null && v > MAX_CASH_BUFFER_MONTHS) return;
                patchDraft((p) => ({ ...p, cash_buffer_months: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            {/* Un colchón guardado que la simulación IGNORA es la peor combinación posible: el
                usuario cree estar protegido por algo que no corrió. La razón la publica el
                servidor con las bandas, así que se dice aquí, junto al campo que la provoca. */}
            {projectionBands &&
            !projectionBands.buffer_active &&
            projectionBands.buffer_inactive_reason === "no_safe_liquid_asset" ? (
              <small className="muted">
                No se está simulando: no tienes un activo líquido sin volatilidad donde vivir.
              </small>
            ) : null}
          </label>
        );

      case "success_threshold_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (%)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              value={String(profileDraft.success_threshold_pct)}
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined || v === null) return;
                patchDraft((p) => ({ ...p, success_threshold_pct: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
          </label>
        );

      default:
        return null;
    }
  };

  const editingPlan = canEditProfile && retirementProfile != null;
  const planFieldList = editingPlan ? planGroupFields(fieldCtx) : [];
  /** Las secciones salen contiguas de `planFields`, así que agrupar es solo recorrer. */
  const advancedSections = useMemo(() => {
    const out: { section: PlanFieldSection; fields: PlanFieldDescriptorAdvanced[] }[] = [];
    if (!editingPlan) return out;
    for (const f of advancedGroupFields(fieldCtx)) {
      const last = out[out.length - 1];
      if (last && last.section === f.section) last.fields.push(f);
      else out.push({ section: f.section, fields: [f] });
    }
    return out;
  }, [editingPlan, fieldCtx]);

  const showIntroBanner = canEditProfile && !introDismissed;
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
          {/* ── 2 · «Tu plan» ─────────────────────────────────────────────────────────────── */}
          <section className="panel">
            <div className="panel-head-row">
              <h3 className="panel-title">Tu plan</h3>
              <HelpPopover
                title={HELP_TEXTS["retirement.strategy"].title}
                body={HELP_TEXTS["retirement.strategy"].body}
              />
            </div>

            {/* El aviso de alta (D33) vive AQUÍ, justo encima de las tarjetas que nombra: antes
                estaba en lo alto de la página y apuntaba a un formulario dos pantallas abajo. */}
            {showIntroBanner ? (
              <div className="banner info-banner retirement-intro-banner" role="status">
                <div className="retirement-intro-banner-text">
                  <strong>Elige tu estrategia de jubilación</strong>
                  <small>
                    Cada una decide QUÉ dispara tu jubilación, y con ello qué te preguntamos
                    debajo.
                  </small>
                </div>
                <button
                  type="button"
                  className="btn ghost"
                  onClick={() => {
                    setIntroDismissed(true);
                    persistRetirementIntroDismissed();
                  }}
                >
                  Entendido
                </button>
              </div>
            ) : null}

            {retirementProfile == null ? (
              <p className="muted tight bordered-top">
                {retirementProfileBusy ? "Cargando…" : "Sin datos."}
              </p>
            ) : (
              <div className="stack bordered-top retirement-config-stack">
                <div
                  className="retirement-mode-grid retirement-strategy-grid"
                  role="radiogroup"
                  aria-label="Estrategia de jubilación"
                >
                  {RETIREMENT_STRATEGIES.map((s) => (
                    <label
                      key={s}
                      className={`retirement-mode-card ${
                        profileDraft.strategy === s ? "is-active" : ""
                      }`}
                    >
                      <input
                        type="radio"
                        name="retirement_strategy"
                        className="sr-only"
                        checked={profileDraft.strategy === s}
                        onChange={() => selectStrategy(s)}
                      />
                      <span className="retirement-mode-name">
                        {RETIREMENT_STRATEGY_LABEL[s]}
                      </span>
                      <span className="retirement-mode-sub">
                        {RETIREMENT_STRATEGY_BLURB[s]}
                      </span>
                    </label>
                  ))}
                </div>

                {/* U2: SOLO los campos que la estrategia elegida usa, en el orden de la tabla. */}
                {planFieldList.map((f) => renderPlanField(f))}

                <button
                  type="button"
                  className="btn ghost text retirement-advanced-link"
                  onClick={openAdvanced}
                >
                  Supuestos y ajustes avanzados ↓
                </button>
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
                    ...(showBand && bandPoints && bandPoints.length > 1
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
              <h4 className="subsection-title">Riesgo</h4>
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
                      value={formatSuccessScenarios(projectionBands.success_probability)}
                      parenthetical={formatSuccessThreshold(
                        projectionBands.success_threshold_pct,
                      )}
                      /* El veredicto lo decide el SERVIDOR (umbral exacto y diez puntos de
                         margen, D28): aquí solo se traduce a la piel que la app ya habla. */
                      tone={(() => {
                        const t = successVerdictTone(projectionBands.success_verdict);
                        return t === "danger" ? "danger" : t === "warn" ? "warn" : "default";
                      })()}
                    />
                  </div>
                  {depletionRows.length > 0 ? (
                    <div className="risk-extra-rows">
                      <div className="risk-extra-row">
                        <span className="label-with-help risk-extra-label">
                          Probabilidad de agotar el capital
                          <HelpPopover
                            title={HELP_TEXTS["retirement.depletion_by_age"].title}
                            body={HELP_TEXTS["retirement.depletion_by_age"].body}
                          />
                        </span>
                        <div className="risk-depletion-grid">
                          {depletionRows.map((r) => (
                            <div key={r.key} className="risk-depletion-cell">
                              <span className="risk-depletion-age">{r.label}</span>
                              <span className="risk-depletion-value">{r.value}</span>
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>
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
                <summary>Detalle del cálculo</summary>
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

          {/* ── 4 · «Avanzado» ────────────────────────────────────────────────────────────────
              U12 — su cabecera ES la línea de supuestos, y está SIEMPRE visible aunque el
              acordeón esté plegado. U2 esconde los campos que la estrategia no usa; sin esto,
              esconder un campo sería forzar su valor en silencio. */}
          {retirementProfile != null ? (
            <details
              className="panel retirement-advanced"
              ref={advancedRef}
              open={advancedOpen}
              onToggle={(e) => setAdvancedOpen(e.currentTarget.open)}
            >
              <summary className="retirement-advanced-summary">
                <span className="retirement-advanced-summary-text">{assumptions}</span>
                <span className="retirement-advanced-summary-cta">Avanzado</span>
              </summary>
              <div className="retirement-advanced-body">
                <p className="muted tight">
                  <span className="label-with-help">
                    Todo esto ya está en tu plan, lo veas o no.
                    <HelpPopover
                      title={HELP_TEXTS["retirement.assumptions"].title}
                      body={HELP_TEXTS["retirement.assumptions"].body}
                    />
                  </span>
                </p>
                {advancedSections.map(({ section, fields }) => (
                  <section key={section} className="retirement-advanced-section">
                    <h4 className="subsection-title">{ADVANCED_SECTION_LABEL[section]}</h4>
                    <div className="stack retirement-config-stack">
                      {fields.map((f) => renderAdvancedField(f))}
                    </div>
                  </section>
                ))}
              </div>
            </details>
          ) : null}
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
