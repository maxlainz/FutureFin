/**
 * Tipos espejo de los `*Response` / `*Row` que devuelve la API Rust. Importes monetarios y
 * porcentajes viajan como `string` (Decimal serializado), no como `number`, para preservar
 * precisión.
 */

export type HealthResponse = {
  status: string;
  service: string;
  version: string;
};

export type UserResponse = {
  id: string;
  username: string;
  /** YYYY-MM-DD desde la API; ausente en clientes antiguos. */
  birth_date?: string | null;
  /**
   * ¿Tiene la cuenta contraseña propia? `false` = identidad delegada (el add-on de Home
   * Assistant crea la cuenta sin contraseña y no debe tener ninguna). Decide qué pide el modal
   * de exportar `.ffbackup`: la contraseña de la cuenta, o una contraseña propia del archivo.
   */
  has_password: boolean;
};

export type FireNumberModeApi = "manual" | "annual_expense" | "current_income";

/** Fuente del ahorro de la simulación:
 *  - `budget` (modo A, default): ingresos y gastos del presupuesto.
 *  - `transactions_avg` (modo B): promedio real de los últimos 12 meses de movimientos.
 *  - `budget_income_real_expense` (modo C): ingresos del presupuesto + gasto real promedio
 *    de los últimos 12 meses (mismo promedio ponderado que B, restando cuotas de préstamos).
 *  Ausente en clientes/backups antiguos → `budget`. */
export type SavingsSourceApi =
  | "budget"
  | "transactions_avg"
  | "budget_income_real_expense";

export type TaxBracketApi = {
  up_to: string | null;
  pct: string;
};

/**
 * Supuestos FIRE **del hogar**. En 5.0.0 perdió cuatro ejes —`fire_number_mode`,
 * `fire_number_manual_amount`, `swr_pct` y `horizon_lifespan_age`—, que pasaron a ser
 * personales y viven en `RetirementProfileApi` (decisión D13): con proyecciones independientes
 * por miembro no podían seguir siendo del hogar. Aquí quedan solo los supuestos compartidos.
 */
export type FireSettingsApi = {
  taxes_enabled: boolean;
  tax_brackets: TaxBracketApi[];
  /** Ausente en clientes/backups antiguos → tratar como `budget`. */
  savings_source?: SavingsSourceApi;
  /** Ventana del promedio de INGRESO en meses (1–60). Solo la usa el modo `transactions_avg`. */
  income_avg_window_months?: number;
  income_avg_window_mode?: AvgWindowModeApi;
  /** Ventana del promedio de GASTO en meses (1–60). La usan `transactions_avg` y
   *  `budget_income_real_expense`. */
  expense_avg_window_months?: number;
  expense_avg_window_mode?: AvgWindowModeApi;
  /** Fracción de plusvalía gravable de la retirada (0..=1, string; default "1") — 4.10.0, #140. */
  taxable_gain_ratio?: string;
};

/** Cómo se cuentan los meses de una ventana del promedio real.
 *  `calendar` = los meses con datos dentro de los últimos N civiles.
 *  `data` = los N meses CON DATOS más recientes, saltando los vacíos. */
export type AvgWindowModeApi = "data" | "calendar";

// ---------------------------------------------------------------------------
// Perfil de jubilación POR USUARIO — `GET|PATCH /v1/auth/me/retirement-profile`
// (5.0.0, issue #207, decisión D13). Espejo EXACTO de `RetirementProfile`
// (`apps/api/src/handlers/retirement_profile.rs`): todo Decimal viaja como string y cada
// enumerado es la misma lista de literales que acepta el servidor.
// ---------------------------------------------------------------------------

/**
 * Las CUATRO estrategias (modelo v2, C7): deciden el trigger de la fecha válida («el éxito define
 * la fecha»). `pension_bridge` YA NO es un literal del tipo — el servidor lo sigue aceptando en la
 * DESERIALIZACIÓN de un perfil guardado como ALIAS de `asap` (activa el puente con sus defaults),
 * publica el aviso `strategy_pension_bridge_migrated` y nunca lo vuelve a EMITIR. El puente pasó
 * de ser una estrategia a ser un ajuste de la tarjeta Pensión disponible en cualquier estrategia
 * (ver `PensionPlanApi.bridge_enabled`).
 */
export type RetirementStrategyApi =
  | "asap"
  | "retire_at_age"
  | "coast"
  | "partial";

/** Catálogo de reglas de retirada (D6). */
export type WithdrawalRuleKindApi =
  | "fixed_real"
  | "percent_of_balance"
  | "hybrid"
  | "guardrails";

/** Qué relación tiene la regla con el gasto declarado (D5). */
export type SpendModeApi = "ceiling" | "rule_is_spend";

/** Base de gasto de la fase de media jornada (D10). */
export type PartialExpenseBasisApi = "retirement" | "regular";

/**
 * Regla de retirada + su modo de gasto. Los `pct` son BRUTOS de impuestos (como el SWR).
 * En el PATCH viaja ENTERA, nunca campo a campo: qué `pct` son obligatorios depende de `kind`.
 */
export type WithdrawalRuleApi = {
  kind: WithdrawalRuleKindApi;
  /** `percent_of_balance` y `guardrails`: % anual del líquido. */
  pct: string | null;
  /** `hybrid`: % de partida. */
  start_pct: string | null;
  /** `hybrid`: % al que se baja tras el latch (estrictamente menor que `start_pct`). */
  end_pct: string | null;
  /** `guardrails`: banda alrededor de la tasa inicial que dispara el ajuste. */
  band_pct: string | null;
  /** `guardrails`: cuánto se recorta/sube la retirada al tocar una banda. */
  adjust_pct: string | null;
  spend_mode: SpendModeApi;
};

/**
 * Pensión pública (u otra renta vitalicia) CON FECHA (D3/D8): su inicio es un flujo de caja más
 * que entra en el bucle — desde el modelo v2 (C1/M4) NO dimensiona ningún objetivo ni se descuenta.
 */
export type PensionPlanApi = {
  /** Importe MENSUAL en euros de HOY (> 0). */
  monthly_amount_today: string;
  starts_at_age: number;
  /** `true` (default) = se indexa a la inflación del hogar; `false` = importe plano. */
  indexed: boolean;
  /** Fracción del importe que se cobra DURANTE la media jornada, en [0, 1]. Default `"0"`. */
  fraction_while_partial: string;
  /** Puente activo (C2/C7): en el mes de jubilación, si faltan ≤ `bridge_max_years` para que
   *  empiece la pensión, el tope de la tasa inicial es `bridge_max_pct` en vez del SWR (nunca un
   *  tope mensual: F1/F3 juzgan el resto). Disponible en CUALQUIER estrategia. Default `false`. */
  bridge_enabled: boolean;
  /** % ANUAL (Decimal-string), estrictamente MAYOR que el SWR — si no fuera mayor no sería un
   *  puente, sería la misma tasa. `null` con `bridge_enabled: false`; default al activarlo: `max(5, swr + 1)` como string decimal. */
  bridge_max_pct: string | null;
  /** Años máximos de antelación sobre el inicio de la pensión durante los que aplica el puente;
   *  la fecha válida nunca cae antes de `pensión − este número` de años. `null` con
   *  `bridge_enabled: false`; default al activarlo: `7`. */
  bridge_max_years: number | null;
};

/** Fase de media jornada (P7, Barista FIRE, M11). Sin fin propio: termina en la jubilación total. */
export type PartialRetirementApi = {
  /** `at_age` (modo A, default): la fase empieza a la edad de `starts_at_age`. `asap` (modo B):
   *  empieza en cuanto el plan puede permitírselo — el servidor resuelve el primer mes posible
   *  (`earliest_partial_start`) y lo publica en `partial_start_month_index` de la serie. */
  mode: "at_age" | "asap";
  /** OBLIGATORIA con `mode: "at_age"`; `null` con `mode: "asap"` (la edad de inicio sale de la
   *  serie, no de aquí). */
  starts_at_age: number | null;
  /** Ingreso MENSUAL en euros de HOY durante la fase (>= 0; `0` = año sabático). */
  income_monthly_today: string;
  expense_basis: PartialExpenseBasisApi;
};

/** Modo de coast/FIRE (M10): `fixed_retirement_age` (modo A, default) resuelve el PRIMER mes en
 *  que se puede dejar de aportar y aun así llegar a `target_retirement_age`; `fixed_stop_age`
 *  (modo B) fija el mes de parada (`coast_stop_age`) y deja que la fecha válida salga donde
 *  salga. */
export type CoastModeApi = "fixed_retirement_age" | "fixed_stop_age";

/**
 * Perfil de jubilación de UN usuario, tal y como lo devuelve el servidor: YA resuelto (defaults y
 * clamps aplicados). Modelo v2 (C1-C8): sin objetivo, sin base de objetivo, sin descuento, sin
 * colchón — el éxito estocástico define la fecha (ver el bloque «plan» de `ProjectionSeriesApi`).
 */
export type RetirementProfileApi = {
  strategy: RetirementStrategyApi;
  /** OBLIGATORIA en `retire_at_age` y con `coast_mode: "fixed_retirement_age"` (en modo `fixed_stop_age` la resuelve el sorteo y el campo se esconde); opcional en `partial`
   *  modo A; ignorada por el resto. */
  target_retirement_age: number | null;
  /** Los cuatro ejes que en 4.15.x vivían en `fire_settings` (mismos defaults y cotas). */
  fire_number_mode: FireNumberModeApi;
  fire_number_manual_amount: string | null;
  swr_pct: string;
  horizon_lifespan_age: number;
  /** Umbral de éxito (C3): entero 80–100, default 95. Restricción sobre la fecha válida —
   *  `< 100` exige que el límite inferior del intervalo de Wilson al 95 % (`success_wilson_low`)
   *  supere `umbral/100`; `= 100` exige CERO fallos de N caminos, y entonces se publica además la
   *  cota de la regla de tres. */
  success_threshold_pct: number;
  coast_mode: CoastModeApi;
  /** OBLIGATORIA con `coast_mode: "fixed_stop_age"`; `null` en cualquier otro caso (con
   *  `fixed_retirement_age` el mes de parada lo resuelve el servidor, `coast_stop_month_index`
   *  en la serie). */
  coast_stop_age: number | null;
  withdrawal_rule: WithdrawalRuleApi;
  pension: PensionPlanApi | null;
  partial_retirement: PartialRetirementApi | null;
};

/** Respuesta de las dos rutas: el perfil resuelto + la fecha de nacimiento (misma pantalla). */
export type RetirementProfileResponseApi = {
  profile: RetirementProfileApi;
  birth_date: string | null;
};

/**
 * Cuerpo del PATCH. **Tri-estado**: una clave ausente NO cambia nada; `null` borra lo opcional.
 * Nunca se manda el perfil entero — un PATCH «solo el SWR» borraría la pensión declarada.
 */
export type RetirementProfilePatchApi = {
  strategy?: RetirementStrategyApi;
  target_retirement_age?: number | null;
  fire_number_mode?: FireNumberModeApi;
  fire_number_manual_amount?: string | null;
  swr_pct?: string;
  horizon_lifespan_age?: number;
  success_threshold_pct?: number;
  coast_mode?: CoastModeApi;
  /** Tri-estado: `null` suelta la edad de parada fija (solo tiene sentido con `coast_mode:
   *  "fixed_stop_age"`, pero el servidor la conserva aunque no aplique — igual que hoy). */
  coast_stop_age?: number | null;
  withdrawal_rule?: WithdrawalRuleApi;
  pension?: PensionPlanApi | null;
  partial_retirement?: PartialRetirementApi | null;
  /** Misma columna que `PATCH /v1/auth/me`: `null` la borra, `"YYYY-MM-DD"` la fija. */
  birth_date?: string | null;
};

export type InstallationSnapshot = {
  id: string;
  base_currency: string;
  /** IANA TZ for civil "today" (liability derive, etc.) */
  calendar_tz?: string;
  /** Tasa de inflación anual %. `0` = target FIRE plano; > 0 = target móvil que crece con la inflación. */
  annual_inflation_assumption_percent: string;
  show_age_mode: string;
  /** Ausente en clientes antiguos; usar `defaultFireSettingsApi`. */
  fire_settings?: FireSettingsApi;
  /** Kill-switch vivo de las tools de escritura MCP. Ausente en backends antiguos → `true`. */
  mcp_write_enabled?: boolean;
  /** `false` mientras el hogar no haya pasado por el asistente de primera vez (3.10.0).
   *  Ausente en backends antiguos → se trata como completado, para no lanzar el asistente a
   *  quien ya tenía su hogar configurado. */
  onboarding_completed?: boolean;
};

export type InstallationAccess = {
  installation: InstallationSnapshot;
  role: "owner" | "member" | "viewer";
};

/** Fila de GET /v1/installation/members (legible por cualquier miembro, viewer incluido). */
export type MemberApiRow = {
  user_id: string;
  username: string;
  role: "owner" | "member" | "viewer";
  joined_at: string;
};

export type InstallationSessionContext = {
  installation_initialized: boolean;
  access: InstallationAccess | null;
};

export type InstallationGate =
  | "loading"
  | "fetch_failed"
  | "member"
  | "pending"
  | "bootstrap";

export type CategoryScope = "asset" | "liability" | "income" | "expense";

export type CategoryRow = {
  id: string;
  scope: CategoryScope;
  name: string;
  sort_index: number;
  /** Categoría POR DEFECTO de su ámbito (4.15.0): la que recibe todo ingreso/gasto que llegue sin
   *  categoría, por cualquier vía. Hay EXACTAMENTE UNA por instalación y ámbito, y solo en
   *  `income`/`expense` (nunca en `asset`/`liability`). No se puede borrar ni desmarcar: se
   *  traslada marcando otra. */
  is_fallback: boolean;
};

export type AssetApiRow = {
  id: string;
  category_id: string;
  name: string;
  current_value: string;
  purchase_price: string | null;
  is_liquid: boolean;
  /** Dueño de la fila (dato de display, no frontera de seguridad). Alimenta el trigger del
   *  modal de snapshot en vista household. Ausente en clientes antiguos. */
  owner_user_id?: string;
  expected_annual_return_percent?: string | null;
  /** Desviación típica anual de los retornos del activo, en % ([0, 100]) — 5.0.0, §A.2.
   *  `null`/ausente o `0` = activo determinista. Solo alimenta las bandas de Monte Carlo; el
   *  camino determinista la ignora. */
  annual_volatility_percent?: string | null;
  /** Aporte del PRIMER MES resuelto por la cascada. Incluye el tramo de los planning flows sin
   *  fecha del mes en curso, así que baja cada día y salta el día 1. El servidor lo envía
   *  siempre; era opcional aquí por deriva del tipo. */
  contribution_nominal_monthly: string;
  /** La misma cascada sobre el neto recurrente (sin planning): el número estable. */
  contribution_recurring_monthly: string;
  /** Tope absoluto en € si una regla apunta a este activo con cap_kind='amount'. */
  contribution_target_amount?: string | null;
  notes: string | null;
  sort_index: number;
};

/**
 * Cuerpo de `POST /v1/assets` y `PATCH /v1/assets/{id}` tal y como lo arma el formulario
 * (`lib/asset-form.ts`).
 *
 * **Tri-estado en los tres campos opcionales de importe** (`purchase_price`,
 * `expected_annual_return_percent`, `annual_volatility_percent`) desde 5.0.0: clave AUSENTE = no
 * cambia, `null` = BORRA el valor guardado, un decimal-string lo fija. Hasta 4.15.x los dos
 * últimos eran opción simple en el servidor y un `null` era indistinguible de omitir: vaciar el
 * campo no devolvía el activo a determinista. Por eso `undefined` y `null` significan cosas
 * distintas aquí y el builder los emite a propósito.
 */
export type AssetWriteBodyApi = {
  category_id?: string;
  name?: string;
  current_value?: string;
  is_liquid?: boolean;
  purchase_price?: string | null;
  expected_annual_return_percent?: string | null;
  annual_volatility_percent?: string | null;
  notes?: string;
};

export type AllocationRuleKind = "fixed" | "percent" | "remainder";
export type AllocationRuleCapKind = "amount" | "months_expense" | "income_multiple";

export type AllocationRuleApiRow = {
  id: string;
  target_asset_id: string;
  priority: number;
  kind: AllocationRuleKind;
  amount?: string | null;
  cap_kind?: AllocationRuleCapKind | null;
  cap_value?: string | null;
  enabled: boolean;
  notes?: string | null;
  owner_user_id?: string | null;
};

/** De dónde sale UN lado (ingreso o gasto) del ahorro efectivo, y con qué base exacta. */
export type SavingsAvgBasisApi = {
  /** `"budget"` = este lado salió del presupuesto (el modo no lo promedia, o cayó por falta de
   *  datos). `"average"` = promedio real de transacciones. */
  basis: "budget" | "average";
  /** Denominador realmente usado. `0` ⟺ `basis === "budget"`. */
  /** Denominador realmente usado: los meses que se promediaron. */
  avg_months: number;
  /** Ventana configurada tras el clamp: permite decir «pediste 12, hay 7». */
  window_months: number;
  window_mode?: "data" | "calendar";
  /** Mes más antiguo incluido, `YYYY-MM`. */
  first_month?: string;
  /** Mes más reciente incluido, `YYYY-MM`. */
  last_month?: string;
  /** `true` ⟺ los meses incluidos NO son consecutivos: no se pueden pintar como un rango. */
  has_gaps: boolean;
};

export type FinancialHealthMetrics = {
  income_monthly_equivalent: string;
  expense_regular_monthly_equivalent: string;
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
  savings_rate: string | null;
  liquid_assets_total: string;
  /** El valor `1200` es el tope del bucle del servidor y significa «al menos 100 años» (un
   *  suelo, no una medida exacta). */
  runway_months: string | null;
  /** `true` cuando la retirada anual (12 × `expense_total_monthly_equivalent`, grosseada con los
   *  tramos fiscales de `fire_settings` igual que el target FIRE) no supera el SWR de la
   *  instalación aplicado a `liquid_assets_total`. En ese caso `runway_months` viene `null`.
   *  Con `swr_pct = 0` nunca es `true`. */
  runway_is_indefinite?: boolean;
  /** Σ de los Próximos PUNTUALES (€, sin ventana ni anualizar) — los recurrentes van aparte. */
  upcoming_inflows_total: string;
  upcoming_outflows_total: string;
  /** **€/MES** (#148): Σ de los Próximos recurrentes por scope, sin mirar sus ventanas.
   *  Jamás se suma con los totales en €. Ausente en backends < 4.11.0. */
  upcoming_recurring_monthly_inflow?: string;
  upcoming_recurring_monthly_outflow?: string;
  upcoming_recurring_count?: number;
  upcoming_coverage_ratio: string | null;
  /** Fuente EFECTIVA del ahorro tras el fallback del servidor (`budget` si el modo
   *  configurado usaba transacciones —`transactions_avg` o `budget_income_real_expense`—
   *  pero no había meses con datos). Ausente en backends antiguos → `budget`. */
  savings_source?: SavingsSourceApi;
  /** Procedencia del lado INGRESO del ahorro efectivo. */
  savings_income_basis?: SavingsAvgBasisApi;
  /** Procedencia del lado GASTO. */
  savings_expense_basis?: SavingsAvgBasisApi;
  /** Neto mensual del PRESUPUESTO, capturado ANTES del override B/C — no sigue el modo. Es el
   *  denominador del delta «vs plan» de la tarjeta de ahorro. */
  savings_expected_monthly_equivalent?: string;
  /** Rendimiento anual **nominal** esperado del patrimonio neto, ya en PORCENTAJE (`"3.5556"` =
   *  3,5556 %/año), no en fracción como `savings_rate`. Suma de `valor × rentabilidad esperada`
   *  de todos los activos menos `principal × TIN` de los pasivos vivos, sobre el patrimonio neto.
   *  Ausente ⟺ el patrimonio neto no es positivo. */
  net_return_nominal_annual_pct?: string | null;
  /** El mismo rendimiento descontada la inflación configurada, dividiendo factores
   *  (`(1+n)/(1+i) − 1`), no restando puntos. Presente/ausente a la vez que el nominal. */
  net_return_real_annual_pct?: string | null;
};

export type CategoryBreakdownLineApi = {
  category_id: string;
  category_name: string;
  total: string;
};

/**
 * El plan de jubilación del usuario tal y como lo publica `GET /v1/summary` (5.0.0 WP5-2b, D27).
 * Modelo v2: «el éxito define la fecha» (C1-C8) — sin objetivo, sin colchón, con `success_of_plan`
 * como KPI central.
 *
 * **No es un cálculo del Resumen**: sale de la MISMA cache de plan direccionada por contenido
 * (`plan_cache`) que pinta Jubilación y Proyección, copiando campos — nunca un sorteo aparte con
 * otra semilla enseñaría dos éxitos distintos del mismo plan en la misma pantalla.
 * `required_savings_monthly` ES `contribution_required_monthly` de `/v1/projection/series` con el
 * nombre que se lee en un Resumen, y `underfunded` viaja con su misma base y sus mismos `null`.
 *
 * **`disposable_monthly` se retiró en A12** (de este tipo y del struct de Rust): el servidor lo
 * publicaba SIEMPRE `null` desde que el modelo v2 se llevó los `disposable_*` de la serie — el
 * ahorro que el coast libera es «disponible» por decisión del usuario, no una cifra que el motor
 * calcule—, y un campo obligatorio que solo puede valer `null` es una promesa de cifra que nadie
 * puede cumplir.
 */
export type SummaryPlanApi = {
  strategy: RetirementStrategyApi | null;
  /** Mes EFECTIVO de jubilación en la rejilla de `points[].month_index` (0 = hoy). `null` con
   *  `absent_reason` y también —sin él— cuando el plan no se jubila dentro del horizonte: eso es
   *  un resultado, no un hueco. */
  jubilacion_month_index: number | null;
  /** €/mes (Decimal-string). `null` con las estrategias por cruce: no hay edad contra la que
   *  resolver nada. */
  required_savings_monthly: string | null;
  /** El rojo de D17. **`null` = la pregunta no aplica**, nunca `false` para decir «no aplica». */
  underfunded: boolean | null;
  /** `household_aggregate` | `projection_unavailable`. `null` ⟺ el plan es el del usuario. */
  absent_reason: string | null;

  // ── modelo v2 — «el éxito define la fecha» (sustituye al KPI D25/D28 de WP6b) ─────────────
  /**
   * FRACCIÓN [0,1]: éxito del plan EN SU FECHA VÁLIDA — caminos que no fallan (F1/F2/F3) de N.
   *
   * **Es EXACTAMENTE el número del fan chart** de `GET /v1/projection/bands` (`success_of_plan`
   * ahí) — el mismo cache de plan, los mismos caminos, la misma semilla. Nunca se recalcula
   * aquí. `null` sin fecha válida (ver `plan_state`).
   */
  success_of_plan: number | null;
  /** FRACCIÓN [0,1]: cota INFERIOR del intervalo de Wilson al 95 % de `success_of_plan` — el
   *  número contra el que el servidor compara el umbral por debajo de 100 % (estable frente a la
   *  semilla y a N, que es justo lo que `success_of_plan` no es). Mismo valor, mismo cache de
   *  plan, que `ProjectionSeriesApi.success_wilson_low` y que `ProjectionBandsApi.success_wilson_low`.
   *  `null` a la vez que `success_of_plan`. */
  success_wilson_low?: number | null;
  /** Umbral EFECTIVO contra el que se evaluó `success_of_plan` (eco del perfil, 80–100). `null`
   *  a la vez que `success_of_plan`. */
  success_threshold_pct: number | null;
  /** Mes de la fecha válida, en la misma rejilla que `jubilacion_month_index` (con el que
   *  coincide cuando la fecha la fija el umbral y no una edad configurada). `null` sin fecha
   *  válida. */
  safe_date_month_index: number | null;
  /** Euros de HOY (Decimal-string), redondeados a CIENTOS hacia arriba: el líquido que sostiene
   *  el plan al umbral si te jubilaras ya. Misma cifra, al euro, que en Jubilación y Proyección.
   *  `null` sin activos líquidos o sin fecha de nacimiento resuelta. */
  needed_capital_today: string | null;
  /** `ready` = el bloque de arriba (fecha, éxito, capital) viaja resuelto; `pending` = el nivel 1
   *  del solve todavía se está calculando (primer GET tras una mutación; reintentar); `absent` =
   *  no se pudo resolver, ver `absent_reason`. */
  plan_state: "ready" | "pending" | "absent";
  success_verdict?: SuccessVerdictApi | null;
  /**
   * Por qué falta específicamente la probabilidad —`success_of_plan`, `success_wilson_low`,
   * `success_threshold_pct` y `success_verdict` van a `null` juntas— cuando el resto del plan SÍ
   * viaja (`plan_state: "ready"`). Distinto de `absent_reason`: «no sabemos tu probabilidad» ≠
   * «no sabemos tu plan», y por eso son dos campos y no uno.
   *
   * **Dos literales reales**, los que emite `apps/api/src/handlers/summary.rs`:
   *
   * - `not_reachable` — **ningún mes del horizonte cumple el umbral**, así que no hay fecha y por
   *   tanto no hay «éxito en su fecha» que medir (5.0.0, WP A12). Mismo literal que
   *   `retirement_date_basis` en la serie y que `success_absent_reason` en
   *   `/v1/projection/bands`. **Ojo**: la serie SÍ publica ahí un `success_of_plan`, pero
   *   significa otra cosa —la mejor observación del solve, el mes que más cerca se quedó—, así
   *   que no se puede copiar a esta tarjeta sin rotular un porcentaje junto a un plan que no
   *   ocurre.
   * - `bands_unavailable` — caso patológico: la serie no pudo representar la fracción. Ya no
   *   existe un sorteo aparte que falle (el éxito es nivel 1 del mismo solve que la fecha).
   *
   * `null` ⟺ la probabilidad viaja, o el plan entero está ausente y lo dice `absent_reason`.
   */
  success_absent_reason?: string | null;
};

export type SummaryResponse = {
  total_assets: string;
  total_liabilities: string;
  net_worth: string;
  debt_to_assets_ratio: string | null;
  financial_health: FinancialHealthMetrics;
  assets_by_category: CategoryBreakdownLineApi[];
  liabilities_by_category: CategoryBreakdownLineApi[];
  /** Tarjeta «Tu plan» (5.0.0, D27). Ausente en backends anteriores a WP5-2b. */
  plan?: SummaryPlanApi;
};

export type BudgetTotalsApi = {
  income_monthly_equivalent: string;
  income_retirement_monthly_equivalent: string;
  /** Gasto del plan: partidas manuales **+ cuotas de pasivos activos** (3.7.0). Es exactamente la
   *  suma de los `entries` de scope `expense`. */
  expense_regular_monthly_equivalent: string;
  /** Suma de entradas con `ends_at_retirement = false` — gastos que persisten tras jubilación.
   *  Solo partidas manuales: una cuota termina con su plan de pago. Es el campo correcto para el
   *  cálculo FIRE (alinea con el servidor). */
  expense_retirement_monthly_equivalent: string;
  /** Idéntico a `expense_regular_monthly_equivalent` desde la 3.7.0. */
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
};

/** Procedencia de una partida: `manual` la escribe el usuario, `liability` la deriva el plan de
 *  pago de un pasivo activo (solo lectura — se edita en Pasivos). */
export type BudgetEntrySourceApi = "manual" | "liability";

export type BudgetEntryApiRow = {
  /** Id de la partida persistida, o del pasivo cuando `source = "liability"`. */
  id: string;
  /** Ausente solo en cuotas de pasivos sin categoría de gasto asignada (pre-3.4.0). */
  category_id?: string;
  scope: "income" | "expense";
  source: BudgetEntrySourceApi;
  /** Presente ⟺ `source = "liability"` (mismo valor que `id`). */
  liability_id?: string;
  /** Etiqueta del pasivo; presente ⟺ `source = "liability"`. */
  label?: string;
  /** Importe mensual. En una cuota, el equivalente mensual del plan (`weekly` → ×52/12). */
  amount: string;
  notes: string | null;
  sort_index: number;
  persists_after_retirement: boolean;
  ends_at_retirement: boolean;
  /** En una cuota, el fin del plan de pago (`null` = plan indefinido). */
  expense_end_date: string | null;
};

export type BudgetSnapshotApi = {
  /** Partidas manuales y cuotas de pasivo en una sola lista, discriminadas por `source`. */
  entries: BudgetEntryApiRow[];
  totals: BudgetTotalsApi;
};

export type PlanningFlowDirectionApi = "inflow" | "outflow";

/** Base del importe (#148): `one_off` = TOTAL en €; `per_month` = €/MES durante la ventana. */
export type PlanningAmountBasisApi = "one_off" | "per_month";

export type PlanningFlowApiRow = {
  id: string;
  category_id: string;
  direction: PlanningFlowDirectionApi;
  title: string;
  expected_amount: string;
  /** SIEMPRE presente — la unidad del importe nunca se infiere de otro campo. */
  amount_basis: PlanningAmountBasisApi;
  due_date: string | null;
  /** Solo `per_month` (skip_serializing_if → ausente en puntuales). */
  window_start_date?: string;
  /** Ausente con `per_month` = ventana SIN FIN (misma convención que payment_end_date null). */
  window_end_date?: string;
  notes: string | null;
  sort_index: number;
  show_in_chart: boolean;
};

export type ProjectionPointApi = {
  month_index: number;
  /** f64 (serializado como número, no Decimal-as-string como los KPIs escalares). */
  net_worth: number;
  /** f64. */
  contributed_capital: number;
  /** f64, euros de HOY (4.6.0). Con inflación 0 vale lo mismo que net_worth. */
  net_worth_real?: number;
  /**
   * f64 (4.8.0, #143): patrimonio LÍQUIDO nominal — Σ activos vendibles + caja, sin restar
   * pasivos. `net_worth` sigue siendo el total del chart.
   *
   * **Ya no decide ningún cruce**: el modelo v2 (5.0.0) retiró `fire_target_series` y con él el
   * cruce que este campo comparaba. Sigue siendo la línea que el motor VENDE en la jubilación
   * (la que mide la tasa inicial de retirada, F2), la que `needed_capital_curve` cruza en la
   * fecha del plan, y —desde la decisión C11 (issue #228)— la línea PRINCIPAL del chart de
   * Jubilación (`MiniProjection`, prop `netWorthSeries`, construida por
   * `retirementNetWorthSeries` en `lib/retirement-chart.ts`): la Proyección y el Resumen
   * conservan el total.
   *
   * Declarado opcional (backend anterior a 4.8.0, o un punto histórico fusionado por
   * `mergeProjectionWithHistory` que no lo trae) — nunca cae al total en silencio: un consumidor
   * que lo necesite trata su ausencia como `null`, jamás como `net_worth`.
   */
  net_worth_liquid?: number;
  /**
   * f64 (5.0.0, §B.8): **retirada NETA del mes** en euros NOMINALES — los euros que de verdad
   * salieron de los activos para cubrir el déficit de caja. `0` en los meses de acumulación y en
   * el mes 0. No es el gasto (el ingreso de la fase lo cubre primero) ni la venta BRUTA (el
   * impuesto de la plusvalía se paga vendiendo de más y ese exceso sigue dentro del patrimonio).
   * Es un flujo del MES, no un acumulado, y se decima por densidad igual que `net_worth`.
   *
   * **5.0.0, pase de correcciones §D**: en modo `rule_is_spend` la regla financia su gasto
   * PRIMERO con el superávit del mes, y ese trozo cuenta aquí — antes se vendía cartera para
   * gastar un dinero que ya estaba en la cuenta y el churn fiscal aparecía de la nada.
   */
  withdrawal?: number;
  /**
   * f64 (5.0.0): recorte de la regla de retirada, `max(0, necesidad − permitido)`. **Informativo**
   * (D22/D24): no resta patrimonio, no cuenta como fracaso y NO es `uncovered_deficit_total`
   * (eso mide lo que los activos no pudieron vender). Todo ceros mientras la regla sea
   * `fixed_real`, que no tiene techo.
   */
  withdrawal_shortfall?: number;
  /** f64 (5.0.0): exceso de la regla sobre la necesidad en modo `rule_is_spend` (se vende y se
   *  gasta). Todo ceros con `fixed_real` / `ceiling`. */
  withdrawal_excess?: number;
  /**
   * f64 (5.0.0, pase de correcciones §F): **gasto del mes que los activos NO pudieron
   * financiar**, neto y `≥ 0`.
   *
   * Es la OTRA mitad de quedarse corto, y no se puede confundir con su vecina:
   * `withdrawal_shortfall` es lo que la REGLA rechazó sacar (hay dinero, el techo no deja);
   * `unmet_need` es lo que la CARTERA no dio (no había de dónde vender). Un mes puede tener las
   * dos, una, o ninguna. Sumar las dos NO da «lo que faltó»: son cortes distintos del mismo mes.
   *
   * Ausente en backends anteriores al pase — la fila del tooltip simplemente no se pinta, que es
   * lo correcto: un `0 €` afirmaría que se financió todo.
   */
  unmet_need?: number;
};

export type ProjectionMilestoneApi = {
  target: string;
  reached_month_index: number;
  reached_date_ymd: string;
};

export type AssetSeriesApi = {
  asset_id: string;
  asset_name: string;
  /** f64[] (paralelo a points). */
  values: number[];
};

/** Fase del motor por la que pasa la simulación (5.0.0, §B.1). Monótonas: la que no ocurre no
 *  aparece en `phase_transitions`. */
export type ProjectionPhaseApi = "accumulating" | "partial" | "retired";

/** Inicio de una fase, en la rejilla de `points[].month_index` (NUNCA una posición de array). */
export type PhaseTransitionApi = {
  phase: ProjectionPhaseApi;
  month_index: number;
};

/** Lecturas de UN miembro dentro del agregado del hogar (5.0.0, D9 / §D): de quién es cada
 *  marcador de la curva agregada y —desde WP5-2— su propia serie, que el chart dibuja como línea
 *  fina bajo la Σ en grueso (D32). */
export type HouseholdMemberProjectionApi = {
  user_id: string;
  username: string;
  strategy: RetirementStrategyApi;
  /** Mes EFECTIVO de jubilación de este miembro, en la rejilla común del hogar. `null` = no se
   *  jubila dentro del horizonte. */
  jubilacion_month_index: number | null;
  /** Años cumplidos de ESTE miembro en ese mes (con SU fecha de nacimiento). */
  jubilacion_age: number | null;
  // `liquid_crossing_month_index` NO se declara: el objetivo con el que cruzaba murió en el modelo
  // v2 y `HouseholdMemberProjection` (`apps/api/src/handlers/projection.rs`) dejó de publicarlo.
  /** El mes efectivo otra vez, con el nombre del motor (= `jubilacion_month_index`, R8). */
  retirement_month_index: number | null;
  // `coast_fire_month_index`, `underfunded`, `required_contribution_monthly` y
  // `disposable_monthly` TAMPOCO se declaran: el agregado del hogar no resuelve el plan de nadie
  // (D9), así que no hay mes coast, margen infra-financiado, aportación mínima ni margen por
  // persona que publicar, y el struct de Rust no los lleva. Los dos primeros sobrevivieron aquí
  // hasta W10 solo porque `memberPlanSentence` (`plan-sentence.ts`) los seguía leyendo sin que el
  // servidor los sirviera jamás — un campo tipado que ninguna respuesta real rellenaba.
  /** Mes de inicio de la media jornada. `null` si esa fase no ocurre. */
  partial_retirement_month_index: number | null;
  /** Mes de inicio de la pensión con fecha. `null` sin pensión declarada. */
  pension_start_month_index: number | null;
  /** Mes en que la cartera de ESTE miembro se vacía (el agregado publica el MÍNIMO). */
  assets_depleted_month_index: number | null;
  /** Avisos de este miembro (p. ej. `birth_date_missing`). */
  warnings: string[];
  /** **Horizonte PROPIO de este miembro en meses**, derivado de SU fecha de nacimiento y de SU
   *  `horizon_lifespan_age`. El agregado se corre al horizonte COMÚN `max(horizontes)`, así que
   *  este número puede ser MENOR que `months`: desde ahí, la curva de esta persona describiría
   *  años que ella no declaró vivir, y la línea fina TERMINA (nunca se extrapola). Ausente en
   *  backends anteriores a WP5-2 ⇒ la línea se dibuja entera. */
  horizon_months?: number;
  /** **Serie de ESTE miembro** (D32), paralela a `points[]`: los mismos `month_index`, la misma
   *  decimación y los mismos `number` (excepción chart-only D4). Es lo que dibuja la «línea fina
   *  por miembro» bajo la suma en grueso. Lleva `month_index` propio —y no dos arrays alineados
   *  por posición— porque se lee POR SEPARADO de `points`. Ausente en backends antiguos. */
  series?: MemberSeriesPointApi[];
  /** Los miembros del hogar NO resuelven una fecha estocástica propia (D9, modelo v2): siempre
   *  este valor fijo. Contrástalo con `SummaryPlanApi.plan_state`/
   *  `ProjectionSeriesApi.plan_absent_reason`, que sí varían.
   *
   *  Lo que este miembro conserva son sus lecturas DETERMINISTAS —el mes de jubilación por edad,
   *  el de media jornada, el de la pensión, el de agotamiento y su horizonte propio— y **ni una
   *  cifra del solve**: no hay éxito, ni fecha válida, ni capital necesario, ni aportación mínima,
   *  ni margen, ni mes coast. Este tipo se sincronizó del todo con `HouseholdMemberProjection`
   *  en W10 (issue #207): el doc anterior citaba varios campos que el servidor había dejado de
   *  publicar con el modelo v2 pero que este tipo, o `memberPlanSentence`, seguían leyendo.
   *  Ausente en backends anteriores al modelo v2. */
  plan_state?: "household_not_solved";
};

/** Un punto de la serie de un miembro del hogar (5.0.0, D32). Deliberadamente **dos importes y no
 *  siete**: el chart del hogar dibuja patrimonio (y compara líquido en las lecturas), y publicar
 *  el resto de columnas por persona multiplicaría el payload para responder algo que nadie
 *  pregunta. Números, no decimal-strings: es la excepción chart-only del contrato del dinero. */
export type MemberSeriesPointApi = {
  /** Mismo número de MES que `points[].month_index` (misma rejilla, misma decimación). NUNCA una
   *  posición de array: con `density=hybrid` la posición 13 es el mes 24. */
  month_index: number;
  /** Patrimonio neto de este miembro en euros NOMINALES de ese mes. */
  net_worth: number;
  /** Su patrimonio LÍQUIDO nominal — la línea que hay que comparar con SU objetivo, no con el del
   *  hogar (que no existe). **El chart NO la dibuja** (D32: una línea fina por miembro, no dos). */
  net_worth_liquid: number;
};

export type ProjectionSeriesApi = {
  points: ProjectionPointApi[];
  /** Hitos en euros nominales (toggle inflación apagado). */
  milestones: ProjectionMilestoneApi[];
  /** Mismos umbrales sobre el patrimonio deflactado a euros de hoy (toggle inflación encendido).
   *  Vacío cuando la inflación es 0 — en ese caso reusa `milestones`. */
  milestones_real?: ProjectionMilestoneApi[];
  compound_outpaces_true_savings_month_index?: number | null;
  /** Primer mes cuya venta bruta necesaria iguala o supera TODO lo drenable (todos los
   *  activos): la cartera se vacía ese mes y desde el siguiente el descubierto se acumula restando
   *  del patrimonio para siempre. Número de MES (misma base que `points[].month_index`), nunca una
   *  posición de array. `null` explícito = no se agota dentro del horizonte, no «no calculado». (#119)
   *
   *  **5.0.0, pase de correcciones §B**: lo decide la VENTA, y solo se publica si alguna venta
   *  posterior se quedó sin financiar. Un aterrizaje exacto —la cartera llega a cero justo
   *  cuando una pensión pasa a cubrir todo el gasto— sale `null`: no se agotó nada, se acabó de
   *  usar. Antes ese caso se publicaba como agotamiento y la app avisaba de una ruina que no
   *  ocurría. */
  assets_depleted_month_index: number | null;
  /** Déficit acumulado NO cubierto al final del horizonte, en euros. `"0.0000"` = cero euros
   *  descubiertos, no «no aplica». Ya se restaba de `net_worth`; aquí se declara. (#119) */
  uncovered_deficit_total: string;
  /** Pasivos cuya cuota no cubre el devengo: la deuda CRECE mes a mes (amortización negativa).
   *  Vacío = ninguno. Deliberadamente más estrecho que el `payment_does_not_reduce_principal` del
   *  calendario de amortización: un `interest_only` (principal congelado) NO aparece aquí — esa
   *  distinción es el valor del campo. (#119) */
  liabilities_negative_amortization: Array<{
    liability_id: string;
    label: string;
    opening_principal: string;
    final_principal: string;
    horizon_months: number;
  }>;
  months: number;
  horizon_years: number;
  horizon_basis: string;
  /** Edad límite configurada del horizonte (85..=105, default 90) — 4.9.0, #149. */
  horizon_lifespan_age?: number;
  /** Patrimonio del último mes en euros de HOY (paridad con simulate) — 4.9.0, #149. */
  final_net_worth_real?: string;
  /** Tasa anual (Decimal-string) con la que el servidor construyó `net_worth_real` y
   *  `milestones_real` — la fuente del deflactor del chart (#136-4a): re-obtenerla de la
   *  instalación era un canal de divergencia silenciosa. Ausente en backends < 4.6.0. */
  deflation_annual_inflation_percent?: string;
  starting_net_worth: string;
  monthly_delta_assumption: string;
  model_note: string;
  /** Mes 0 de la serie = esta fecha civil (servidor). */
  anchor_date_ymd?: string;
  show_age_mode?: string;
  /** Decisión del servidor: eje en años cumplidos (no inferir solo desde instalación en cliente). */
  use_age_on_x_axis?: boolean;
  viewer_birth_date?: string | null;
  /** Primer mes en que el patrimonio neto ≥ objetivo FIRE móvil del mes en curso. `null` si no alcanzado. */
  jubilacion_month_index?: number | null;
  /** Fecha civil del cruce (`YYYY-MM-DD`), ya resuelta en servidor. */
  jubilacion_date_ymd?: string | null;
  /** Años cumplidos en esa fecha; ausente sin fecha de nacimiento resuelta. */
  jubilacion_age?: number | null;
  /** Posición (índice de array, base 0) en `points` / `asset_series[].values`
   *  correspondiente al mes de jubilación. `null` ⟺ no hay cruce. Convención: el punto servido
   *  inmediatamente ANTERIOR o igual al mes del cruce — existe porque `jubilacion_month_index` no
   *  indexa nada (con `density=hybrid` los arrays llevan muchos menos puntos que meses). */
  jubilacion_series_position?: number | null;
  asset_series?: AssetSeriesApi[];
  /** Densidad de los puntos serializados. Default `monthly`. Con `hybrid` el cliente recibe ~82 puntos en lugar de ~841. */
  density?: "monthly" | "hybrid";
  /** Fuente EFECTIVA del ahorro usada por el engine para esta serie (tras el fallback del
   *  servidor a `budget` cuando el modo con transacciones no tenía meses con datos).
   *  Ausente en backends antiguos → `budget`. */
  savings_source?: SavingsSourceApi;
  /** Procedencia del lado INGRESO del promedio. Sustituye al escalar
   *  `savings_source_months_with_data`, que el backend dejó de enviar en 3.9.0 al hacerse las
   *  ventanas configurables por lado: con dos ventanas no existe *un* número de meses. */
  savings_income_basis?: SavingsAvgBasisApi;
  /** Procedencia del lado GASTO del promedio. */
  savings_expense_basis?: SavingsAvgBasisApi;

  // ── 5.0.0 — estrategia, fases y agregado del hogar (§B.8, §C, §D del plan de #207) ──────
  /** Estrategia con la que se simuló. **`null` en `view=household`**: el agregado suma N
   *  simulaciones y la de cada miembro viaja en `members[]`. Decide QUÉ significa
   *  `jubilacion_month_index` (un objetivo alcanzado o una edad impuesta). */
  strategy?: RetirementStrategyApi | null;
  /** Mes EFECTIVO de jubilación, en la rejilla de `points[].month_index`. **El mismo valor** que
   *  `jubilacion_month_index` (R8): viaja con los dos nombres porque `jubilacion_*` es el
   *  contrato publicado desde 1.x. `null` en `household`. */
  retirement_month_index?: number | null;
  /** Posición (índice de array) del mes de jubilación en `points`. Gemelo de
   *  `jubilacion_series_position`, misma convención (último punto servido cuyo `month_index` no
   *  pasa del mes de jubilación). */
  retirement_series_position?: number | null;
  /** Por qué los `jubilacion_*`/`retirement_*` están vacíos POR CONSTRUCCIÓN:
   *  `household_aggregate` | `no_retirement_trigger`. `null` ⟺ hay trigger, y entonces un índice
   *  nulo significa «no se alcanza dentro del horizonte», que es un resultado, no un hueco. */
  jubilacion_absent_reason?: string | null;
  /** Por qué falta el marcador «tu dinero trabaja más que tú» (`household_aggregate`). */
  compound_outpaces_true_savings_absent_reason?: string | null;
  /** Fases atravesadas y el mes de la rejilla en que empieza cada una. Siempre arranca con
   *  `accumulating` en el mes 0. **Vacío en `household`**. El orden ES el dato: las fases son
   *  monótonas y la que no ocurre no aparece. */
  phase_transitions?: PhaseTransitionApi[];
  /** Primer mes con pensión pública con fecha. `null` hasta WP3. */
  pension_start_month_index?: number | null;
  /** Primer mes de media jornada. `null` hasta WP3. */
  partial_retirement_month_index?: number | null;
  /** Avisos de esta simulación (literales cerrados): `birth_date_missing`,
   *  `target_retirement_age_missing` (una estrategia por edad degrada a `asap` sin fecha de
   *  nacimiento), `pension_unpaid_during_partial` (B9: la pensión con fecha cae dentro de la fase
   *  de media jornada y no se cobra hasta la jubilación total), `coast_not_reachable`,
   *  `partial_never_starts`, `partial_never_fully_retires` (M10/M11: los fallos de solve — no se
   *  llega ni parando/empezando de aportar, o se empieza la fase y nunca se jubila del todo),
   *  `no_volatility_declared` (C5: aviso, no bloqueo — ningún activo declara σ),
   *  `strategy_pension_bridge_migrated` (C7: un perfil guardado con el literal retirado
   *  `pension_bridge` migró solo a `asap` + puente activado). Vacío = nada que advertir. En
   *  `household` va vacío y los avisos viajan por miembro. */
  warnings?: string[];
  /** Un elemento por miembro del hogar, **solo en `view=household`** (D9). Vacío en `mine`.
   *  Cada fila trae sus marcadores, su horizonte propio y —desde WP5-2— **su serie**
   *  (`members[].series`), que es lo que el chart pinta como línea fina bajo la Σ. */
  members?: HouseholdMemberProjectionApi[];

  // ── modelo v2 — el bloque «plan»: «el éxito define la fecha» (C1-C8, sustituye a WP5-2b) ────
  //
  // TODO este bloque va a `null`/vacío en `view=household`: el hogar no resuelve UNA fecha
  // estocástica de N personas (`plan_absent_reason: household_aggregate`); lo que sí existe por
  // persona viaja en `members[]` (`HouseholdMemberProjectionApi.plan_state`).
  /** Qué fija la fecha del plan: `success_threshold` (el sorteo resolvió la fecha válida al
   *  umbral del perfil), `target_age` (la edad configurada manda sin sorteo — `coast`/`partial`
   *  en modo A) o `not_reachable` (ningún mes del horizonte cumple el umbral).
   *
   *  **Son TRES y no hay un cuarto**: las tres constantes de
   *  `apps/api/src/handlers/retirement_solver.rs` (`DATE_BASIS_SUCCESS_THRESHOLD`,
   *  `DATE_BASIS_TARGET_AGE`, `DATE_BASIS_NOT_REACHABLE`). El `pending` que esta unión declaró
   *  hasta A12 **nunca se emitió**: el nivel 1 del solve se resuelve EN LÍNEA, dentro del permiso
   *  de la serie, así que no existe una ventana en la que la base esté «calculándose». El único
   *  «calculando» real de esta respuesta es `needed_capital_curve_state: "computing"` — el nivel
   *  2, que sí llega en segundo plano en un GET posterior. */
  retirement_date_basis?: "success_threshold" | "target_age" | "not_reachable";
  /** Umbral EFECTIVO con el que se evaluó la fecha (eco del perfil, entero 80–100, default 95) —
   *  para no tener que ir a buscarlo a otra respuesta. */
  success_threshold_pct?: number;
  /** Mes VÁLIDO (número de MES, nunca una posición de array): el primero en que, jubilándose ahí,
   *  ≥ umbral de los caminos ya no vuelven a necesitar trabajar (definición A). `null` con
   *  `retirement_date_basis: "not_reachable"`. */
  safe_date_month_index?: number | null;
  /** Posición (índice de array) en `points[]` del mes de la fecha válida — misma convención que
   *  `jubilacion_series_position` (el punto servido inmediatamente anterior o igual). `null` sin
   *  fecha válida. */
  safe_date_series_position?: number | null;
  /** Fecha civil (`YYYY-MM-DD`) de la fecha válida, ya resuelta en servidor. `null` sin fecha
   *  válida. */
  safe_date_date_ymd?: string | null;
  /** Años cumplidos en la fecha válida. `null` sin fecha de nacimiento resuelta o sin fecha
   *  válida. */
  safe_date_age?: number | null;
  /** Mes de la fecha válida evaluada AL 100 % (cero fallos de N), publicado siempre al lado de la
   *  fecha al umbral configurado — «al 100 % serían N años más». `null` = no se alcanza ni al
   *  final del horizonte. */
  safe_date_at_100_month_index?: number | null;
  /** El mismo mes evaluado AL 90 %. Junto al de arriba acota la fecha configurada entre las dos. */
  safe_date_at_90_month_index?: number | null;
  /** FRACCIÓN [0,1]: éxito real del plan EN SU FECHA VÁLIDA (caminos sin fallo F1/F2/F3 de N).
   *  Es la cifra base, antes de aplicar Wilson. `null` sin fecha válida. */
  success_of_plan?: number | null;
  /** FRACCIÓN [0,1]: límite INFERIOR del intervalo de Wilson al 95 % sobre `success_of_plan`
   *  (C3) — la magnitud que de verdad decide el umbral, estable frente a la semilla y N. */
  success_wilson_low?: number | null;
  /** Semiancho del intervalo de Wilson, en PUNTOS PORCENTUALES (Decimal-string, `"1.2000"` =
   *  ±1,2 pp). Con 0 fallos de N **no es cero**: es la cota que publica la regla de tres. */
  success_sampling_error_pp?: string | null;
  /** Número de caminos con los que se CONFIRMÓ la fecha (2.500 por defecto). `null` sin sorteo
   *  (p. ej. `retirement_date_basis: "target_age"`). */
  paths_used?: number | null;
  /** STRING de dígitos (`u64`; `JSON.parse` lo redondearía por encima de 2^53), la semilla del
   *  sorteo que resolvió esta fecha — mismo motivo que `ProjectionBandsApi.seed`. `null` sin
   *  sorteo. */
  seed?: string | null;
  /** Euros de HOY (Decimal-string), redondeados a CIENTOS hacia arriba: el líquido que, invertido
   *  con tu mezcla de activos, sostiene el plan al umbral si te jubilaras YA. Misma cifra, al
   *  euro, en Jubilación, Resumen y Proyección. `null` con el motivo en el campo hermano
   *  `needed_capital_absent_reason` de la respuesta (`no_liquid_assets` | `threshold_unreachable`
   *  | `month_beyond_horizon`) — **nunca** en `plan_absent_reason`, que es otra lista y no incluye
   *  ninguno de los tres: ahí solo viajan las razones por las que no hay PLAN, y aquí falta una
   *  CIFRA de un plan que sí existe. Confundirlas fue lo que metió `no_liquid_assets` en la unión
   *  de `plan_absent_reason` y en cuatro tablas de copy que nunca podían dispararse. */
  needed_capital_today?: string | null;
  /** Por qué falta `needed_capital_today` cuando el resto del plan SÍ está resuelto —
   *  `no_liquid_assets` (el hogar no tiene líquido que escalar, o la escala ya lo deja a cero en
   *  el cierre anterior) | `threshold_unreachable` (ni multiplicando la cartera se cumple el
   *  umbral: el plan no falla por capital) | `month_beyond_horizon` (el mes pedido cae fuera del
   *  horizonte de la entrada) | `already_covered` (ni dividiendo la cartera por 256 se INCUMPLE el
   *  umbral: **no hace falta capital adicional hoy**). Mismos cuatro literales que
   *  `crates/engine-stochastic/src/needed_capital.rs` (`ABSENT_NO_LIQUID_ASSETS`,
   *  `ABSENT_THRESHOLD_UNREACHABLE`, `ABSENT_MONTH_BEYOND_HORIZON`, `ABSENT_ALREADY_COVERED`).
   *  `null` ⟺ hay cifra, o no hay plan (`plan_absent_reason`).
   *
   *  **Los tres primeros y el cuarto no dicen lo mismo**: los tres primeros son «este método no
   *  puede medirlo» y `already_covered` es una RESPUESTA — la necesidad existe pero cae por debajo
   *  de `λ_min × tu líquido`, así que no hay nada que reunir. Enseñarlo con el mismo guion mudo que
   *  los otros tres tira la única de las cuatro que es una buena noticia. */
  needed_capital_absent_reason?:
    | "no_liquid_assets"
    | "threshold_unreachable"
    | "month_beyond_horizon"
    | "already_covered"
    | null;
  /** f64[] (excepción chart-only) paralelo a `points[]`: capital necesario por edad en cada mes
   *  de la rejilla, en euros NOMINALES de cada mes (la SPA la deflacta con el mismo factor que el
   *  patrimonio).
   *
   *  **Qué mide cada nodo (C9, 2026-09-07): «lo que necesitas TENER (líquido) a esa edad para
   *  jubilarte entonces al umbral».** El solver fija la acumulación hasta el mes anterior en la
   *  trayectoria central del plan y solo sortea el tramo jubilado, así que todos los caminos
   *  llegan al nodo con esa misma cifra. **No** es el percentil de la cartera de hoy proyectada
   *  hasta esa edad, que es lo que la curva publicaba antes y lo que la hacía crecer sin techo con
   *  el horizonte. No tiene por qué cruzar la línea central del patrimonio en la fecha: la fecha la
   *  decide el éxito sobre miles de caminos, y va como marca vertical (C4).
   *
   *  **Un elemento `null` significa una de dos cosas, nunca «aquí hacen falta 0 €»**: (a) ese punto
   *  no cae en la rejilla gruesa que el nivel 2 evaluó, o (b) el nodo SÍ se evaluó y salió
   *  `already_covered` — no hace falta capital adicional ahí, lo normal a partir del mes en que la
   *  pensión cubre el gasto. El array entero es `null` mientras
   *  `needed_capital_curve_state !== "ready"`. */
  needed_capital_curve?: (number | null)[] | null;
  /** `ready` = curva completa; `computing` = el nivel 2 (segundo plano) sigue en marcha —
   *  reintentar más tarde; `unavailable` = no se pudo calcular. */
  needed_capital_curve_state?: "ready" | "computing" | "unavailable";
  /** €/mes (Decimal-string): aportación mínima que hace cumplir el umbral en la fecha objetivo
   *  (`retire_at_age`) o en el mes coast A. `null` con estrategias que no publican aportación. */
  contribution_required_monthly?: string | null;
  /** €/mes (Decimal-string): el techo de la búsqueda — el sobrante máximo mensual del horizonte.
   *  Denominador de la cifra de arriba («cuánto de mi margen se lleva el plan»). */
  contribution_required_search_ceiling?: string | null;
  /** `true` ⟺ ni con el techo entero se llega al umbral (`contribution_required_monthly ===
   *  contribution_required_search_ceiling`). **`null` = la pregunta no aplica** a esta estrategia,
   *  nunca `false` para decir «no aplica». */
  contribution_underfunded?: boolean | null;
  /** Número de MES: el primero en que se puede dejar de aportar y aun así llegar (coast A) o el
   *  fijado por el usuario (coast B, eco de `coast_stop_age`). `null` con cualquier estrategia que
   *  no sea `coast`, y también con `coast` cuando no se alcanza (+ `coast_not_reachable`). */
  coast_stop_month_index?: number | null;
  /** Número de MES de inicio de la media jornada: el de `partial_retirement.starts_at_age`
   *  (modo A) o el primero que el plan puede permitirse (modo B, `earliest_partial_start`). `null`
   *  sin fase parcial, o si nunca se puede empezar (+ `partial_never_starts`). */
  partial_start_month_index?: number | null;
  /** Éxito por AÑO de jubilación posible (nivel 2; alimenta la tira bajo el eje del chart): un
   *  punto por año de la rejilla con su `success` en [0,1]. `null` mientras el nivel 2 no está
   *  listo. */
  success_by_retirement_year?: { month_index: number; success: number }[] | null;
  /** Por qué el bloque «plan» entero (fecha, éxito, capital) NO se publica: `birth_date_missing`
   *  (C5: sin edad no hay contra qué resolver nada), `months_override` (la petición fijó
   *  `?months` y no simula sobre el horizonte propio del plan) o `household_aggregate` (agregado
   *  del hogar, D9: no hay UNA fecha de N personas). `null` ⟺ el bloque viaja entero.
   *
   *  **Son TRES y solo tres**, las tres constantes que alimentan el campo en
   *  `apps/api/src/handlers/projection.rs` (`PLAN_ABSENT_BIRTH_DATE_MISSING`,
   *  `PLAN_ABSENT_MONTHS_OVERRIDE`, `ABSENT_HOUSEHOLD_AGGREGATE`). El cuarto literal que esta
   *  unión declaró hasta 5.0.0 —`no_liquid_assets`— **no se emite aquí jamás**: vive en
   *  `needed_capital_absent_reason` (`crates/engine-stochastic/src/needed_capital.rs`), que dice
   *  por qué falta una CIFRA, no por qué falta el plan. Cuatro tablas de copy de la SPA lo
   *  traducían a una frase que ningún backend podía disparar.
   *
   *  **Y el del hogar es `household_aggregate`, NO `household_not_solved`** (A12). El agregado sale
   *  de `ABSENT_HOUSEHOLD_AGGREGATE` (`projection.rs`), y lo pinea
   *  `apps/api/tests/projection_household_aggregate.rs` comparando el campo con `"household_aggregate"`.
   *  `household_not_solved` existe, pero es el valor FIJO de otro campo
   *  (`HouseholdMemberProjectionApi.plan_state`): la unión declaraba ese, así que en
   *  `view=household` la SPA no reconocía el literal que le llegaba y caía a su copy genérico —
   *  «Tu plan no tiene fecha válida», que además dice otra cosa (el hogar no es que no llegue: es
   *  que no tiene UN plan). */
  plan_absent_reason?:
    | "birth_date_missing"
    | "months_override"
    | "household_aggregate"
    | null;
  /** Euros de HOY (Decimal-string): el número FIRE CLÁSICO (perpetuidad sobre el gasto de
   *  jubilación indexado ÷ SWR del perfil — 25× solo al 4 % —, SIN pensión ni objetivo puente — pin de `fire-parity.json`).
   *  Puramente informativo: NO decide la fecha ni el capital necesario. `null` ⟺
   *  `fire_number_classic_absent_reason`. */
  fire_number_classic_today?: string | null;
  /** Por qué falta el número clásico (`manual_amount_missing` | `net_need_not_positive` |
   *  `swr_not_positive` — este último también cubre `swr_pct = 0`). `null` ⟺ sí lo hay. */
  fire_number_classic_absent_reason?: string | null;
  /** Por qué la pensión declarada no participa en el plan: `birth_date_missing` (sin edad no se
   *  sabe si ya cobra en la fecha válida). `null` ⟺ no hay pensión declarada, o participa con
   *  normalidad. */
  pension_absent_reason?: "birth_date_missing" | null;
};

/**
 * Un punto del abanico de percentiles (5.0.0 WP6b, D28) — espejo de `ProjectionBandPoint`
 * (`apps/api/src/handlers/projection_bands.rs`).
 *
 * **`month_index` es la MISMA rejilla que `points[]` de `/v1/projection/series`** (las bandas
 * viajan siempre a densidad `hybrid`, sin `?density`), así que las dos se dibujan en el mismo eje
 * sin traducir nada — y sin indexar arrays: la posición 13 es el mes 24 en las dos.
 *
 * Los seis valores son `number` (f64 a 2 decimales), no Decimal-string: son valores de CHART, la
 * excepción declarada D4/I3, igual que `points[].net_worth`.
 */
export type ProjectionBandPointApi = {
  month_index: number;
  net_worth_p10: number;
  net_worth_p50: number;
  net_worth_p90: number;
  /** Bandas del LÍQUIDO. Por HTTP viajan siempre; ausentes ⇒ el backend no las mandó, **nunca
   *  cero**. La SPA no las dibuja hoy (el abanico es de patrimonio, como la línea determinista). */
  net_worth_liquid_p10?: number;
  net_worth_liquid_p50?: number;
  net_worth_liquid_p90?: number;
};

/**
 * Un punto del fallo ACUMULADO por edad (modelo v2, sustituye a `DepletionProbabilityPointApi`):
 * cuenta las TRES formas de fallo del camino (F1 cartera agotada, F2 tasa inicial superada, F3
 * recorte de regla bajo la necesidad ordinaria), no solo el agotamiento de cartera.
 * `age: null` ⟺ el usuario no tiene fecha de nacimiento (la fila sigue existiendo: la cifra es
 * real aunque no se pueda rotular con una edad).
 */
export type FailureProbabilityPointApi = {
  month_index: number;
  age: number | null;
  /** FRACCIÓN [0,1]: probabilidad ACUMULADA de haber fallado por CUALQUIER motivo hasta este mes.
   *  `null` = ese punto de la rejilla no se resolvió (nivel 2 aún en marcha). */
  probability: number | null;
  /** Desglose `[F1, F2, F3]` de esa misma fracción acumulada, mismo orden que
   *  `ProjectionBandsApi.failures_by_kind`. `null` a la vez que `probability`. */
  by_kind: [number, number, number] | null;
};

/** `green` | `amber` | `red`: verde ⟺ `success_wilson_low` cumple `success_threshold_pct`; ámbar
 *  ⟺ el umbral cae DENTRO del intervalo de Wilson (ni claramente sí ni claramente no); rojo el
 *  resto. Lo decide el SERVIDOR contra el umbral del perfil (C3) — el cliente no lo recalcula. */
export type SuccessVerdictApi = "green" | "amber" | "red";

/**
 * `GET /v1/projection/bands?paths&seed` (5.0.0 WP6b) — espejo de `ProjectionBandsResponse`.
 *
 * **Solo existe en `view=mine`**: los percentiles no suman entre miembros y el servidor devuelve
 * 400 `household_bands_unavailable` en Hogar (por eso `view` es siempre `"mine"`).
 */
export type ProjectionBandsApi = {
  /** Siempre `"mine"`. Se ecoa como en el resto de respuestas con scope. */
  view: string;
  months: number;
  horizon_basis: string;
  anchor_date_ymd: string;
  paths: number;
  /**
   * **STRING de dígitos, no número**: es un `u64` y `JSON.parse` lo redondea por encima de 2^53.
   * Reenviarlo como número devolvería OTRO sorteo sin que nada fallara.
   */
  seed: string;
  /** Fijo `[10, 50, 90]`, en el orden de los campos de `points[]`. */
  percentiles: number[];
  points: ProjectionBandPointApi[];
  /** FRACCIÓN [0,1]: éxito real del plan EN SU FECHA VÁLIDA — caminos sin fallo (F1/F2/F3) de N.
   *  Es la cifra base, antes de Wilson; sustituye a la vieja `success_probability` (redefinida en
   *  el modelo v2 — «el éxito define la fecha», C1-C8).
   *
   *  **`null` cuando el escenario sorteado no lleva mes de jubilación** (5.0.0, WP A12), y
   *  entonces `success_absent_reason` dice por qué. Hasta A12 el servidor publicaba ahí un `1`
   *  mecánico —el motor solo clasifica fallos estando jubilado, así que un escenario que nunca se
   *  jubila no puede fallar— y la SPA lo pintaba como «100 %, verde». */
  success_of_plan: number | null;
  /** Umbral EFECTIVO contra el que se evaluó `success_of_plan`/`success_wilson_low` (eco del
   *  perfil, entero 80–100, default 95). **Viaja siempre**, también con `success_absent_reason`:
   *  el umbral es del perfil y existe aunque no haya éxito que medir contra él. */
  success_threshold_pct: number;
  /** FRACCIÓN [0,1]: límite INFERIOR del intervalo de Wilson al 95 % sobre `success_of_plan`
   *  (C3) — la magnitud real que decide `success_verdict`, estable frente a semilla y N.
   *  `null` a la vez que `success_of_plan`. */
  success_wilson_low: number | null;
  /** Semiancho del intervalo de Wilson, en PUNTOS PORCENTUALES (Decimal-string, `"1.2000"` =
   *  ±1,2 pp). Con 0 fallos de N **no es cero**: es la cota que publica la regla de tres.
   *  `null` a la vez que `success_of_plan` — un `"0"` ahí afirmaría una medición sin error. */
  success_sampling_error_pp: string | null;
  /** Conteo de caminos fallidos por tipo, en orden fijo `[F1 cartera agotada, F2 tasa inicial
   *  superada, F3 recorte de regla bajo la necesidad ordinaria]`. La suma es ≤ `paths` — los
   *  caminos con éxito no cuentan en ninguna casilla.
   *
   *  **Con `success_absent_reason` puesto vale `[0, 0, 0]`**, y ese cero significa «sin
   *  jubilación no hay fallo que contar», nunca «riesgo cero». Léelo siempre con esa etiqueta
   *  delante (ver `success_absent_reason`). */
  failures_by_kind: [number, number, number];
  /** Veredicto del semáforo, decidido por el SERVIDOR contra `success_threshold_pct` (C3): verde
   *  ⟺ `success_wilson_low` cumple el umbral; ámbar ⟺ el umbral cae dentro del intervalo de
   *  Wilson; rojo el resto. La SPA nunca lo recalcula. `null` a la vez que `success_of_plan`: el
   *  semáforo de un plan que no ocurre es la AUSENCIA de semáforo, no un color. */
  success_verdict: SuccessVerdictApi | null;
  /**
   * **Por qué no hay éxito que publicar** (5.0.0, WP A12): las cuatro cifras de arriba
   * (`success_of_plan`, `success_wilson_low`, `success_sampling_error_pp`, `success_verdict`)
   * viajan a `null` juntas y este campo dice la causa. `null` ⟺ las cuatro viajan resueltas.
   *
   * **Solo DOS literales son alcanzables por esta ruta**, y no es una simplificación de la SPA:
   * `assemble_bands_response` propaga el `plan_absent_reason` del ensamblado, pero de sus tres
   * valores solo `birth_date_missing` llega aquí — `months_override` necesita un `?months=` que
   * este endpoint no acepta, y `household_not_solved` un `view=household` que aquí es 400
   * `household_bands_unavailable`. El otro literal es `not_reachable`: hay plan y ningún mes del
   * horizonte cumple el umbral (mismo literal que `retirement_date_basis` en la serie, para que
   * las dos superficies no nombren de dos maneras la misma situación).
   *
   * **Con este campo puesto, los ceros del resto de la respuesta cambian de significado.**
   * `failures_by_kind`, `failure_probability_by_age`, `months_below_need_p50` y
   * `withdrawal_to_need_ratio_p50` siguen viajando porque describen la trayectoria del patrimonio
   * **sin jubilarse**, que es una respuesta legítima; pero sus ceros dicen «sin jubilación no hay
   * fallo que contar», NUNCA «riesgo cero». Pintarlos como tranquilizadores es exactamente el
   * bug que A12 vino a cerrar, y por eso `buildRiskExtraRows` (`lib/risk-bands.ts`) no emite ni
   * una de esas filas cuando este campo no es `null`.
   *
   * **Viaja siempre, también como `null`**: una clave que desaparece obliga a distinguir «no
   * falta nada» de «este servidor no publica el motivo».
   */
  success_absent_reason: "not_reachable" | "birth_date_missing" | null;
  /** Fallo ACUMULADO por edad — sustituye a `depletion_probability_by_age`: cuenta las TRES
   *  formas de fallo (F1–F3), no solo el agotamiento de cartera. Vacío ⟺ ningún camino se jubila
   *  dentro del horizonte. Con `success_absent_reason` trae UNA sola fila, la del horizonte, y
   *  vale 0 — el mismo cero de `failures_by_kind`, con la misma lectura. */
  failure_probability_by_age: FailureProbabilityPointApi[];
  /** Mediana de MESES jubilados en que **se gastó menos de lo necesario**. **5.0.0, pase de
   *  correcciones §F**: cuenta las dos formas de quedarse corto —el techo de la regla y lo que
   *  la cartera no pudo pagar—, así que ya **no** es 0 por construcción con `fixed_real`: esa
   *  regla no tiene techo, pero su cartera sí se puede quedar sin nada. */
  months_below_need_p50: number;
  /** FRACCIÓN (Decimal-string): qué parte de la necesidad se pagó DE VERDAD. `1` = entera.
   *  `null` cuando ningún camino tiene meses jubilados con necesidad positiva. Misma corrección
   *  §F: incluye el descubierto, no solo el recorte de la regla (un caso medido pasó de `1,0` a
   *  `0,0865` al dejar de ignorar lo que la cartera no financió). */
  withdrawal_to_need_ratio_p50: string | null;
  /** `false` ⟺ **ningún activo declara volatilidad**: las tres bandas SON la línea determinista,
   *  y la UI tiene que decirlo en vez de dibujar un abanico plano que se lee como certeza. */
  any_volatility_declared: boolean;
  strategy: RetirementStrategyApi;
  // `retirement_trigger` NO se declara aquí: el backend v2 dejó de publicarlo con el objetivo que
  // nombraba (lo pinea `assert_the_v1_fields_are_gone`, `apps/api/tests/projection_bands.rs`), así
  // que un campo obligatorio en este tipo era una promesa que ninguna respuesta cumplía.
  computed_in_ms: number;
  model_note: string;
};

export type FfbackupImportCounts = {
  assets: number;
  liabilities: number;
  budget_entries: number;
  planning_flows: number;
  categories_in_backup: number;
  categories_already_present: number;
  categories_to_create: number;
};

export type FfbackupImportPreviewResponse = {
  schema_version: number;
  app_version: string;
  exported_at: string;
  username_original: string;
  counts: FfbackupImportCounts;
  birth_date_will_change: boolean;
  ui_preferences_present: boolean;
};

export type FfbackupImportApplyResponse = {
  imported: FfbackupImportCounts;
  ui_preferences: {
    person_scope?: string | null;
    projection_focus?: string | null;
  };
};

/**
 * Modelo de amortización del pasivo (4.2.0). Literales exactos del wire — espejo de
 * `RepaymentModel` en `apps/api/src/handlers/liabilities.rs` (serde `snake_case`).
 */
export type LiabilityRepaymentModelApi =
  | "fixed_payments"
  | "french"
  | "interest_only"
  | "revolving";

export type LiabilityApiRow = {
  id: string;
  category_id: string;
  /** Categoría de GASTO de la cuota; ausente solo en pasivos pre-3.4.0 sin asignar. */
  expense_category_id?: string;
  label: string;
  type_tag: string | null;
  principal_derived_from_plan?: boolean;
  /**
   * Siempre presente en el wire (columna NOT NULL con default `fixed_payments`, y el backend no
   * la omite nunca) — por eso NO es opcional aquí: un pasivo sin modelo no existe.
   */
  repayment_model: LiabilityRepaymentModelApi;
  principal: string;
  apr_percent: string | null;
  payment_amount: string | null;
  payment_frequency: "monthly" | "weekly" | null;
  payment_end_date: string | null;
  /**
   * «Plan vencido con saldo» (4.7.0, #145): `payment_end_date < hoy` con `principal > 0`. La
   * deuda no se extinguió por calendario — sigue visible, congelada y marcada.
   */
  plan_expired_with_balance: boolean;
  /** Cuota mínima revolving: % del saldo de apertura. `null` en los demás modelos (4.7.0). */
  min_payment_pct: string | null;
  /** Suelo en euros de la cuota mínima revolving. `null` en los demás modelos (4.7.0). */
  min_payment_eur: string | null;
  notes: string | null;
  sort_index: number;
};

// ---------------------------------------------------------------------------
// Perspectiva histórica (snapshots de patrimonio) — v1.5.0
// ---------------------------------------------------------------------------

/** Tipo de snapshot. Singular, igual que el CHECK de DB (`kind IN ('asset','liability')`). */
export type HistorySnapshotKindApi = "asset" | "liability";

/**
 * Un punto de la serie histórica. Todo numérico = f64 (misma excepción chart-only que
 * `ProjectionPointApi`), no Decimal-string. `month_index ≤ 0`, contiguo e **incluye el 0**.
 */
export type HistoryPointApi = {
  month_index: number;
  /** `null` en TODA la serie cuando `liabilities_snapshotted` es `false`: sin el pasivo
   *  fotografiado entero, `assets_total − liabilities_total` no es un patrimonio neto sino el
   *  total de activos con otro nombre. El campo siempre viaja (como `null`), nunca se omite. */
  net_worth: number | null;
  assets_total: number;
  liabilities_total: number;
};

/** Serie histórica por activo (paralela a `points`). `asset_id` = `source_item_id`. f64[]. */
export type HistoryAssetSeriesApi = {
  asset_id: string;
  asset_name: string;
  values: number[];
};

/** Marcador de snapshot en el eje temporal (posición x y exponente de deflación). */
export type HistoryMarkerApi = {
  date_ymd: string;
  month_index: number;
  /** `month_index + (día−1)/días_del_mes`, redondeado a 4 decimales por el servidor. */
  month_fraction: number;
  kind: HistorySnapshotKindApi;
  /** `capture` = foto que tomó la app; `backfill` = valores tecleados a posteriori para una fecha
   *  pasada. Un backfill puede estar en cualquier fecha, incluso muy remota. */
  source: "capture" | "backfill";
  owner_user_id: string;
  total: number;
};

/**
 * Respuesta de `GET /v1/history/series`. 0 snapshots → arrays vacíos. Espejo exacto de §2 del
 * plan. `anchor_date_ymd` debe coincidir con el de la proyección para poder fusionar.
 */
export type HistorySeriesApi = {
  anchor_date_ymd: string;
  anchor_month_first_ymd: string;
  view: string;
  /** Ventana emitida, en meses hacia atrás desde el mes 0. Omitir `window_months` en la petición
   *  ya NO devuelve todo el histórico (default 120 desde 4.4.0): el chart pide 1200 a propósito. */
  window_months: number;
  /** `true` ⇒ hay snapshots anteriores a la ventana y la serie está recortada. */
  window_truncated: boolean;
  /** Snapshot más antiguo del scope, esté o no dentro de la ventana. */
  first_snapshot_date_ymd?: string;
  first_snapshot_month_index?: number;
  points: HistoryPointApi[];
  asset_series: HistoryAssetSeriesApi[];
  markers: HistoryMarkerApi[];
  /** `false` ⇒ el pasivo del scope NO está fotografiado entero (ningún snapshot de pasivo, o
   *  —en hogar— algún miembro sin ninguno), así que `points[].liabilities_total` está a 0 o
   *  incompleto por falta de datos, no porque no haya deuda. Es el interruptor de
   *  `points[].net_worth`: `net_worth === null` ⟺ este flag es `false`. El chart lo consume vía
   *  `mergeProjectionWithHistory`, que en ese caso pinta `assets_total` y lo dice en la leyenda. */
  liabilities_snapshotted: boolean;
};

/** Ítem de un snapshot (CRUD Decimal-as-string). Los términos solo aplican a pasivos. */
export type HistorySnapshotItemApi = {
  item_id: string;
  label: string;
  value: string;
  apr_percent?: string;
  payment_amount?: string;
  payment_frequency?: "monthly" | "weekly";
  /** Modelo del pasivo al capturar la foto (4.7.0, #129); ausente en fotos anteriores. */
  repayment_model?: LiabilityRepaymentModelApi;
};

/** Cabecera + ítems de un snapshot. Total derivado (Σ ítems), nunca almacenado. */
export type HistorySnapshotApi = {
  id: string;
  kind: HistorySnapshotKindApi;
  snapshot_date_ymd: string;
  source: "capture" | "backfill";
  total: string;
  items: HistorySnapshotItemApi[];
  created_at: string;
  updated_at: string;
};

/**
 * Ítem sugerido por `GET /v1/history/snapshots/prefill` para autocompletar el grid del modal.
 * `item_id` preserva el enlace entre snapshots (se reenvía en POST/PUT). `existed: false` ⇒
 * `value: "0"` (el ítem no existía en esa fecha según los datos del usuario). `basis` documenta
 * de dónde sale el valor: `interpolated` (entre dos snapshots), `first_snapshot` (anterior al
 * primero), `live` (valor actual), `not_owned` (ítem de otro usuario). Términos solo en pasivos.
 */
export type HistoryPrefillItemApi = {
  item_id: string;
  label: string;
  value: string;
  existed: boolean;
  basis: "interpolated" | "first_snapshot" | "live" | "not_owned";
  apr_percent?: string;
  payment_amount?: string;
  payment_frequency?: "monthly" | "weekly";
};

/** Respuesta de `GET /v1/history/snapshots/prefill`. Ítems ordenados: existentes primero
 *  (label ASC), luego `not_owned`. */
export type HistoryPrefillResponseApi = {
  date_ymd: string;
  kind: HistorySnapshotKindApi;
  items: HistoryPrefillItemApi[];
};

// ---------------------------------------------------------------------------
// Histórico de gasto mensual (transacciones) — v1.6.0
// Espejo EXACTO de apps/api/src/handlers/transactions/schema.rs (autoridad).
// Importes = string decimal firmado; los Option con skip_serializing_if se
// OMITEN cuando null → aquí opcionales (`?`). Prefijo de rutas `/v1/transactions/*`.
// ---------------------------------------------------------------------------

/** `expense` | `income` | `savings`. `savings` exige `category_id` null. */
export type TransactionKindApi = "expense" | "income" | "savings";

/** Origen de la petición de import/preview. `auto` = autodetección por cabecera. */
export type TransactionImportSourceApi = "auto" | "myinvestor" | "n26";

/** Un movimiento. `import_id` ausente = manual (efectivo). Amount firmado (neg = cargo). */
export type TransactionApi = {
  id: string;
  import_id?: string;
  /** `myinvestor` | `n26` | `manual` | … */
  source: string;
  op_date: string;
  value_date?: string;
  concept: string;
  amount: string;
  currency: string;
  kind?: TransactionKindApi;
  category_id?: string;
  category_name?: string;
  linked_asset_id?: string;
  linked_liability_id?: string;
  notes?: string;
  /** Presente si el movimiento fue materializado por una regla recurrente. */
  recurring_rule_id?: string;
  /** Contrapartida de la conciliación de transferencia. Presente ⇒ el movimiento está CONCILIADO
   *  (pata de un traspaso interno): sigue en el listado pero el servidor ya lo excluye de todos
   *  los totales/promedios. Los cuatro campos siguientes solo llegan con contrapartida viva. */
  transfer_counterpart_id?: string;
  /** Instante de la conciliación (ISO). */
  transfer_reconciled_at?: string;
  /** `auto` (pase del matcher) | `manual` (par conciliado a mano). */
  transfer_reconciled_source?: "auto" | "manual";
  /** Concepto de la contrapartida (denormalizado: puede estar en otro mes). */
  transfer_counterpart_concept?: string;
  /** `YYYY-MM-DD` de la contrapartida (denormalizado). */
  transfer_counterpart_op_date?: string;
  created_at: string;
  updated_at: string;
};

/** Un mes con datos (para el selector). `is_complete=false` = mes civil en curso. DESC. */
export type TransactionMonthApi = {
  month: string;
  is_complete: boolean;
  txn_count: number;
};

/** Un batch de import (deshacer import = borrar batch, CASCADE). */
export type TransactionImportBatchApi = {
  id: string;
  source: string;
  account_asset_id?: string;
  account_asset_name?: string;
  original_filename?: string;
  created_at: string;
  txn_count: number;
};

/** Recurrencia opcional al crear un movimiento: enviar `{}` marca «repetir cada mes». Las reglas
 *  tienen resolución mensual (sin día): la instancia del mes M se fecha en su último día y solo se
 *  materializa con M ya cerrado. */
export type TransactionRecurrenceApi = Record<string, never>;

/** Cuerpo de `POST /v1/transactions` (alta manual, 201). También el item de `/batch`. */
export type CreateTransactionRequest = {
  op_date: string;
  value_date?: string;
  concept: string;
  amount: string;
  kind: TransactionKindApi;
  category_id?: string;
  linked_asset_id?: string;
  linked_liability_id?: string;
  notes?: string;
  recurrence?: TransactionRecurrenceApi;
};

/** Cuerpo de `POST /v1/transactions/batch` (1..1000). */
export type BatchCreateTransactionsRequest = {
  transactions: CreateTransactionRequest[];
};

/** Cuerpo de `PATCH /v1/transactions/{id}`. En importadas, op_date/amount/concept → 400. */
export type PatchTransactionRequest = {
  op_date?: string;
  value_date?: string;
  clear_value_date?: boolean;
  concept?: string;
  amount?: string;
  kind?: TransactionKindApi;
  category_id?: string;
  clear_category?: boolean;
  linked_asset_id?: string;
  clear_linked_asset?: boolean;
  linked_liability_id?: string;
  clear_linked_liability?: boolean;
  notes?: string;
  clear_notes?: boolean;
};

/**
 * Una asignación provisional de la sesión del wizard: el concepto de una fila que el usuario ya
 * clasificó y lo que le asignó. El servidor la convierte en una regla EFÍMERA (mismo motor de
 * patrones y misma precedencia que las reglas persistidas) y recalcula las sugerencias del
 * preview; nada se persiste. Solo generan regla las que llevan `category_id` o `kind: "savings"`
 * — el mismo gate que el aprendizaje del confirm. `category_id` viaja explícitamente a `null`
 * cuando no aplica.
 */
export type ImportPendingAssignmentApi = {
  concept: string;
  kind: TransactionKindApi;
  category_id: string | null;
};

/** Cuerpo de `POST /v1/transactions/import/preview`. */
export type ImportPreviewRequest = {
  source: TransactionImportSourceApi;
  file_b64: string;
  account_asset_id?: string;
  /** Máximo 200 entradas (el backend responde 400 `pending_assignments_too_many` por encima). */
  pending_assignments?: ImportPendingAssignmentApi[];
};

/** Una fila del preview del import. `status` new/already_imported. */
export type ImportPreviewRowApi = {
  index: number;
  op_date: string;
  value_date?: string;
  concept: string;
  amount: string;
  currency: string;
  status: "new" | "already_imported";
  suggested_kind: TransactionKindApi;
  suggested_category_id?: string;
  suggested_category_name?: string;
  suggested_transfer: boolean;
  currency_warning: boolean;
  matched_rule_id?: string;
  /** De dónde sale `suggested_category_id` (4.15.0): `"rule"` = la casó una regla de
   *  categorización; `"fallback"` = no la casó ninguna y el servidor puso la categoría por
   *  defecto del ámbito. Ausente cuando no hay categoría sugerida (p. ej. `kind = savings`).
   *  La distinción importa: una categoría por defecto NO debe propagarse como si fuera una
   *  clasificación decidida (ni al automatch del preview, ni al aprendizaje de reglas). */
  suggested_category_source?: "rule" | "fallback";
};

/** Respuesta de `POST /v1/transactions/import/preview`. */
export type ImportPreviewResponseApi = {
  /** Preset detectado: `myinvestor` | `n26`. */
  source: string;
  file_sha256: string;
  row_count: number;
  new_count: number;
  already_imported_count: number;
  suggested_transfer_count: number;
  precategorized_count: number;
  currency_warning_count: number;
  rows: ImportPreviewRowApi[];
};

/** Una decisión de import, PARALELA POR ÍNDICE a las filas del preview (una por fila). */
export type ImportDecisionApi = {
  discard?: boolean;
  force?: boolean;
  kind: TransactionKindApi;
  category_id?: string;
  linked_asset_id?: string;
  linked_liability_id?: string;
};

/** Cuerpo de `POST /v1/transactions/import/confirm`. */
export type ImportConfirmRequest = {
  source: TransactionImportSourceApi;
  file_b64: string;
  file_sha256: string;
  decisions: ImportDecisionApi[];
  learn_rules?: boolean;
  account_asset_id?: string;
  original_filename?: string;
};

/** Respuesta de `POST /v1/transactions/import/confirm`. */
export type ImportConfirmResponseApi = {
  import_id?: string;
  imported: number;
  skipped_already_imported: number;
  discarded: number;
  rules_learned: number;
  /** Pares auto-conciliados tras el import (cruzando TODO el dataset, no solo este lote). */
  reconciled_pairs: number;
};

/** Respuesta de `POST /v1/transactions/reconcile` (pase de auto-conciliación). Idempotente. */
export type ReconcileRunResponseApi = {
  pairs_created: number;
  /** `pairs_created × 2` (las dos patas de cada par). */
  transactions_reconciled: number;
};

/** Las dos patas tras conciliar (`POST`) o desconciliar (`DELETE`) `/v1/transactions/{id}/reconcile`. */
export type ReconcilePairResponseApi = {
  transaction: TransactionApi;
  counterpart: TransactionApi;
};

/** Regla de categorización aprendida/manual. */
export type TransactionRuleApi = {
  id: string;
  match_kind: "substring" | "prefix" | "exact";
  pattern: string;
  source?: string;
  assign_kind?: TransactionKindApi;
  assign_category_id?: string;
  assign_category_name?: string;
  created_at: string;
  updated_at: string;
};

/** Una línea de comparativa por categoría (magnitudes POSITIVAS todas). */
export type CategoryComparisonLineApi = {
  category_id?: string;
  category_name: string;
  actual: string;
  budget: string;
  /** `null` ⟺ `avg_months == 0` (sin meses reales que promediar en la ventana). */
  avg: string | null;
  /** `actual − budget`. `null` ⟺ `has_actual_data == false` (el mes no tiene ningún movimiento). */
  delta_vs_budget: string | null;
  /** `actual − avg`. `null` ⟺ `avg_months == 0` o `has_actual_data == false`. */
  delta_vs_avg: string | null;
};

/** Bloque {actual, avg} para savings e income. */
export type SummaryBlockActualAvgApi = {
  actual: string;
  /** `null` ⟺ `avg_months == 0` (misma regla que `CategoryComparisonLineApi.avg`). */
  avg: string | null;
};

/** Totales de la comparativa. */
export type SummaryTotalsApi = {
  expense_actual: string;
  expense_budget: string;
  /** `null` ⟺ `avg_months == 0`. Sigue la misma regla que las filas: si una fila no tiene media,
   *  el total tampoco puede tenerla (sería la suma de nada). */
  expense_avg: string | null;
  income_actual: string;
  income_budget: string;
  /** `null` ⟺ `avg_months == 0`. */
  income_avg: string | null;
  savings_actual: string;
  /** `null` ⟺ `avg_months == 0`. */
  savings_avg: string | null;
  net_actual: string;
  /** Ahorro promedio = `income_avg − expense_avg`, sobre los MISMOS meses reales y la misma
   *  ventana que sus dos sumandos. `null` ⟺ `avg_months == 0`. Puede ser negativo (gastaste más
   *  de lo que ingresaste). NO es el «Ahorro mensual» del Resumen, que sigue el modo de ahorro
   *  configurado. */
  net_avg: string | null;
  /** Devoluciones del mes seleccionado: suma de los importes POSITIVOS de los movimientos de
   *  gasto (reembolsos, abonos, copagos). Magnitud ≥ 0, NUNCA null — un mes sin devoluciones
   *  emite un cero de verdad. Ya está descontada dentro de `expense_actual` y de la categoría de
   *  cada fila: es una cifra derivada para explicarlo, no un sumando aparte. */
  refunds_actual: string;
  /** La misma magnitud, promediada sobre los meses reales de la ventana. `null` ⟺
   *  `avg_months == 0`. */
  refunds_avg: string | null;
};

/** De qué meses sale el promedio de la comparativa mensual. */
export type AvgBasisApi = {
  /** Meses reales incluidos (≥ 1). */
  months: number;
  /** Mes más antiguo incluido (`YYYY-MM`). */
  first_month: string;
  /** Mes más reciente incluido (`YYYY-MM`). */
  last_month: string;
  /** `true` ⟺ los meses incluidos no son consecutivos. */
  has_gaps: boolean;
};

/** Respuesta de `GET /v1/transactions/summary`. */
export type TransactionsSummaryApi = {
  year: number;
  /** 1-12. */
  month: number;
  /** `true` si el mes seleccionado es el mes civil en curso. NO significa «faltan datos»: significa
   *  «el mes no ha terminado». Para eso está `has_actual_data`. */
  is_partial: boolean;
  /** Movimientos del mes seleccionado que entran en la comparativa (cualquier kind, recurrentes
   *  materializados incluidos, transferencias conciliadas excluidas). */
  actual_txn_count: number;
  /** `actual_txn_count > 0`. Un mes sin importar y un mes de gasto cero producían la misma
   *  respuesta (`actual: "0.0000"` en todas las filas); con `false` todos los `delta_vs_*` van a
   *  `null` en vez de comparar contra un mes vacío. */
  has_actual_data: boolean;
  /** Ventana del promedio solicitada: `3` | `6` | `12` | `ytd` | `all`. */
  avg_window: string;
  /** Nº de meses que abarca la ventana del promedio. */
  window_months: number;
  /**
   * Nº de meses de la ventana con movimientos de cualquier tipo, recurrentes incluidos.
   * Describe lo que hay. NO es el denominador del promedio — ese es `avg_months`.
   */
  months_with_data: number;
  /**
   * Denominador del promedio: meses de la ventana con al menos un movimiento REAL (no recurrente).
   * Si 0, no hay promedio.
   */
  avg_months: number;
  /** De qué meses sale el promedio. Ausente ⟺ `avg_months === 0`. */
  avg_basis?: AvgBasisApi;
  /** Por qué no hay promedio, cuando no lo hay. Ausente cuando sí lo hay. */
  avg_unavailable_reason?: "empty_window" | "only_recurring_months";
  /** `household` | `mine`. */
  view: string;
  expense_categories: CategoryComparisonLineApi[];
  income_categories: CategoryComparisonLineApi[];
  savings: SummaryBlockActualAvgApi;
  income: SummaryBlockActualAvgApi;
  totals: SummaryTotalsApi;
};

// ---------------------------------------------------------------------------
// Serie mensual por categoría (`GET /v1/transactions/category-series`, v4.3.1 — hoy solo
// consumida por la tool MCP `get_category_monthly_series`; sin caller en la SPA todavía).
// ---------------------------------------------------------------------------

/** Un mes de la serie de una categoría. Magnitudes ≥ 0 (misma convención de signos que la
 *  comparativa: gasto = `−Σ(amount)`, ingreso = `+Σ(amount)`); un reembolso puede dejarla negativa. */
export type CategoryMonthPointApi = {
  /** `YYYY-MM`. */
  month: string;
  total: string;
  /** `true` ⟺ ese mes tiene al menos un movimiento (cualquier kind, conciliadas aparte) en el
   *  scope pedido. Distingue las dos lecturas de un `total` a cero que la serie cero-rellenada
   *  volvía indistinguibles: «ese mes no gastaste en esta categoría» (`true`) frente a «de ese mes
   *  no hay datos» (`false`). */
  has_data: boolean;
};

/** Serie de una categoría: un punto por cada mes de la ventana (cero-relleno). */
export type CategoryMonthlySeriesEntryApi = {
  /** `null` = movimientos sin categoría. */
  category_id?: string;
  category_name?: string;
  months: CategoryMonthPointApi[];
};

/** Respuesta de `GET /v1/transactions/category-series`. El último mes de la ventana es el mes
 *  civil en curso (parcial). */
export type CategoryMonthlySeriesResponseApi = {
  /** `household` | `mine`. */
  view: string;
  /** `expense` | `income`. */
  kind: string;
  /** Amplitud efectiva de la ventana (1..=60). */
  window_months: number;
  /** Primer mes (`YYYY-MM`) con algún movimiento en el scope, de toda la historia, no solo de la
   *  ventana. Ausente ⟺ el usuario no tiene ni un movimiento. */
  first_month_with_data?: string;
  /** Solo categorías con ≥1 movimiento del `kind` en la ventana. */
  series: CategoryMonthlySeriesEntryApi[];
};

/** Una regla recurrente (`GET /v1/transactions/recurring`). Amount = string decimal firmado. */
export type RecurringRuleApi = {
  id: string;
  concept: string;
  amount: string;
  kind: TransactionKindApi;
  category_id?: string;
  category_name?: string;
  linked_asset_id?: string;
  linked_liability_id?: string;
  notes?: string;
  /** YYYY-MM-DD del último mes materializado. */
  origin_month: string;
  created_at: string;
  updated_at: string;
};

/** Respuesta de `POST /v1/transactions/recurring/materialize`. */
export type RecurringMaterializeResponse = {
  rules_processed: number;
  materialized: number;
};

/**
 * Un mes del cash-flow histórico (`GET /v1/history/cashflow`). `month_index` 0 = mes actual,
 * negativos = pasado. `expense`/`savings` son NEGATIVOS (signo real), `income` positivo.
 * Todo Decimal-string.
 */
export type CashflowMonthApi = {
  month_index: number;
  date_ymd: string;
  /** ≤ 0 */
  expense: string;
  /** ≥ 0 */
  income: string;
  /** ≤ 0 */
  savings: string;
  /** `expense + income + savings`: variación de caja. **Incluye** los traspasos a ahorro, así que
   *  un mes con una aportación grande sale negativo sin ser una pérdida. */
  cash_delta: string;
  /** `income + expense`: ingresos menos gastos, **sin** ahorro. Misma cifra que
   *  `totals.net_actual` de la comparativa mensual. Es la que responde a «¿fue buen mes?». */
  income_minus_expense: string;
};

/** Punto del grid fino del cash-flow histórico. `month_fraction` es el eje X real
 *  (mes fraccional, negativo en el pasado); cada punto lleva su fecha — nunca se
 *  indexa por posición. */
export type CashflowFineGridPointApi = {
  date_ymd: string;
  month_index: number;
  month_fraction: number;
};

export type CashflowFineAssetSeriesApi = {
  asset_id: string;
  asset_name: string;
  /** f64 (excepción de arrays de series, como la proyección). Paralelo a `grid`. */
  values: number[];
};

/** Serie fina anclada a snapshots (semanal/diaria). Solo presente cuando hay
 *  transacciones vinculadas a algún asset y snapshots que anclen. */
export type CashflowFineApi = {
  resolution: "weekly" | "daily";
  grid: CashflowFineGridPointApi[];
  asset_series: CashflowFineAssetSeriesApi[];
  /** Paralelo a `grid`: Σ assets moldeados − Σ pasivos amortizados. f64.
   *  `null` cuando `liabilities_snapshotted` es false: sin el pasivo fotografiado entero esto
   *  serían los activos disfrazados de neto (mismo invariante que `HistorySeriesPointApi`). */
  net_worth: number[] | null;
};

/**
 * Respuesta de `GET /v1/history/cashflow`. El frontend de C1-C6/C8 usa SOLO `months[]`;
 * el campo `fine` (serie fina) lo consume el overlay del chart grande.
 */
export type HistoryCashflowApi = {
  anchor_date_ymd: string;
  anchor_month_first_ymd: string;
  view: string;
  months: CashflowMonthApi[];
  /** `true` solo si TODOS los usuarios del scope tienen algún snapshot de pasivo. Con `false`
   *  no hay patrimonio neto histórico y `fine.net_worth` llega `null`. */
  liabilities_snapshotted: boolean;
  fine?: CashflowFineApi;
  /** Por qué falta `fine`: `not_requested` | `window_too_large_for_curve` |
   *  `no_asset_linked_transactions` | `no_snapshots_to_anchor`. `null` ⟺ `fine` viaja. */
  fine_absent_reason?: string | null;
};

/**
 * Token de API (`GET /v1/api-tokens`) — credencial Bearer del servidor MCP (`/mcp`).
 * Nunca incluye el secreto: `token_prefix` son los primeros caracteres (`ffp_XXXXXXXX`)
 * para identificarlo. Timestamps ISO; `expires_at`/`last_used_at`/`revoked_at` ausentes
 * cuando no aplican.
 */
export type ApiTokenApi = {
  id: string;
  label: string;
  token_prefix: string;
  /** `"read_write"` (histórico) | `"read_only"` — con `read_only` ninguna tool de escritura de
   *  `/mcp` acepta el token, aunque autentica y lee igual. Siempre presente. */
  scope: string;
  created_at: string;
  expires_at?: string | null;
  last_used_at?: string | null;
  revoked_at?: string | null;
};

/** Respuesta del `POST /v1/api-tokens`: el campo `token` (secreto) SOLO viaja aquí, una vez. */
export type CreateApiTokenResponseApi = ApiTokenApi & { token: string };

/**
 * `GET /v1/oauth/authorize-details` — validación del authorization request para la
 * pantalla de consentimiento. `consent` = pintar; `invalid_request` = error FATAL
 * (jamás redirigir); `redirect_error` = navegar a `redirect_to` (lleva `?error=…`).
 * `client_name` es texto declarado por la app (NO verificado); `redirect_host` sí.
 */
export type OAuthAuthorizeDetailsApi = {
  status: "consent" | "invalid_request" | "redirect_error";
  client_name?: string;
  client_uri?: string | null;
  redirect_host?: string;
  resource?: string;
  already_connected?: boolean;
  connected_at?: string | null;
  error_code?: string;
  redirect_to?: string;
};

/** `POST /v1/oauth/authorize` — a dónde navegar (código o `error=access_denied`). */
export type OAuthAuthorizeDecisionApi = { redirect_to: string };

/** `GET /v1/oauth/connections` — apps conectadas por OAuth del propio usuario. */
export type OAuthConnectionApi = {
  id: string;
  client_name: string;
  client_uri?: string | null;
  redirect_host?: string | null;
  created_at: string;
  last_used_at?: string | null;
};
