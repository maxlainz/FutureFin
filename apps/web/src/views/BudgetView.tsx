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
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import { GearIcon, PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyOrDash,
} from "../lib/format";
import {
  PAYMENT_FREQ_LABEL,
  budgetCategoryMap,
  type LedgerPersonScope,
  sortBudgetEntriesMacStyle,
} from "../lib/ledger";
import { AllocationRulesPanel } from "./AllocationRulesPanel";

export type BudgetScopeToggle = "income" | "expense";

const BUDGET_SCOPE_LABEL: Record<BudgetScopeToggle, string> = {
  income: "Ingreso",
  expense: "Gasto",
};

function budgetDerivedCatLabel(categories: CategoryRow[], id: string): string {
  return categories.find((x) => x.id === id)?.name ?? id.slice(0, 8);
}

export function BudgetView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  budgetModalOpen,
  closeBudgetModal,
  openNewBudgetModal,
  budgetSnapshot,
  budgetLoading,
  budgetIncomeCategories,
  budgetExpenseCategories,
  budgetLiabilityCategories,
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
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  budgetModalOpen: boolean;
  closeBudgetModal: () => void;
  openNewBudgetModal: (scope?: BudgetScopeToggle) => void;
  budgetSnapshot: BudgetSnapshotApi | null;
  budgetLoading: boolean;
  budgetIncomeCategories: CategoryRow[];
  budgetExpenseCategories: CategoryRow[];
  budgetLiabilityCategories: CategoryRow[];
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
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

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

  const derivedLines = budgetSnapshot?.derived_from_liabilities ?? [];

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

  const incomeEntries = sortedEntries.filter((e) => e.scope === "income");
  const expenseEntries = sortedEntries.filter((e) => e.scope === "expense");

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
                : `Mensual · ${currency}`}
        </p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership &&
      budgetIncomeCategories.length === 0 &&
      budgetExpenseCategories.length === 0 &&
      !budgetLoading ? (
        <div className="banner info-banner">
          <strong>Ingresos/Gastos</strong> ·{" "}
          <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {hasMembership ? (
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
                <span>Importe mensual</span>
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
              <span>Notas (opc.)</span>
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
      ) : (
        <div className="budget-two-col">
          <section className="panel budget-col">
            <div className="panel-head-row">
              <h3 className="panel-title">Ingresos</h3>
              {canEdit && hasMembership && budgetIncomeCategories.length > 0 ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nueva línea de ingreso"
                  onClick={() => openNewBudgetModal("income")}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
            {incomeEntries.length === 0 ? (
              <p className="muted bordered-top">
                No hay líneas de ingreso en el presupuesto.
              </p>
            ) : (
              <div className="table-scroll table-scroll--budget-lines bordered-top">
                <table className="assets-table assets-table--budget-lines">
                  <thead>
                    <tr>
                      <th>Categoría</th>
                      <th className="num">Importe mensual</th>
                      {canEdit ? (
                        <th className="asset-actions-cell">
                          <span className="sr-only">Acciones</span>
                        </th>
                      ) : null}
                    </tr>
                  </thead>
                  <tbody>
                    {incomeEntries.map((row) => (
                      <tr key={row.id}>
                        <td>
                          {categoryMapForSort.get(row.category_id)?.name ??
                            row.category_id.slice(0, 8)}
                        </td>
                        <td className="num">
                          {formatCurrencyAmount(row.amount, currencyIso)}
                        </td>
                        {canEdit ? (
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
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <div className="budget-expenses-column">
            <section className="panel budget-col">
              <div className="panel-head-row">
                <h3 className="panel-title">Gastos</h3>
                {canEdit &&
                hasMembership &&
                budgetExpenseCategories.length > 0 ? (
                  <button
                    type="button"
                    className="btn primary icon-btn ledger-toolbar-add"
                    aria-label="Nueva línea de gasto"
                    onClick={() => openNewBudgetModal("expense")}
                  >
                    <PlusIcon />
                  </button>
                ) : null}
              </div>
              {expenseEntries.length === 0 ? (
                <p className="muted bordered-top">
                  No hay líneas de gasto recurrentes en el presupuesto.
                </p>
              ) : (
                <div className="table-scroll table-scroll--budget-lines bordered-top">
                  <table className="assets-table assets-table--budget-lines">
                    <thead>
                      <tr>
                        <th>Categoría</th>
                        <th className="num">Importe mensual</th>
                        {canEdit ? (
                          <th className="asset-actions-cell">
                            <span className="sr-only">Acciones</span>
                          </th>
                        ) : null}
                      </tr>
                    </thead>
                    <tbody>
                      {expenseEntries.map((row) => (
                        <tr key={row.id}>
                          <td>
                            {categoryMapForSort.get(row.category_id)?.name ??
                              row.category_id.slice(0, 8)}
                          </td>
                          <td className="num">
                            {formatCurrencyAmount(row.amount, currencyIso)}
                          </td>
                          {canEdit ? (
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
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <section className="panel budget-col">
              <h3 className="panel-title">Derivado de pasivos</h3>
              {derivedLines.length === 0 ? (
                <p className="muted bordered-top">
                  No hay cuotas derivadas en este momento.
                </p>
              ) : (
                <div className="table-scroll bordered-top">
                  <table className="assets-table">
                    <thead>
                      <tr>
                        <th>Concepto</th>
                        <th>Categoría pasivo</th>
                        <th className="num">Cuota</th>
                        <th>Frec.</th>
                        <th className="num">Equiv. mensual</th>
                      </tr>
                    </thead>
                    <tbody>
                      {derivedLines.map((row) => (
                        <tr key={row.liability_id}>
                          <td>{row.label}</td>
                          <td>
                            {budgetDerivedCatLabel(
                              budgetLiabilityCategories,
                              row.category_id,
                            )}
                          </td>
                          <td className="num">
                            {formatCurrencyAmount(row.amount, currencyIso)}
                          </td>
                          <td>{PAYMENT_FREQ_LABEL[row.frequency]}</td>
                          <td className="num">
                            {formatCurrencyAmount(
                              row.monthly_equivalent,
                              currencyIso,
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </div>
        </div>
      )}
    </div>
  );
}
