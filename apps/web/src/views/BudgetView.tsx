import type { Dispatch, FormEvent, SetStateAction } from "react";
import type {
  AllocationRuleApiRow,
  AllocationRuleCapKind,
  AllocationRuleKind,
  AssetApiRow,
  BudgetEntryApiRow,
  BudgetSnapshotApi,
  CategoryRow,
  InstallationAccess,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import { GearIcon, PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import { PlanningDirectionChart } from "../components/charts/PlanningDirectionChart";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyOrDash,
  parseDisplayDecimal,
} from "../lib/format";
import {
  budgetCategoryMap,
  sortBudgetEntriesMacStyle,
} from "../lib/ledger";
import { useIsMobile } from "../lib/responsive";
import { AllocationRulesPanel } from "./AllocationRulesPanel";

export type BudgetScopeToggle = "income" | "expense";

const BUDGET_SCOPE_LABEL: Record<BudgetScopeToggle, string> = {
  income: "Ingreso",
  expense: "Gasto",
};

/** Etiqueta de la categoría de una partida. Solo una cuota de pasivo anterior a la 3.4.0 puede
 *  venir sin categoría: se marca en vez de esconderse, porque sí cuenta en el total. */
function budgetEntryCatLabel(
  categoryById: Map<string, CategoryRow>,
  row: BudgetEntryApiRow,
): string {
  if (!row.category_id) return "Sin categoría";
  return categoryById.get(row.category_id)?.name ?? row.category_id.slice(0, 8);
}

export function BudgetView({
  installation,
  installationBusy,
  hasMembership,
  canEdit,
  formError,
  budgetModalOpen,
  closeBudgetModal,
  openNewBudgetModal,
  budgetSnapshot,
  budgetLoading,
  budgetIncomeCategories,
  budgetExpenseCategories,
  budgetFormScope,
  setBudgetFormScope,
  budgetFormCategoryId,
  setBudgetFormCategoryId,
  budgetFormAmount,
  setBudgetFormAmount,
  budgetFormNotes,
  setBudgetFormNotes,
  budgetFormPersistsAfterRetirement,
  setBudgetFormPersistsAfterRetirement,
  budgetFormExpenseEndType,
  setBudgetFormExpenseEndType,
  budgetFormExpenseEndDate,
  setBudgetFormExpenseEndDate,
  editingBudgetEntryId,
  budgetSaving,
  submitBudgetForm,
  deleteBudgetEntryRow,
  beginEditBudgetEntry,
  assets,
  allocationRules,
  allocationRulesBusy,
  allocationRulesError,
  allocationPanelOpen,
  openAllocationPanel,
  closeAllocationPanel,
  ruleModalOpen,
  openNewRuleModal,
  closeRuleModal,
  ruleFormTargetAsset,
  setRuleFormTargetAsset,
  ruleFormKind,
  setRuleFormKind,
  ruleFormAmount,
  setRuleFormAmount,
  ruleFormCapKind,
  setRuleFormCapKind,
  ruleFormCapValue,
  setRuleFormCapValue,
  editingRuleId,
  ruleSaving,
  submitRuleForm,
  deleteRule,
  moveRule,
  beginEditRule,
  onOpenCategorySettings,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  canEdit: boolean;
  formError: string | null;
  budgetModalOpen: boolean;
  closeBudgetModal: () => void;
  openNewBudgetModal: (scope?: BudgetScopeToggle) => void;
  budgetSnapshot: BudgetSnapshotApi | null;
  budgetLoading: boolean;
  budgetIncomeCategories: CategoryRow[];
  budgetExpenseCategories: CategoryRow[];
  budgetFormScope: BudgetScopeToggle;
  setBudgetFormScope: Dispatch<SetStateAction<BudgetScopeToggle>>;
  budgetFormCategoryId: string;
  setBudgetFormCategoryId: Dispatch<SetStateAction<string>>;
  budgetFormAmount: string;
  setBudgetFormAmount: Dispatch<SetStateAction<string>>;
  budgetFormNotes: string;
  setBudgetFormNotes: Dispatch<SetStateAction<string>>;
  budgetFormPersistsAfterRetirement: boolean;
  setBudgetFormPersistsAfterRetirement: Dispatch<SetStateAction<boolean>>;
  budgetFormExpenseEndType: "never" | "retirement" | "date";
  setBudgetFormExpenseEndType: Dispatch<SetStateAction<"never" | "retirement" | "date">>;
  budgetFormExpenseEndDate: string;
  setBudgetFormExpenseEndDate: Dispatch<SetStateAction<string>>;
  editingBudgetEntryId: string | null;
  budgetSaving: boolean;
  submitBudgetForm: (e: FormEvent) => void;
  deleteBudgetEntryRow: (id: string) => void;
  beginEditBudgetEntry: (row: BudgetEntryApiRow) => void;
  assets: AssetApiRow[];
  allocationRules: AllocationRuleApiRow[];
  allocationRulesBusy: boolean;
  allocationRulesError: string | null;
  allocationPanelOpen: boolean;
  openAllocationPanel: () => void;
  closeAllocationPanel: () => void;
  ruleModalOpen: boolean;
  openNewRuleModal: () => void;
  closeRuleModal: () => void;
  ruleFormTargetAsset: string;
  setRuleFormTargetAsset: Dispatch<SetStateAction<string>>;
  ruleFormKind: AllocationRuleKind;
  setRuleFormKind: Dispatch<SetStateAction<AllocationRuleKind>>;
  ruleFormAmount: string;
  setRuleFormAmount: Dispatch<SetStateAction<string>>;
  ruleFormCapKind: "none" | AllocationRuleCapKind;
  setRuleFormCapKind: Dispatch<SetStateAction<"none" | AllocationRuleCapKind>>;
  ruleFormCapValue: string;
  setRuleFormCapValue: Dispatch<SetStateAction<string>>;
  editingRuleId: string | null;
  ruleSaving: boolean;
  submitRuleForm: (e: FormEvent) => void;
  deleteRule: (id: string) => void;
  moveRule: (id: string, dir: "up" | "down") => void;
  beginEditRule: (r: AllocationRuleApiRow) => void;
  /**
   * Lleva a `Ajustes → Categorías`. Única salida cuando el hogar se ha quedado sin categorías
   * de ingreso ni de gasto y por tanto no puede haber presupuesto.
   */
  onOpenCategorySettings?: () => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const isMobile = useIsMobile();

  const ruleIndex = editingRuleId
    ? allocationRules.findIndex((r) => r.id === editingRuleId)
    : -1;

  const categoryMapForSort = budgetCategoryMap(
    budgetIncomeCategories,
    budgetExpenseCategories,
  );

  const budgetEntriesRaw = Array.isArray(budgetSnapshot?.entries)
    ? budgetSnapshot.entries
    : [];

  const sortedEntries =
    budgetSnapshot && !budgetLoading
      ? sortBudgetEntriesMacStyle(budgetEntriesRaw, categoryMapForSort)
      : [];

  const formCats =
    budgetFormScope === "income"
      ? budgetIncomeCategories
      : budgetExpenseCategories;

  const snap =
    hasMembership && !budgetLoading && budgetSnapshot !== null
      ? budgetSnapshot
      : null;

  const incTot = snap
    ? formatCurrencyOrDash(snap.totals?.income_monthly_equivalent, currencyIso)
    : METRIC_DASH;
  const expTot = snap
    ? formatCurrencyOrDash(
        snap.totals?.expense_total_monthly_equivalent,
        currencyIso,
      )
    : METRIC_DASH;
  const netM = snap
    ? formatCurrencyOrDash(snap.totals?.net_monthly_equivalent, currencyIso)
    : METRIC_DASH;

  const incomeNum = snap
    ? parseDisplayDecimal(snap.totals?.income_monthly_equivalent ?? "") ?? 0
    : 0;
  const expenseNum = snap
    ? parseDisplayDecimal(
        snap.totals?.expense_total_monthly_equivalent ?? "",
      ) ?? 0
    : 0;

  // Política de ceros: la unidad es el BLOQUE. Con líneas de presupuesto se pintan las tres
  // KPIs aunque alguna valga 0 €; sin ninguna, la banda, la distribución y las dos columnas
  // dejan paso a un único estado vacío (tres ceros y dos tablas vacías no explican nada).
  const budgetIsEmpty =
    hasMembership && !budgetLoading && sortedEntries.length === 0;
  const noBudgetCategories =
    budgetIncomeCategories.length === 0 && budgetExpenseCategories.length === 0;

  const incomeEntries = sortedEntries.filter((e) => e.scope === "income");
  // Incluye las cuotas de pasivo (`source === "liability"`), que el servidor sirve como una
  // partida de gasto más desde la 3.7.0 y el orden coloca detrás de la manual de su categoría.
  const expenseEntries = sortedEntries.filter((e) => e.scope === "expense");
  const hasQuotaEntries = expenseEntries.some((e) => e.source === "liability");

  return (
    <div className="workspace budget-page">
      <div className="workspace-header">
        <h2 className="workspace-title">Presupuesto</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : budgetLoading
                ? "Cargando…"
                : "Mensual"}
        </p>
      </div>

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {/* `formError` gatea el estado vacío (y solo el estado vacío: `budgetIsEmpty` sigue
          decidiendo el resto del layout): cuando el loader falla vacía entradas Y categorías, y
          la vista acusaba al usuario de no tener ninguna («no queda ninguna») cuando lo que había
          pasado es que no se pudo leer nada. El error ya se pinta en la banda global de App.tsx. */}
      {budgetIsEmpty && !formError ? (
        noBudgetCategories ? (
          <EmptyState
            title="Faltan categorías"
            description="El presupuesto ordena lo que entra y lo que sale por categoría. No queda ninguna, así que crea las que uses y vuelve aquí."
            actionLabel={canEdit ? "Crear categorías" : undefined}
            onAction={canEdit ? onOpenCategorySettings : undefined}
          />
        ) : (
          <EmptyState
            title="Sin presupuesto"
            description="Aquí planificas tu mes: cuánto entra y cuánto sale de forma recurrente. La diferencia es el ahorro con el que FutureFin proyecta tu patrimonio."
            actionLabel={canEdit ? "Añadir ingreso" : undefined}
            onAction={canEdit ? () => openNewBudgetModal("income") : undefined}
            secondaryLabel={canEdit ? "Añadir gasto" : undefined}
            onSecondary={canEdit ? () => openNewBudgetModal("expense") : undefined}
          />
        )
      ) : null}

      {hasMembership && !budgetIsEmpty ? (
        <div
          className="metric-grid workspace-kpi-strip metric-grid--budget-summary"
          aria-label="Resumen del presupuesto"
        >
          <MetricCard label="Ingresos totales" value={incTot} />
          <MetricCard label="Gastos totales" value={expTot} />
          <MetricCard
            label="Neto"
            value={netM}
            action={
              <button
                type="button"
                className="btn ghost icon-btn metric-card__action-btn"
                onClick={openAllocationPanel}
                aria-label="Asignación del sobrante"
                title={`Asignación del sobrante · ${allocationRules.length} ${allocationRules.length === 1 ? "regla" : "reglas"}`}
              >
                <GearIcon />
              </button>
            }
          />
        </div>
      ) : null}

      {hasMembership && !budgetIsEmpty ? (
        <section className="panel">
          <h3 className="panel-title">Distribución</h3>
          {budgetLoading ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : incomeNum + expenseNum > 0 ? (
            <PlanningDirectionChart
              inflow={incomeNum}
              outflow={expenseNum}
            />
          ) : (
            <p className="muted bordered-top">Sin proporción.</p>
          )}
        </section>
      ) : null}

      {hasMembership ? (
        <Modal
          title="Asignación del sobrante"
          open={allocationPanelOpen}
          onClose={closeAllocationPanel}
          wide
        >
          <AllocationRulesPanel
            assets={assets}
            rules={allocationRules}
            busy={allocationRulesBusy}
            error={allocationRulesError}
            canEdit={canEdit}
            currencyIso={currencyIso}
            openNewRuleModal={openNewRuleModal}
            beginEditRule={beginEditRule}
            deleteRule={deleteRule}
            moveRule={moveRule}
            embedded
          />
        </Modal>
      ) : null}

      {canEdit && hasMembership ? (
        <Modal
          title={editingRuleId ? "Editar regla" : "Nueva regla de asignación"}
          open={ruleModalOpen}
          onClose={closeRuleModal}
        >
          <form className="asset-form stack" onSubmit={submitRuleForm}>
            <ModalFormError message={allocationRulesError} />
            <label className="field">
              <span>Destino (activo)</span>
              <select
                value={ruleFormTargetAsset}
                onChange={(e) => setRuleFormTargetAsset(e.target.value)}
                required
                disabled={assets.length === 0}
              >
                {assets.length === 0 ? (
                  <option value="">— Sin activos —</option>
                ) : null}
                {assets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="asset-form-grid">
              <label className="field">
                <span>Tipo</span>
                <select
                  value={ruleFormKind}
                  onChange={(e) =>
                    setRuleFormKind(e.target.value as AllocationRuleKind)
                  }
                >
                  <option value="fixed">Cantidad fija €/mes</option>
                  <option value="percent">% del sobrante restante</option>
                  <option value="remainder">Resto (lo que quede)</option>
                </select>
              </label>
              {ruleFormKind !== "remainder" ? (
                <label className="field">
                  <span>
                    {ruleFormKind === "fixed" ? "Importe €/mes" : "Porcentaje"}
                  </span>
                  <input
                    value={ruleFormAmount}
                    onChange={(e) => setRuleFormAmount(e.target.value)}
                    inputMode="decimal"
                    placeholder="0"
                    required
                    autoComplete="off"
                  />
                </label>
              ) : null}
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Tope opcional</span>
                <select
                  value={ruleFormCapKind}
                  onChange={(e) =>
                    setRuleFormCapKind(
                      e.target.value as "none" | AllocationRuleCapKind,
                    )
                  }
                >
                  <option value="none">Sin tope</option>
                  <option value="amount">Cantidad fija €</option>
                  <option value="months_expense">N × gasto mensual</option>
                  <option value="income_multiple">N × ingreso mensual</option>
                </select>
              </label>
              {ruleFormCapKind !== "none" ? (
                <label className="field">
                  <span>
                    {ruleFormCapKind === "amount"
                      ? "Tope en €"
                      : ruleFormCapKind === "months_expense"
                        ? "N meses de gasto"
                        : "N múltiplo de ingreso"}
                  </span>
                  <input
                    value={ruleFormCapValue}
                    onChange={(e) => setRuleFormCapValue(e.target.value)}
                    inputMode="decimal"
                    placeholder="0"
                    required
                    autoComplete="off"
                  />
                </label>
              ) : null}
            </div>
            <p className="muted tight">
              {ruleFormKind === "remainder" && ruleFormCapKind === "none"
                ? "Esta regla absorberá todo lo que quede del sobrante. Se coloca automáticamente al final del orden; solo puede haber una por usuario."
                : ruleFormKind === "remainder"
                  ? "Esta regla absorbe lo que quede hasta su tope. Se inserta antes del resto sin tope."
                  : ruleFormKind === "fixed"
                    ? "Se aporta esta cantidad fija mensual antes de seguir la cascada. Se inserta antes del resto sin tope."
                    : "Se aporta este % sobre lo que quede del sobrante en este paso de la cascada (no del sobrante total)."}
            </p>
            <div className="asset-form-actions">
              <button type="submit" className="btn primary" disabled={ruleSaving}>
                {editingRuleId ? "Guardar cambios" : "Añadir regla"}
              </button>
              <button
                type="button"
                className="btn ghost"
                onClick={closeRuleModal}
                disabled={ruleSaving}
              >
                Cancelar
              </button>
            </div>
            {isMobile && editingRuleId ? (
              <>
                <div className="modal-mobile-reorder">
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={ruleSaving || ruleIndex <= 0}
                    onClick={() => moveRule(editingRuleId, "up")}
                  >
                    ▲ Subir
                  </button>
                  <button
                    type="button"
                    className="btn ghost"
                    disabled={
                      ruleSaving ||
                      ruleIndex < 0 ||
                      ruleIndex >= allocationRules.length - 1
                    }
                    onClick={() => moveRule(editingRuleId, "down")}
                  >
                    ▼ Bajar
                  </button>
                </div>
                <div className="modal-mobile-delete-row">
                  <button
                    type="button"
                    className="btn ghost danger"
                    disabled={ruleSaving}
                    onClick={() => {
                      deleteRule(editingRuleId);
                      closeRuleModal();
                    }}
                  >
                    Eliminar regla
                  </button>
                </div>
              </>
            ) : null}
          </form>
        </Modal>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">Solo lectura.</p>
      ) : null}

      {canEdit &&
      hasMembership &&
      (budgetIncomeCategories.length > 0 ||
        budgetExpenseCategories.length > 0) ? (
        <Modal
          title={
            editingBudgetEntryId
              ? "Editar línea de presupuesto"
              : "Nueva línea de presupuesto"
          }
          open={budgetModalOpen}
          onClose={closeBudgetModal}
        >
          <form className="asset-form stack" onSubmit={submitBudgetForm}>
            <ModalFormError message={formError} />
            <div className="segmented" role="tablist" aria-label="Ámbito">
              <button
                type="button"
                role="tab"
                aria-selected={budgetFormScope === "income"}
                className={budgetFormScope === "income" ? "active" : ""}
                onClick={() => setBudgetFormScope("income")}
              >
                Ingreso
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={budgetFormScope === "expense"}
                className={budgetFormScope === "expense" ? "active" : ""}
                onClick={() => setBudgetFormScope("expense")}
              >
                Gasto
              </button>
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría ({BUDGET_SCOPE_LABEL[budgetFormScope]})</span>
                <select
                  value={budgetFormCategoryId}
                  onChange={(e) => setBudgetFormCategoryId(e.target.value)}
                  required
                  disabled={formCats.length === 0}
                >
                  {formCats.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Importe mensual (neto)</span>
                <input
                  value={budgetFormAmount}
                  onChange={(e) => setBudgetFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opcional)</span>
              <textarea
                value={budgetFormNotes}
                onChange={(e) => setBudgetFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            {budgetFormScope === "income" ? (
              <label className="field field--checkbox">
                <input
                  type="checkbox"
                  checked={budgetFormPersistsAfterRetirement}
                  onChange={(e) =>
                    setBudgetFormPersistsAfterRetirement(e.target.checked)
                  }
                />
                <span>Persiste tras jubilación</span>
              </label>
            ) : (
              <>
                <label className="field">
                  <span>Fin del gasto</span>
                  <select
                    value={budgetFormExpenseEndType}
                    onChange={(e) =>
                      setBudgetFormExpenseEndType(
                        e.target.value as "never" | "retirement" | "date",
                      )
                    }
                  >
                    <option value="never">Sin fecha de fin</option>
                    <option value="retirement">Al jubilarse</option>
                    <option value="date">Hasta la fecha…</option>
                  </select>
                </label>
                {budgetFormExpenseEndType === "date" && (
                  <label className="field">
                    <span>Fecha de fin</span>
                    <input
                      type="date"
                      value={budgetFormExpenseEndDate}
                      onChange={(e) => setBudgetFormExpenseEndDate(e.target.value)}
                      required
                    />
                  </label>
                )}
              </>
            )}
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={budgetSaving || formCats.length === 0}
              >
                {editingBudgetEntryId ? "Guardar cambios" : "Añadir línea"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={budgetSaving}
                onClick={() => closeBudgetModal()}
              >
                Cancelar
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {budgetLoading ? (
        <section className="panel">
          <h3 className="panel-title">Detalle</h3>
          <p className="muted bordered-top">Cargando líneas de presupuesto…</p>
        </section>
      ) : budgetIsEmpty ? null : (
        <div className="budget-two-col">
          <section className="panel budget-col">
            <div className="panel-head-row">
              <h3 className="panel-title">Ingresos</h3>
              {canEdit && hasMembership ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nueva línea de ingreso"
                  title={
                    budgetIncomeCategories.length === 0
                      ? "Necesitas una categoría de ingreso"
                      : "Nueva línea de ingreso"
                  }
                  disabled={budgetIncomeCategories.length === 0}
                  onClick={() => openNewBudgetModal("income")}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
            {incomeEntries.length === 0 ? (
              <EmptyState
                embedded
                title="Sin ingresos"
                description="Apunta tu nómina y cualquier otra entrada que se repita cada mes."
                actionLabel={
                  canEdit && budgetIncomeCategories.length > 0
                    ? "Añadir ingreso"
                    : undefined
                }
                onAction={
                  canEdit && budgetIncomeCategories.length > 0
                    ? () => openNewBudgetModal("income")
                    : undefined
                }
              />
            ) : (
              <div className="table-scroll table-scroll--budget-lines bordered-top">
                <table className="assets-table assets-table--budget-lines">
                  <thead>
                    <tr>
                      <th>Categoría</th>
                      <th className="num">Importe mensual (neto)</th>
                      {!isMobile && canEdit ? (
                        <th className="asset-actions-cell">
                          <span className="sr-only">Acciones</span>
                        </th>
                      ) : null}
                    </tr>
                  </thead>
                  <tbody>
                    {incomeEntries.map((row) => {
                      const rowTappable = isMobile && canEdit;
                      return (
                        <tr
                          key={row.id}
                          className={rowTappable ? "row-tappable" : undefined}
                          role={rowTappable ? "button" : undefined}
                          tabIndex={rowTappable ? 0 : undefined}
                          onClick={
                            rowTappable ? () => beginEditBudgetEntry(row) : undefined
                          }
                          onKeyDown={
                            rowTappable
                              ? (e) => {
                                  if (e.key === "Enter" || e.key === " ") {
                                    e.preventDefault();
                                    beginEditBudgetEntry(row);
                                  }
                                }
                              : undefined
                          }
                        >
                          <td>
                            {budgetEntryCatLabel(categoryMapForSort, row)}
                          </td>
                          <td className="num">
                            {formatCurrencyAmount(row.amount, currencyIso)}
                            {rowTappable ? (
                              <span className="row-chevron" aria-hidden>
                                ›
                              </span>
                            ) : null}
                          </td>
                          {!isMobile && canEdit ? (
                            <td className="asset-actions-cell">
                              <div className="budget-row-actions">
                                <button
                                  type="button"
                                  className="btn ghost icon-btn"
                                  aria-label="Editar línea"
                                  disabled={budgetSaving}
                                  onClick={() => beginEditBudgetEntry(row)}
                                >
                                  <RowEditIcon />
                                </button>
                                <button
                                  type="button"
                                  className="btn ghost danger icon-btn"
                                  aria-label="Eliminar línea"
                                  disabled={budgetSaving}
                                  onClick={() => deleteBudgetEntryRow(row.id)}
                                >
                                  <RowTrashIcon />
                                </button>
                              </div>
                            </td>
                          ) : null}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <div className="budget-expenses-column">
            <section className="panel budget-col">
              <div className="panel-head-row">
                <h3 className="panel-title">Gastos</h3>
                {canEdit && hasMembership ? (
                  <button
                    type="button"
                    className="btn primary icon-btn ledger-toolbar-add"
                    aria-label="Nueva línea de gasto"
                    title={
                      budgetExpenseCategories.length === 0
                        ? "Necesitas una categoría de gasto"
                        : "Nueva línea de gasto"
                    }
                    disabled={budgetExpenseCategories.length === 0}
                    onClick={() => openNewBudgetModal("expense")}
                  >
                    <PlusIcon />
                  </button>
                ) : null}
              </div>
              {expenseEntries.length === 0 ? (
                <EmptyState
                  embedded
                  title="Sin gastos"
                  description="Apunta los gastos fijos del mes: alquiler, suministros, seguros. Las cuotas de tus pasivos aparecen aquí solas."
                  actionLabel={
                    canEdit && budgetExpenseCategories.length > 0
                      ? "Añadir gasto"
                      : undefined
                  }
                  onAction={
                    canEdit && budgetExpenseCategories.length > 0
                      ? () => openNewBudgetModal("expense")
                      : undefined
                  }
                />
              ) : (
                <div className="table-scroll table-scroll--budget-lines bordered-top">
                  <table className="assets-table assets-table--budget-lines">
                    <thead>
                      <tr>
                        <th>Categoría</th>
                        <th className="num">Importe mensual (neto)</th>
                        {!isMobile && canEdit ? (
                          <th className="asset-actions-cell">
                            <span className="sr-only">Acciones</span>
                          </th>
                        ) : null}
                      </tr>
                    </thead>
                    <tbody>
                      {expenseEntries.map((row) => {
                        // La cuota deriva del plan de pago del pasivo: no es editable aquí, así
                        // que ni abre el modal en móvil ni ofrece acciones en escritorio.
                        const isQuota = row.source === "liability";
                        const rowTappable = isMobile && canEdit && !isQuota;
                        return (
                          <tr
                            key={row.id}
                            className={rowTappable ? "row-tappable" : undefined}
                            role={rowTappable ? "button" : undefined}
                            tabIndex={rowTappable ? 0 : undefined}
                            onClick={
                              rowTappable ? () => beginEditBudgetEntry(row) : undefined
                            }
                            onKeyDown={
                              rowTappable
                                ? (e) => {
                                    if (e.key === "Enter" || e.key === " ") {
                                      e.preventDefault();
                                      beginEditBudgetEntry(row);
                                    }
                                  }
                                : undefined
                            }
                          >
                            <td>
                              {budgetEntryCatLabel(categoryMapForSort, row)}
                              {isQuota ? (
                                <span
                                  className="chip budget-quota-chip"
                                  title="Cuota del plan de pago de un pasivo. Se edita en Pasivos."
                                >
                                  Cuota · {row.label}
                                </span>
                              ) : null}
                            </td>
                            <td className="num">
                              {formatCurrencyAmount(row.amount, currencyIso)}
                              {rowTappable ? (
                                <span className="row-chevron" aria-hidden>
                                  ›
                                </span>
                              ) : null}
                            </td>
                            {!isMobile && canEdit ? (
                              <td className="asset-actions-cell">
                                {isQuota ? null : (
                                  <div className="budget-row-actions">
                                    <button
                                      type="button"
                                      className="btn ghost icon-btn"
                                      aria-label="Editar línea"
                                      disabled={budgetSaving}
                                      onClick={() => beginEditBudgetEntry(row)}
                                    >
                                      <RowEditIcon />
                                    </button>
                                    <button
                                      type="button"
                                      className="btn ghost danger icon-btn"
                                      aria-label="Eliminar línea"
                                      disabled={budgetSaving}
                                      onClick={() => deleteBudgetEntryRow(row.id)}
                                    >
                                      <RowTrashIcon />
                                    </button>
                                  </div>
                                )}
                              </td>
                            ) : null}
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
              {hasQuotaEntries ? (
                <p className="muted tight">
                  Las líneas marcadas «Cuota» las genera el plan de pago de un
                  pasivo y ya cuentan en el total. Se editan en{" "}
                  <strong>Pasivos</strong>; no las dupliques como partida
                  propia.
                </p>
              ) : null}
            </section>
          </div>
        </div>
      )}
    </div>
  );
}
