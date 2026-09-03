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
  ProjectionSeriesApi,
  RetirementProfileApi,
  RetirementProfilePatchApi,
  RetirementStrategyApi,
  SummaryResponse,
  UserResponse,
  WithdrawalRuleKindApi,
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
  grossUpNetAnnualFire,
  normalizeInstallationFireSettings,
  savingsSourceUsesTransactions,
} from "../lib/fire";
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
  strategyRequiresTargetAge,
  targetBasisSource,
} from "../lib/retirementProfile";
import { messageForError } from "../lib/errorMessages";
import { type LedgerPersonScope } from "../lib/ledger";
import {
  persistRetirementIntroDismissed,
  readRetirementIntroDismissed,
} from "../lib/retirement-intro";
import { settingsSubTabPath } from "../lib/navigation";
import { appUrl } from "../lib/basePath";
import {
  complementaryProjectionTickLabel,
  formatYearsEsFromMonths,
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

/** Un decimal tecleado por el usuario, listo para el wire: coma española → punto. */
function typedDecimal(raw: string): string {
  return raw.replace(",", ".");
}

/** Un decimal opcional: vacío = «no hay valor», que en el perfil es `null`, no `0`. */
function typedDecimalOrNull(raw: string): string | null {
  const t = raw.trim();
  return t === "" ? null : typedDecimal(raw);
}

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
  retirementProfile,
  retirementProfileBusy,
  retirementProfileError,
  retirementProfileSaving,
  user,
  calendarTz,
  scopeReadOnly,
  onSaveRetirementProfile,
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
  navigate: (path: string, replace?: boolean) => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  /**
   * Aviso de alta (D33), una sola vez por navegador. Vive en estado local además de en
   * `localStorage` para que el descarte sea inmediato aunque el almacenamiento esté bloqueado.
   */
  const [introDismissed, setIntroDismissed] = useState<boolean>(() =>
    readRetirementIntroDismissed(),
  );
  const birthDateMissing = !user?.birth_date?.trim();
  const showIntroBanner = hasMembership && !scopeReadOnly && !introDismissed;

  /**
   * El plan de jubilación es dato PERSONAL: lo edita cualquier rol, `viewer` incluido (así lo
   * acepta `patch_retirement_profile_core`). Lo único que lo bloquea es la vista Hogar, que es
   * un agregado de N personas y no tiene un perfil al que atribuir el cambio.
   */
  const canEditProfile = hasMembership && !scopeReadOnly;

  // ── Borrador del perfil y su autoguardado ─────────────────────────────────────────────────
  const [profileDraft, setProfileDraft] = useState<RetirementProfileApi>(() =>
    defaultRetirementProfileApi(),
  );
  /**
   * Lo que el servidor tiene AHORA MISMO, y por tanto la base contra la que se calcula el PATCH
   * mínimo. Es un ref y no estado porque solo lo lee el guardado, nunca la pintada.
   */
  const syncedProfileRef = useRef<RetirementProfileApi>(defaultRetirementProfileApi());
  /** Aviso cuando el guardado automático se salta un cambio por datos inválidos. */
  const [profileIssue, setProfileIssue] = useState<string | null>(null);
  const profileSaveTimerRef = useRef(0);
  const profileSaveSeqRef = useRef(0);
  const skipProfileAutosaveRef = useRef(true);
  /** `null` mientras no se ha inicializado el borrador con lo que trajo el servidor. */
  const profileInitializedRef = useRef<RetirementProfileApi | null>(null);

  useEffect(() => {
    if (!retirementProfile) {
      // Cierre de sesión o cambio de hogar: el siguiente perfil que llegue vuelve a inicializar.
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
  /** Hay cambios sin guardar (se usa para rotular la vista previa del objetivo). */
  const profileDirty =
    retirementProfile != null &&
    !isEmptyRetirementProfilePatch(
      buildRetirementProfilePatch(savedProfile, profileDraft),
    );

  const runProfileSave = useCallback(() => {
    if (!canEditProfile) return;
    const patch = buildRetirementProfilePatch(syncedProfileRef.current, profileDraft);
    if (isEmptyRetirementProfilePatch(patch)) {
      setProfileIssue(null);
      return;
    }
    // La guarda de validez habla con los MISMOS códigos que el servidor, así que la frase sale
    // del catálogo único. Sin ella el pie del panel seguiría prometiendo «Guardado automático»
    // mientras el PATCH se estrella contra un 400 y el cambio se pierde en silencio.
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
        // `saved.target_basis` es la elección ALMACENADA (`target_basis_stored`, `null` =
        // derivada): App.tsx la sustituye al recibir la respuesta. Resincronizar el borrador con
        // ella lo mantiene alineado con lo que el servidor tiene guardado, sin convertir en
        // elección explícita lo que sigue siendo una derivación.
        setProfileDraft((d) =>
          d.target_basis === saved.target_basis
            ? d
            : { ...d, target_basis: saved.target_basis },
        );
      })
      .catch(() => {
        // El banner lo pinta App.tsx (`saveRetirementProfilePatch` rellena su error antes de
        // relanzar). Aquí solo hay que NO marcar como guardado.
      });
  }, [profileDraft, canEditProfile, onSaveRetirementProfile]);

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
    // Sin nada que guardar no se arma el temporizador. Este efecto se re-ejecuta en CADA
    // re-render (el callback de guardado es una función nueva por render, como el resto de los
    // `onSave*` de `App.tsx`), y sin esta salida temprana un flujo de re-renders ajenos —la
    // serie de proyección llegando, por ejemplo— reiniciaría el debounce una y otra vez.
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

  /** Atajo para editar un campo del borrador (todo el formulario autosalva). */
  const patchDraft = useCallback(
    (fn: (p: RetirementProfileApi) => RetirementProfileApi) => {
      setProfileDraft((prev) => fn(prev));
    },
    [],
  );

  // ── Ejes del hogar que siguen alimentando la vista previa del objetivo ────────────────────
  const houseFire = useMemo(
    () => normalizeInstallationFireSettings(installation?.installation.fire_settings),
    [installation?.installation.fire_settings],
  );

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

  // Vista previa LOCAL del objetivo con los ajustes del draft (sin guardar). El modo y el SWR
  // salen del PERFIL (5.0.0); la fiscalidad sigue siendo del hogar. `computeFireAnnualNeedNetEur`
  // y `grossUpNetAnnualFire` siguen duplicados en cliente solo para esto: el cruce (mes, fecha,
  // objetivo al cruce) ya no se recalcula aquí — lee siempre del servidor.
  const firePreview = useMemo(() => {
    const expenseM = fireExpenseM;
    const incomeM = fireIncomeM;
    const incomeRetM =
      retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent;
    const needAnnual = computeFireAnnualNeedNetEur(
      {
        fire_number_mode: profileDraft.fire_number_mode,
        fire_number_manual_amount: profileDraft.fire_number_manual_amount,
      },
      expenseM,
      incomeM,
      incomeRetM,
    );
    const swrN = parseDisplayDecimal(profileDraft.swr_pct);
    const brackets = houseFire.tax_brackets;
    const taxOn = houseFire.taxes_enabled;

    let targetNoPen: number | null = null;
    if (needAnnual !== null && needAnnual > 0 && swrN !== null && swrN > 0) {
      const grossNoPen = grossUpNetAnnualFire(
        needAnnual,
        brackets,
        taxOn,
        Number(houseFire.taxable_gain_ratio ?? "1"),
      );
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
    profileDraft.fire_number_mode,
    profileDraft.fire_number_manual_amount,
    profileDraft.swr_pct,
    houseFire,
    fireExpenseM,
    fireIncomeM,
    retirementBudgetSnapshot?.totals.income_retirement_monthly_equivalent,
    projectionSeries?.fire_target_debt_component,
  ]);

  // Lecturas del servidor — SIEMPRE, nunca recalculadas en cliente: el cruce depende de la
  // simulación mensual completa y el cliente no puede rehacerla (ni debe fingirlo).
  const jubMi =
    typeof projectionSeries?.jubilacion_month_index === "number"
      ? projectionSeries.jubilacion_month_index
      : null;
  const mc = projectionSeries?.months ?? 0;
  /**
   * El CRUCE del objetivo, que desde 5.0.0 es una LECTURA y ya no el trigger (R8/D17): el mes en
   * que el líquido habría bastado. Con `asap` coincide con `jubMi` por construcción, así que solo
   * se enseña cuando manda la edad (`retirement_trigger === "target_age"`), donde los dos meses
   * pueden separarse en cualquier dirección: te jubilas sin llegar (cruce después) o podrías
   * haberte ido antes (cruce antes). `null` con razón declarada = la pregunta no aplica; `null`
   * sin razón = hay objetivo y no se cruza en el horizonte, que es un resultado.
   */
  const crossingMi =
    typeof projectionSeries?.liquid_crossing_month_index === "number"
      ? projectionSeries.liquid_crossing_month_index
      : null;
  const crossingIsSeparateReading =
    projectionSeries?.retirement_trigger === "target_age";
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
  const targetToday = profileDirty ? firePreview.targetNoPen : serverTargetToday;
  const targetTodayReady =
    retirementMetricsReady && (profileDirty ? firePreviewReady : true);

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
      String(profileDraft.fire_number_manual_amount ?? ""),
    );
    if (!(m !== null && m > 0)) return METRIC_DASH;
    return renderRetirementAmount(m, m / 12);
  }, [profileDraft.fire_number_manual_amount, renderRetirementAmount]);

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

  const lblOpts = {
    birthDateIso: axisBirth,
    anchorDateYmd: axisAnchor,
    calendarTz,
  };

  const strategy = profileDraft.strategy;
  const basis = effectiveTargetBasis(profileDraft);
  /**
   * De dónde sale la base marcada en el radio. `derived` = nadie la ha elegido y el servidor la
   * deriva (R6: puente si hay pensión declarada, perpetuidad si no), así que se rotula
   * «(derivada)» y el radio marcado NO es una decisión del usuario: sigue moviéndose sola si
   * mañana declara una pensión. `stored` = está fijada a mano, y entonces se ofrece soltarla.
   */
  const basisSource = targetBasisSource(profileDraft);
  const rule = profileDraft.withdrawal_rule;
  const pension = profileDraft.pension;
  const partial = profileDraft.partial_retirement;
  /** El pie de panel que dice si hay algo en vuelo. Mismo literal en los cinco paneles. */
  const saveFooter = retirementProfileSaving ? "Guardando…" : "Guardado automático.";

  /**
   * Elegir estrategia SIEMBRA el bloque que esa estrategia necesita para poder rellenarse. La
   * excepción es la edad objetivo: el servidor la exige y no hay ninguna edad que podamos
   * inventar por el usuario, así que se deja vacía y el aviso lo dice.
   */
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

  /** Campo numérico entero opcional: vacío borra, cualquier entero se acepta y lo juzga la guarda. */
  const intFieldValue = (v: number | null) => (v == null ? "" : String(v));
  const readIntField = (raw: string): number | null | undefined => {
    const t = raw.trim();
    if (t === "") return null;
    const n = Number(t);
    // `undefined` = «no es un entero, ignora la pulsación»: es el patrón de la casa para no
    // dejar un número a medio teclear dentro del borrador que autosalva.
    if (!Number.isInteger(n) || n < 0 || n > 200) return undefined;
    return n;
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

      {showIntroBanner ? (
        <div className="banner info-banner retirement-intro-banner" role="status">
          <div className="retirement-intro-banner-text">
            <strong>Elige tu estrategia de jubilación</strong>
            {birthDateMissing ? (
              <small>
                Añade tu fecha de nacimiento en «Tu cuenta» para las estrategias
                por edad.{" "}
                <a
                  href={appUrl(settingsSubTabPath("general"))}
                  onClick={(e) => {
                    if (
                      e.button !== 0 ||
                      e.metaKey ||
                      e.altKey ||
                      e.ctrlKey ||
                      e.shiftKey
                    )
                      return;
                    e.preventDefault();
                    navigate(settingsSubTabPath("general"));
                  }}
                >
                  Ir a Tu cuenta
                </a>
              </small>
            ) : null}
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

      {retirementProfileError ? (
        <div className="banner error-banner">{retirementProfileError}</div>
      ) : null}

      {profileIssue ? (
        <div className="banner error-banner" role="alert">
          {profileIssue}
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
                  : profileDirty
                    ? "vista previa · sin guardar"
                    : targetAtCrossNominal !== null && targetAtCrossNominal > 0
                      ? `${formatCurrencyNumber(targetAtCrossNominal, currencyIso)} al cruce`
                      : undefined
              }
            />
            {/* 5.0.0 (R8): esta tarjeta ya NO es «el primer cruce». Es la jubilación EFECTIVA
                —el mes en que el motor te jubila de verdad, sea por cruce o por edad—, que es
                el mes que marcan el chart y el Resumen. El cruce del objetivo pasa a ser una
                lectura y solo aparece debajo cuando manda la edad y los dos meses difieren:
                con «Cuanto antes» son el mismo instante y rotularlo dos veces sugeriría dos
                hechos donde hay uno. */}
            <MetricCard
              label="Jubilación"
              helpId="retirement.crossing_reading"
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
              detail={
                retirementMetricsReady && crossingIsSeparateReading && mc > 0
                  ? crossingMi !== null
                    ? `Cruce del objetivo: ${projectionXTickLabel(crossingMi, mc, {
                        ageUiMode: axisAgeMode,
                        birthDateIso: axisBirth,
                        anchorDateYmd: axisAnchor,
                        calendarTz,
                      })}`
                    : "Cruce del objetivo: no cruza en el horizonte"
                  : undefined
              }
            />
            <MetricCard
              label="Años hasta la jubilación"
              value={
                retirementMetricsReady && jubMi !== null
                  ? formatYearsEsFromMonths(jubMi)
                  : METRIC_DASH
              }
            />
            {/* D31 — el margen es un TILE y nada más (ni área en el chart ni acción). La cifra
                llega con los campos de Monte Carlo y los solves del servidor; hasta entonces la
                tarjeta existe con un guion y la ayuda explica qué enseñará, en vez de aparecer
                de la nada en la versión siguiente. Solo tiene sentido con una edad objetivo:
                en «Cuanto antes» todo el ahorro va al objetivo por definición. */}
            {strategy !== "asap" ? (
              <MetricCard
                label="Margen disponible"
                helpId="retirement.disposable"
                value={METRIC_DASH}
                parenthetical="aún no se calcula"
              />
            ) : null}
          </div>
          {retirementMetricsReady && projectionSeries?.fire_target_absent_reason ? (
            <p className="muted tight">
              {FIRE_TARGET_ABSENT_REASON_ES[projectionSeries.fire_target_absent_reason] ??
                "No se calcula fecha de cruce."}
            </p>
          ) : null}
        </>
      ) : null}

      {hasMembership &&
      projectionSeries &&
      projectionSeries.points.length > 0 ? (
        (() => {
          const horizon = projectionSeries.horizon_years;
          const jubMiChart =
            typeof projectionSeries.jubilacion_month_index === "number"
              ? projectionSeries.jubilacion_month_index
              : null;
          const jubLabel =
            jubMiChart != null
              ? // Meses del horizonte, no puntos del array: con `density=hybrid` `pts.length`
                // (~82) no es el número de meses y la etiqueta relativa elegía «m» donde tocaba «a».
                projectionXTickLabel(jubMiChart, projectionSeries.months, {
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
          const alreadyRetired = jubMiChart === 0;
          // Si hay jubilación FUTURA, recortamos la serie a jub+12 (un año después del cruce). El
          // eje Y se zoom-ajusta entre NW(hoy) y NW(fin).
          const clampToMonth =
            jubMiChart != null && !alreadyRetired ? jubMiChart + 12 : null;
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
                  {/* Solo la VENTANA, sin cifras a propósito: el rango numérico (patrimonio
                      hoy → patrimonio al año del cruce) se leía como un tercer objetivo junto
                      a los dos de la tarjeta. El chart recorta a cruce+12 igualmente; la
                      etiqueta no menciona ese año de padding. */}
                  {clampToMonth != null
                    ? "de hoy a la jubilación"
                    : `${horizon} a`}
                </span>
              </div>
              <MiniProjection
                series={projectionSeries}
                height={240}
                showFire={hasFire}
                showJub={true}
                /* D29 reducida: la banda de fases bajo el plot. Aquí sí (es la vista del plan);
                   en el mini de 12 meses del Resumen no, porque en un año no cambia la fase. */
                showPhases
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
                  ...(jubMiChart != null
                    ? ([
                        {
                          key: "jub",
                          // R8: la línea marca la jubilación EFECTIVA (cruce o edad), no «el
                          // primer cruce» — con una estrategia por edad puede no haber cruce
                          // ninguno y la línea sigue estando donde el motor te jubila.
                          label: `Jubilación · ${jubLabel ?? ""}`.trim(),
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
              ) : jubMiChart == null ? (
                <p
                  className="muted tight"
                  style={{ marginTop: "0.6rem", fontSize: "0.78rem" }}
                >
                  Sin jubilación dentro del horizonte ({horizon} a). Aumenta el
                  horizonte o ajusta el aporte mensual.
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

      {/* D9/D32: en la vista Hogar los formularios del plan NO se enseñan deshabilitados, se
          ocultan. Un formulario gris con los números de un agregado de N personas invita a
          teclear en él y no dice de quién es lo que se ve. */}
      {!hasMembership ? null : scopeReadOnly ? (
        <section className="panel muted-panel">
          <h3 className="panel-title">Tu plan de jubilación</h3>
          <p className="muted tight">
            Solo lectura. Cambia a la vista «Yo» para editar tu estrategia, tu
            objetivo y tu retirada.
          </p>
        </section>
      ) : retirementProfile == null ? (
        <section className="panel muted-panel">
          <h3 className="panel-title">Tu plan de jubilación</h3>
          <p className="muted tight">
            {retirementProfileBusy ? "Cargando…" : "Sin datos."}
          </p>
        </section>
      ) : (
        <>
          {/* ── 1. Estrategia (D26): cinco tarjetas + solo los campos de la elegida ────── */}
          <section className="panel">
            <div className="panel-head-row">
              <h3 className="panel-title">Tu estrategia</h3>
              <HelpPopover
                title={HELP_TEXTS["retirement.strategy"].title}
                body={HELP_TEXTS["retirement.strategy"].body}
              />
            </div>
            <div className="stack bordered-top retirement-config-stack">
              <div
                className="retirement-mode-grid retirement-strategy-grid"
                role="radiogroup"
                aria-label="Estrategia de jubilación"
              >
                {RETIREMENT_STRATEGIES.map((s) => (
                  <label
                    key={s}
                    className={`retirement-mode-card ${strategy === s ? "is-active" : ""}`}
                  >
                    <input
                      type="radio"
                      name="retirement_strategy"
                      className="sr-only"
                      checked={strategy === s}
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

              {/* Edad objetivo: obligatoria en las dos estrategias por edad, opcional en media
                  jornada (donde marca el fin de la fase parcial). En «Cuanto antes» y en el
                  puente NO se enseña: esas se disparan por cruce y una edad ahí no haría nada. */}
              {strategyRequiresTargetAge(strategy) || strategy === "partial" ? (
                <label className="field">
                  <span className="label-with-help">
                    {strategy === "partial"
                      ? "Edad de jubilación total (opc.)"
                      : "Edad de jubilación objetivo"}
                    <HelpPopover
                      title={HELP_TEXTS["retirement.target_age"].title}
                      body={HELP_TEXTS["retirement.target_age"].body}
                    />
                  </span>
                  <input
                    inputMode="numeric"
                    value={intFieldValue(profileDraft.target_retirement_age)}
                    placeholder={strategy === "partial" ? "—" : "p. ej. 60"}
                    onChange={(e) => {
                      const v = readIntField(e.target.value);
                      if (v === undefined) return;
                      patchDraft((p) => ({ ...p, target_retirement_age: v }));
                    }}
                    onBlur={() => queueProfileSave(0)}
                  />
                  {strategyRequiresTargetAge(strategy) &&
                  profileDraft.target_retirement_age == null ? (
                    <small className="muted">
                      {messageForError("target_retirement_age_required", null)} Sin
                      guardar.
                    </small>
                  ) : birthDateMissing ? (
                    <small className="muted">
                      Sin tu fecha de nacimiento esta edad no se puede convertir en un mes:
                      añádela en «Tu cuenta».
                    </small>
                  ) : null}
                </label>
              ) : null}

              {/* Media jornada: la fase intermedia con su ingreso y su base de gasto. */}
              {strategy === "partial" && partial ? (
                <>
                  <div className="field-row">
                    <label className="field">
                      <span className="label-with-help">
                        Media jornada: edad de inicio
                        <HelpPopover
                          title={HELP_TEXTS["retirement.partial"].title}
                          body={HELP_TEXTS["retirement.partial"].body}
                        />
                      </span>
                      <input
                        inputMode="numeric"
                        value={String(partial.starts_at_age)}
                        onChange={(e) => {
                          const v = readIntField(e.target.value);
                          if (v === undefined || v === null) return;
                          patchDraft((p) =>
                            p.partial_retirement
                              ? {
                                  ...p,
                                  partial_retirement: {
                                    ...p.partial_retirement,
                                    starts_at_age: v,
                                  },
                                }
                              : p,
                          );
                        }}
                        onBlur={() => queueProfileSave(0)}
                      />
                    </label>
                    <label className="field">
                      <span>Ingreso mensual en la fase</span>
                      <input
                        inputMode="decimal"
                        placeholder="0"
                        value={partial.income_monthly_today}
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
                    </label>
                  </div>
                  <label className="field">
                    <span>Gasto durante la media jornada</span>
                    <select
                      value={partial.expense_basis}
                      onChange={(e) =>
                        patchDraft((p) =>
                          p.partial_retirement
                            ? {
                                ...p,
                                partial_retirement: {
                                  ...p.partial_retirement,
                                  expense_basis:
                                    e.target.value === "regular"
                                      ? "regular"
                                      : "retirement",
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
                </>
              ) : null}
              <p className="muted tight">{saveFooter}</p>
            </div>
          </section>

          {/* ── 2. Pensión: opcional siempre, obligatoria en el puente ──────────────────── */}
          <section className="panel">
            <div className="panel-head-row">
              <h3 className="panel-title">Pensión pública</h3>
              <HelpPopover
                title={HELP_TEXTS["retirement.pension"].title}
                body={HELP_TEXTS["retirement.pension"].body}
              />
            </div>
            <div className="stack bordered-top retirement-config-stack">
              <label className="field checkbox-field">
                <input
                  type="checkbox"
                  checked={pension != null}
                  // El puente ES la pensión: quitarla dejaría la estrategia sin su dato y el
                  // servidor rechazaría el PATCH. Se bloquea aquí en vez de dejar que el
                  // usuario descubra el error después de haber borrado sus cifras.
                  disabled={strategy === "pension_bridge" && pension != null}
                  onChange={(e) =>
                    patchDraft((p) => ({
                      ...p,
                      pension: e.target.checked ? newPensionPlanDraft() : null,
                    }))
                  }
                />
                <span>Cuento con una pensión</span>
              </label>
              {strategy === "pension_bridge" ? (
                <small className="muted">
                  «Puente hasta la pensión» la necesita: el objetivo se dimensiona con los años
                  que van de tu jubilación a la primera paga.
                </small>
              ) : null}

              {pension ? (
                <>
                  <div className="field-row">
                    <label className="field">
                      <span>Importe mensual (euros de hoy)</span>
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
                  <label className="field checkbox-field">
                    <input
                      type="checkbox"
                      checked={pension.indexed}
                      onChange={(e) =>
                        setPension((p) => ({ ...p, indexed: e.target.checked }))
                      }
                    />
                    <span>Sube cada año con la inflación</span>
                  </label>
                  {/* Solo con media jornada declarada: fuera de esa fase la fracción no se
                      aplica nunca, y un campo que no hace nada es peor que no estar. */}
                  {partial ? (
                    <label className="field">
                      <span>Parte que cobras en media jornada (0 a 1)</span>
                      <input
                        inputMode="decimal"
                        placeholder="0"
                        value={pension.fraction_while_partial}
                        onChange={(e) =>
                          setPension((p) => ({
                            ...p,
                            fraction_while_partial: typedDecimal(e.target.value),
                          }))
                        }
                        onBlur={() => queueProfileSave(0)}
                      />
                    </label>
                  ) : null}
                </>
              ) : null}
              <p className="muted tight">{saveFooter}</p>
            </div>
          </section>

          {/* ── 3. Objetivo anual + base del objetivo ───────────────────────────────────── */}
          <section className="panel">
            <h3 className="panel-title">
              Objetivo anual <span className="muted">(en dinero de hoy)</span>
            </h3>
            <div className="stack bordered-top retirement-config-stack">
              <div
                className="retirement-mode-grid"
                role="radiogroup"
                aria-label="Modo objetivo anual"
              >
                <label
                  className={`retirement-mode-card ${
                    profileDraft.fire_number_mode === "manual" ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="fire_mode"
                    className="sr-only"
                    checked={profileDraft.fire_number_mode === "manual"}
                    onChange={() =>
                      patchDraft((p) => ({ ...p, fire_number_mode: "manual" }))
                    }
                  />
                  <span className="retirement-mode-name">Manual</span>
                  <span className="retirement-mode-sub retirement-mode-amount">
                    {retirementObjectiveManualAnnualDisplay}
                  </span>
                </label>
                <label
                  className={`retirement-mode-card ${
                    profileDraft.fire_number_mode === "annual_expense" ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="fire_mode"
                    className="sr-only"
                    checked={profileDraft.fire_number_mode === "annual_expense"}
                    onChange={() =>
                      patchDraft((p) => ({ ...p, fire_number_mode: "annual_expense" }))
                    }
                  />
                  <span className="retirement-mode-name">Gasto actual</span>
                  <span className="retirement-mode-sub retirement-mode-amount">
                    {retirementObjectiveExpenseAnnualDisplay}
                  </span>
                </label>
                <label
                  className={`retirement-mode-card ${
                    profileDraft.fire_number_mode === "current_income" ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="fire_mode"
                    className="sr-only"
                    checked={profileDraft.fire_number_mode === "current_income"}
                    onChange={() =>
                      patchDraft((p) => ({ ...p, fire_number_mode: "current_income" }))
                    }
                  />
                  <span className="retirement-mode-name">Ingresos actuales</span>
                  <span className="retirement-mode-sub retirement-mode-amount">
                    {retirementObjectiveIncomeAnnualDisplay}
                  </span>
                </label>
              </div>

              {profileDraft.fire_number_mode === "manual" ? (
                <label className="field">
                  <span>Gasto anual neto objetivo</span>
                  <input
                    inputMode="decimal"
                    value={profileDraft.fire_number_manual_amount ?? ""}
                    onChange={(e) =>
                      patchDraft((p) => ({
                        ...p,
                        fire_number_manual_amount: typedDecimalOrNull(e.target.value),
                      }))
                    }
                    onBlur={() => queueProfileSave(0)}
                  />
                </label>
              ) : null}

              {/* Base del objetivo. En el puente NO se ofrece: esa estrategia ES el puente y el
                  servidor la fuerza, así que un radio ahí enseñaría una opción sin efecto. */}
              {strategy === "pension_bridge" ? (
                <p className="muted tight">
                  Base del objetivo: <strong>puente hasta la pensión</strong> · la fija la
                  estrategia.
                </p>
              ) : (
                <div className="field">
                  <div
                    className="retirement-radio-stack"
                    role="radiogroup"
                    aria-label="Base del objetivo"
                  >
                    {/* El rótulo va DENTRO del stack: `.field-label-text` le da `flex: 1 0 100%`
                        para que ocupe su propia línea sobre las opciones cuando estas envuelven. */}
                    <span className="label-with-help field-label-text">
                      Base del objetivo
                      <HelpPopover
                        title={HELP_TEXTS["retirement.target_basis"].title}
                        body={HELP_TEXTS["retirement.target_basis"].body}
                      />
                      {/* La opción marcada puede no ser una elección: mientras nadie la fija, la
                          deriva el servidor a partir de si hay pensión declarada. Decirlo evita
                          que se lea como una decisión tomada — y explica por qué se mueve sola
                          al declarar una pensión. */}
                      {basisSource === "derived" ? (
                        <span className="muted"> (derivada)</span>
                      ) : null}
                    </span>
                    <label className="field checkbox-field">
                      <input
                        type="radio"
                        name="target_basis"
                        checked={basis === "perpetuity"}
                        onChange={() =>
                          patchDraft((p) => ({ ...p, target_basis: "perpetuity" }))
                        }
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
                  {/* FUERA del `radiogroup`: un botón enfocable entre radios rompe la navegación
                      con flechas del grupo. Sin esta salida, fijar la base a mano es irreversible
                      desde la UI — el tri-estado del PATCH acepta `null` para volver a derivarla
                      y hasta aquí no había forma de mandarlo. */}
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
              )}

              {basis === "bridge_to_pension" ? (
                <label className="field">
                  <span className="label-with-help">
                    Descuento del puente
                    <HelpPopover
                      title={HELP_TEXTS["retirement.bridge_discount"].title}
                      body={HELP_TEXTS["retirement.bridge_discount"].body}
                    />
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
                    {(
                      ["expected_return", "swr", "none"] as const
                    ).map((b) => (
                      <option key={b} value={b}>
                        {BRIDGE_DISCOUNT_BASIS_LABEL[b]}
                      </option>
                    ))}
                  </select>
                </label>
              ) : null}
              <p className="muted tight">{saveFooter}</p>
            </div>
          </section>

          {/* ── 4. Retirada: cuánto sale del patrimonio y con qué relación con el gasto ─── */}
          <section className="panel">
            <h3 className="panel-title">Retirada</h3>
            <div className="stack bordered-top retirement-config-stack">
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
                    (parseDisplayDecimal(profileDraft.swr_pct) ?? 0) * 10,
                  )}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    patchDraft((p) => ({ ...p, swr_pct: String(v / 10) }));
                  }}
                  onBlur={() => queueProfileSave(0)}
                />
                <span className="muted tight">
                  {formatPercentAmount(profileDraft.swr_pct)}
                </span>
              </label>

              <label className="field">
                <span className="label-with-help">
                  Regla de retirada
                  <HelpPopover
                    title={HELP_TEXTS["retirement.withdrawal_rule"].title}
                    body={HELP_TEXTS["retirement.withdrawal_rule"].body}
                  />
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
                    [
                      "fixed_real",
                      "percent_of_balance",
                      "hybrid",
                      "guardrails",
                    ] as const
                  ).map((k) => (
                    <option key={k} value={k}>
                      {WITHDRAWAL_RULE_KIND_LABEL[k]}
                    </option>
                  ))}
                </select>
              </label>

              {/* Cada regla pide SUS porcentajes y no los de otra: enseñar los cinco a la vez
                  invitaría a rellenar campos que la regla elegida ni mira. */}
              {rule.kind === "percent_of_balance" ? (
                <label className="field">
                  <span>Porcentaje anual del saldo</span>
                  <input
                    inputMode="decimal"
                    placeholder="p. ej. 4"
                    value={rule.pct ?? ""}
                    onChange={(e) =>
                      patchDraft((p) => ({
                        ...p,
                        withdrawal_rule: {
                          ...p.withdrawal_rule,
                          pct: typedDecimalOrNull(e.target.value),
                        },
                      }))
                    }
                    onBlur={() => queueProfileSave(0)}
                  />
                </label>
              ) : null}

              {rule.kind === "hybrid" ? (
                <div className="field-row">
                  <label className="field">
                    <span>Porcentaje de partida</span>
                    <input
                      inputMode="decimal"
                      placeholder="p. ej. 5"
                      value={rule.start_pct ?? ""}
                      onChange={(e) =>
                        patchDraft((p) => ({
                          ...p,
                          withdrawal_rule: {
                            ...p.withdrawal_rule,
                            start_pct: typedDecimalOrNull(e.target.value),
                          },
                        }))
                      }
                      onBlur={() => queueProfileSave(0)}
                    />
                  </label>
                  <label className="field">
                    <span>Porcentaje al que bajas</span>
                    <input
                      inputMode="decimal"
                      placeholder="p. ej. 3,5"
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
                  </label>
                </div>
              ) : null}

              {rule.kind === "guardrails" ? (
                <>
                  <label className="field">
                    <span>Porcentaje de partida</span>
                    <input
                      inputMode="decimal"
                      placeholder="p. ej. 5"
                      value={rule.pct ?? ""}
                      onChange={(e) =>
                        patchDraft((p) => ({
                          ...p,
                          withdrawal_rule: {
                            ...p.withdrawal_rule,
                            pct: typedDecimalOrNull(e.target.value),
                          },
                        }))
                      }
                      onBlur={() => queueProfileSave(0)}
                    />
                  </label>
                  <div className="field-row">
                    <label className="field">
                      <span>Banda que dispara el ajuste (%)</span>
                      <input
                        inputMode="decimal"
                        placeholder="p. ej. 20"
                        value={rule.band_pct ?? ""}
                        onChange={(e) =>
                          patchDraft((p) => ({
                            ...p,
                            withdrawal_rule: {
                              ...p.withdrawal_rule,
                              band_pct: typedDecimalOrNull(e.target.value),
                            },
                          }))
                        }
                        onBlur={() => queueProfileSave(0)}
                      />
                    </label>
                    <label className="field">
                      <span>Cuánto ajustas al tocarla (%)</span>
                      <input
                        inputMode="decimal"
                        placeholder="p. ej. 10"
                        value={rule.adjust_pct ?? ""}
                        onChange={(e) =>
                          patchDraft((p) => ({
                            ...p,
                            withdrawal_rule: {
                              ...p.withdrawal_rule,
                              adjust_pct: typedDecimalOrNull(e.target.value),
                            },
                          }))
                        }
                        onBlur={() => queueProfileSave(0)}
                      />
                    </label>
                  </div>
                </>
              ) : null}

              <div className="field">
                <div
                  className="retirement-radio-stack"
                  role="radiogroup"
                  aria-label="Cómo se aplica la regla"
                >
                  <span className="label-with-help field-label-text">
                    Cómo se aplica la regla
                    <HelpPopover
                      title={HELP_TEXTS["retirement.spend_mode"].title}
                      body={HELP_TEXTS["retirement.spend_mode"].body}
                    />
                  </span>
                  <label className="field checkbox-field">
                    <input
                      type="radio"
                      name="spend_mode"
                      checked={rule.spend_mode === "ceiling"}
                      onChange={() =>
                        patchDraft((p) => ({
                          ...p,
                          withdrawal_rule: {
                            ...p.withdrawal_rule,
                            spend_mode: "ceiling",
                          },
                        }))
                      }
                    />
                    <span>Techo: retiro como mucho la regla</span>
                  </label>
                  <label className="field checkbox-field">
                    <input
                      type="radio"
                      name="spend_mode"
                      checked={rule.spend_mode === "rule_is_spend"}
                      onChange={() =>
                        patchDraft((p) => ({
                          ...p,
                          withdrawal_rule: {
                            ...p.withdrawal_rule,
                            spend_mode: "rule_is_spend",
                          },
                        }))
                      }
                    />
                    <span>La regla es mi gasto</span>
                  </label>
                </div>
              </div>
              <p className="muted tight">{saveFooter}</p>
            </div>
          </section>

          {/* ── 5. Horizonte y riesgo ───────────────────────────────────────────────────── */}
          <section className="panel">
            <h3 className="panel-title">Horizonte y riesgo</h3>
            <div className="stack bordered-top retirement-config-stack">
              <label className="field">
                <span className="label-with-help">
                  Horizonte: edad límite
                  <HelpPopover
                    title={HELP_TEXTS["settings.horizon_age"].title}
                    body={HELP_TEXTS["settings.horizon_age"].body}
                  />
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

              <div className="field-row">
                <label className="field">
                  <span className="label-with-help">
                    Colchón de caja (meses)
                    <HelpPopover
                      title={HELP_TEXTS["retirement.cash_buffer"].title}
                      body={HELP_TEXTS["retirement.cash_buffer"].body}
                    />
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
                </label>
                <label className="field">
                  <span className="label-with-help">
                    Umbral de éxito (%)
                    <HelpPopover
                      title={HELP_TEXTS["retirement.success_threshold"].title}
                      body={HELP_TEXTS["retirement.success_threshold"].body}
                    />
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
              </div>
              <p className="muted tight">{saveFooter}</p>
            </div>
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
