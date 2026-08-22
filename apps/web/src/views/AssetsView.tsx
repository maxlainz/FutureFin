import { useMemo, type Dispatch, type FormEvent, type SetStateAction } from "react";
import type {
  AssetApiRow,
  CategoryRow,
  InstallationAccess,
  ProjectionSeriesApi,
} from "../api/types";
import { EmptyState } from "../components/EmptyState";
import { MetricCard } from "../components/MetricCard";
import { Modal, ModalFormError } from "../components/Modal";
import { SnapshotButton } from "../components/SnapshotButton";
import { PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import { addMonthsCivil, parseYmdComponents, todayYmdInTimeZone } from "../lib/dates";
import {
  METRIC_DASH,
  assetImplicitTotalReturnLabel,
  formatCurrencyAmount,
  formatCurrencyNumber,
  formatCurrencyOrDash,
  formatPercentAmount,
  formatPercentDisplay,
  formatPercentDisplaySigned,
  parseDisplayDecimal,
} from "../lib/format";
import { formatProjectionHoverMonthYear } from "../lib/dates";
import { useIsMobile } from "../lib/responsive";
import {
  assetContributionMonthlyEstimateNum,
  assetPortfolioCostTotals,
  formatAssetContributionNominalCell,
  formatProjectionMilestoneCompactLabel,
  groupRowsByCategoryOrdered,
  type LedgerPersonScope,
  roundUpToHundred,
} from "../lib/ledger";

export function AssetsView({
  installation,
  installationBusy,
  hasMembership,
  ledgerPersonScope,
  canEdit,
  formError,
  projectionSeries,
  anchorDateYmd,
  calendarTz,
  assetModalOpen,
  closeAssetModal,
  openNewAssetModal,
  assets,
  assetsBusy,
  assetCategories,
  assetFormCategoryId,
  setAssetFormCategoryId,
  assetFormName,
  setAssetFormName,
  assetFormValue,
  setAssetFormValue,
  assetFormPurchase,
  setAssetFormPurchase,
  assetFormLiquid,
  setAssetFormLiquid,
  assetFormExpectedReturn,
  setAssetFormExpectedReturn,
  assetFormNotes,
  setAssetFormNotes,
  editingAssetId,
  assetSaving,
  submitAssetForm,
  deleteAssetRow,
  beginEditAsset,
  onSaveSnapshot,
  onOpenCategorySettings,
}: {
  installation: InstallationAccess | null;
  installationBusy: boolean;
  hasMembership: boolean;
  ledgerPersonScope: LedgerPersonScope;
  canEdit: boolean;
  formError: string | null;
  projectionSeries: ProjectionSeriesApi | null;
  anchorDateYmd: string | null;
  calendarTz: string;
  assetModalOpen: boolean;
  closeAssetModal: () => void;
  openNewAssetModal: () => void;
  assets: AssetApiRow[];
  assetsBusy: boolean;
  assetCategories: CategoryRow[];
  assetFormCategoryId: string;
  setAssetFormCategoryId: Dispatch<SetStateAction<string>>;
  assetFormName: string;
  setAssetFormName: Dispatch<SetStateAction<string>>;
  assetFormValue: string;
  setAssetFormValue: Dispatch<SetStateAction<string>>;
  assetFormPurchase: string;
  setAssetFormPurchase: Dispatch<SetStateAction<string>>;
  assetFormLiquid: boolean;
  setAssetFormLiquid: Dispatch<SetStateAction<boolean>>;
  assetFormExpectedReturn: string;
  setAssetFormExpectedReturn: Dispatch<SetStateAction<string>>;
  assetFormNotes: string;
  setAssetFormNotes: Dispatch<SetStateAction<string>>;
  editingAssetId: string | null;
  assetSaving: boolean;
  submitAssetForm: (e: FormEvent) => void;
  deleteAssetRow: (id: string) => void;
  beginEditAsset: (a: AssetApiRow) => void;
  /** Captura un snapshot de activos de hoy. `true` = guardado; lanza `Error` si falla. */
  onSaveSnapshot?: () => Promise<void>;
  /**
   * Lleva a `Ajustes → Categorías`. Única salida cuando el hogar se ha quedado sin categorías de
   * activo (un hogar nuevo nace con las por defecto, así que solo pasa si las borras todas).
   */
  onOpenCategorySettings?: () => void;
}) {
  const currency = installation?.installation.base_currency ?? METRIC_DASH;
  const currencyIso = installation?.installation.base_currency ?? "";
  const isMobile = useIsMobile();

  const assetMetricsReady = hasMembership && !assetsBusy;
  const assetsTotalVal = assetMetricsReady
    ? assets.reduce(
        (acc, a) => acc + (parseDisplayDecimal(a.current_value) ?? 0),
        0,
      )
    : null;
  const assetsLiquidVal = assetMetricsReady
    ? assets.reduce((acc, a) => {
        if (!a.is_liquid) return acc;
        return acc + (parseDisplayDecimal(a.current_value) ?? 0);
      }, 0)
    : null;
  const liquidPctParen =
    assetMetricsReady &&
    assetsTotalVal !== null &&
    assetsLiquidVal !== null &&
    assetsTotalVal > 0
      ? formatPercentDisplay((assetsLiquidVal / assetsTotalVal) * 100)
      : undefined;

  const assetCostTotals = assetMetricsReady
    ? assetPortfolioCostTotals(assets)
    : null;
  const assetPnlMoney =
    assetCostTotals !== null
      ? assetCostTotals.currentOnCost - assetCostTotals.cost
      : null;
  const assetPnlPctSigned =
    assetCostTotals !== null && assetCostTotals.cost > 0
      ? (assetCostTotals.currentOnCost / assetCostTotals.cost - 1) * 100
      : null;

  const assetTargetReachMap = useMemo<Map<string, string | null>>(() => {
    const out = new Map<string, string | null>();
    const seriesList = projectionSeries?.asset_series ?? [];
    // `values` es paralelo a `points`, y con `density=hybrid` el servidor DIEZMA la serie
    // (meses 0..12, 24, 36…): la posición 13 del array es el mes 24, no el 13. El mes real vive
    // en `points[i].month_index` — usar la posición como offset de meses adelantaba la fecha en
    // que un activo alcanza su objetivo hasta varios años.
    const pts = projectionSeries?.points ?? [];
    if (seriesList.length === 0 || pts.length === 0) return out;
    const anchorStr =
      anchorDateYmd != null && anchorDateYmd.trim() !== ""
        ? anchorDateYmd.trim()
        : todayYmdInTimeZone(calendarTz);
    const anchor = parseYmdComponents(anchorStr);
    if (!anchor) return out;
    for (const a of assets) {
      const target = parseDisplayDecimal(
        String(a.contribution_target_amount ?? ""),
      );
      if (target == null || target <= 0) continue;
      const series = seriesList.find((s) => s.asset_id === a.id);
      if (!series) continue;
      let reachedMonth: number | null = null;
      for (let i = 0; i < series.values.length; i++) {
        const v = parseDisplayDecimal(String(series.values[i] ?? ""));
        if (v != null && v >= target) {
          reachedMonth = pts[i]?.month_index ?? null;
          break;
        }
      }
      if (reachedMonth == null) {
        out.set(a.id, null);
        continue;
      }
      const at = addMonthsCivil(anchor.y, anchor.m, anchor.d, reachedMonth);
      out.set(a.id, formatProjectionHoverMonthYear(at));
    }
    return out;
  }, [
    projectionSeries?.asset_series,
    projectionSeries?.points,
    assets,
    anchorDateYmd,
    calendarTz,
  ]);

  return (
    <div className="workspace">
      <div className="workspace-header">
        <h2 className="workspace-title">Activos</h2>
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

      {/* Política de ceros: el bloque entero, no tarjeta a tarjeta. Con activos se pintan las
          tres cifras aunque alguna valga 0 €; sin ninguno la banda desaparece y hablan los
          estados vacíos de abajo (tres ceros en la portada fingirían precisión). */}
      {hasMembership && (assetsBusy || assets.length > 0) ? (
        <div className="metric-grid workspace-kpi-strip">
          <MetricCard
            label="Valor total"
            value={
              assetMetricsReady && assetsTotalVal !== null
                ? formatCurrencyNumber(assetsTotalVal, currencyIso)
                : METRIC_DASH
            }
          />
          <MetricCard
            label="Valor líquido"
            value={
              assetMetricsReady && assetsLiquidVal !== null
                ? formatCurrencyNumber(assetsLiquidVal, currencyIso)
                : METRIC_DASH
            }
            parenthetical={liquidPctParen}
          />
          <MetricCard
            label="Ganancia vs compra"
            value={
              assetMetricsReady && assetPnlMoney !== null
                ? formatCurrencyNumber(assetPnlMoney, currencyIso)
                : METRIC_DASH
            }
            parenthetical={
              assetPnlPctSigned !== null && Number.isFinite(assetPnlPctSigned)
                ? formatPercentDisplaySigned(assetPnlPctSigned)
                : undefined
            }
          />
        </div>
      ) : null}

      {!canEdit && hasMembership ? (
        <p className="muted tight">Solo lectura.</p>
      ) : null}

      {canEdit && hasMembership && assetCategories.length > 0 ? (
        <Modal
          title={editingAssetId ? "Editar activo" : "Nuevo activo"}
          open={assetModalOpen}
          onClose={closeAssetModal}
        >
          <form className="asset-form stack" onSubmit={submitAssetForm}>
            <ModalFormError message={formError} />
            <div className="asset-form-grid">
              <label className="field">
                <span>Categoría</span>
                <select
                  value={assetFormCategoryId}
                  onChange={(e) => setAssetFormCategoryId(e.target.value)}
                  required
                >
                  {assetCategories.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Nombre</span>
                <input
                  value={assetFormName}
                  onChange={(e) => setAssetFormName(e.target.value)}
                  required
                  maxLength={200}
                  placeholder="p. ej. Fondo índice"
                />
              </label>
              <label className="field">
                <span>Valor actual</span>
                <input
                  value={assetFormValue}
                  onChange={(e) => setAssetFormValue(e.target.value)}
                  required
                  inputMode="decimal"
                  placeholder="0"
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span>Precio compra (opc.)</span>
                <input
                  value={assetFormPurchase}
                  onChange={(e) => setAssetFormPurchase(e.target.value)}
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
              <label className="field checkbox-field">
                <input
                  type="checkbox"
                  checked={assetFormLiquid}
                  onChange={(e) => setAssetFormLiquid(e.target.checked)}
                />
                <span>Líquido</span>
              </label>
              <label className="field">
                <span>Rentab. anual esperada % (opc.)</span>
                <input
                  value={assetFormExpectedReturn}
                  onChange={(e) => setAssetFormExpectedReturn(e.target.value)}
                  inputMode="decimal"
                  placeholder="—"
                  autoComplete="off"
                />
              </label>
            </div>
            <label className="field">
              <span>Notas (opc.)</span>
              <textarea
                value={assetFormNotes}
                onChange={(e) => setAssetFormNotes(e.target.value)}
                rows={2}
                maxLength={4000}
              />
            </label>
            <div className="asset-form-actions">
              <button
                type="submit"
                className="btn primary"
                disabled={assetSaving}
              >
                {editingAssetId ? "Guardar cambios" : "Añadir activo"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={assetSaving}
                onClick={() => closeAssetModal()}
              >
                Cancelar
              </button>
            </div>
            {isMobile && editingAssetId ? (
              <div className="modal-mobile-delete-row">
                <button
                  type="button"
                  className="btn ghost danger"
                  disabled={assetSaving}
                  onClick={() => deleteAssetRow(editingAssetId)}
                >
                  Eliminar activo
                </button>
              </div>
            ) : null}
          </form>
        </Modal>
      ) : null}

      <div className="ledger-list-section">
        <div className="ledger-list-toolbar">
          <div className="panel-head-row">
            <h3 className="panel-title">Activos por categoría</h3>
            <div className="ledger-toolbar-actions">
              {onSaveSnapshot && canEdit && hasMembership && assets.length > 0 ? (
                <SnapshotButton
                  kind="asset"
                  disabled={assetSaving}
                  onSave={onSaveSnapshot}
                />
              ) : null}
              {/* Siempre visible mientras se pueda editar: ocultarlo sin categorías dejaba al
                  usuario sin salida. Sin categorías queda inerte y el estado vacío de abajo
                  explica por qué y lleva a crearlas. */}
              {canEdit && hasMembership ? (
                <button
                  type="button"
                  className="btn primary icon-btn ledger-toolbar-add"
                  aria-label="Nuevo activo"
                  title={
                    assetCategories.length === 0
                      ? "Necesitas una categoría de activo antes de crear uno"
                      : "Nuevo activo"
                  }
                  disabled={assetCategories.length === 0}
                  onClick={() => openNewAssetModal()}
                >
                  <PlusIcon />
                </button>
              ) : null}
            </div>
          </div>
          {assetsBusy ? <p className="muted">Cargando…</p> : null}
        </div>
        {/* `formError` gatea el estado vacío: cuando el loader falla vacía lista Y categorías, y
            sin este gate la vista acusaba al usuario de no tener categorías («no queda ninguna»)
            justo cuando el problema era que no se había podido leer nada. El error ya se pinta
            en la banda global de App.tsx; aquí basta con no contradecirlo. */}
        {!assetsBusy && !formError && hasMembership && assets.length === 0 ? (
          assetCategories.length === 0 ? (
            <EmptyState
              title="Falta una categoría de activo"
              description="Cada activo se guarda dentro de una categoría (cuentas, fondos, inmuebles). No queda ninguna, así que créala antes de registrar nada."
              actionLabel={canEdit ? "Crear categorías" : undefined}
              onAction={canEdit ? onOpenCategorySettings : undefined}
            />
          ) : (
            <EmptyState
              title="Sin activos"
              description="Aquí anotas lo que tienes: cuentas, fondos, tu casa, el coche. Con su valor FutureFin calcula tu patrimonio y proyecta cómo crece."
              actionLabel={canEdit ? "Añadir activo" : undefined}
              onAction={canEdit ? openNewAssetModal : undefined}
            />
          )
        ) : null}
        {!assetsBusy && assets.length > 0 ? (
          <div className="ledger-by-category-stack">
            {groupRowsByCategoryOrdered(assets, assetCategories, {
              sortRowsDescending: {
                value: (a) => parseDisplayDecimal(a.current_value) ?? 0,
                tieBreak: (a, b) => a.name.localeCompare(b.name, "es"),
              },
              categoryTotalDescending: (items) =>
                items.reduce(
                  (acc, a) => acc + (parseDisplayDecimal(a.current_value) ?? 0),
                  0,
                ),
            }).map((g) => {
              const catTotalVal = g.items.reduce(
                (acc, a) => acc + (parseDisplayDecimal(a.current_value) ?? 0),
                0,
              );
              const showPurchase = g.items.some((a) => {
                const v = parseDisplayDecimal(String(a.purchase_price ?? ""));
                return v != null && v > 0;
              });
              const showReturn = g.items.some(
                (a) =>
                  a.expected_annual_return_percent != null &&
                  String(a.expected_annual_return_percent).trim() !== "",
              );
              const showContribution = g.items.some(
                (a) => assetContributionMonthlyEstimateNum(a) > 0,
              );
              return (
                <section key={g.categoryId} className="panel ledger-category-panel">
                  <div className="panel-head-row">
                    <h3 className="panel-title">{g.label}</h3>
                    <span className="ledger-category-total">
                      {formatCurrencyNumber(catTotalVal, currencyIso)}
                    </span>
                  </div>
                  <div className="table-scroll bordered-top">
                    <table className="assets-table">
                      <thead>
                        <tr>
                          <th>Nombre</th>
                          <th
                            className="num"
                            title="Valor actual. Cuando una regla de asignación apunta a este activo con un tope en € concreto, se muestra como Actual / Objetivo."
                          >
                            Valor
                          </th>
                          {!isMobile && showPurchase ? (
                            <th className="num">Compra</th>
                          ) : null}
                          {!isMobile && showPurchase ? (
                            <th
                              className="num"
                              title="Variación vs precio de compra (no anualizada)"
                            >
                              Δ compra
                            </th>
                          ) : null}
                          {!isMobile && showReturn ? (
                            <th
                              className="num"
                              title="Rentabilidad anual esperada (proyección)"
                            >
                              Rent. % a.a.
                            </th>
                          ) : null}
                          {!isMobile && showContribution ? (
                            <th
                              className="num"
                              title="Aporte estimado del primer mes (suma de reglas de Presupuesto → Asignación del sobrante que apuntan a este activo). Incluye flujos puntuales de Próximos. Cero si todas las reglas anteriores agotan el sobrante antes de llegar a este activo."
                            >
                              Aporte
                            </th>
                          ) : null}
                          {!isMobile && canEdit ? (
                            <th className="asset-actions-cell">
                              <span className="sr-only">Acciones</span>
                            </th>
                          ) : null}
                        </tr>
                      </thead>
                      <tbody>
                        {g.items.map((a) => {
                          const target = parseDisplayDecimal(
                            String(a.contribution_target_amount ?? ""),
                          );
                          const currentVal = parseDisplayDecimal(a.current_value);
                          const targetMet =
                            target != null &&
                            currentVal != null &&
                            currentVal >= target;
                          const targetCompact =
                            target != null && target > 0 && !targetMet
                              ? formatProjectionMilestoneCompactLabel(
                                  String(roundUpToHundred(target)),
                                )
                              : null;
                          const targetReachLabel =
                            assetTargetReachMap.get(a.id) ?? null;
                          const rowTappable = isMobile && canEdit;
                          const subParts: string[] = [];
                          if (showPurchase) {
                            subParts.push(
                              `Compra ${formatCurrencyOrDash(a.purchase_price, currencyIso)}`,
                            );
                            subParts.push(
                              `Δ ${
                                assetImplicitTotalReturnLabel(
                                  a.current_value,
                                  a.purchase_price,
                                ) ?? METRIC_DASH
                              }`,
                            );
                          }
                          if (showReturn) {
                            subParts.push(
                              `Rent. ${
                                a.expected_annual_return_percent != null &&
                                a.expected_annual_return_percent !== ""
                                  ? formatPercentAmount(
                                      a.expected_annual_return_percent,
                                    )
                                  : METRIC_DASH
                              }`,
                            );
                          }
                          if (showContribution) {
                            subParts.push(
                              `Aporte ${formatAssetContributionNominalCell(a, currencyIso)}`,
                            );
                          }
                          return (
                            <tr
                              key={a.id}
                              className={rowTappable ? "row-tappable" : undefined}
                              role={rowTappable ? "button" : undefined}
                              tabIndex={rowTappable ? 0 : undefined}
                              onClick={rowTappable ? () => beginEditAsset(a) : undefined}
                              onKeyDown={
                                rowTappable
                                  ? (e) => {
                                      if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        beginEditAsset(a);
                                      }
                                    }
                                  : undefined
                              }
                            >
                              <td>
                                {a.name}
                                {isMobile && subParts.length > 0 ? (
                                  <span className="cell-subline">
                                    {subParts.join(" · ")}
                                  </span>
                                ) : null}
                              </td>
                              <td className="num">
                                {targetCompact ? (
                                  <span
                                    className="asset-target-tag"
                                    title={
                                      targetReachLabel
                                        ? `Objetivo alcanzado en ${targetReachLabel}`
                                        : undefined
                                    }
                                  >
                                    (Obj. {targetCompact}){" "}
                                  </span>
                                ) : null}
                                {formatCurrencyAmount(a.current_value, currencyIso)}
                                {rowTappable ? (
                                  <span className="row-chevron" aria-hidden>
                                    ›
                                  </span>
                                ) : null}
                              </td>
                              {!isMobile && showPurchase ? (
                                <td className="num">
                                  {formatCurrencyOrDash(a.purchase_price, currencyIso)}
                                </td>
                              ) : null}
                              {!isMobile && showPurchase ? (
                                <td className="num muted">
                                  {assetImplicitTotalReturnLabel(
                                    a.current_value,
                                    a.purchase_price,
                                  ) ?? METRIC_DASH}
                                </td>
                              ) : null}
                              {!isMobile && showReturn ? (
                                <td className="num muted">
                                  {a.expected_annual_return_percent != null &&
                                  a.expected_annual_return_percent !== ""
                                    ? formatPercentAmount(
                                        a.expected_annual_return_percent,
                                      )
                                    : METRIC_DASH}
                                </td>
                              ) : null}
                              {!isMobile && showContribution ? (
                                <td className="num muted tight">
                                  {formatAssetContributionNominalCell(a, currencyIso)}
                                </td>
                              ) : null}
                              {!isMobile && canEdit ? (
                                <td className="asset-actions-cell">
                                  <div className="budget-row-actions">
                                    <button
                                      type="button"
                                      className="btn ghost icon-btn"
                                      aria-label="Editar activo"
                                      disabled={assetSaving}
                                      onClick={() => beginEditAsset(a)}
                                    >
                                      <RowEditIcon />
                                    </button>
                                    <button
                                      type="button"
                                      className="btn ghost danger icon-btn"
                                      aria-label="Eliminar activo"
                                      disabled={assetSaving}
                                      onClick={() => deleteAssetRow(a.id)}
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
