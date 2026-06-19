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
  runway_months: string | null;
  upcoming_inflows_total: string;
  upcoming_outflows_total: string;
  upcoming_coverage_ratio: string | null;
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
