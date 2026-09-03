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
import type {
  CategoryRow,
  InstallationSnapshot,
  RetirementProfileApi,
  RetirementProfilePatchApi,
} from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { toApiDecimalString } from "../lib/format";
import {
  buildOnboardingPlanPatch,
  emptyOnboardingPlanState,
  onboardingPlanFields,
  strategyNeedsBirthDate,
  validateOnboardingPlan,
  type OnboardingPlanFieldKey,
  type OnboardingPlanState,
} from "../lib/onboarding-plan";
import {
  RETIREMENT_STRATEGIES,
  RETIREMENT_STRATEGY_BLURB,
  RETIREMENT_STRATEGY_LABEL,
} from "../lib/retirementProfile";

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
  userBirthDate,
  onSaveRetirementProfile,
  onFinished,
  onSkip,
}: {
  open: boolean;
  installation: InstallationSnapshot;
  assetCategories: CategoryRow[];
  /** `user.birth_date` de la sesión (`App.tsx`): si ya existe, el paso «Tu plan» la trae
   *  precargada en vez de pedirla otra vez. */
  userBirthDate: string | null;
  /** Mismo helper que usa Jubilación (`saveRetirementProfilePatch` en `App.tsx`): un PATCH
   *  mínimo a `/v1/auth/me/retirement-profile` que además actualiza el `retirementProfile` en
   *  memoria y recarga la proyección — así Resumen/Jubilación no se quedan enseñando el plan
   *  por defecto hasta el siguiente login. */
  onSaveRetirementProfile: (
    patch: RetirementProfilePatchApi,
  ) => Promise<RetirementProfileApi | null>;
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

  // U8: el paso «Tu plan» ya no pregunta inflación ni SWR — se quedan en su default (2,5 % / 3,5 %)
  // y se cambian luego desde Ajustes → Plan / Jubilación. Lo que pide es lo esencial para tener un
  // plan: fecha de nacimiento, estrategia y los campos que esa estrategia exige (`onboarding-plan.ts`).
  const [planState, setPlanState] = useState<OnboardingPlanState>(() => ({
    ...emptyOnboardingPlanState(),
    birthDate: userBirthDate ?? "",
  }));
  const planEssentials = onboardingPlanFields(planState.strategy);
  const planIssues = validateOnboardingPlan(planState);
  const planIssueFor = (field: OnboardingPlanFieldKey): string | null =>
    planIssues.find((i) => i.field === field)?.message ?? null;
  const planLabelFor = (id: (typeof planEssentials)[number]["id"]): string =>
    planEssentials.find((f) => f.id === id)?.label ?? "";

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
      // Un único PATCH mínimo (S12): fecha de nacimiento + estrategia + solo los esenciales que
      // esa estrategia exige (`buildOnboardingPlanPatch`). El SWR, la inflación, la regla de
      // retirada y el resto de supuestos se quedan en su default — se afinan luego desde
      // Ajustes → Plan / Jubilación, no aquí.
      await onSaveRetirementProfile(buildOnboardingPlanPatch(planState));
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
              Con tu fecha de nacimiento y cómo quieres jubilarte, FutureFin ya puede calcular tu
              plan. Todo lo demás —tasa de retirada, regla de retirada, pensión indexada…— se
              afina luego desde Jubilación; aquí basta con lo esencial.
            </p>
            <label className="field">
              <span>
                Fecha de nacimiento
                {strategyNeedsBirthDate(planState.strategy) ? "" : " (opcional)"}
              </span>
              <input
                type="date"
                value={planState.birthDate}
                onChange={(e) =>
                  setPlanState((s) => ({ ...s, birthDate: e.target.value }))
                }
              />
              <small className="muted">
                {planIssueFor("birthDate") ??
                  (strategyNeedsBirthDate(planState.strategy)
                    ? "Esta estrategia se dispara por edad: sin fecha de nacimiento no se puede simular tal y como la has elegido."
                    : "Se usa para mostrar tu edad en vez del mes en Jubilación. Puedes añadirla más tarde desde «Tu cuenta».")}
              </small>
            </label>

            <div
              className="retirement-mode-grid retirement-strategy-grid"
              role="radiogroup"
              aria-label="Estrategia de jubilación"
            >
              {RETIREMENT_STRATEGIES.map((s) => (
                <label
                  key={s}
                  className={`retirement-mode-card ${planState.strategy === s ? "is-active" : ""}`}
                >
                  <input
                    type="radio"
                    name="onboarding_retirement_strategy"
                    className="sr-only"
                    checked={planState.strategy === s}
                    onChange={() => setPlanState((prev) => ({ ...prev, strategy: s }))}
                  />
                  <span className="retirement-mode-name">{RETIREMENT_STRATEGY_LABEL[s]}</span>
                  <span className="retirement-mode-sub">{RETIREMENT_STRATEGY_BLURB[s]}</span>
                </label>
              ))}
            </div>

            {planState.strategy === "retire_at_age" || planState.strategy === "coast" ? (
              <label className="field">
                <span>{planLabelFor("target_retirement_age")}</span>
                <input
                  inputMode="numeric"
                  value={planState.targetRetirementAge}
                  placeholder="p. ej. 60"
                  autoComplete="off"
                  onChange={(e) =>
                    setPlanState((s) => ({ ...s, targetRetirementAge: e.target.value }))
                  }
                />
                {planIssueFor("targetRetirementAge") ? (
                  <small className="muted">{planIssueFor("targetRetirementAge")}</small>
                ) : null}
              </label>
            ) : null}

            {planState.strategy === "partial" ? (
              <div className="field-row">
                <label className="field">
                  <span>{planLabelFor("partial_start_age")}</span>
                  <input
                    inputMode="numeric"
                    value={planState.partialStartAge}
                    placeholder="p. ej. 55"
                    autoComplete="off"
                    onChange={(e) =>
                      setPlanState((s) => ({ ...s, partialStartAge: e.target.value }))
                    }
                  />
                  {planIssueFor("partialStartAge") ? (
                    <small className="muted">{planIssueFor("partialStartAge")}</small>
                  ) : null}
                </label>
                <label className="field">
                  <span>{planLabelFor("partial_income")}</span>
                  <input
                    inputMode="decimal"
                    value={planState.partialIncome}
                    placeholder="p. ej. 800"
                    autoComplete="off"
                    onChange={(e) =>
                      setPlanState((s) => ({ ...s, partialIncome: e.target.value }))
                    }
                  />
                  {planIssueFor("partialIncome") ? (
                    <small className="muted">{planIssueFor("partialIncome")}</small>
                  ) : null}
                </label>
              </div>
            ) : null}

            {planState.strategy === "pension_bridge" ? (
              <div className="field-row">
                <label className="field">
                  <span>{planLabelFor("pension_amount")}</span>
                  <input
                    inputMode="decimal"
                    value={planState.pensionAmount}
                    placeholder="p. ej. 1200"
                    autoComplete="off"
                    onChange={(e) =>
                      setPlanState((s) => ({ ...s, pensionAmount: e.target.value }))
                    }
                  />
                  {planIssueFor("pensionAmount") ? (
                    <small className="muted">{planIssueFor("pensionAmount")}</small>
                  ) : null}
                </label>
                <label className="field">
                  <span>{planLabelFor("pension_start_age")}</span>
                  <input
                    inputMode="numeric"
                    value={planState.pensionStartAge}
                    placeholder="p. ej. 67"
                    autoComplete="off"
                    onChange={(e) =>
                      setPlanState((s) => ({ ...s, pensionStartAge: e.target.value }))
                    }
                  />
                  {planIssueFor("pensionStartAge") ? (
                    <small className="muted">{planIssueFor("pensionStartAge")}</small>
                  ) : null}
                </label>
              </div>
            ) : null}

            <div className="asset-form-actions">
              <button type="button" className="btn ghost" onClick={() => setStep(1)} disabled={busy}>
                Atrás
              </button>
              <button
                type="button"
                className="btn primary"
                onClick={() => void savePlan()}
                disabled={busy || planIssues.length > 0}
              >
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
                <strong>Proyección</strong>: cómo evoluciona tu patrimonio con los datos de
                arriba.
              </li>
            </ul>
            <p>
              Tu plan vive en Jubilación: <strong>{RETIREMENT_STRATEGY_LABEL[planState.strategy]}</strong>.
              Cambia la estrategia, la tasa de retirada o cualquier otro supuesto cuando quieras
              desde ahí.
            </p>
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
