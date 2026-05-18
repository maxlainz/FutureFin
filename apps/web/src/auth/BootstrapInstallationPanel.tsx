import type { Dispatch, FormEvent, SetStateAction } from "react";

export function BootstrapInstallationPanel({
  installationBusy,
  setupCurrency,
  setSetupCurrency,
  setupCalendarTz,
  setSetupCalendarTz,
  setupInstallation,
}: {
  installationBusy: boolean;
  setupCurrency: "EUR" | "USD" | "GBP";
  setSetupCurrency: (v: "EUR" | "USD" | "GBP") => void;
  setupCalendarTz: string;
  setSetupCalendarTz: Dispatch<SetStateAction<string>>;
  setupInstallation: (e: FormEvent) => void;
}) {
  return (
    <section className="panel">
      <h3 className="panel-title">Inicializar instalación</h3>
      <form className="stack bordered-top" onSubmit={setupInstallation}>
        <label className="field">
          <span>Moneda base</span>
          <select
            value={setupCurrency}
            onChange={(e) =>
              setSetupCurrency(e.target.value as "EUR" | "USD" | "GBP")
            }
          >
            <option value="EUR">EUR</option>
            <option value="USD">USD</option>
            <option value="GBP">GBP</option>
          </select>
        </label>
        <label className="field">
          <span>Zona horaria (IANA)</span>
          <select
            value={
              [
                "UTC",
                "Europe/Madrid",
                "Europe/London",
                "America/New_York",
                "America/Los_Angeles",
              ].includes(setupCalendarTz)
                ? setupCalendarTz
                : "__custom__"
            }
            onChange={(e) => {
              const v = e.target.value;
              if (v === "__custom__") return;
              setSetupCalendarTz(v);
            }}
          >
            <option value="UTC">UTC</option>
            <option value="Europe/Madrid">Europe/Madrid</option>
            <option value="Europe/London">Europe/London</option>
            <option value="America/New_York">America/New_York</option>
            <option value="America/Los_Angeles">America/Los_Angeles</option>
            <option value="__custom__">Otra (editar abajo)</option>
          </select>
        </label>
        <label className="field">
          <span>IANA exacta (opcional)</span>
          <input
            value={setupCalendarTz}
            onChange={(e) => setSetupCalendarTz(e.target.value)}
            placeholder="Europe/Madrid"
            maxLength={64}
            autoComplete="off"
          />
        </label>
        <button type="submit" className="btn primary" disabled={installationBusy}>
          Crear instalación
        </button>
      </form>
    </section>
  );
}
