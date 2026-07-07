import type { Dispatch, FormEvent, SetStateAction } from "react";
import type {
  CategoryRow,
  InstallationAccess,
  PlanningFlowApiRow,
  PlanningFlowDirectionApi,
} from "../api/types";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import { PlanningDirectionChart } from "../components/charts/PlanningDirectionChart";
import { PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyNumber,
  parseDisplayDecimal,
} from "../lib/format";
import { budgetCategoryMap, type LedgerPersonScope } from "../lib/ledger";
import { useIsMobile } from "../lib/responsive";
import type { BudgetScopeToggle } from "./BudgetView";

const PLANNING_DIRECTION_LABEL: Record<PlanningFlowDirectionApi, string> = {
  inflow: "Entrada",
  outflow: "Salida",
};

export function UpcomingView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  planningModalOpen,
  closePlanningModal,
  openNewPlanningModal,
  planningFlows,
  planningLoading,
  planningIncomeCategories,
  planningExpenseCategories,
  planningFormScope,
  setPlanningFormScope,
  planningFormCategoryId,
  setPlanningFormCategoryId,
  planningFormTitle,
  setPlanningFormTitle,
  planningFormAmount,
  setPlanningFormAmount,
  planningFormDue,
  setPlanningFormDue,
  planningFormNotes,
  setPlanningFormNotes,
  planningFormShowInChart,
  setPlanningFormShowInChart,
  editingPlanningFlowId,
  planningSaving,
  submitPlanningFlowForm,
  deletePlanningFlowRow,
  beginEditPlanningFlow,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  planningModalOpen: boolean;
  closePlanningModal: () => void;
  openNewPlanningModal: () => void;
  planningFlows: PlanningFlowApiRow[];
  planningLoading: boolean;
  planningIncomeCategories: CategoryRow[];
  planningExpenseCategories: CategoryRow[];
  planningFormScope: BudgetScopeToggle;
  setPlanningFormScope: Dispatch<SetStateAction<BudgetScopeToggle>>;
  planningFormCategoryId: string;
  setPlanningFormCategoryId: Dispatch<SetStateAction<string>>;
  planningFormTitle: string;
  setPlanningFormTitle: Dispatch<SetStateAction<string>>;
  planningFormAmount: string;
  setPlanningFormAmount: Dispatch<SetStateAction<string>>;
  planningFormDue: string;
  setPlanningFormDue: Dispatch<SetStateAction<string>>;
  planningFormNotes: string;
  setPlanningFormNotes: Dispatch<SetStateAction<string>>;
  planningFormShowInChart: boolean;
  setPlanningFormShowInChart: Dispatch<SetStateAction<boolean>>;
  editingPlanningFlowId: string | null;
  planningSaving: boolean;
  submitPlanningFlowForm: (e: FormEvent) => void;
  deletePlanningFlowRow: (id: string) => void;
  beginEditPlanningFlow: (row: PlanningFlowApiRow) => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const isMobile = useIsMobile();
  const categoryById = budgetCategoryMap(
    planningIncomeCategories,
    planningExpenseCategories,
  );

  const formCats =
    planningFormScope === "income"
      ? planningIncomeCategories
      : planningExpenseCategories;

  const planningInflowSum = planningFlows
    .filter((f) => f.direction === "inflow")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);
  const planningOutflowSum = planningFlows
    .filter((f) => f.direction === "outflow")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);

  const planningWorkspaceSub = installationBusy
    ? "Cargando…"
    : !hasMembership
      ? "Sin acceso hasta aprobación."
      : planningLoading
        ? "Cargando…"
        : `Importes · ${currency}`;

  const flowsNetTotal = !planningLoading
    ? planningFlows.reduce((acc, f) => {
        const amt = parseDisplayDecimal(f.expected_amount) ?? 0;
        return (
          acc +
          (f.direction === "inflow"
            ? amt
            : f.direction === "outflow"
              ? -amt
              : 0)
        );
      }, 0)
    : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Próximos</h2>
        <p className="workspace-sub">{planningWorkspaceSub}</p>
      </div>

      {hasMembership && ledgerPersonScope === "mine" ? (
        <div className="banner info-banner tight-banner">
          <strong>Mío</strong> · sin titular en <strong>Hogar</strong>
        </div>
      ) : null}

      {hasMembership ? (
        <div className="metric-grid workspace-kpi-strip planning-direction-strip">
          <MetricCard
            label="Entradas (suma)"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningInflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Salidas (suma)"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningOutflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Neto planificado"
            value={
              flowsNetTotal !== null
                ? formatCurrencyNumber(flowsNetTotal, currencyIso)
                : METRIC_DASH
            }
          />
        </div>
      ) : null}

      {hasMembership ? (
        <section className="panel">
          <h3 className="panel-title">Distribución</h3>
          {planningLoading ? (
            <p className="muted bordered-top">Cargando…</p>
          ) : planningFlows.length === 0 ? (
            <p className="muted bordered-top">Sin datos.</p>
          ) : planningInflowSum + planningOutflowSum > 0 ? (
            <PlanningDirectionChart
              inflow={planningInflowSum}
              outflow={planningOutflowSum}
            />
          ) : (
            <p className="muted bordered-top">Sin proporción.</p>
          )}
        </section>
      ) : null}

      {!installationBusy && !hasMembership ? (
        <div className="banner info-banner">Sin acceso al hogar.</div>
      ) : null}

      {hasMembership &&
      planningIncomeCategories.length === 0 &&
      planningExpenseCategories.length === 0 &&
      !planningLoading ? (
        <div className="banner info-banner">
          <strong>Ingresos/Gastos</strong> ·{" "}
          <strong>Ajustes → Categorías</strong>
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">Solo lectura.</p>
      ) : null}

      {canEdit &&
      hasMembership &&
      (planningIncomeCategories.length > 0 ||
        planningExpenseCategories.length > 0) ? (
        <Modal
          title={
            editingPlanningFlowId ? "Editar flujo planificado" : "Nuevo flujo"
          }
          open={planningModalOpen}
          onClose={closePlanningModal}
        >
          <form className="asset-form stack" onSubmit={submitPlanningFlowForm}>
            <ModalFormError message={formError} />
            <div className="segmented" role="tablist" aria-label="Dirección">
              <button
                type="button"
                role="tab"
                aria-selected={planningFormScope === "income"}
                className={planningFormScope === "income" ? "active" : ""}
                onClick={() => setPlanningFormScope("income")}
              >
                Entrada (ingreso)
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={planningFormScope === "expense"}
                className={planningFormScope === "expense" ? "active" : ""}
                onClick={() => setPlanningFormScope("expense")}
              >
                Salida (gasto)
              </button>
            </div>
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={planningFormCategoryId}
                  onChange={(e) => setPlanningFormCategoryId(e.target.value)}
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
                <span>Título</span>
                <input
                  value={planningFormTitle}
                  onChange={(e) => setPlanningFormTitle(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Nómina marzo"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Importe esperado</span>
                <input
                  value={planningFormAmount}
                  onChange={(e) => setPlanningFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Fecha prevista (opc.)</span>
                <input
                  type="date"
                  value={planningFormDue}
                  onChange={(e) => {
                    setPlanningFormDue(e.target.value);
                    if (e.target.value.trim() === "") {
                      setPlanningFormShowInChart(false);
                    }
                  }}
                />
              </label>
              {planningFormDue.trim() !== "" ? (
                <label className="field checkbox-field">
                  <input
                    type="checkbox"
                    checked={planningFormShowInChart}
                    onChange={(e) =>
                      setPlanningFormShowInChart(e.target.checked)
                    }
                  />
                  <span>Mostrar en la gráfica</span>
                </label>
              ) : null}
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={planningFormNotes}
                onChange={(e) => setPlanningFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={planningSaving || formCats.length === 0}
              >
                {editingPlanningFlowId ? "Guardar cambios" : "Añadir flujo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={planningSaving}
                onClick={() => closePlanningModal()}
              >
                Cancelar
              </button>
            </div>
            {isMobile && editingPlanningFlowId ? (
              <div className="modal-mobile-delete-row">
                <button
                  type="button"
                  className="btn ghost danger"
                  disabled={planningSaving}
                  onClick={() => deletePlanningFlowRow(editingPlanningFlowId)}
                >
                  Eliminar flujo
                </button>
              </div>
            ) : null}
          </form>
        </Modal>
      ) : null}

      <section className="panel">
        <div className="panel-head-row">
          <h3 className="panel-title">Lista</h3>
          {canEdit &&
          hasMembership &&
          (planningIncomeCategories.length > 0 ||
            planningExpenseCategories.length > 0) ? (
            <button
              type="button"
              className="btn primary icon-btn ledger-toolbar-add"
              aria-label="Nuevo flujo planificado"
              onClick={() => openNewPlanningModal()}
            >
              <PlusIcon />
            </button>
          ) : null}
        </div>
        {planningLoading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : planningFlows.length === 0 ? (
          <p className="muted bordered-top">
            No hay flujos planificados en esta instalación.
          </p>
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  {isMobile ? null : <th>Dirección</th>}
                  {isMobile ? null : <th>Categoría</th>}
                  <th>Título</th>
                  <th className="num">Importe</th>
                  {isMobile ? null : <th>Fecha prevista</th>}
                  {!isMobile && canEdit ? (
                    <th className="asset-actions-cell">
                      <span className="sr-only">Acciones</span>
                    </th>
                  ) : null}
                </tr>
              </thead>
              <tbody>
                {planningFlows.map((row) => {
                  const rowTappable = isMobile && canEdit;
                  const categoryLabel =
                    categoryById.get(row.category_id)?.name ??
                    row.category_id.slice(0, 8);
                  return (
                    <tr
                      key={row.id}
                      className={rowTappable ? "row-tappable" : undefined}
                      role={rowTappable ? "button" : undefined}
                      tabIndex={rowTappable ? 0 : undefined}
                      onClick={
                        rowTappable ? () => beginEditPlanningFlow(row) : undefined
                      }
                      onKeyDown={
                        rowTappable
                          ? (e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                beginEditPlanningFlow(row);
                              }
                            }
                          : undefined
                      }
                    >
                      {isMobile ? null : (
                        <td>{PLANNING_DIRECTION_LABEL[row.direction]}</td>
                      )}
                      {isMobile ? null : <td>{categoryLabel}</td>}
                      <td>
                        {row.title}
                        {isMobile ? (
                          <span className="cell-subline">
                            {PLANNING_DIRECTION_LABEL[row.direction]} · {categoryLabel}{" "}
                            · {row.due_date ?? METRIC_DASH}
                          </span>
                        ) : null}
                      </td>
                      <td className="num">
                        {formatCurrencyAmount(row.expected_amount, currencyIso)}
                        {rowTappable ? (
                          <span className="row-chevron" aria-hidden>
                            ›
                          </span>
                        ) : null}
                      </td>
                      {isMobile ? null : <td>{row.due_date ?? METRIC_DASH}</td>}
                      {!isMobile && canEdit ? (
                        <td className="asset-actions-cell">
                          <div className="budget-row-actions">
                            <button
                              type="button"
                              className="btn ghost icon-btn"
                              aria-label="Editar flujo planificado"
                              disabled={planningSaving}
                              onClick={() => beginEditPlanningFlow(row)}
                            >
                              <RowEditIcon />
                            </button>
                            <button
                              type="button"
                              className="btn ghost danger icon-btn"
                              aria-label="Eliminar flujo planificado"
                              disabled={planningSaving}
                              onClick={() => deletePlanningFlowRow(row.id)}
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
    </div>
  );
}
