import type { Dispatch, FormEvent, SetStateAction } from "react";
import type {
  CategoryRow,
  InstallationAccess,
  PlanningAmountBasisApi,
  PlanningFlowApiRow,
  PlanningFlowDirectionApi,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import { PlanningDirectionChart } from "../components/charts/PlanningDirectionChart";
import { PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import { formatDateDmy, todayYmdInTimeZone } from "../lib/dates";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyNumber,
  parseDisplayDecimal,
} from "../lib/format";
import { budgetCategoryMap } from "../lib/ledger";
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
  planningFormBasis,
  setPlanningFormBasis,
  planningFormWindowStart,
  setPlanningFormWindowStart,
  planningFormWindowEnd,
  setPlanningFormWindowEnd,
  planningFormNotes,
  setPlanningFormNotes,
  planningFormShowInChart,
  setPlanningFormShowInChart,
  editingPlanningFlowId,
  planningSaving,
  submitPlanningFlowForm,
  deletePlanningFlowRow,
  beginEditPlanningFlow,
  onOpenCategorySettings,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
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
  planningFormBasis: PlanningAmountBasisApi;
  setPlanningFormBasis: Dispatch<SetStateAction<PlanningAmountBasisApi>>;
  planningFormWindowStart: string;
  setPlanningFormWindowStart: Dispatch<SetStateAction<string>>;
  planningFormWindowEnd: string;
  setPlanningFormWindowEnd: Dispatch<SetStateAction<string>>;
  planningFormNotes: string;
  setPlanningFormNotes: Dispatch<SetStateAction<string>>;
  planningFormShowInChart: boolean;
  setPlanningFormShowInChart: Dispatch<SetStateAction<boolean>>;
  editingPlanningFlowId: string | null;
  planningSaving: boolean;
  submitPlanningFlowForm: (e: FormEvent) => void;
  deletePlanningFlowRow: (id: string) => void;
  beginEditPlanningFlow: (row: PlanningFlowApiRow) => void;
  /**
   * Lleva a `Ajustes → Categorías`. Única salida cuando no queda ninguna categoría de ingreso
   * ni de gasto donde anotar un movimiento previsto.
   */
  onOpenCategorySettings?: () => void;
}) {
  const currencyIso = installation?.installation.base_currency ?? "";
  const isMobile = useIsMobile();
  // #126: un Próximo con fecha anterior al día 1 del mes en curso ya no desaparece de la
  // proyección — carga íntegro en el mes actual, y esta marca es la única señal que el usuario
  // tiene de que ese apunte ya está moviendo su curva. Comparación de strings ISO: es segura.
  const calendarTz = installation?.installation.calendar_tz?.trim() || "UTC";
  const anchorMonthFirstYmd = `${todayYmdInTimeZone(calendarTz).slice(0, 7)}-01`;
  const rowIsOverdue = (row: PlanningFlowApiRow): boolean =>
    Boolean(row.due_date && row.due_date < anchorMonthFirstYmd);
  // #148: la celda de calendario de un recurrente es su periodo, no un vencimiento.
  const rowPeriodLabel = (row: PlanningFlowApiRow): string => {
    if (row.amount_basis === "per_month") {
      const start = row.window_start_date
        ? formatDateDmy(row.window_start_date)
        : METRIC_DASH;
      const end = row.window_end_date ? formatDateDmy(row.window_end_date) : "sin fin";
      return `${start} – ${end}`;
    }
    return row.due_date ? formatDateDmy(row.due_date) : METRIC_DASH;
  };
  const categoryById = budgetCategoryMap(
    planningIncomeCategories,
    planningExpenseCategories,
  );

  const formCats =
    planningFormScope === "income"
      ? planningIncomeCategories
      : planningExpenseCategories;

  // #148: los totales en € suman SOLO puntuales — el expected_amount de un recurrente son
  // €/MES y mezclarlo aquí sería un error de magnitud (el mismo que se arregló en /v1/summary).
  const planningInflowSum = planningFlows
    .filter((f) => f.direction === "inflow" && f.amount_basis !== "per_month")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);
  const planningOutflowSum = planningFlows
    .filter((f) => f.direction === "outflow" && f.amount_basis !== "per_month")
    .reduce((acc, f) => acc + (parseDisplayDecimal(f.expected_amount) ?? 0), 0);
  const recurringRows = planningFlows.filter((f) => f.amount_basis === "per_month");
  const recurringMonthlyNet = recurringRows.reduce((acc, f) => {
    const amt = parseDisplayDecimal(f.expected_amount) ?? 0;
    return acc + (f.direction === "inflow" ? amt : -amt);
  }, 0);

  const planningWorkspaceSub = installationBusy
    ? "Cargando…"
    : !hasMembership
      ? "Sin acceso hasta aprobación."
      : planningLoading
        ? "Cargando…"
        : "Importes";

  // Política de ceros: la unidad es el BLOQUE. Con flujos planificados se pintan las tres
  // cifras aunque alguna valga 0 €; sin ninguno, la banda y la distribución desaparecen y
  // habla el estado vacío de la lista (tres ceros no explican para qué sirve la pestaña).
  const planningIsEmpty = !planningLoading && planningFlows.length === 0;
  const noPlanningCategories =
    planningIncomeCategories.length === 0 &&
    planningExpenseCategories.length === 0;

  const flowsNetTotal = !planningLoading
    ? planningFlows.reduce((acc, f) => {
        if (f.amount_basis === "per_month") {
          return acc; // €/mes: su neto va en la tarjeta «Recurrentes», no en este total en €.
        }
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

      {hasMembership && !planningIsEmpty ? (
        <div className="metric-grid workspace-kpi-strip planning-direction-strip">
          <MetricCard
            label="Entradas (suma)"
            helpId="upcoming.inflows"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningInflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Salidas (suma)"
            helpId="upcoming.outflows"
            value={
              !planningLoading
                ? formatCurrencyNumber(planningOutflowSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Neto planificado"
            helpId="upcoming.net"
            value={
              flowsNetTotal !== null
                ? formatCurrencyNumber(flowsNetTotal, currencyIso)
                : METRIC_DASH
            }
          />
          {recurringRows.length > 0 ? (
            <MetricCard
              label="Recurrentes (neto /mes)"
              helpId="upcoming.recurring_net"
              value={
                !planningLoading
                  ? `${formatCurrencyNumber(recurringMonthlyNet, currencyIso)} /mes`
                  : METRIC_DASH
              }
            />
          ) : null}
        </div>
      ) : null}

      {hasMembership && !planningIsEmpty ? (
        <section className="panel">
          <h3 className="panel-title">Distribución</h3>
          {planningLoading ? (
            <p className="muted bordered-top">Cargando…</p>
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
            {/* #148: la base del importe. «Puntual» = total en €; «Recurrente» = €/mes durante
                un periodo. Cambiar de pestaña no borra nada hasta guardar. */}
            <div className="segmented" role="tablist" aria-label="Tipo de flujo">
              <button
                type="button"
                role="tab"
                aria-selected={planningFormBasis === "one_off"}
                className={planningFormBasis === "one_off" ? "active" : ""}
                onClick={() => setPlanningFormBasis("one_off")}
              >
                Puntual
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={planningFormBasis === "per_month"}
                className={planningFormBasis === "per_month" ? "active" : ""}
                onClick={() => setPlanningFormBasis("per_month")}
              >
                Recurrente
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
                <span>
                  {planningFormBasis === "per_month"
                    ? "Importe mensual"
                    : "Importe esperado"}
                </span>
                <input
                  value={planningFormAmount}
                  onChange={(e) => setPlanningFormAmount(e.target.value)}
                  required
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              {planningFormBasis === "per_month" ? (
                <>
                  <label className="field">
                    <span>Desde</span>
                    <input
                      type="date"
                      value={planningFormWindowStart}
                      onChange={(e) => setPlanningFormWindowStart(e.target.value)}
                      required
                    />
                  </label>
                  <label className="field">
                    <span>Hasta (opcional — vacío = sin fin)</span>
                    <input
                      type="date"
                      value={planningFormWindowEnd}
                      onChange={(e) => setPlanningFormWindowEnd(e.target.value)}
                    />
                  </label>
                </>
              ) : (
                <>
                  <label className="field">
                    <span>Fecha prevista (opcional)</span>
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
                </>
              )}
            </div>
            <label className="field">
              <span>Notas (opcional)</span>
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
          {/* Siempre visible mientras se pueda editar (misma razón que en Activos):
              ocultarlo sin categorías dejaba al usuario sin salida. */}
          {canEdit && hasMembership ? (
            <button
              type="button"
              className="btn primary icon-btn ledger-toolbar-add"
              aria-label="Nuevo movimiento previsto"
              title={
                noPlanningCategories
                  ? "Necesitas una categoría de ingreso o de gasto"
                  : "Nuevo movimiento previsto"
              }
              disabled={noPlanningCategories}
              onClick={() => openNewPlanningModal()}
            >
              <PlusIcon />
            </button>
          ) : null}
        </div>
        {planningLoading ? (
          <p className="muted bordered-top">Cargando…</p>
        ) : planningFlows.length === 0 ? (
          !hasMembership ? (
            <p className="muted bordered-top">Sin acceso.</p>
          ) : formError ? (
            // Un fallo del loader vacía movimientos Y categorías: sin este caso la vista acusaba
            // al usuario de no tener categorías cuando lo que había pasado es que no se pudo leer
            // nada. El error ya se pinta en la banda global de App.tsx; aquí basta con no
            // contradecirlo.
            null
          ) : noPlanningCategories ? (
            <EmptyState
              embedded
              title="Faltan categorías"
              description="Cada movimiento previsto se anota en una categoría de ingreso o de gasto. No queda ninguna, así que créalas antes de planificar."
              actionLabel={canEdit ? "Crear categorías" : undefined}
              onAction={canEdit ? onOpenCategorySettings : undefined}
            />
          ) : (
            <EmptyState
              embedded
              title="Sin movimientos previstos"
              description="Aquí apuntas lo puntual que ya sabes que llega: la paga extra, el seguro del coche, una reforma. FutureFin lo coloca en la proyección en su fecha."
              actionLabel={canEdit ? "Añadir movimiento previsto" : undefined}
              onAction={canEdit ? openNewPlanningModal : undefined}
            />
          )
        ) : (
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  {isMobile ? null : <th>Dirección</th>}
                  {isMobile ? null : <th>Categoría</th>}
                  <th>Título</th>
                  <th className="num">Importe</th>
                  {isMobile ? null : <th>Fecha / periodo</th>}
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
                            · {rowPeriodLabel(row)}
                            {row.amount_basis === "per_month" ? " · /mes" : null}
                            {rowIsOverdue(row) ? " · vencido, carga este mes" : null}
                          </span>
                        ) : null}
                      </td>
                      <td className="num">
                        {formatCurrencyAmount(row.expected_amount, currencyIso)}
                        {isMobile ? null : (
                          // Slot SIEMPRE reservado (design-system: adornos en celdas numéricas
                          // van en ancho fijo o la columna se desalinea).
                          <span className="amount-unit-slot" aria-hidden={row.amount_basis !== "per_month"}>
                            {row.amount_basis === "per_month" ? "/mes" : ""}
                          </span>
                        )}
                        {rowTappable ? (
                          <span className="row-chevron" aria-hidden>
                            ›
                          </span>
                        ) : null}
                      </td>
                      {isMobile ? null : (
                        <td>
                          {rowPeriodLabel(row)}
                          {rowIsOverdue(row) ? (
                            <span
                              className={
                                row.direction === "outflow"
                                  ? "chip upcoming-overdue-chip neg"
                                  : "chip upcoming-overdue-chip"
                              }
                              title="La fecha ya pasó: el importe carga íntegro en el mes en curso de la proyección, no desaparece. Bórralo o cámbiale la fecha si ya se liquidó."
                            >
                              Vencido · se carga este mes
                            </span>
                          ) : null}
                        </td>
                      )}
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
