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
};

export type InstallationAccess = {
  installation: InstallationSnapshot;
  role: "owner" | "member" | "viewer";
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
  /** Aporte estimado mes 1 derivado de las reglas de asignación del sobrante. */
  contribution_nominal_monthly?: string;
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

export type FinancialHealthMetrics = {
  income_monthly_equivalent: string;
  expense_regular_monthly_equivalent: string;
  expense_derived_monthly_equivalent: string;
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
  savings_rate: string | null;
  monthly_net_excluding_derived_debt: string;
  savings_rate_excluding_derived_debt: string | null;
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
  /** Nº de meses con datos usados por el promedio (0 en modo `budget`). */
  savings_source_months_with_data?: number;
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
  expense_regular_monthly_equivalent: string;
  /** Suma de entradas con `ends_at_retirement = false` — gastos que persisten tras jubilación.
   *  Es el campo correcto para el cálculo FIRE (alinea con el servidor). */
  expense_retirement_monthly_equivalent: string;
  expense_derived_monthly_equivalent: string;
  expense_total_monthly_equivalent: string;
  net_monthly_equivalent: string;
};

export type BudgetEntryApiRow = {
  id: string;
  category_id: string;
  scope: "income" | "expense";
  /** Importe mensual */
  amount: string;
  notes: string | null;
  sort_index: number;
  persists_after_retirement: boolean;
  ends_at_retirement: boolean;
  expense_end_date: string | null;
};

export type DerivedBudgetLineApi = {
  liability_id: string;
  category_id: string;
  label: string;
  amount: string;
  frequency: "monthly" | "weekly";
  monthly_equivalent: string;
  notes: string;
};

export type BudgetSnapshotApi = {
  entries: BudgetEntryApiRow[];
  derived_from_liabilities: DerivedBudgetLineApi[];
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
  /** Nº de meses con datos usados por el promedio (0 en modo `budget`). */
  savings_source_months_with_data?: number;
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

export type LiabilityApiRow = {
  id: string;
  category_id: string;
  label: string;
  type_tag: string | null;
  principal_derived_from_plan?: boolean;
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
  net_worth: number;
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

/** Recurrencia opcional al crear un movimiento: enviar `{}` marca «repetir cada mes» (el día por
 *  defecto es el de `op_date`); `day_of_month` la fuerza a otro día del mes. */
export type TransactionRecurrenceApi = {
  day_of_month?: number;
};

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
  avg: string;
  delta_vs_budget: string;
  delta_vs_avg: string;
};

/** Bloque {actual, avg} para savings e income. */
export type SummaryBlockActualAvgApi = {
  actual: string;
  avg: string;
};

/** Totales de la comparativa. */
export type SummaryTotalsApi = {
  expense_actual: string;
  expense_budget: string;
  expense_avg: string;
  income_actual: string;
  income_budget: string;
  income_avg: string;
  savings_actual: string;
  savings_avg: string;
  net_actual: string;
};

/** Respuesta de `GET /v1/transactions/summary`. */
export type TransactionsSummaryApi = {
  year: number;
  /** 1-12. */
  month: number;
  is_partial: boolean;
  /** Ventana del promedio solicitada: `3` | `6` | `12` | `ytd` | `all`. */
  avg_window: string;
  /** Nº de meses que abarca la ventana del promedio. */
  window_months: number;
  /** Nº de meses (dentro de la ventana) que tienen datos → si 0, no hay promedio. */
  months_with_data: number;
  /** `household` | `mine`. */
  view: string;
  expense_categories: CategoryComparisonLineApi[];
  income_categories: CategoryComparisonLineApi[];
  savings: SummaryBlockActualAvgApi;
  income: SummaryBlockActualAvgApi;
  totals: SummaryTotalsApi;
};

/** Una regla recurrente (`GET /v1/transactions/recurring`). Amount = string decimal firmado. */
export type RecurringRuleApi = {
  id: string;
  concept: string;
  amount: string;
  kind: TransactionKindApi;
  category_id?: string;
  category_name?: string;
  /** 1-31: día del mes en que se materializa. */
  day_of_month: number;
  linked_asset_id?: string;
  linked_liability_id?: string;
  notes?: string;
  /** YYYY-MM-DD del último mes materializado. */
  last_materialized_month: string;
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
  expense: string;
  income: string;
  savings: string;
  net: string;
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
  /** Paralelo a `grid`: Σ assets moldeados − Σ pasivos amortizados. f64. */
  net_worth: number[];
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
  fine?: CashflowFineApi;
};
