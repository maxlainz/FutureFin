import type { Dispatch, FormEvent, SetStateAction } from "react";
import type {
  CategoryRow,
  InstallationAccess,
  LiabilityApiRow,
  LiabilityRepaymentModelApi,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
import { MetricCard } from "../components/MetricCard";
import { InlineHint, Modal, ModalFormError } from "../components/Modal";
import { SnapshotButton } from "../components/SnapshotButton";
import { PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import {
  METRIC_DASH,
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatCurrencyOrDash,
  formatPercentAmount,
  formatPercentDisplay,
  parseDisplayDecimal,
} from "../lib/format";
import {
  PAYMENT_FREQ_LABEL,
  REPAYMENT_MODEL_LABEL,
  REPAYMENT_MODEL_ORDER,
  type LedgerPersonScope,
  type LiabilityPaymentFreq,
  groupRowsByCategoryOrdered,
  liabilitiesApproxMonthlyInterestSum,
  liabilitiesWeightedAprPercent,
  liabilityDerivedPrincipalPreview,
  liabilityPaymentMonthlyEquivalentNum,
} from "../lib/ledger";
import { useIsMobile } from "../lib/responsive";

export function LiabilitiesView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  liabilityModalOpen,
  closeLiabilityModal,
  openNewLiabilityModal,
  liabilities,
  liabilitiesBusy,
  liabilityCategories,
  liabilityExpenseCategories,
  liabilityFormCategoryId,
  setLiabilityFormCategoryId,
  liabilityFormExpenseCategoryId,
  setLiabilityFormExpenseCategoryId,
  liabilityFormLabel,
  setLiabilityFormLabel,
  liabilityFormTypeTag,
  setLiabilityFormTypeTag,
  liabilityFormPrincipal,
  setLiabilityFormPrincipal,
  liabilityFormApr,
  setLiabilityFormApr,
  liabilityFormPaymentAmount,
  setLiabilityFormPaymentAmount,
  liabilityFormPaymentFrequency,
  setLiabilityFormPaymentFrequency,
  liabilityFormPaymentEnd,
  setLiabilityFormPaymentEnd,
  liabilityFormNotes,
  setLiabilityFormNotes,
  liabilityFormDerivePrincipal,
  setLiabilityFormDerivePrincipal,
  liabilityFormRepaymentModel,
  setLiabilityFormRepaymentModel,
  liabilityFormMinPct,
  setLiabilityFormMinPct,
  liabilityFormMinEur,
  setLiabilityFormMinEur,
  editingLiabilityId,
  liabilitySaving,
  submitLiabilityForm,
  deleteLiabilityRow,
  beginEditLiability,
  onSaveSnapshot,
  onOpenCategorySettings,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  liabilityModalOpen: boolean;
  closeLiabilityModal: () => void;
  openNewLiabilityModal: () => void;
  liabilities: LiabilityApiRow[];
  liabilitiesBusy: boolean;
  liabilityCategories: CategoryRow[];
  /** Categorías scope `expense` para la categoría de la cuota (obligatoria al crear, 3.4.0). */
  liabilityExpenseCategories: CategoryRow[];
  liabilityFormCategoryId: string;
  setLiabilityFormCategoryId: Dispatch<SetStateAction<string>>;
  liabilityFormExpenseCategoryId: string;
  setLiabilityFormExpenseCategoryId: Dispatch<SetStateAction<string>>;
  liabilityFormLabel: string;
  setLiabilityFormLabel: Dispatch<SetStateAction<string>>;
  liabilityFormTypeTag: string;
  setLiabilityFormTypeTag: Dispatch<SetStateAction<string>>;
  liabilityFormPrincipal: string;
  setLiabilityFormPrincipal: Dispatch<SetStateAction<string>>;
  liabilityFormApr: string;
  setLiabilityFormApr: Dispatch<SetStateAction<string>>;
  liabilityFormPaymentAmount: string;
  setLiabilityFormPaymentAmount: Dispatch<SetStateAction<string>>;
  liabilityFormPaymentFrequency: LiabilityPaymentFreq;
  setLiabilityFormPaymentFrequency: Dispatch<SetStateAction<LiabilityPaymentFreq>>;
  liabilityFormPaymentEnd: string;
  setLiabilityFormPaymentEnd: Dispatch<SetStateAction<string>>;
  liabilityFormNotes: string;
  setLiabilityFormNotes: Dispatch<SetStateAction<string>>;
  liabilityFormDerivePrincipal: boolean;
  setLiabilityFormDerivePrincipal: Dispatch<SetStateAction<boolean>>;
  /** Modelo de amortización del pasivo (4.2.0); `fixed_payments` es el histórico. */
  liabilityFormRepaymentModel: LiabilityRepaymentModelApi;
  setLiabilityFormRepaymentModel: Dispatch<
    SetStateAction<LiabilityRepaymentModelApi>
  >;
  /** Cuota mínima revolving: % del saldo de apertura (solo aplica con modelo revolving). */
  liabilityFormMinPct: string;
  setLiabilityFormMinPct: Dispatch<SetStateAction<string>>;
  /** Suelo en euros de la cuota mínima revolving. */
  liabilityFormMinEur: string;
  setLiabilityFormMinEur: Dispatch<SetStateAction<string>>;
  editingLiabilityId: string | null;
  liabilitySaving: boolean;
  submitLiabilityForm: (e: FormEvent) => void;
  deleteLiabilityRow: (id: string) => void;
  beginEditLiability: (row: LiabilityApiRow) => void;
  /** Captura un snapshot de pasivos de hoy. `true` = guardado; lanza `Error` si falla. */
  onSaveSnapshot?: () => Promise<void>;
  /**
   * Lleva a `Ajustes → Categorías`. Única salida cuando faltan las categorías que un pasivo
   * necesita: la suya y la de gasto donde se atribuye la cuota.
   */
  onOpenCategorySettings?: () => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";
  const isMobile = useIsMobile();

  // Crear un pasivo exige DOS categorías: la suya y la de gasto donde cae la cuota (el select
  // de la cuota es `required` en el alta). Sin alguna de las dos el formulario no se puede
  // completar, así que el botón queda inerte y el estado vacío lo explica.
  const liabilityCreationBlocked =
    liabilityCategories.length === 0 || liabilityExpenseCategories.length === 0;

  const derivePreview = liabilityDerivedPrincipalPreview(
    liabilityFormPaymentAmount,
    liabilityFormPaymentFrequency,
    liabilityFormPaymentEnd,
    installation?.installation.calendar_tz ?? "UTC",
    currencyIso,
    liabilityFormRepaymentModel,
    liabilityFormApr,
  );

  const liabilityMetricsReady = hasMembership && !liabilitiesBusy;
  const liabilityPrincipalSum = liabilityMetricsReady
    ? liabilities.reduce(
        (acc, r) => acc + (parseDisplayDecimal(r.principal) ?? 0),
        0,
      )
    : null;
  const liabilitiesMonthlyServiceSum = liabilityMetricsReady
    ? liabilities.reduce(
        (acc, r) => acc + liabilityPaymentMonthlyEquivalentNum(r),
        0,
      )
    : null;

  const liabilitiesWeightedApr = liabilityMetricsReady
    ? liabilitiesWeightedAprPercent(liabilities)
    : null;

  const liabilitiesApproxMonthlyInterest = liabilityMetricsReady
    ? liabilitiesApproxMonthlyInterestSum(liabilities)
    : null;

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Pasivos</h2>
        <p className="workspace-sub">
          {installationBusy
            ? "Cargando…"
            : !hasMembership
              ? "Sin acceso hasta aprobación."
              : `Moneda ${currency}`}
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

      {/* Política de ceros: se decide por bloque. Con pasivos se pintan las cuatro cifras
          aunque alguna valga 0 €; sin ninguno la banda entera desaparece en favor del estado
          vacío de abajo. */}
      {hasMembership && (liabilitiesBusy || liabilities.length > 0) ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Principal total" helpId="liabilities.principal_total"
            value={
              liabilityMetricsReady && liabilityPrincipalSum !== null
                ? formatCurrencyNumber(liabilityPrincipalSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Servicio mensual equivalente" helpId="liabilities.monthly_service"
            value={
              liabilityMetricsReady && liabilitiesMonthlyServiceSum !== null
                ? formatCurrencyNumber(liabilitiesMonthlyServiceSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="TAE media ponderada" helpId="liabilities.weighted_apr"
            value={
              liabilityMetricsReady && liabilitiesWeightedApr !== null
                ? formatPercentDisplay(liabilitiesWeightedApr)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Interés mensual aprox." helpId="liabilities.approx_monthly_interest"
            value={
              liabilityMetricsReady && liabilitiesApproxMonthlyInterest !== null
                ? formatCurrencyNumber(
                    liabilitiesApproxMonthlyInterest,
                    currencyIso,
                  )
                : METRIC_DASH
            }
          />
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">Solo lectura.</p>
      ) : null}

      {canEdit && hasMembership && liabilityCategories.length > 0 ? (
        <Modal
          title={editingLiabilityId ? "Editar pasivo" : "Nuevo pasivo"}
          open={liabilityModalOpen}
          onClose={closeLiabilityModal}
        >
          <form className="asset-form stack" onSubmit={submitLiabilityForm}>
            <ModalFormError message={formError} />
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={liabilityFormCategoryId}
                  onChange={(e) => setLiabilityFormCategoryId(e.target.value)}
                  required
                >
                  {liabilityCategories.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Etiqueta</span>
                <input
                  value={liabilityFormLabel}
                  onChange={(e) => setLiabilityFormLabel(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Préstamo coche"
                />
              </label>
              <label className="field">
                <span className="checkbox-label-with-hint">
                  Categoría de la cuota (gasto)
                  <InlineHint title="El presupuesto y la comparativa de Movimientos atribuyen ahí la cuota del plan — no la dupliques como partida propia." />
                </span>
                <select
                  value={liabilityFormExpenseCategoryId}
                  onChange={(e) =>
                    setLiabilityFormExpenseCategoryId(e.target.value)
                  }
                  required={!editingLiabilityId}
                >
                  {editingLiabilityId && !liabilityFormExpenseCategoryId ? (
                    <option value="">Sin asignar (elige una)</option>
                  ) : null}
                  {liabilityExpenseCategories.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Tipo (opc.)</span>
                <input
                  value={liabilityFormTypeTag}
                  onChange={(e) => setLiabilityFormTypeTag(e.target.value)}
                  maxLength={120}
                  placeholder="Etiqueta libre"
                />
              </label>
              <label className="field">
                <span className="checkbox-label-with-hint">
                  Modelo
                  <InlineHint title="Francés: el préstamo español típico — cada cuota paga interés y amortiza el resto (una hipoteca de 200.000 € a 1.000 €/mes al 3 % tarda 278 meses y ~78.000 € de intereses, no 200 meses y 0 €). Sin intereses (0 %): la cuota va íntegra a principal; solo para deudas realmente gratuitas. Solo intereses (carencia): la cuota del mes es el interés del saldo; el principal no baja. Revolving: cuota mínima = % del saldo con suelo en €. Todos menos «Sin intereses» exigen TIN > 0 y cuota mensual." />
                </span>
                <select
                  value={liabilityFormRepaymentModel}
                  onChange={(e) =>
                    setLiabilityFormRepaymentModel(
                      e.target.value as LiabilityRepaymentModelApi,
                    )
                  }
                >
                  {REPAYMENT_MODEL_ORDER.map((m) => (
                    <option key={m} value={m}>
                      {REPAYMENT_MODEL_LABEL[m]}
                    </option>
                  ))}
                </select>
              </label>
              {liabilityFormRepaymentModel === "revolving" ? (
                <>
                  <label className="field">
                    <span className="checkbox-label-with-hint">
                      Mínimo % saldo
                      <InlineHint title="Cuota mínima revolving: porcentaje del saldo de cada mes, con el suelo en € como mínimo absoluto. Se exige al menos uno de los dos > 0." />
                    </span>
                    <input
                      value={liabilityFormMinPct}
                      onChange={(e) => setLiabilityFormMinPct(e.target.value)}
                      inputMode="decimal"
                      placeholder="p. ej. 3"
                      autoComplete="off"
                    />
                  </label>
                  <label className="field">
                    <span>Mínimo suelo €</span>
                    <input
                      value={liabilityFormMinEur}
                      onChange={(e) => setLiabilityFormMinEur(e.target.value)}
                      inputMode="decimal"
                      placeholder="p. ej. 30"
                      autoComplete="off"
                    />
                  </label>
                </>
              ) : null}
              <label
                className="field"
                style={{
                  gridColumn: "1 / -1",
                  flexDirection: "row",
                  alignItems: "flex-start",
                  gap: "0.5rem",
                }}
              >
                <input
                  type="checkbox"
                  checked={liabilityFormDerivePrincipal}
                  onChange={(e) =>
                    setLiabilityFormDerivePrincipal(e.target.checked)
                  }
                  style={{ marginTop: "0.2rem" }}
                />
                <span className="checkbox-label-with-hint">
                  Derivar principal desde el plan
                  <InlineHint title="Hoy civil = zona Calendario. Semanal ≈ días÷7." />
                </span>
              </label>
              <label className="field">
                <span>
                  Principal
                  {liabilityFormDerivePrincipal ? " (calculado al guardar)" : ""}
                </span>
                <input
                  value={liabilityFormPrincipal}
                  onChange={(e) => setLiabilityFormPrincipal(e.target.value)}
                  required={!liabilityFormDerivePrincipal}
                  disabled={liabilityFormDerivePrincipal}
                  inputMode="decimal"
                  autoComplete="off"
                />
                {liabilityFormDerivePrincipal && derivePreview ? (
                  <span className="muted tight">
                    Vista previa ~{derivePreview} (hoy en{" "}
                    {installation?.installation.calendar_tz ?? "UTC"})
                  </span>
                ) : null}
              </label>
              <label className="field">
                <span>TAE % (opc.)</span>
                <input
                  value={liabilityFormApr}
                  onChange={(e) => setLiabilityFormApr(e.target.value)}
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>
                  Cuota plan
                  {liabilityFormDerivePrincipal ? "" : " (opc.)"}
                </span>
                <input
                  value={liabilityFormPaymentAmount}
                  onChange={(e) => setLiabilityFormPaymentAmount(e.target.value)}
                  inputMode="decimal"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Frecuencia</span>
                <select
                  value={liabilityFormPaymentFrequency}
                  onChange={(e) =>
                    setLiabilityFormPaymentFrequency(
                      e.target.value as LiabilityPaymentFreq,
                    )
                  }
                >
                  <option value="">Sin plan</option>
                  <option value="monthly">Mensual</option>
                  <option value="weekly">Semanal</option>
                </select>
              </label>
              <label className="field">
                <span>
                  Fin plan
                  {liabilityFormDerivePrincipal ? "" : " (opc.)"}
                </span>
                <input
                  type="date"
                  value={liabilityFormPaymentEnd}
                  onChange={(e) => setLiabilityFormPaymentEnd(e.target.value)}
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={liabilityFormNotes}
                onChange={(e) => setLiabilityFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={liabilitySaving}
              >
                {editingLiabilityId ? "Guardar cambios" : "Añadir pasivo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={liabilitySaving}
                onClick={() => closeLiabilityModal()}
              >
                Cancelar
              </button>
            </div>
            {isMobile && editingLiabilityId ? (
              <div className="modal-mobile-delete-row">
                <button
                  type="button"
                  className="btn ghost danger"
                  disabled={liabilitySaving}
                  onClick={() => deleteLiabilityRow(editingLiabilityId)}
                >
                  Eliminar pasivo
                </button>
              </div>
            ) : null}
          </form>
        </Modal>
      ) : null}

      <div className="ledger-list-section">
        <div className="ledger-list-toolbar">
          <div className="panel-head-row">
            <h3 className="panel-title">Pasivos por categoría</h3>
            <div className="ledger-toolbar-actions">
              {onSaveSnapshot &&
              canEdit &&
              hasMembership &&
              liabilities.length > 0 ? (
                <SnapshotButton
                  kind="liability"
                  disabled={liabilitySaving}
                  onSave={onSaveSnapshot}
                />
              ) : null}
              {/* Siempre visible mientras se pueda editar (misma razón que en Activos):
                  ocultarlo sin categorías dejaba al usuario sin salida. */}
              {canEdit && hasMembership ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nuevo pasivo"
                  title={
                    liabilityCreationBlocked
                      ? "Necesitas una categoría de pasivo y otra de gasto antes de crear uno"
                      : "Nuevo pasivo"
                  }
                  disabled={liabilityCreationBlocked}
                  onClick={() => openNewLiabilityModal()}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
          </div>
          {liabilitiesBusy ? <p className="muted">Cargando…</p> : null}
        </div>
        {/* `formError` gatea el estado vacío: un fallo del loader vacía lista Y categorías, y sin
            este gate la vista culpaba al usuario («faltan categorías») de un error de carga. El
            error ya se pinta en la banda global de App.tsx. */}
        {!liabilitiesBusy && !formError && hasMembership && liabilities.length === 0 ? (
          liabilityCreationBlocked ? (
            <EmptyState
              title="Faltan categorías para tus deudas"
              description="Un pasivo necesita su propia categoría (hipoteca, préstamo) y una categoría de gasto donde atribuir la cuota. Créalas y vuelve aquí."
              actionLabel={canEdit ? "Crear categorías" : undefined}
              onAction={canEdit ? onOpenCategorySettings : undefined}
            />
          ) : (
            <EmptyState
              title="Sin pasivos"
              description="Aquí anotas lo que debes: hipoteca, préstamos, financiaciones. FutureFin resta su principal del patrimonio y lleva su cuota al presupuesto."
              actionLabel={canEdit ? "Añadir pasivo" : undefined}
              onAction={canEdit ? openNewLiabilityModal : undefined}
            />
          )
        ) : null}
        {!liabilitiesBusy && liabilities.length > 0 ? (
          <div className="ledger-by-category-stack">
            {groupRowsByCategoryOrdered(liabilities, liabilityCategories, {
              sortRowsDescending: {
                value: (row) => parseDisplayDecimal(row.principal) ?? 0,
                tieBreak: (a, b) => a.label.localeCompare(b.label, "es"),
              },
              categoryTotalDescending: (items) =>
                items.reduce(
                  (acc, row) => acc + (parseDisplayDecimal(row.principal) ?? 0),
                  0,
                ),
            }).map((g) => {
              const catPrincipal = g.items.reduce(
                (acc, row) => acc + (parseDisplayDecimal(row.principal) ?? 0),
                0,
              );
              return (
                <section key={g.categoryId} className="panel ledger-category-panel">
                  <div className="panel-head-row">
                    <h3 className="panel-title">{g.label}</h3>
                    <span className="ledger-category-total">
                      {formatCurrencyNumber(catPrincipal, currencyIso)}
                    </span>
                  </div>
                  <div className="table-scroll bordered-top">
                    <table className="assets-table">
                      <thead>
                        <tr>
                          <th>Etiqueta</th>
                          <th className="num">Principal</th>
                          {isMobile ? null : <th className="num">TAE %</th>}
                          {isMobile ? null : <th className="num">Cuota</th>}
                          {isMobile ? null : <th>Frec.</th>}
                          {isMobile ? null : <th>Fin plan</th>}
                          {!isMobile && canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((row) => {
                          const rowTappable = isMobile && canEdit;
                          const aprLabel =
                            row.apr_percent != null &&
                            String(row.apr_percent).trim() !== ""
                              ? formatPercentAmount(row.apr_percent)
                              : METRIC_DASH;
                          const freqLabel = row.payment_frequency
                            ? PAYMENT_FREQ_LABEL[row.payment_frequency]
                            : METRIC_DASH;
                          return (
                            <tr
                              key={row.id}
                              className={rowTappable ? "row-tappable" : undefined}
                              role={rowTappable ? "button" : undefined}
                              tabIndex={rowTappable ? 0 : undefined}
                              onClick={
                                rowTappable ? () => beginEditLiability(row) : undefined
                              }
                              onKeyDown={
                                rowTappable
                                  ? (e) => {
                                      if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        beginEditLiability(row);
                                      }
                                    }
                                  : undefined
                              }
                            >
                              <td>
                                {row.label}
                                {/* Solo cuando el modelo NO es el histórico: un chip «Cuota fija»
                                    en cada fila sería ruido en el 100 % de los pasivos previos a
                                    4.2.0. El chip marca lo que es información nueva. */}
                                {row.repayment_model &&
                                row.repayment_model !== "fixed_payments" ? (
                                  <>
                                    {" "}
                                    <span
                                      className="chip"
                                      title="Modelo de amortización del pasivo"
                                    >
                                      {REPAYMENT_MODEL_LABEL[
                                        row.repayment_model
                                      ] ?? row.repayment_model}
                                    </span>
                                  </>
                                ) : null}
                                {!row.expense_category_id ? (
                                  <span
                                    className="muted"
                                    title="Edítalo y asígnale una categoría de gasto para que el presupuesto y Movimientos emparejen su cuota."
                                  >
                                    {" "}
                                    · sin categoría de cuota
                                  </span>
                                ) : null}
                                {isMobile ? (
                                  <span className="cell-subline">
                                    TAE {aprLabel} · Cuota{" "}
                                    {formatCurrencyOrDash(
                                      row.payment_amount,
                                      currencyIso,
                                    )}{" "}
                                    · {freqLabel} · Fin{" "}
                                    {row.payment_end_date ?? METRIC_DASH}
                                  </span>
                                ) : null}
                              </td>
                              <td className="num">
                                {formatCurrencyAmount(row.principal, currencyIso)}
                                {row.principal_derived_from_plan ? (
                                  <span
                                    className="muted"
                                    title="Principal derivado del plan"
                                  >
                                    {" "}
                                    deriv.
                                  </span>
                                ) : null}
                                {rowTappable ? (
                                  <span className="row-chevron" aria-hidden>
                                    ›
                                  </span>
                                ) : null}
                              </td>
                              {isMobile ? null : (
                                <td className="num">{aprLabel}</td>
                              )}
                              {isMobile ? null : (
                                <td className="num">
                                  {formatCurrencyOrDash(
                                    row.payment_amount,
                                    currencyIso,
                                  )}
                                </td>
                              )}
                              {isMobile ? null : <td>{freqLabel}</td>}
                              {isMobile ? null : (
                                <td>{row.payment_end_date ?? METRIC_DASH}</td>
                              )}
                              {!isMobile && canEdit ? (
                                <td className="asset-actions-cell">
                                  <div className="budget-row-actions">
                                    <button
                                      type="button"
                                      className="btn ghost icon-btn"
                                      aria-label="Editar pasivo"
                                      disabled={liabilitySaving}
                                      onClick={() => beginEditLiability(row)}
                                    >
                                      <RowEditIcon />
                                    </button>
                                    <button
                                      type="button"
                                      className="btn ghost danger icon-btn"
                                      aria-label="Eliminar pasivo"
                                      disabled={liabilitySaving}
                                      onClick={() => deleteLiabilityRow(row.id)}
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
                </section>
              );
            })}
          </div>
        ) : null}
      </div>
    </div>
  );
}
