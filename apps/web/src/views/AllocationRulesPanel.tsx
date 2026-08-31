import type {
  AllocationRuleApiRow,
  AllocationRuleCapKind,
  AllocationRuleKind,
  AssetApiRow,
} from "../api/types";
import { PlusIcon, RowEditIcon, RowTrashIcon } from "../components/icons";
import {
  formatCurrencyNumber,
  formatPercentAmount,
  formatPercentDisplay,
  parseDisplayDecimal,
} from "../lib/format";
import { useIsMobile } from "../lib/responsive";

function formatAllocationCap(
  capKind: AllocationRuleCapKind | null | undefined,
  capValue: string | null | undefined,
  currencyIso: string,
): string {
  if (!capKind || capValue == null) return "—";
  const n = parseDisplayDecimal(String(capValue));
  if (n == null) return "—";
  switch (capKind) {
    case "amount":
      return formatCurrencyNumber(n, currencyIso);
    case "months_expense":
      return `${formatPercentAmount(n.toString()).replace(" %", "")} × gasto`;
    case "income_multiple":
      return `${formatPercentAmount(n.toString()).replace(" %", "")} × ingreso`;
    default:
      return "—";
  }
}

function formatAllocationAmount(
  kind: AllocationRuleKind,
  amount: string | null | undefined,
  currencyIso: string,
): string {
  if (kind === "remainder") return "Resto";
  if (amount == null) return "—";
  const n = parseDisplayDecimal(String(amount));
  if (n == null) return "—";
  if (kind === "fixed") return formatCurrencyNumber(n, currencyIso);
  if (kind === "percent") return formatPercentDisplay(n);
  return "—";
}

export function AllocationRulesPanel({
  assets,
  rules,
  busy,
  error,
  canEdit,
  currencyIso,
  openNewRuleModal,
  beginEditRule,
  deleteRule,
  moveRule,
  embedded = false,
}: {
  assets: AssetApiRow[];
  rules: AllocationRuleApiRow[];
  busy: boolean;
  error: string | null;
  canEdit: boolean;
  currencyIso: string;
  openNewRuleModal: () => void;
  beginEditRule: (r: AllocationRuleApiRow) => void;
  deleteRule: (id: string) => void;
  moveRule: (id: string, dir: "up" | "down") => void;
  embedded?: boolean;
}) {
  const isMobile = useIsMobile();
  const assetById = new Map(assets.map((a) => [a.id, a.name]));
  const sinkIndex = rules.findIndex(
    (r) => r.kind === "remainder" && !r.cap_kind,
  );
  const hasSink = sinkIndex >= 0;

  return (
    <section
      className={
        embedded
          ? "allocation-rules-panel allocation-rules-panel--embedded"
          : "panel allocation-rules-panel"
      }
    >
      {embedded ? (
        canEdit ? (
          <div className="panel-head-row">
            <span />
            <button
              type="button"
              className="btn primary icon-btn"
              aria-label="Nueva regla"
              onClick={openNewRuleModal}
              disabled={assets.length === 0}
            >
              <PlusIcon />
            </button>
          </div>
        ) : null
      ) : (
        <div className="panel-head-row">
          <h3 className="panel-title">Asignación del sobrante</h3>
          {canEdit ? (
            <button
              type="button"
              className="btn primary icon-btn"
              aria-label="Nueva regla"
              onClick={openNewRuleModal}
              disabled={assets.length === 0}
            >
              <PlusIcon />
            </button>
          ) : null}
        </div>
      )}
      <p className="muted tight">
        Cascada en orden ascendente. Cada mes, sobre el sobrante (ingresos −
        gastos − cuotas de deuda + flujos puntuales de Próximos), cada regla
        coge su parte y lo que queda baja a la siguiente:
      </p>
      <ul className="muted tight allocation-rules-help">
        <li>
          <strong>Fija (€)</strong>: aporta exactamente esa cantidad si hay
          sobrante disponible.
        </li>
        <li>
          <strong>%</strong>: aporta ese porcentaje del sobrante que queda
          <em> en ese paso de la cascada</em> (no del sobrante total inicial).
        </li>
        <li>
          <strong>Resto</strong>: absorbe lo que quede después de las anteriores.
          La regla resto <em>sin tope</em> es única por usuario y siempre va al
          final. Puedes poner varias reglas resto <em>con tope</em> antes (p.ej.
          "fondo emergencia hasta 3 meses de gasto") y la cascada saltará la
          regla cuando se llene.
        </li>
      </ul>
      {error ? <p className="form-error">{error}</p> : null}
      {assets.length === 0 ? (
        <p className="muted">Crea activos primero para poder asignar reglas.</p>
      ) : busy ? (
        <p className="muted">Cargando…</p>
      ) : rules.length === 0 ? (
        <p className="muted">
          Sin reglas todavía. Crea tu primer activo y la regla «resto» nace
          sola con él.
        </p>
      ) : (
        <>
          {!hasSink ? (
            // 4.12.1: estado residual (no debería verse — con activos, el sumidero es
            // indestructible). Si aparece, el ahorro NO se está simulando: dilo sin rodeos.
            <div className="banner error-banner">
              Falta la regla <strong>Resto sin tope</strong>: tu ahorro mensual
              no tiene destino y la proyección NO lo está contando. Añádela.
            </div>
          ) : hasSink && sinkIndex !== rules.length - 1 ? (
            <div className="banner info-banner">
              La regla <strong>Resto sin tope</strong> debe ser la última. Las
              reglas posteriores (#{sinkIndex + 2}…) recibirán 0&nbsp;€.
            </div>
          ) : null}
          <div className="table-scroll bordered-top">
            <table className="assets-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Destino</th>
                  {isMobile ? null : <th>Tipo</th>}
                  <th className="num">Cantidad</th>
                  {isMobile ? null : <th className="num">Tope</th>}
                  {!isMobile && canEdit ? (
                    <th className="asset-actions-cell">
                      <span className="sr-only">Acciones</span>
                    </th>
                  ) : null}
                </tr>
              </thead>
              <tbody>
                {rules.map((r, i) => {
                  const rowTappable = isMobile && canEdit;
                  const typeLabel =
                    r.kind === "fixed"
                      ? "Fija"
                      : r.kind === "percent"
                        ? "Porcentaje"
                        : "Resto";
                  const capLabel = formatAllocationCap(
                    r.cap_kind,
                    r.cap_value,
                    currencyIso,
                  );
                  return (
                    <tr
                      key={r.id}
                      className={rowTappable ? "row-tappable" : undefined}
                      role={rowTappable ? "button" : undefined}
                      tabIndex={rowTappable ? 0 : undefined}
                      onClick={rowTappable ? () => beginEditRule(r) : undefined}
                      onKeyDown={
                        rowTappable
                          ? (e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                beginEditRule(r);
                              }
                            }
                          : undefined
                      }
                    >
                      <td>{i + 1}</td>
                      <td>
                        {assetById.get(r.target_asset_id) ?? "—"}
                        {isMobile ? (
                          <span className="cell-subline">
                            {typeLabel} · Tope {capLabel}
                          </span>
                        ) : null}
                      </td>
                      {isMobile ? null : <td>{typeLabel}</td>}
                      <td className="num">
                        {formatAllocationAmount(r.kind, r.amount, currencyIso)}
                        {rowTappable ? (
                          <span className="row-chevron" aria-hidden>
                            ›
                          </span>
                        ) : null}
                      </td>
                      {isMobile ? null : (
                        <td className="num muted">{capLabel}</td>
                      )}
                      {!isMobile && canEdit ? (
                        <td className="asset-actions-cell">
                          <div className="budget-row-actions">
                            <button
                              type="button"
                              className="btn ghost icon-btn"
                              aria-label="Subir prioridad"
                              disabled={i === 0}
                              onClick={() => moveRule(r.id, "up")}
                            >
                              ▲
                            </button>
                            <button
                              type="button"
                              className="btn ghost icon-btn"
                              aria-label="Bajar prioridad"
                              disabled={i === rules.length - 1}
                              onClick={() => moveRule(r.id, "down")}
                            >
                              ▼
                            </button>
                            <button
                              type="button"
                              className="btn ghost icon-btn"
                              aria-label="Editar regla"
                              onClick={() => beginEditRule(r)}
                            >
                              <RowEditIcon />
                            </button>
                            <button
                              type="button"
                              className="btn ghost danger icon-btn"
                              aria-label="Eliminar regla"
                              onClick={() => deleteRule(r.id)}
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
        </>
      )}
    </section>
  );
}
