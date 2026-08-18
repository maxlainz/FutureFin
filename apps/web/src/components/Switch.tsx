/**
 * Switch accesible (track + thumb, `role="switch"`) del design system. Extraído de los toggles
 * de ProjectionView al añadir el toggle de escritura MCP en Ajustes. Estilos en App.css
 * (`.ff-switch*`, tokens `--ff-*`); `variant="chart"` conserva el label small-caps compacto de
 * las barras de chart.
 */
export function Switch({
  checked,
  onChange,
  label,
  ariaLabel,
  disabled,
  variant,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
  ariaLabel: string;
  disabled?: boolean;
  variant?: "chart";
}) {
  return (
    <label className={variant === "chart" ? "ff-switch ff-switch--chart" : "ff-switch"}>
      {label ? <span className="ff-switch-label">{label}</span> : null}
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        aria-label={ariaLabel}
      />
      <span className="ff-switch-track" aria-hidden="true">
        <span className="ff-switch-thumb" />
      </span>
    </label>
  );
}
