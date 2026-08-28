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

export type FireSettingsApi = {
  fire_number_mode: FireNumberModeApi;
  fire_number_manual_amount: string | null;
  fire_number_expense_adjustment_pct: string | null;
  swr_pct: string;
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
};

/** Cómo se cuentan los meses de una ventana del promedio real.
 *  `calendar` = los meses con datos dentro de los últimos N civiles.
 *  `data` = los N meses CON DATOS más recientes, saltando los vacíos. */
export type AvgWindowModeApi = "data" | "calendar";

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
  upcoming_inflows_total: string;
  upcoming_outflows_total: string;
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
   *  de todos los activos menos `principal × TAE` de los pasivos vivos, sobre el patrimonio neto.
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

export type SummaryResponse = {
  total_assets: string;
  total_liabilities: string;
  net_worth: string;
  debt_to_assets_ratio: string | null;
  financial_health: FinancialHealthMetrics;
  assets_by_category: CategoryBreakdownLineApi[];
  liabilities_by_category: CategoryBreakdownLineApi[];
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

export type PlanningFlowApiRow = {
  id: string;
  category_id: string;
  direction: PlanningFlowDirectionApi;
  title: string;
  expected_amount: string;
  due_date: string | null;
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

export type ProjectionSeriesApi = {
  points: ProjectionPointApi[];
  /** Hitos en euros nominales (toggle inflación apagado). */
  milestones: ProjectionMilestoneApi[];
  /** Mismos umbrales sobre el patrimonio deflactado a euros de hoy (toggle inflación encendido).
   *  Vacío cuando la inflación es 0 — en ese caso reusa `milestones`. */
  milestones_real?: ProjectionMilestoneApi[];
  compound_outpaces_true_savings_month_index?: number | null;
  months: number;
  horizon_years: number;
  horizon_basis: string;
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
  /** Objetivo FIRE base en euros de hoy. El target real de cada mes crece con la inflación. */
  jubilacion_target_net_worth?: string | null;
  /** Serie del target FIRE ajustado por inflación, paralela a `points`. f64[] (vacío cuando no hay FIRE). */
  fire_target_series?: number[];
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
  /** `month_index + (día−1)/días_del_mes`. */
  month_fraction: number;
  kind: HistorySnapshotKindApi;
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

/** Cuerpo de `POST /v1/transactions/import/preview`. */
export type ImportPreviewRequest = {
  source: TransactionImportSourceApi;
  file_b64: string;
  account_asset_id?: string;
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
