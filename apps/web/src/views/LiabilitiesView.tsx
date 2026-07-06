import type { Dispatch, FormEvent, SetStateAction } from "react";
import type {
  CategoryRow,
  InstallationAccess,
  LiabilityApiRow,
} from "../api/types";
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
  type LedgerPersonScope,
  type LiabilityPaymentFreq,
  groupRowsByCategoryOrdered,
  liabilitiesApproxMonthlyInterestSum,
  liabilitiesWeightedAprPercent,
  liabilityDerivedPrincipalPreview,
  liabilityPaymentMonthlyEquivalentNum,
} from "../lib/ledger";

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
  liabilityFormCategoryId,
  setLiabilityFormCategoryId,
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
  editingLiabilityId,
  liabilitySaving,
  submitLiabilityForm,
  deleteLiabilityRow,
  beginEditLiability,
  onSaveSnapshot,
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
  liabilityFormCategoryId: string;
  setLiabilityFormCategoryId: Dispatch<SetStateAction<string>>;
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
  editingLiabilityId: string | null;
  liabilitySaving: boolean;
  submitLiabilityForm: (e: FormEvent) => void;
  deleteLiabilityRow: (id: string) => void;
  beginEditLiability: (row: LiabilityApiRow) => void;
  /** Captura un snapshot de pasivos de hoy. `true` = guardado; lanza `Error` si falla. */
  onSaveSnapshot?: () => Promise<void>;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";

  const derivePreview = liabilityDerivedPrincipalPreview(
    liabilityFormPaymentAmount,
    liabilityFormPaymentFrequency,
    liabilityFormPaymentEnd,
    installation?.installation.calendar_tz ?? "UTC",
    currencyIso,
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

      {hasMembership ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Principal total"
            value={
              liabilityMetricsReady && liabilityPrincipalSum !== null
                ? formatCurrencyNumber(liabilityPrincipalSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Servicio mensual equivalente"
            value={
              liabilityMetricsReady && liabilitiesMonthlyServiceSum !== null
                ? formatCurrencyNumber(liabilitiesMonthlyServiceSum, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="TAE media ponderada"
            value={
              liabilityMetricsReady && liabilitiesWeightedApr !== null
                ? formatPercentDisplay(liabilitiesWeightedApr)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Interés mensual aprox."
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

      {hasMembership && liabilityCategories.length === 0 && !liabilitiesBusy ? (
        <div className="banner info-banner">
          <strong>Pasivos</strong> · <strong>Ajustes → Categorías</strong>
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
                <span>Tipo (opc.)</span>
                <input
                  value={liabilityFormTypeTag}
                  onChange={(e) => setLiabilityFormTypeTag(e.target.value)}
                  maxLength={120}
                  placeholder="Etiqueta libre"
                />
              </label>
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
              {canEdit && hasMembership && liabilityCategories.length > 0 ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nuevo pasivo"
                  onClick={() => openNewLiabilityModal()}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
          </div>
          {liabilitiesBusy ? (
            <p className="muted">Cargando…</p>
          ) : liabilities.length === 0 ? (
            <p className="muted">No hay pasivos en esta instalación.</p>
          ) : null}
        </div>
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
                          <th className="num">TAE %</th>
                          <th className="num">Cuota</th>
                          <th>Frec.</th>
                          <th>Fin plan</th>
                          {canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((row) => (
                          <tr key={row.id}>
                            <td>{row.label}</td>
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
                            </td>
                            <td className="num">
                              {row.apr_percent != null &&
                              String(row.apr_percent).trim() !== ""
                                ? formatPercentAmount(row.apr_percent)
                                : METRIC_DASH}
                            </td>
                            <td className="num">
                              {formatCurrencyOrDash(
                                row.payment_amount,
                                currencyIso,
                              )}
                            </td>
                            <td>
                              {row.payment_frequency
                                ? PAYMENT_FREQ_LABEL[row.payment_frequency]
                                : METRIC_DASH}
                            </td>
                            <td>{row.payment_end_date ?? METRIC_DASH}</td>
                            {canEdit ? (
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
                        ))}
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
