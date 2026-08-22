/**
 * Asistente de primera vez (3.10.0).
 *
 * Por qué existe: hasta ahora, quien acababa de instalar FutureFin aterrizaba en un Resumen en
 * blanco, con la zona horaria en `UTC` (así que «hoy» podía no ser hoy), la divisa clavada en
 * euros sin forma de cambiarla, la inflación a 0 —lo que hacía que la pestaña Jubilación abriera
 * con un aviso de advertencia antes de que hubiera metido un solo dato— y ninguna pista sobre
 * por dónde empezar.
 *
 * El asistente no configura la app entera: pide lo mínimo que la app **no puede adivinar** y que,
 * si se queda mal, hace que todas las cifras salgan raras. Todo lo demás se toca luego en Ajustes.
 *
 * Es saltable a propósito. Quien ya sabe lo que hace no debería tener que pasar por aquí, y quien
 * lo salta puede volver a abrirlo desde Ajustes → General.
 */

import { useState } from "react";
import { apiPatch, apiPost } from "../api/client";
import type { CategoryRow, FireSettingsApi, InstallationSnapshot } from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { toApiDecimalString } from "../lib/format";

/** Divisas que acepta el backend (`normalize_currency`). Una sola por instalación: FutureFin no
 *  convierte ni mezcla divisas, así que esto se elige una vez y define toda la contabilidad. */
const CURRENCIES: { code: string; label: string }[] = [
  { code: "EUR", label: "Euro (€)" },
  { code: "USD", label: "Dólar estadounidense ($)" },
  { code: "GBP", label: "Libra esterlina (£)" },
];

/** Zona horaria del navegador, que acierta en la inmensa mayoría de los casos. El backend por
 *  defecto ponía `UTC`, y con eso «el gasto de hoy» podía caer en el día equivocado. */
function browserTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "Europe/Madrid";
  } catch {
    return "Europe/Madrid";
  }
}

type Step = 1 | 2 | 3 | 4;

export function OnboardingWizard({
  open,
  installation,
  assetCategories,
  onFinished,
  onSkip,
}: {
  open: boolean;
  installation: InstallationSnapshot;
  assetCategories: CategoryRow[];
  /** Se llama tras marcar el hogar como configurado: App recarga instalación y ledger. */
  onFinished: () => void;
  /** Cierra sin marcar nada. El asistente volverá a salir en la próxima carga. */
  onSkip: () => void;
}) {
  const [step, setStep] = useState<Step>(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [currency, setCurrency] = useState(installation.base_currency || "EUR");
  const [tz, setTz] = useState(
    installation.calendar_tz && installation.calendar_tz !== "UTC"
      ? installation.calendar_tz
      : browserTimeZone(),
  );

  const [inflation, setInflation] = useState("2,5");
  const [swrPct, setSwrPct] = useState("3,5");

  const [assetCategoryId, setAssetCategoryId] = useState(assetCategories[0]?.id ?? "");
  const [assetName, setAssetName] = useState("");
  const [assetValue, setAssetValue] = useState("");
  const [assetLiquid, setAssetLiquid] = useState(true);

  if (!open) return null;

  async function run(fn: () => Promise<void>, next: Step | "done") {
    setBusy(true);
    setError(null);
    try {
      await fn();
      if (next === "done") {
        await apiPatch("/v1/installation", { onboarding_completed: true });
        onFinished();
      } else {
        setStep(next);
      }
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "No se ha podido guardar.");
    } finally {
      setBusy(false);
    }
  }

  const saveBasics = () =>
    run(async () => {
      await apiPatch("/v1/installation", {
        base_currency: currency,
        calendar_tz: tz.trim(),
      });
    }, 2);

  const savePlan = () =>
    run(async () => {
      const fire: FireSettingsApi = {
        ...(installation.fire_settings as FireSettingsApi),
        swr_pct: toApiDecimalString(swrPct),
      };
      await apiPatch("/v1/installation", {
        annual_inflation_assumption_percent: toApiDecimalString(inflation),
        fire_settings: fire,
      });
    }, 3);

  const saveFirstAsset = () =>
    run(async () => {
      // Paso opcional: sin nombre o sin valor se pasa de largo sin crear nada.
      if (!assetCategoryId || !assetName.trim() || !assetValue.trim()) return;
      await apiPost("/v1/assets", {
        category_id: assetCategoryId,
        name: assetName.trim(),
        current_value: toApiDecimalString(assetValue),
        is_liquid: assetLiquid,
      });
    }, 4);

  return (
    <Modal title="Bienvenido a FutureFin" open={open} onClose={onSkip} wide>
      <div className="onboarding-wizard">
        <ol className="onboarding-steps" aria-label="Pasos de la configuración inicial">
          {(["Tu hogar", "Tu plan", "Primer activo", "Listo"] as const).map((label, i) => (
            <li
              key={label}
              className={`onboarding-step ${step === i + 1 ? "is-active" : ""} ${
                step > i + 1 ? "is-done" : ""
              }`}
              aria-current={step === i + 1 ? "step" : undefined}
            >
              <span className="onboarding-step-num">{i + 1}</span>
              {label}
            </li>
          ))}
        </ol>

        <ModalFormError message={error} />

        {step === 1 ? (
          <section>
            <p>
              Dos datos que FutureFin no puede adivinar y que afectan a todas las cifras: en qué
              divisa llevas tus cuentas y en qué zona horaria vives.
            </p>
            <label className="field">
              <span>Divisa</span>
              <select value={currency} onChange={(e) => setCurrency(e.target.value)}>
                {CURRENCIES.map((c) => (
                  <option key={c.code} value={c.code}>
                    {c.label}
                  </option>
                ))}
              </select>
              <small className="muted">
                Una sola por hogar: FutureFin no convierte entre divisas. Podrás cambiarla luego,
                pero los importes ya guardados no se reconvierten.
              </small>
            </label>
            <label className="field">
              <span>Zona horaria</span>
              <input
                value={tz}
                onChange={(e) => setTz(e.target.value)}
                placeholder="Europe/Madrid"
                autoComplete="off"
              />
              <small className="muted">
                Define qué día es «hoy» al calcular cuotas y movimientos. La hemos rellenado con la
                de tu navegador.
              </small>
            </label>
            <div className="asset-form-actions">
              <button type="button" className="btn ghost" onClick={onSkip} disabled={busy}>
                Lo configuro luego
              </button>
              <button type="button" className="btn primary" onClick={() => void saveBasics()} disabled={busy}>
                Continuar
              </button>
            </div>
          </section>
        ) : null}

        {step === 2 ? (
          <section>
            <p>
              FutureFin proyecta tu patrimonio a futuro y calcula cuándo podrías dejar de depender
              de tu sueldo. Para eso necesita dos supuestos. Los dos se cambian cuando quieras.
            </p>
            <label className="field">
              <span>Inflación anual estimada (%)</span>
              <input
                value={inflation}
                onChange={(e) => setInflation(e.target.value)}
                inputMode="decimal"
                placeholder="2,5"
                autoComplete="off"
              />
              <small className="muted">
                Hace que tu objetivo crezca con el tiempo, para que refleje lo que costará vivir
                entonces y no lo que cuesta hoy. Con 0 el objetivo se queda plano.
              </small>
            </label>
            <label className="field">
              <span>Tasa de retirada segura (%)</span>
              <input
                value={swrPct}
                onChange={(e) => setSwrPct(e.target.value)}
                inputMode="decimal"
                placeholder="3,5"
                autoComplete="off"
              />
              <small className="muted">
                Qué porcentaje de tu patrimonio podrías gastar al año sin agotarlo. 3,5 % es un
                punto de partida prudente.
              </small>
            </label>
            <div className="asset-form-actions">
              <button type="button" className="btn ghost" onClick={() => setStep(1)} disabled={busy}>
                Atrás
              </button>
              <button type="button" className="btn primary" onClick={() => void savePlan()} disabled={busy}>
                Continuar
              </button>
            </div>
          </section>
        ) : null}

        {step === 3 ? (
          <section>
            <p>
              Añade tu primera cuenta o inversión para que el Resumen deje de estar vacío. Puedes
              saltarte este paso y hacerlo luego desde la pestaña Activos.
            </p>
            {assetCategories.length === 0 ? (
              <p className="muted">
                No hay categorías de activo. Créalas en Ajustes → Categorías y vuelve por aquí.
              </p>
            ) : (
              <>
                <label className="field">
                  <span>Categoría</span>
                  <select
                    value={assetCategoryId}
                    onChange={(e) => setAssetCategoryId(e.target.value)}
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
                    value={assetName}
                    onChange={(e) => setAssetName(e.target.value)}
                    placeholder="Cuenta corriente"
                    autoComplete="off"
                  />
                </label>
                <label className="field">
                  <span>Valor actual</span>
                  <input
                    value={assetValue}
                    onChange={(e) => setAssetValue(e.target.value)}
                    inputMode="decimal"
                    placeholder="1500"
                    autoComplete="off"
                  />
                </label>
                <label className="field field--checkbox">
                  <input
                    type="checkbox"
                    checked={assetLiquid}
                    onChange={(e) => setAssetLiquid(e.target.checked)}
                  />
                  <span>Es dinero disponible (puedo gastarlo sin vender nada)</span>
                </label>
              </>
            )}
            <div className="asset-form-actions">
              <button type="button" className="btn ghost" onClick={() => setStep(2)} disabled={busy}>
                Atrás
              </button>
              <button type="button" className="btn primary" onClick={() => void saveFirstAsset()} disabled={busy}>
                {assetName.trim() && assetValue.trim() ? "Añadir y continuar" : "Saltar este paso"}
              </button>
            </div>
          </section>
        ) : null}

        {step === 4 ? (
          <section>
            <p>Ya está. Esto es lo que tienes por delante:</p>
            <ul className="onboarding-summary">
              <li>
                <strong>Activos</strong> y <strong>Pasivos</strong>: lo que tienes y lo que debes.
                De ahí sale tu patrimonio neto.
              </li>
              <li>
                <strong>Presupuesto</strong>: tus ingresos y gastos de cada mes. Es lo que la
                proyección da por hecho que se repite.
              </li>
              <li>
                <strong>Movimientos</strong>: lo que ha pasado de verdad. Puedes importar el CSV de
                tu banco.
              </li>
              <li>
                <strong>Jubilación</strong> y <strong>Proyección</strong>: cuándo llegas a tu
                objetivo con los datos de arriba.
              </li>
            </ul>
            <p className="muted">
              Tu hogar ya tiene un juego de categorías para empezar. Cámbialas, bórralas o añade
              las tuyas en Ajustes → Categorías.
            </p>
            <div className="asset-form-actions">
              <button type="button" className="btn primary" onClick={() => void run(async () => {}, "done")} disabled={busy}>
                Empezar
              </button>
            </div>
          </section>
        ) : null}
      </div>
    </Modal>
  );
}
