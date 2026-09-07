/**
 * Jubilación — rediseño UX U1b (5.0.0, issue #207, decisiones U1–U12 y S1–S11) reescrito por el
 * **modelo v2**: «el éxito define la fecha» (C1–C8).
 *
 * La página tiene TRES bloques y ningún acordeón. Desde W13 (informe del owner, 2026-09-07) el
 * segundo y el tercero van EN DOS COLUMNAS a ancho completo —el plan a la izquierda, el resultado a
 * la derecha— a partir de ~1.100 px, y apilados (plan primero) por debajo; la maquetación la pone
 * `.retirement-layout` con `auto-fit`, sin ningún breakpoint nuevo:
 *
 *  1. **Cabecera**: el título y UN solo indicador de guardado (S6). Antes había seis pies
 *     «Guardado automático.», uno por panel, que podían contradecirse entre sí.
 *  2. **«Tu plan»** (configuración): una TARJETA POR TEMA —Estrategia · Edades · Pensión · Gasto
 *     en jubilación · Retirada · Horizonte—, cada una con su frase de qué hace, y **solo los
 *     campos que la estrategia elegida Y SU MODO usan**. La tabla U2 vive en
 *     `lib/plan-fields.ts` y aquí no se re-decide nada.
 *  3. **«Resultado»**: la FRASE del plan (`lib/plan-sentence.ts`), los avisos, tres tarjetas
 *     (`buildRetirementTilesV2`), **un solo gráfico** —eje Y, banda coloreada por el fallo
 *     acumulado, la curva de capital necesario, la marca de tu fecha y la tira de éxito por año
 *     de jubilación—, el bloque «Riesgo» y un «Detalle del cálculo» plegado. La línea del
 *     gráfico y su banda son el patrimonio LÍQUIDO (decisión C11, issue #228), no el total que
 *     enseñan la Proyección y el Resumen: es la magnitud que mide la curva de capital necesario
 *     y el éxito del sorteo, y dibujar el total invitaría a leer un cruce que no decide nada.
 *
 * ## Qué cambió con el modelo v2, y por qué no vuelve
 *
 * - **La fecha la decide el ÉXITO, no un cruce contra un objetivo.** No hay objetivo que
 *   dimensionar (C1/M4), así que se fueron los campos «Base del objetivo» y «Descuento del
 *   puente», la línea del objetivo FIRE del chart y la lectura del cruce del líquido. Lo que
 *   ocupa su sitio es el **umbral de éxito** (C3), que vuelve al formulario como la restricción
 *   que fija la fecha válida — no como el corte de semáforo fijo al 100 % que V7 retiró.
 * - **El colchón de caja desapareció como MECANISMO** (M6): la caja es un activo y la regla de
 *   ahorro decide cuánto se guarda. Con él se fue su línea informativa del bloque «Riesgo».
 * - **El puente dejó de ser una estrategia** (C7): el selector tiene CUATRO tarjetas y el puente
 *   es un ajuste de la tarjeta Pensión (interruptor + tasa + años), disponible en cualquiera de
 *   ellas. Un perfil guardado con el literal retirado llega ya migrado, con su aviso.
 * - **Coast y jornada reducida tienen dos MODOS cada una** (M10/M11), y el modo decide qué edad
 *   se pregunta: la que el modo no usa no se pinta en gris, no se pinta.
 *
 * Cinco invariantes que este archivo no puede romper:
 *
 *  - **Un solo porcentaje de retirada** (U4). El slider es `swr_pct` —el tope de la tasa inicial
 *    en la fecha— y el formulario **jamás** manda `withdrawal_rule.pct` ni `start_pct`: el
 *    servidor los hereda del SWR y publica de dónde salieron (`pct_source`). Su mínimo es 0,1 %
 *    y no 0: un plan que retira el 0 % no es un plan.
 *  - **Todo por MES** (`month_index`), nunca por posición de `points[]`: con `density=hybrid` la
 *    posición 13 es el mes 24. La única excepción es `safe_date_series_position`, que es una
 *    posición **publicada por el servidor** y solo se usa como tal.
 *  - **`null` no es cero**: una tarjeta que la estrategia no responde no se pinta con guion, y
 *    `pending` («calculando…») no es lo mismo que ausente.
 *  - **Ni una cifra se recalcula aquí.** El éxito, el umbral, el veredicto, la fecha y el capital
 *    necesario vienen del MISMO sorteo que dibuja el chart; la escala de color sale de UN solo
 *    `riskCutoffsForThreshold`. Dos derivaciones de la misma magnitud es cómo la pantalla acaba
 *    enseñando dos cifras del mismo plan.
 *  - **En Hogar no hay plan** (D9/U10): el agregado no tiene estrategia propia, así que se
 *    enseñan las frases por miembro —con su tono (B7)— y nada más: ni tarjetas, ni chart, ni
 *    formulario.
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
import { HELP_TEXTS, type HelpText, type HelpTextId } from "../lib/helpTexts";
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
  COAST_MODE_LABEL,
  HORIZON_LIFESPAN_AGE_OPTIONS,
  MAX_BRIDGE_PCT,
  MAX_BRIDGE_YEARS,
  MAX_SUCCESS_THRESHOLD_PCT,
  MAX_SWR_PCT,
  MIN_BRIDGE_YEARS,
  MIN_SUCCESS_THRESHOLD_PCT,
  PARTIAL_MODE_LABEL,
  RETIREMENT_STRATEGIES,
  RETIREMENT_STRATEGY_BLURB,
  RETIREMENT_STRATEGY_LABEL,
  WITHDRAWAL_RULE_KIND_LABEL,
  buildRetirementProfilePatch,
  defaultRetirementProfileApi,
  isEmptyRetirementProfilePatch,
  newPartialRetirementDraft,
  newPensionPlanDraft,
  normalizeRetirementProfile,
  retirementProfileIssue,
  withBridgeEnabled,
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
  type PlanCardId,
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
import {
  NO_LAST_GOOD,
  nextLastGood,
  type LastGood,
} from "../lib/stale-data";
import {
  buildRetirementChartMarkers,
  chartValidDateMark,
  retirementNetWorthSeries,
} from "../lib/retirement-chart";
import {
  buildRiskExtraRows,
  formatSamplingErrorPp,
  riskFootnote,
  scenariosPerHundred,
  showsNoVolatilityNotice,
  showsRiskGradient,
  successAbsentReasonEs,
} from "../lib/risk-bands";
import {
  failureKindsAtMonth,
  failureProbabilityAtMonth,
  riskColorForProbability,
  riskCutoffsForThreshold,
  riskGradientStops,
} from "../lib/risk-gradient";
import { resolvePlanMilestoneCivil } from "../lib/plan-card";
import { type LedgerPersonScope } from "../lib/ledger";
import { TAB_PATH, settingsSubTabPath } from "../lib/navigation";
import { appUrl } from "../lib/basePath";
import {
  NEEDED_CAPITAL_SERIES,
  PROJECTION_INFLATION_ADJUSTED_STORAGE_KEY,
  deflationFactorAt,
  neededCurveForChart,
  projectionXTickLabel,
  resolveProjectionAxisAgeMode,
  successStripForChart,
} from "../lib/projection-chart";

/**
 * El catálogo de ayudas **sin creerse que la clave existe** (puente a W8).
 *
 * Los módulos puros del modelo v2 (`lib/retirement-tiles.ts`, `lib/risk-bands.ts`,
 * `lib/retirement-form.ts`) ya nombran las seis claves nuevas —`retirement.needed_capital`,
 * `retirement.safe_date`, `retirement.success_threshold`, `retirement.bridge_settings`,
 * `retirement.coast_mode`, `retirement.partial_mode`, `retirement.failure_by_age`— con el
 * `as unknown as HelpTextId` que W2 estrenó, porque el escáner de `helpTexts.test.ts` tiene que
 * verlas consumidas ANTES de que W8 las escriba. Consecuencia: hasta W8, `HELP_TEXTS[id]` es
 * `undefined` para ellas y un `.title` directo **revienta la pantalla en runtime**.
 *
 * Esta función es el único acceso al catálogo de esta vista: sin texto, no hay interrogante. El
 * `as` no se propaga a la UI y la página sigue funcionando mientras el catálogo llega.
 */
function helpTextOrNull(id: HelpTextId): HelpText | null {
  return (HELP_TEXTS as Partial<Record<HelpTextId, HelpText>>)[id] ?? null;
}

/** Interrogante de una ayuda, o nada si W8 todavía no ha escrito su texto. */
function HelpFor({ id }: { id: HelpTextId }) {
  const t = helpTextOrNull(id);
  return t ? <HelpPopover title={t.title} body={t.body} /> : null;
}

/**
 * Las ayudas que ESTA vista nombra por su cuenta (las de campo viven en `PLAN_FIELD_HELP` y las
 * de tarjeta/fila las traen los módulos puros). El valor es un OBJETO con la clave entrecomillada
 * al lado —una de las tres formas que el escáner de `helpTexts.test.ts` reconoce—, así que la
 * ayuda cuenta como consumida desde ya y W8 no la encontrará huérfana.
 *
 * (Esa forma NO se escribe literalmente en este comentario: el escáner no distingue código de
 * prosa y se contaría a sí mismo, registrando un id que no existe — CLAUDE.md, «comandos que se
 * cuentan a sí mismos», y el mismo tropiezo que ya está documentado en `lib/risk-bands.ts`.)
 */
const RESULT_HELP = {
  /** La escala de color de la banda: fallo ACUMULADO por edad, las tres formas juntas. */
  failureByAge: { helpId: "retirement.failure_by_age" },
} as const;

/** Un decimal tecleado por el usuario, listo para el wire: coma española → punto. */
function typedDecimal(raw: string): string {
  return raw.replace(",", ".");
}

/** Un decimal opcional: vacío = «no hay valor», que en el perfil es `null`, no `0`. */
function typedDecimalOrNull(raw: string): string | null {
  const t = raw.trim();
  return t === "" ? null : typedDecimal(raw);
}

/**
 * Los tres campos del PUENTE dentro de la tarjeta «Pensión» (W13).
 *
 * El owner, 2026-09-07: «el módulo de pensión no usa el ancho completo; que lo use. Si quieres
 * optimizar, pon a la izquierda el puente». La tarjeta pasa a ocupar el ancho de su columna y su
 * contenido se parte en dos sub-columnas — el puente y la pensión —, que es la partición que ya
 * existía conceptualmente (C7: el puente es un AJUSTE de la pensión) y no tenía forma visual.
 *
 * La lista vive aquí y no en `plan-fields.ts` porque es una decisión de MAQUETACIÓN: la tabla U2
 * dice qué campos existen y en qué tarjeta caen, no cómo se reparten dentro de ella.
 */
const BRIDGE_FIELD_IDS: ReadonlySet<PlanFieldId> = new Set<PlanFieldId>([
  "bridge_enabled",
  "bridge_max_pct",
  "bridge_max_years",
]);

/**
 * Las tarjetas que ocupan el ANCHO ENTERO de la columna del plan (W13).
 *
 * `strategy` y `spending` ya lo hacían: su contenido es una rejilla de radio-cards que en media
 * columna quedaba ilegible. **`pension` se suma** por la razón del owner —era la tarjeta que peor
 * aprovechaba el ancho— y porque es la única que contiene dos sub-temas (la pensión y su puente):
 * a media anchura los apilaba en una columna larguísima de siete controles.
 *
 * El criterio, para la siguiente: una tarjeta va ancha si su contenido es una rejilla propia o si
 * tiene DOS sub-temas que se leen mejor en paralelo. `ages`, `withdrawal` y `horizon` traen uno a
 * tres campos de un mismo tema y se emparejan bien de dos en dos — a ancho completo dejarían una
 * banda de aire a la derecha de cada campo.
 */
const WIDE_PLAN_CARDS: ReadonlySet<PlanCardId> = new Set<PlanCardId>([
  "strategy",
  "pension",
  "spending",
]);

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
  householdMemberCount,
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
  /** Nº de miembros del hogar (`GET /v1/installation/members`, carga de `App.tsx`). `null` =
   *  aún no ha llegado. Solo alimenta el aviso B10 (el gasto manual pre-5.0.0 venía del HOGAR;
   *  con un único miembro nunca hubo ambigüedad que revisar). */
  householdMemberCount: number | null;
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
        // Sin resincronización del borrador: con el modelo v2 ya no hay ningún campo cuya
        // elección ALMACENADA difiera de la resuelta (`target_basis` era el único, y murió con
        // el objetivo). Copiar el perfil guardado encima del borrador aquí pisaría lo que el
        // usuario esté tecleando mientras el PATCH viaja.
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
  // Su sitio natural es «Tu cuenta», pero tres de las cuatro estrategias no se pueden simular sin
  // ella: mandar al usuario a otra pestaña a mitad de la elección es donde se abandona el plan.
  // El PATCH del perfil acepta `birth_date` (misma columna que `PATCH /v1/auth/me`), así que se
  // guarda por el mismo camino y no hay una segunda vía de escritura.
  const [birthDraft, setBirthDraft] = useState("");
  /** B4/B11 — el aviso de la tarjeta «Edades» ENLAZA con el campo, no solo lo menciona: sin
   *  esta ref, «ponla aquí» era un texto sin destino y el usuario tenía que encontrar el campo
   *  él solo entre el resto de la tarjeta (varias más allá en coast/partial). */
  const birthDateInputRef = useRef<HTMLInputElement | null>(null);
  const focusBirthDateField = useCallback(() => {
    const el = birthDateInputRef.current;
    if (!el) return;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.focus({ preventScroll: true });
  }, []);
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

  // ── B10 · el gasto manual venía del HOGAR en 4.x, ahora es solo tuyo ──────────────────────
  //
  // `fire_number_mode: "manual"` era, antes de 5.0.0, un campo de `fire_settings` (owner-only,
  // compartido por todo el hogar); con D13 pasó a `RetirementProfile` (personal, uno por
  // usuario). Un hogar de 2+ personas que ya tenía un importe manual guardado se encuentra, sin
  // avisarle, con que la cifra que veía como «la del hogar» ahora es solo la suya — y puede ser
  // la MISMA cifra copiada a cada miembro por la migración, no lo que cada uno querría declarar
  // por separado. Con un único miembro no hubo agregación que deshacer: el aviso no aplica.
  //
  // Bandera de localStorage por instalación+usuario (aceptable per WP): no hay endpoint que
  // registre «ya lo revisé», así que se apaga en cuanto el usuario TOCA el campo — editarlo es
  // la señal de que ya lo ha mirado, se quede como estaba o no.
  const manualAmountMigrationKey = useMemo(() => {
    const instId = installation?.installation.id;
    const uid = user?.id;
    return instId && uid
      ? `ff.retirement.fire-number-manual-migrated.v1.${instId}.${uid}`
      : null;
  }, [installation?.installation.id, user?.id]);

  const [manualAmountMigrationAcked, setManualAmountMigrationAcked] = useState(false);
  useEffect(() => {
    if (!manualAmountMigrationKey) {
      setManualAmountMigrationAcked(false);
      return;
    }
    try {
      setManualAmountMigrationAcked(
        window.localStorage.getItem(manualAmountMigrationKey) === "1",
      );
    } catch {
      setManualAmountMigrationAcked(false);
    }
  }, [manualAmountMigrationKey]);

  const ackManualAmountMigration = useCallback(() => {
    setManualAmountMigrationAcked(true);
    if (!manualAmountMigrationKey) return;
    try {
      window.localStorage.setItem(manualAmountMigrationKey, "1");
    } catch {
      /* sin storage, el aviso simplemente reaparece la próxima vez */
    }
  }, [manualAmountMigrationKey]);

  const showManualAmountMigrationNotice =
    profileDraft.fire_number_mode === "manual" &&
    (householdMemberCount ?? 0) > 1 &&
    !manualAmountMigrationAcked;

  // ── Stale-while-revalidate: lo último bueno se sigue pintando (W13) ───────────────────────
  //
  // El informe del owner (2026-09-07): «durante el cálculo la GUI parpadea constantemente… si
  // recargara in situ sin mover nada aún sería tolerable: actualizar la data, no descargar y
  // cargar nada nuevo».
  //
  // La causa era estructural, no cosmética: TODO el panel «Resultado» colgaba de
  // `retirementMetricsReady`, que incluía `!projectionBusy && !retirementBusy`. Cada guardado del
  // autosave dispara un PATCH → `loadProjectionSeriesPage()` + `loadProjectionBands()`, y durante
  // esos 2–5 s la vista se apagaba entera con la respuesta anterior todavía en memoria: la frase
  // volvía a «Calculando tu plan…», las tres tarjetas SE DESMONTABAN (de ahí el salto de altura)
  // y el chart perdía marcas, curva y tira de éxito. Con el two-phase fetch (hybrid + monthly)
  // eso pasaba dos veces por recarga.
  //
  // La latch (`lib/stale-data.ts`, pura y testeada) invierte la regla: mientras hay una petición
  // en vuelo se conserva la ÚLTIMA respuesta buena y solo cambia un booleano. Un `null` con la
  // carga ya apagada sí suelta el dato — es un error o un scope vacío, y seguir enseñando cifras
  // de otro momento sería mentir.
  //
  // Se aplica sobre un `ref` DURANTE el render (no en un efecto) a propósito: un efecto pintaría
  // primero el hueco y lo rellenaría en el siguiente commit, que es justo el parpadeo que esto
  // viene a quitar. Es seguro porque `nextLastGood` es idempotente (test), así que el doble
  // render de StrictMode no la altera.
  const latchScopeRef = useRef(scopeReadOnly);
  const seriesLatchRef = useRef<LastGood<ProjectionSeriesApi>>(NO_LAST_GOOD);
  const bandsLatchRef = useRef<LastGood<ProjectionBandsApi>>(NO_LAST_GOOD);
  if (latchScopeRef.current !== scopeReadOnly) {
    // Cambiar de ámbito cambia lo que las cifras MIDEN: conservar las del ámbito anterior
    // mientras llega el nuevo sería enseñar el plan de otro conjunto de personas.
    latchScopeRef.current = scopeReadOnly;
    seriesLatchRef.current = NO_LAST_GOOD;
    bandsLatchRef.current = NO_LAST_GOOD;
  }
  seriesLatchRef.current = nextLastGood(
    seriesLatchRef.current,
    projectionSeries,
    projectionBusy || retirementBusy,
  );
  bandsLatchRef.current = nextLastGood(
    bandsLatchRef.current,
    projectionBands,
    projectionBandsBusy,
  );
  /** La serie que la vista PINTA: la del servidor, o la última buena mientras llega la siguiente. */
  const shownSeries = seriesLatchRef.current.value;
  /** Ídem para el sorteo. */
  const shownBands = bandsLatchRef.current.value;
  /** Hay una revalidación en vuelo: el ÚNICO indicio de «calculando» que se permite en pantalla
   *  es un punto junto al título. Ni spinners que sustituyan contenido, ni contenedores que
   *  cambien de altura. */
  const metricsRefreshing =
    seriesLatchRef.current.refreshing || bandsLatchRef.current.refreshing;

  // ── Ejes y rotuladores ────────────────────────────────────────────────────────────────────
  const axisAgeMode = shownSeries
    ? resolveProjectionAxisAgeMode(shownSeries, installation)
    : "dates";
  const axisBirth =
    shownSeries?.viewer_birth_date?.trim() || birthDate || null;
  const axisAnchor = shownSeries?.anchor_date_ymd?.trim() || null;
  const mc = shownSeries?.months ?? 0;

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

  /**
   * Mes de la rejilla → EDAD CUMPLIDA, con el mismo calendario civil que el eje (el ancla de la
   * proyección + la fecha de nacimiento). Lo consumen la frase (`ageAt`) y las tarjetas
   * (`monthAge`): una sola aritmética para que las dos digan la misma edad del mismo mes.
   *
   * **`null` sin fecha de nacimiento, y ahí se acaba** (B5). No se estima restando años a nada:
   * una edad inventada en la frase-hito se copia como si fuera exacta, y es justo la razón por la
   * que el servidor deja el bloque «plan» sin publicar (`plan_absent_reason:
   * "birth_date_missing"`) en vez de resolverlo a ojo.
   */
  const ageAt = useCallback(
    (mi: number): number | null =>
      resolvePlanMilestoneCivil({
        monthIndex: mi,
        anchorDateYmd: axisAnchor,
        birthDateIso: axisBirth,
      }).age,
    [axisAnchor, axisBirth],
  );

  const configuredSavingsUsesTransactions = savingsSourceUsesTransactions(
    installation?.installation.fire_settings?.savings_source,
  );
  /**
   * Hay cifras que enseñar. **Ya no mira los flags de carga** (W13): con la latch de arriba,
   * «hay dato» y «hay una recarga en vuelo» son dos preguntas distintas, y mezclarlas era lo que
   * apagaba la pantalla entera en cada autosave. Lo segundo lo dice `metricsRefreshing`, y su
   * única consecuencia visible es el punto junto al título.
   */
  const retirementMetricsReady = hasMembership && shownSeries != null;

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
        series: retirementMetricsReady ? shownSeries : null,
        targetRetirementAge: savedProfile.target_retirement_age ?? null,
        monthLabel,
        ageAt,
        currencyIso,
        ageMode: axisAgeMode,
        // El modo del perfil GUARDADO: es el que el servidor simuló. Con el del borrador la
        // frase describiría un coast que la respuesta de al lado no resolvió.
        coastMode: savedProfile.coast_mode,
      }),
    [
      retirementMetricsReady,
      shownSeries,
      savedProfile.target_retirement_age,
      savedProfile.coast_mode,
      monthLabel,
      ageAt,
      currencyIso,
      axisAgeMode,
    ],
  );

  /** Las tarjetas se leen del perfil GUARDADO, no del borrador: una edad del borrador
   *  nombraría un plan que la respuesta del servidor no simuló. */
  const tilesInput = useMemo(
    () => ({
      series: retirementMetricsReady ? shownSeries : null,
      currencyIso,
      monthLabel,
      monthAge: ageAt,
      targetRetirementAge: savedProfile.target_retirement_age ?? null,
    }),
    [
      retirementMetricsReady,
      shownSeries,
      currencyIso,
      monthLabel,
      ageAt,
      savedProfile.target_retirement_age,
    ],
  );
  const tiles = useMemo(() => buildRetirementTilesV2(tilesInput), [tilesInput]);
  /**
   * Las filas del «Detalle», **sin los avisos**: `retirementDetailRows` los añade al final como
   * filas `notice:` porque hay consumidores que solo pintan el plegado, y esta vista los enseña
   * arriba con su banner. Contar lo mismo dos veces en la misma pantalla es ruido, y el filtro
   * va por el prefijo de la key —que es estable por contrato— y no por el tono.
   */
  const planDetailRows = useMemo(
    () => retirementDetailRows(tilesInput).filter((r) => !r.key.startsWith("notice:")),
    [tilesInput],
  );
  /**
   * TODOS los avisos suben al panel de resultado, cada uno con su piel: el rojo de D17 como
   * `error-banner` y los ámbar —`strategy_pension_bridge_migrated`, `no_volatility_declared`,
   * `coast_not_reachable`…— como `info-banner`.
   *
   * Antes solo subía el rojo y el resto se leía en el «Detalle» plegado, que es donde nadie mira
   * cuando la cifra de arriba le acaba de cambiar sola: un perfil migrado desde la estrategia
   * retirada `pension_bridge` (C7) tiene que enterarse en la primera pantalla, no al desplegar.
   * Como contrapartida, las filas `notice:` de `retirementDetailRows` se filtran abajo: contarlo
   * dos veces en la misma pantalla es ruido.
   */
  const notices = useMemo(
    () =>
      buildRetirementNotices(
        retirementMetricsReady ? shownSeries : null,
        savedProfile.target_retirement_age ?? null,
      ),
    [retirementMetricsReady, shownSeries, savedProfile.target_retirement_age],
  );

  /**
   * El aviso de «sin volatilidad declarada» tiene DOS fuentes que dicen lo mismo desde sitios
   * distintos: el `warnings[]` de la serie (lo trae `buildRetirementNotices`) y
   * `any_volatility_declared` de las bandas. Cuando hay bandas se pinta **donde importa** —junto
   * al éxito que deja de medir riesgo, y con el enlace a Activos que la lib no puede llevar—, así
   * que se saca de los banners de arriba: la misma frase dos veces en la misma pantalla es ruido.
   * Sin bandas cargadas todavía, el banner de arriba es el único sitio donde puede decirse.
   */
  const noVolatilityInRiskBlock = showsNoVolatilityNotice(shownBands);
  const topNotices = useMemo(
    () =>
      noVolatilityInRiskBlock
        ? notices.filter((n) => n.code !== "no_volatility_declared")
        : notices,
    [notices, noVolatilityInRiskBlock],
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
    const raw = shownSeries?.deflation_annual_inflation_percent;
    const parsed = raw != null ? Number(raw) : Number.NaN;
    return Number.isFinite(parsed) ? parsed : installationInflationPct;
  }, [shownSeries?.deflation_annual_inflation_percent, installationInflationPct]);

  /** UN solo deflactor para patrimonio, objetivo y banda. Deflactar solo unos los separaría y
   *  el abanico dejaría de contener a la línea que dice contener. */
  const chartDeflator = useMemo(() => {
    const pct = inflationAdjusted ? deflationPct : 0;
    return (mi: number) => deflationFactorAt(mi, pct);
  }, [inflationAdjusted, deflationPct]);

  /**
   * Puntos de banda en euros NOMINALES: la deflactación la aplica el chart, una sola vez.
   *
   * **Banda LÍQUIDA (decisión C11, issue #228), no del total.** La línea principal del chart es
   * ahora el patrimonio líquido, así que la banda tiene que medir la misma magnitud —una banda
   * del total sobre una línea líquida sería un abanico de otra escala que no la contiene. Por
   * HTTP los `net_worth_liquid_p10/p90` viajan siempre (`projection_bands.rs`,
   * `assemble_bands_response`: siempre `Some`); se filtra igual, con el MISMO criterio que
   * `MiniProjection` ya aplica al resto de puntos de la banda, para no caer al total en silencio
   * si algún día dejaran de venir.
   */
  const bandPoints = useMemo(() => {
    if (!shownBands) return null;
    return shownBands.points.flatMap((p) => {
      const p10 = p.net_worth_liquid_p10;
      const p90 = p.net_worth_liquid_p90;
      if (typeof p10 !== "number" || typeof p90 !== "number") return [];
      return [{ month: p.month_index, p10, p90 }];
    });
  }, [shownBands]);

  /**
   * La línea PRINCIPAL del chart (decisión C11, issue #228): el patrimonio LÍQUIDO, no el total
   * con vivienda incluida y deuda restada — es la magnitud que mide `needed_capital_curve` y el
   * éxito del sorteo, y dibujar el total invitaría a leer un cruce que no es el que decide la
   * fecha. NOMINAL: `MiniProjection` la deflacta con el MISMO `chartDeflator` que ya aplica al
   * total. La Proyección y el Resumen conservan el total — ahí no hay curva de capital que
   * comparar.
   */
  const chartNetWorthSeries = useMemo(
    () => retirementNetWorthSeries(shownSeries),
    [shownSeries],
  );

  /**
   * La marca VERTICAL de la fecha del plan (C4) y, cuando no la hay, la nota que lo explica.
   * `chartValidDateMark` decide las cuatro lecturas —fecha válida, «como pediste», sin fecha y
   * «calculando»— y su rótulo lleva el éxito, contado con la MISMA función que la frase.
   */
  const validDate = useMemo(
    () => chartValidDateMark(retirementMetricsReady ? shownSeries : null),
    [retirementMetricsReady, shownSeries],
  );

  /**
   * Los hitos secundarios. **Sin el de jubilación cuando hay marca de fecha**: los dos saldrían
   * en el mismo mes y el chart pintaría dos verticales sobre la misma X (el contrato de
   * `MiniProjection.validDateMark` lo dice con todas las letras). La marca gana porque es la que
   * lleva el éxito en el rótulo.
   */
  const chartMarkers = useMemo(() => {
    const pts = shownSeries?.points;
    if (!pts || pts.length === 0) return [];
    const all = buildRetirementChartMarkers(shownSeries, {
      startMonth: pts[0]!.month_index,
      endMonth: pts[pts.length - 1]!.month_index,
    });
    return validDate.mark == null ? all : all.filter((m) => m.kind !== "retirement");
  }, [shownSeries, validDate.mark]);

  /**
   * Los dos cortes de la escala de color, derivados del UMBRAL DEL PERFIL (C3). Salen de la
   * serie —el umbral con el que el servidor resolvió la fecha— y solo caen a las bandas si la
   * serie no lo publica. Un umbral ausente lo trata `riskCutoffsForThreshold` como 100, que es
   * el caso más exigente: ante un umbral que no llegó, la escala no se ablanda sola.
   *
   * Los tiñen los TRES sitios que hablan de riesgo —banda, tira de éxito y escala de la
   * leyenda—, y por eso se calculan UNA vez: tres derivaciones del mismo umbral en tres sitios
   * es exactamente cómo se destiñe una leyenda sin que nada falle.
   */
  const riskCutoffs = useMemo(
    () =>
      riskCutoffsForThreshold(
        shownSeries?.success_threshold_pct ?? shownBands?.success_threshold_pct,
      ),
    [shownSeries?.success_threshold_pct, shownBands?.success_threshold_pct],
  );

  /**
   * El color de la banda (V2/V5). Los extremos son los MISMOS que los de `chartMarkers` —el
   * primer y el último mes de la serie cargada— porque son los que el chart usa para repartir su
   * eje X: el `<linearGradient>` va en `userSpaceOnUse` entre esos dos meses, y con otros el
   * mapeo mes→color se desplazaría sin que nada fallara.
   *
   * Quién AUTORIZA a teñir lo decide `showsRiskGradient` (`lib/risk-bands.ts`), no esta vista:
   * sus dos vetos —sin volatilidad declarada y sin éxito que medir (A12)— son de contrato, y los
   * dos acaban pintando la banda de VERDE ENTERO sobre un sorteo que no midió riesgo. Con `[]` la
   * banda vuelve al acento plano (`MiniProjection` solo usa el degradado con ≥ 2 paradas), que es
   * la lectura honesta: una trayectoria de patrimonio, sin un juicio de riesgo encima.
   */
  const gradientStops = useMemo(() => {
    const pts = shownSeries?.points;
    if (!showBand || !pts || pts.length === 0) return [];
    if (!shownBands || !showsRiskGradient(shownBands)) return [];
    return riskGradientStops({
      points: shownBands.failure_probability_by_age,
      monthStart: pts[0]!.month_index,
      monthEnd: pts[pts.length - 1]!.month_index,
      cutoffs: riskCutoffs,
    });
  }, [showBand, shownSeries, shownBands, riskCutoffs]);

  /**
   * Rótulo del hover. Sale de `failureProbabilityAtMonth`, **la misma función que colorea**: un
   * tooltip alimentado por otro cálculo podría contradecir al tinte y nadie lo notaría.
   *
   * El desglose por causa va entre paréntesis y NO se interpola (`failureKindsAtMonth` devuelve
   * la muestra real más cercana): tres acumuladas interpoladas por separado darían un trío que
   * no suma la cifra que el propio tooltip enseña justo delante.
   */
  const chartHoverLabel = useMemo(() => {
    if (gradientStops.length < 2 || !shownBands) return null;
    const points = shownBands.failure_probability_by_age;
    return (mi: number): string | null => {
      const p = failureProbabilityAtMonth(points, mi);
      if (p == null) return null;
      const kinds = failureKindsAtMonth(points, mi);
      const breakdown =
        kinds == null
          ? ""
          : ` (sin dinero ${formatPercentDisplay(kinds[0] * 100)}, tasa ${formatPercentDisplay(
              kinds[1] * 100,
            )}, regla ${formatPercentDisplay(kinds[2] * 100)})`;
      return `${monthLabel(mi)} · ${formatPercentDisplay(p * 100)} de los escenarios ya han fallado${breakdown}`;
    };
  }, [gradientStops, shownBands, monthLabel]);

  /**
   * La curva «Capital necesario» (C4), en euros NOMINALES y por MES: la deflactación la aplica
   * `MiniProjection`, una sola vez, con el MISMO `chartDeflator` que ya aplica al patrimonio y a
   * la banda (`chartNetWorthSeries`/`bandPoints` arriba). Sustituye a la línea del objetivo FIRE:
   * en v2 no hay objetivo que cruzar — la fecha la fija el éxito— y lo que se dibuja es el
   * líquido que hace cumplir el umbral jubilándose en cada mes.
   *
   * **Bug corregido (issue #228, W12): esta curva NO se deflacta aquí.** Hasta el 5.0.0 se
   * llamaba a `neededCurveForChart(projectionSeries, chartDeflator)` — deflactando el nodo — y
   * `MiniProjection` volvía a deflactarlo con su propio prop `deflator`, encogiendo la curva dos
   * veces (a 3 %/30 años, ~59 % de más de lo debido) y sesgando con ella el dominio del eje Y.
   *
   * `neededCurveForChart` devuelve un array PARALELO a `points[]` (o `null` entero si el nivel 2
   * no ha terminado o la longitud no cuadra); aquí solo se le pega su `month_index`, nunca su
   * posición.
   */
  const neededCurve = useMemo(() => {
    const pts = shownSeries?.points;
    if (!pts || pts.length === 0) return null;
    const values = neededCurveForChart(shownSeries);
    if (values == null) return null;
    return pts.map((p, i) => ({ month: p.month_index, value: values[i] ?? null }));
  }, [shownSeries]);

  /** La tira de éxito por AÑO de jubilación bajo el eje X. El rótulo lo compone la vista, que es
   *  quien sabe si el eje va en fechas o en edades. */
  const successStrip = useMemo(() => {
    const pts = successStripForChart(retirementMetricsReady ? shownSeries : null);
    return pts.map((p) => {
      const n = scenariosPerHundred(p.success);
      return {
        monthIndex: p.monthIndex,
        success: p.success,
        label:
          n == null
            ? undefined
            : `si te fueras en ${monthLabel(p.monthIndex)}: ${n} de cada 100`,
      };
    });
  }, [retirementMetricsReady, shownSeries, monthLabel]);

  // ── Riesgo compacto ───────────────────────────────────────────────────────────────────────
  const riskExtraRows = useMemo(
    () => buildRiskExtraRows({ bands: shownBands }),
    [shownBands],
  );

  // ── Hogar (U10): frases por miembro y nada más ────────────────────────────────────────────
  const memberLines = useMemo(
    () => householdPlanLines(shownSeries?.members, monthLabel),
    [shownSeries?.members, monthLabel],
  );

  // ── Editores compartidos ──────────────────────────────────────────────────────────────────
  const selectStrategy = useCallback(
    (s: RetirementStrategyApi) => {
      patchDraft((p) => {
        const next: RetirementProfileApi = { ...p, strategy: s };
        if (s === "partial" && next.partial_retirement == null) {
          next.partial_retirement = newPartialRetirementDraft();
        }
        // Ninguna estrategia crea ya el bloque de pensión: «Puente hasta la pensión» dejó de ser
        // una estrategia (C7) y el puente es un ajuste de la tarjeta Pensión, que solo existe
        // cuando el usuario declara una.
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
    return <HelpFor id={entry.helpId} />;
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
              ref={birthDateInputRef}
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

      // ── M10 · los dos modos de coast ──────────────────────────────────────────────────────
      // El modo va ANTES que su edad porque decide CUÁL de las dos edades tiene sentido: en el
      // modo A fijas la jubilación y el plan resuelve cuándo puedes dejar de aportar; en el B
      // fijas la parada y la fecha sale del sorteo. Por eso la edad que el modo no usa
      // desaparece (`lib/plan-fields.ts`) en vez de quedarse en gris.
      case "coast_mode":
        return (
          <div className="field" key={f.id}>
            <span className="label-with-help field-label-text">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <div
              className="retirement-mode-grid"
              role="radiogroup"
              aria-label="Qué fijas tú en Coast FIRE"
            >
              {(["fixed_retirement_age", "fixed_stop_age"] as const).map((mode) => (
                <label
                  key={mode}
                  className={`retirement-mode-card ${
                    profileDraft.coast_mode === mode ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="coast_mode"
                    className="sr-only"
                    checked={profileDraft.coast_mode === mode}
                    onChange={() => patchDraft((p) => ({ ...p, coast_mode: mode }))}
                  />
                  <span className="retirement-mode-name">{COAST_MODE_LABEL[mode]}</span>
                  <span className="retirement-mode-sub">
                    {mode === "fixed_retirement_age"
                      ? "el plan resuelve cuándo puedes dejar de aportar"
                      : "el plan resuelve a qué fecha te lleva"}
                  </span>
                </label>
              ))}
            </div>
          </div>
        );

      case "coast_stop_age":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              value={intFieldValue(profileDraft.coast_stop_age)}
              placeholder="p. ej. 45"
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined) return;
                patchDraft((p) => ({ ...p, coast_stop_age: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? (
              <RequiredHint />
            ) : (
              <small className="muted">
                Desde esa edad no entra ni un euro más: lo que haya, crece solo.
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

      // ── M11 · los dos modos de la jornada reducida ────────────────────────────────────────
      case "partial_mode":
        return (
          <div className="field" key={f.id}>
            <span className="label-with-help field-label-text">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <div
              className="retirement-mode-grid"
              role="radiogroup"
              aria-label="Cuándo empieza la media jornada"
            >
              {(["at_age", "asap"] as const).map((mode) => (
                <label
                  key={mode}
                  className={`retirement-mode-card ${
                    (partial?.mode ?? "at_age") === mode ? "is-active" : ""
                  }`}
                >
                  <input
                    type="radio"
                    name="partial_mode"
                    className="sr-only"
                    checked={(partial?.mode ?? "at_age") === mode}
                    onChange={() =>
                      patchDraft((p) =>
                        p.partial_retirement
                          ? {
                              ...p,
                              partial_retirement: {
                                ...p.partial_retirement,
                                mode,
                                // Cambiar a «en cuanto pueda» SUELTA la edad: la resuelve el
                                // servidor y conservarla dejaría en el borrador un dato que la
                                // simulación no mira. Al volver al modo A el campo pide la suya.
                                starts_at_age:
                                  mode === "asap"
                                    ? null
                                    : p.partial_retirement.starts_at_age,
                              },
                            }
                          : p,
                      )
                    }
                  />
                  <span className="retirement-mode-name">{PARTIAL_MODE_LABEL[mode]}</span>
                  <span className="retirement-mode-sub">
                    {mode === "at_age"
                      ? "tú pones la edad de inicio"
                      : "el plan busca el primer mes que se lo puede permitir"}
                  </span>
                </label>
              ))}
            </div>
          </div>
        );

      // La edad de inicio solo existe en el modo A: en «en cuanto pueda» la resuelve el servidor
      // (`partial_start_month_index`) y `lib/plan-fields.ts` ni siquiera emite el campo.
      case "partial_start_age":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              value={intFieldValue(partial?.starts_at_age ?? null)}
              placeholder="p. ej. 60"
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined) return;
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
        );

      // El ingreso se pinta APARTE de la edad (antes iban juntos en una fila): con el modo B no
      // hay edad que pintar, y colgar el ingreso de un campo que no se emite lo habría hecho
      // desaparecer justo en la mitad de la fase que el usuario sí decide.
      case "partial_income":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
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
        );

      // La pensión se pinta entera en su primer campo: la casilla y sus dos cifras son un bloque.
      case "pension_amount":
        return (
          <div className="stack" key={f.id}>
            <label className="field checkbox-field">
              <input
                type="checkbox"
                checked={pension != null}
                onChange={(e) =>
                  patchDraft((p) => ({
                    ...p,
                    // El borrador arranca con el PUENTE APAGADO (C7): tener pensión no implica
                    // querer adelantar la fecha vendiendo por encima de tu tasa.
                    pension: e.target.checked ? newPensionPlanDraft(p.swr_pct) : null,
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
            ) : null}
          </div>
        );
      case "pension_start_age":
        return null; // se pinta con `pension_amount`

      // ── C2/C7 · el PUENTE: un ajuste de la pensión, no una estrategia ─────────────────────
      //
      // Encenderlo tiene que dejar los dos números YA PUESTOS (`withBridgeEnabled`): el servidor
      // los rellena con `max(5, swr + 1)` % y 7 años, y un interruptor que deja dos huecos en
      // pantalla enseñaría un puente sin tope donde el perfil guardado tiene uno.
      case "bridge_enabled":
        return (
          <div className="field" key={f.id}>
            <span className="label-with-help field-label-text">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <Switch
              checked={pension?.bridge_enabled === true}
              onChange={(on) =>
                setPension((p) => withBridgeEnabled(p, on, profileDraft.swr_pct))
              }
              ariaLabel="Activar el puente hasta la pensión"
              label={pension?.bridge_enabled === true ? "Activado" : "Desactivado"}
            />
            <p className="muted tight">
              Jubilación anticipada: sin sueldo no hay aportaciones; se vende hasta la pensión.
            </p>
          </div>
        );

      case "bridge_max_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label} (%)
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="decimal"
              placeholder="p. ej. 5"
              value={pension?.bridge_max_pct ?? ""}
              onChange={(e) =>
                setPension((p) => ({
                  ...p,
                  bridge_max_pct: typedDecimalOrNull(e.target.value),
                }))
              }
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? (
              <RequiredHint />
            ) : (
              <small className="muted">
                Mayor que tu tasa de retirada ({formatPercentAmount(profileDraft.swr_pct)}) y
                hasta {formatPercentAmount(String(MAX_BRIDGE_PCT))}: si no la supera no es un
                puente, es la misma tasa.
              </small>
            )}
          </label>
        );

      case "bridge_max_years":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <input
              inputMode="numeric"
              placeholder="p. ej. 7"
              value={intFieldValue(pension?.bridge_max_years ?? null)}
              onChange={(e) => {
                const v = readIntField(e.target.value);
                if (v === undefined) return;
                setPension((p) => ({ ...p, bridge_max_years: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? (
              <RequiredHint />
            ) : (
              <small className="muted">
                Entre {MIN_BRIDGE_YEARS} y {MAX_BRIDGE_YEARS}. Tu fecha nunca cae más de estos
                años antes de la primera paga.
              </small>
            )}
          </label>
        );

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
            {/* `f.label` (`lib/plan-fields.ts`) ya dice «Gasto anual manual»: un sufijo fijo
                aquí repetía «gasto anual» dos veces en la misma línea (copy_fixes #9 de la
                revisión: la etiqueta cambió sola y el sufijo se quedó pisándola). */}
            <span>{f.label}</span>
            <input
              inputMode="decimal"
              placeholder="p. ej. 24000"
              value={profileDraft.fire_number_manual_amount ?? ""}
              onChange={(e) => {
                ackManualAmountMigration();
                patchDraft((p) => ({
                  ...p,
                  fire_number_manual_amount: typedDecimalOrNull(e.target.value),
                }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            {missing ? <RequiredHint /> : null}
            {/* B10 — el importe manual venía del HOGAR en 4.x; con 5.0.0 es solo tuyo. */}
            {showManualAmountMigrationNotice ? (
              <p className="muted tight">
                Este importe venía del hogar en 4.x y ahora es solo tuyo: revísalo.
              </p>
            ) : null}
          </label>
        );

      // ── Supuestos con default: umbral, retirada, pensión fina y horizonte ───────────────
      //
      // Antes vivían en el acordeón «Avanzado»; desde V3 caen en la tarjeta de su tema
      // (`lib/plan-fields.ts`) y comparten switch con el resto.
      //
      // El UMBRAL va el primero de «Retirada» porque en v2 **es la restricción que decide la
      // fecha válida**, no un corte de semáforo: mover este slider mueve la fecha, el capital
      // necesario y el color de la banda a la vez.
      case "success_threshold_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            <input
              type="range"
              min={MIN_SUCCESS_THRESHOLD_PCT}
              max={MAX_SUCCESS_THRESHOLD_PCT}
              step={1}
              value={profileDraft.success_threshold_pct}
              onChange={(e) => {
                const v = Number(e.target.value);
                if (!Number.isInteger(v)) return;
                patchDraft((p) => ({ ...p, success_threshold_pct: v }));
              }}
              onBlur={() => queueProfileSave(0)}
            />
            <span className="retirement-slider-value">
              {formatPercentDisplay(profileDraft.success_threshold_pct)}
            </span>
            <small className="muted">
              de tus escenarios tienen que aguantar hasta el horizonte.
              {profileDraft.success_threshold_pct >= MAX_SUCCESS_THRESHOLD_PCT
                ? " Al 100 % no puede fallar ni uno de los caminos sorteados."
                : ""}
            </small>
          </label>
        );

      case "swr_pct":
        return (
          <label className="field" key={f.id}>
            <span className="label-with-help">
              {f.label}
              {fieldHelp(f.id)}
            </span>
            {/* El slider trabaja en DÉCIMAS de punto (`value = pct · 10`) para poder pisar los
                0,1 sin `step` fraccionario. El mínimo es 1 —0,1 %— y NO 0: un plan que retira
                el 0 % no es un plan, y con la regla por saldo el servidor lo rechaza
                (`withdrawal_pct_out_of_range`) sin que nada en pantalla lo explique. */}
            <input
              type="range"
              min={1}
              max={MAX_SWR_PCT * 10}
              step={1}
              value={Math.max(
                1,
                Math.round((parseDisplayDecimal(profileDraft.swr_pct) ?? 0) * 10),
              )}
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
              lo máximo que sacas el primer año sobre tu líquido.
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
                // B8 — al cambiar de regla se SUELTAN los subcampos de la anterior. Sin esto, el
                // `end_pct` de la híbrida o la banda de las bandas viajaban en el PATCH de una
                // regla que no los usa: el servidor los guarda, la pantalla no los enseña, y al
                // volver a aquella regla reaparecía un número que nadie recordaba haber puesto.
                // `spend_mode` vuelve a `ceiling` en `fixed_real` por el mismo motivo: esa regla
                // no tiene modo que aplicar (`lib/plan-fields.ts` ni siquiera pinta el campo).
                patchDraft((p) => ({
                  ...p,
                  withdrawal_rule: {
                    ...p.withdrawal_rule,
                    kind,
                    end_pct: kind === "hybrid" ? p.withdrawal_rule.end_pct : null,
                    band_pct: kind === "guardrails" ? p.withdrawal_rule.band_pct : null,
                    adjust_pct: kind === "guardrails" ? p.withdrawal_rule.adjust_pct : null,
                    spend_mode:
                      kind === "fixed_real" ? "ceiling" : p.withdrawal_rule.spend_mode,
                  },
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

  /**
   * La tarjeta «Pensión» en DOS sub-columnas (W13): la pensión y el puente.
   *
   * **Orden del DOM: pensión primero.** Apilado (móvil, tableta y la columna estrecha) es el
   * orden correcto — sin pensión declarada el puente ni siquiera existe —, y a lo ancho el CSS
   * lo invierte con `flex-direction: row-reverse` para dejar el puente a la IZQUIERDA, que es lo
   * que pidió el owner. Invertir con CSS y no con el DOM mantiene el orden de tabulación y el de
   * lectura de un lector de pantalla alineados con la dependencia real entre los dos bloques.
   *
   * Sin puente que enseñar (pensión no declarada) no se abren sub-columnas: una columna sola con
   * su rótulo anunciaría una partición que no existe.
   */
  const renderPensionCard = (fields: PlanFieldDescriptor[]): ReactNode => {
    const bridge = fields.filter((f) => BRIDGE_FIELD_IDS.has(f.id));
    const own = fields.filter((f) => !BRIDGE_FIELD_IDS.has(f.id));
    if (bridge.length === 0 || own.length === 0) {
      return fields.map((f) => renderField(f));
    }
    return (
      <div className="retirement-card-cols">
        <div className="retirement-card-col">
          <h5 className="retirement-subcard-title">Tu pensión</h5>
          {own.map((f) => renderField(f))}
        </div>
        <div className="retirement-card-col">
          <h5 className="retirement-subcard-title">El puente hasta la pensión</h5>
          {bridge.map((f) => renderField(f))}
        </div>
      </div>
    );
  };

  const editingPlan = canEditProfile && retirementProfile != null;
  /** Las tarjetas a pintar, ya sin vacías y en `PLAN_CARD_ORDER` (V3). Una sola lista: el
   *  formulario dejó de tener dos mitades el día que dejó de tener un acordeón. */
  const cardGroups = useMemo(
    () => (editingPlan ? planCardGroups(fieldCtx) : []),
    [editingPlan, fieldCtx],
  );

  const chartReady =
    hasMembership && shownSeries != null && shownSeries.points.length > 0;

  // `planPending` (el spinner y el `aria-busy` del panel «Resultado») se retiró en A12: colgaba de
  // `retirement_date_basis === "pending"`, un literal que **el servidor nunca emite** — el nivel 1
  // del solve se resuelve EN LÍNEA, dentro del permiso de la serie, así que cuando esta respuesta
  // llega la fecha ya está decidida. El único cálculo que de verdad llega tarde es el nivel 2, y
  // tiene su propia señal justo debajo (`neededCurveComputing`), que sí se puede leer.

  /** El nivel 2 (segundo plano) todavía está resolviendo la curva de capital necesario por edad:
   *  la línea del chart llegará sola en un GET posterior. */
  const neededCurveComputing =
    retirementMetricsReady && shownSeries?.needed_capital_curve_state === "computing";

  /**
   * C5/B4/B11 — sin fecha de nacimiento no hay fecha válida, y hay que decirlo **donde se
   * arregla**: en la tarjeta «Edades», que es la que trae el campo. Las dos señales son del
   * servidor y significan lo mismo desde ángulos distintos: el bloque «plan» no se publica
   * (`plan_absent_reason`) y el horizonte cayó a su fallback demográfico (`horizon_basis`).
   */
  const birthDateBlocksPlan =
    shownSeries != null &&
    (shownSeries.plan_absent_reason === "birth_date_missing" ||
      shownSeries.horizon_basis === "fallback_no_demographics");

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
              {/* B7 — cada línea lleva el TONO de esa persona. Sin él, un miembro
                  infra-financiado o al que le falta la fecha de nacimiento se leía exactamente
                  igual que uno que llega, mientras su propia tarjeta sí lo pintaba de rojo. */}
              {memberLines.map((l) => (
                <li key={l.userId} className={`retirement-sentence--${l.tone}`}>
                  {l.text}
                </li>
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
        /* ── Maquetación de la página (W13): DOS COLUMNAS a ancho completo ─────────────────
           El owner, 2026-09-07: «datos a la izquierda y resultado a la derecha». Hasta aquí la
           vista iba «plan arriba, resultado debajo» (U1) y dentro del tope de 66rem de
           `.app-main`, así que en un monitor ancho la mitad de la pantalla estaba vacía y el
           resultado quedaba fuera de la vista mientras se tocaba el plan — que es justo cuando
           hace falta mirarlo.

           La rejilla NO estrena breakpoint: `repeat(auto-fit, minmax(min(100%, 34rem), 1fr))`
           da dos columnas a partir de ~1.100 px y UNA por debajo, con el plan primero por orden
           del DOM. Es el mismo idioma que el design system ya sanciona para las bandas de KPIs
           («no-op en escritorio»), y evita que un ancho concreto quede clavado en un `@media`. */
        <div className="retirement-layout">
          <div className="retirement-col retirement-col--plan">
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
                      WIDE_PLAN_CARDS.has(card) ? " retirement-card--wide" : ""
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
                      ) : card === "pension" ? (
                        renderPensionCard(fields)
                      ) : (
                        fields.map((f) => renderField(f))
                      )}
                      {/* C5/B4/B11 — el aviso va en la tarjeta que trae el campo, no arriba del
                          todo: sin fecha de nacimiento el servidor no publica NADA del plan y
                          el usuario tiene que saber que se arregla dos líneas más arriba. */}
                      {card === "ages" && birthDateBlocksPlan ? (
                        <p className="muted tight">
                          Sin tu fecha de nacimiento no hay fecha válida:{" "}
                          <button
                            type="button"
                            className="btn ghost text"
                            onClick={focusBirthDateField}
                          >
                            ponla aquí
                          </button>
                          .
                        </p>
                      ) : null}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </section>
          </div>

          <div className="retirement-col retirement-col--result">
          {/* ── 3 · «Resultado» ───────────────────────────────────────────────────────────── */}
          <section className="panel">
            <div className="panel-head-row retirement-result-head">
              <h3 className="panel-title">Resultado</h3>
              <HelpPopover
                title={HELP_TEXTS["retirement.plan_sentence"].title}
                body={HELP_TEXTS["retirement.plan_sentence"].body}
              />
              {/* W13 — el ÚNICO indicio de «se está recalculando». Un punto y una palabra junto
                  al título: no sustituye contenido, no ocupa una línea propia y no cambia la
                  altura de nada (`.retirement-refreshing` no envuelve). Lo que había antes era
                  la pantalla entera apagándose, que es lo que el owner leyó como parpadeo. */}
              {metricsRefreshing ? (
                /* SIN `aria-live`: el indicador de guardado de la cabecera ya es una región viva
                   (`role="status"`, `saveIndicatorLabel`) y anuncia «Guardando…» en el mismo
                   instante. Dos regiones vivas que se disparan a la vez con cada tecla convierten
                   el lector de pantalla en el equivalente sonoro del parpadeo que esto arregla.
                   El texto sigue ahí y se lee al navegar por la cabecera. */
                <span className="retirement-refreshing">
                  <span className="retirement-refreshing-dot" aria-hidden />
                  Actualizando…
                </span>
              ) : null}
            </div>

            {/* U7 — la cabecera de resultados es una FRASE, no tres tarjetas que el usuario
                tenga que volver a juntar en su cabeza. Los dos estados que no son un plan
                —bloque ausente y `not_reachable`— los dice la propia frase
                (`lib/plan-sentence.ts`): aquí no se re-decide ninguno. */}
            <p className={`retirement-sentence retirement-sentence--${sentence.tone}`}>
              {retirementMetricsReady ? sentence.text : "Calculando tu plan…"}
            </p>
            {/* W13 — slot de altura RESERVADA. La nota aparece al teclear y desaparece cuando el
                guardado cuaja: sin el slot, esa línea entraba y salía del flujo justo en el
                instante en que las cifras de abajo se actualizan, y la página daba el salto que
                el owner describe como «la altura baila». */}
            <div className="retirement-note-slot">
              {profileDirty ? (
                <p className="muted tight">
                  Tus cambios aún no están en estas cifras: se recalculan al guardar.
                </p>
              ) : null}
            </div>

            {/* Todos los avisos, con su piel: el rojo de D17 como error y los ámbar como info.
                No es un error (la simulación existe y la fecha no se mueve): dicen que el plan
                está degradado, y van antes de las cifras porque cambian cómo se leen todas. */}
            {topNotices.map((n) => (
              <div
                key={n.code}
                className={`banner ${n.tone === "danger" ? "error-banner" : "info-banner"}`}
                role="status"
              >
                {n.text}
              </div>
            ))}

            {installationInflationPct <= 0 ? (
              <div className="banner info-banner">
                Con la inflación a 0 %, tu gasto de jubilación se queda plano en dinero de hoy:
                la fecha que ves puede ser optimista frente a lo que costará vivir entonces.{" "}
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
            {tiles.length === 0 ? (
              /* W13 — placeholders del MISMO tamaño, no un hueco. `buildRetirementTilesV2`
                 devuelve siempre 2–3 tarjetas en cuanto hay serie, así que una lista vacía solo
                 significa «todavía no ha llegado la primera respuesta»: reservar su altura evita
                 que la página entera se desplace cuando llega. */
              <div
                className="metric-grid retirement-tiles-grid retirement-tiles-grid--placeholder"
                aria-hidden
              >
                <div className="retirement-tile-placeholder" />
                <div className="retirement-tile-placeholder" />
                <div className="retirement-tile-placeholder" />
              </div>
            ) : (
              <div className="metric-grid retirement-tiles-grid">
                {tiles.map((t) => (
                  <MetricCard
                    key={t.key}
                    label={t.label}
                    /* El id puede ser uno de los que W8 todavía no ha escrito: se pasa solo si
                       el catálogo lo tiene, porque `MetricCard` lo lee sin red y un `undefined`
                       reventaría la tarjeta. Cuando W8 aterrice, el interrogante aparece solo. */
                    helpId={
                      helpTextOrNull(t.helpId as HelpTextId)
                        ? (t.helpId as HelpTextId)
                        : undefined
                    }
                    value={t.value}
                    parenthetical={t.subtitle}
                    tone={t.tone === "danger" ? "danger" : "default"}
                  />
                ))}
              </div>
            )}

            {/* Nivel 2 en segundo plano: la curva de capital necesario por edad llegará sola —
                y desde W13 se PIDE sola (sondeo con backoff en `App.tsx`, cotas en
                `lib/stale-data.ts`), en vez de quedarse en «calculando» hasta que el usuario
                cambiara de pestaña. Mismo slot de altura reservada que la nota de arriba. */}
            <div className="retirement-note-slot">
              {neededCurveComputing ? (
                <p className="muted tight">Calculando el capital necesario por edad…</p>
              ) : null}
            </div>

            {/* U5 — UN gráfico: patrimonio, capital necesario, banda de escenarios, la marca de
                tu fecha y los hitos del plan, todos sobre el mismo eje y hasta el horizonte.
                Antes eran dos charts con ejes X distintos que el usuario emparejaba a ojo. */}
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
                  series={shownSeries}
                  height={260}
                  /* El hito de jubilación lo dibujan la MARCA de la fecha (con su éxito en el
                     rótulo) y, si no la hay, las marcas secundarias: dejar también `showJub`
                     pintaría dos líneas verticales en el mismo mes. */
                  showJub={false}
                  showPhases
                  showAreas={false}
                  zoomY
                  /* Decisión C11 (#228): la línea principal es el patrimonio LÍQUIDO, no el
                     total — la misma magnitud que la banda de arriba y que la curva de capital
                     necesario de abajo. */
                  netWorthSeries={chartNetWorthSeries}
                  band={showBand ? bandPoints : null}
                  markers={chartMarkers}
                  neededCurve={neededCurve}
                  validDateMark={validDate.mark}
                  successStrip={successStrip}
                  successStripCutoffs={riskCutoffs}
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
                      /* Decisión C11 (#228): la línea del chart de Jubilación es el patrimonio
                         LÍQUIDO, no el total — «Patrimonio neto» (Resumen, Proyección) sería
                         una etiqueta que promete otra cifra. */
                      label: "Patrimonio líquido",
                      color: "var(--proj-nw)",
                      swatch: "line",
                    },
                    /* La curva de capital necesario sustituye a la del objetivo FIRE: en v2 no
                       hay objetivo que cruzar, hay el líquido que hace cumplir tu umbral
                       jubilándote en cada mes. Su rótulo y su color viven en
                       `NEEDED_CAPITAL_SERIES` para que la leyenda no pueda desincronizarse de
                       la polilínea. */
                    ...(neededCurve && neededCurve.some((p) => p.value != null)
                      ? ([
                          {
                            key: NEEDED_CAPITAL_SERIES.key,
                            label: NEEDED_CAPITAL_SERIES.label,
                            color: NEEDED_CAPITAL_SERIES.color,
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
                {/* La nota que sustituye a la MARCA cuando no hay ninguna que pintar: sin
                    fecha válida al umbral, o el nivel 1 todavía resolviéndola. Va bajo la
                    leyenda porque explica una ausencia del chart, no una serie. */}
                {validDate.note ? (
                  <p className="muted tight">{validDate.note}</p>
                ) : null}
                {/* La ESCALA del color (V5). No es un ítem de `ChartLegend` a propósito: una
                    leyenda nombra SERIES, y esto es una escala continua — meterla ahí la haría
                    parecer una línea más del gráfico.
                    Los tres peldaños salen de `riskCutoffs`, los MISMOS que tiñen la banda y la
                    tira de éxito: con el umbral al 80 % el rojo empieza en el 20 % de fallo, y
                    una escala fija diría «10 % o más» sobre una banda que no se pone roja ahí. */}
                {gradientStops.length > 1 ? (
                  <p className="retirement-risk-scale">
                    <span className="label-with-help">
                      <strong>Banda 10–90 %</strong>
                      <HelpFor id={RESULT_HELP.failureByAge.helpId} />
                    </span>{" "}
                    · el color dice qué parte de los escenarios ya ha fallado a esa edad —se
                    quedaron sin dinero, se pasaron de tasa inicial o su regla no cubrió el
                    gasto:{" "}
                    {(
                      [
                        [0, "ninguno"],
                        [riskCutoffs.amber, formatPercentDisplay(riskCutoffs.amber * 100)],
                        [
                          riskCutoffs.red,
                          `${formatPercentDisplay(riskCutoffs.red * 100)} o más`,
                        ],
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
                              "--ff-risk-swatch": riskColorForProbability(p, riskCutoffs),
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
              ) : !shownBands ? (
                projectionBandsBusy ? (
                  <p className="muted tight">Sorteando escenarios…</p>
                ) : (
                  <p className="muted tight">Aún no hay escenarios que mostrar.</p>
                )
              ) : (
                <>
                  {/* C5 — sin σ declarada el sorteo no dispersa y el éxito sale 0 % o 100 % por
                      construcción. Es un AVISO, no un bloqueo: las cifras de abajo siguen
                      siendo las que el servidor calculó, y esconderlas dejaría la pantalla sin
                      su desglose justo cuando más hace falta explicarlo. */}
                  {noVolatilityInRiskBlock ? (
                    <div className="banner info-banner">
                      Sin volatilidad declarada: la banda es la línea y el éxito no mide riesgo.
                      Añade la volatilidad anual a tus activos.{" "}
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
                  ) : null}
                  {/* El KPI «Éxito del plan» ya vive en la CABECERA (`buildRetirementTilesV2`,
                      tile #2, siempre presente). Aquí no se repite la tarjeta: repetirla es
                      cómo la misma pantalla acaba enseñando dos cifras del mismo sorteo. Lo que
                      queda es lo que la cabecera no lleva — la precisión, las filas de detalle y
                      la nota de coste/semilla. */}
                  {/* La PRECISIÓN de la cifra de al lado, con los números del servidor y sin
                      aritmética de cliente: el semiancho de Wilson y el tamaño de la muestra.
                      Sin ella, un 95,0 % se lee como exacto cuando lo que hay es un intervalo —
                      y con 0 fallos ni siquiera hay intervalo, sino la cota de la regla de tres
                      que la propia tarjeta ya cita en su subtítulo.

                      Con `success_absent_reason` (A12) no hay cifra que acotar: el sorteo publica
                      `success_sampling_error_pp: null` y la línea de siempre imprimiría «Precisión
                      del sorteo: — sobre 2500 caminos», un guion mudo que se lee como «el dato no
                      ha llegado» cuando lo cierto es que la pregunta no tiene respuesta. Se
                      sustituye por el MOTIVO, que es lo único que se puede afirmar. */}
                  {shownBands.success_absent_reason != null ? (
                    <p className="muted tight">
                      Sin éxito que medir:{" "}
                      {successAbsentReasonEs(shownBands.success_absent_reason)}. Los{" "}
                      {shownBands.paths} caminos sorteados describen tu patrimonio sin
                      jubilarte: la banda sigue siendo tuya, pero no hay ningún plan al que
                      ponerle una probabilidad.
                    </p>
                  ) : (
                    <p className="muted tight">
                      Precisión del sorteo:{" "}
                      {formatSamplingErrorPp(shownBands.success_sampling_error_pp)} sobre{" "}
                      {shownBands.paths} caminos (intervalo de Wilson al 95 %; con cero
                      fallos, la cota de la regla de tres).
                    </p>
                  )}
                  {/* Lo que hace AUDITABLE el número grande: por qué falla el que falla, cuánto
                      se apretó el cinturón el que aguantó. Antes vivían en el «Detalle»
                      plegado, que es donde nadie mira cuando la cifra de arriba no le cuadra. */}
                  {riskExtraRows.length > 0 ? (
                    <div className="risk-extra-rows">
                      {riskExtraRows.map((r) => (
                        <div key={r.key} className="risk-extra-row">
                          <div className="risk-extra-head">
                            {/* La ayuda cuelga del RÓTULO, no del bloque: estas filas miden
                                cosas distintas y una sola ayuda arriba explicaría la que el
                                usuario no está mirando. */}
                            <span
                              className={
                                r.helpId
                                  ? "label-with-help risk-extra-label"
                                  : "risk-extra-label"
                              }
                            >
                              {r.label}
                              {r.helpId ? <HelpFor id={r.helpId as HelpTextId} /> : null}
                            </span>
                            <span className="risk-extra-value">{r.value}</span>
                          </div>
                          {r.detail ? (
                            <span className="risk-extra-detail">{r.detail}</span>
                          ) : null}
                        </div>
                      ))}
                    </div>
                  ) : null}
                  {/* Coste, tamaño de la muestra y semilla: sin ellos la probabilidad no tiene
                      precisión declarada ni se puede reproducir el sorteo. */}
                  <p className="risk-footnote">{riskFootnote(shownBands)}</p>
                </>
              )}
            </div>

            {/* ── «Detalle del cálculo» ─────────────────────────────────────────────────────
                No es un cajón de sastre: son las lecturas de SEGUNDO orden —las que matizan una
                cifra de arriba en vez de responder una pregunta propia— más los avisos. Que
                estén plegadas no las hace opcionales; que estén fuera de la cabecera es lo que
                permite leer la cabecera de un vistazo. */}
            {planDetailRows.length > 0 || shownBands ? (
              <details className="retirement-detail bordered-top">
                <summary className="details-trigger">Detalle del cálculo</summary>
                <div className="risk-extra-rows">
                  {planDetailRows.map((r) => (
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
                          {r.helpId ? <HelpFor id={r.helpId as HelpTextId} /> : null}
                        </span>
                        <span className="risk-extra-value">{r.value}</span>
                      </div>
                    </div>
                  ))}
                  {shownBands ? (
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
              </details>
            ) : null}
          </section>
          </div>
        </div>
      )}

      {hasMembership &&
      !projectionBusy &&
      !retirementBusy &&
      (!shownSeries || !retirementBudgetSnapshot) ? (
        <div className="banner info-banner">Sin datos.</div>
      ) : null}
    </div>
  );
}
