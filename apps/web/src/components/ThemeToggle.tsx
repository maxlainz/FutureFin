/**
 * Segmented de 3 opciones: Auto / Claro / Oscuro.
 * El estado vive en App.tsx (useState + localStorage vía lib/theme).
 */

import type { ThemePref } from "../lib/theme";

const OPTIONS: { value: ThemePref; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "light", label: "Claro" },
  { value: "dark", label: "Oscuro" },
];

export function ThemeToggle({
  value,
  onChange,
}: {
  value: ThemePref;
  onChange: (next: ThemePref) => void;
}) {
  return (
    <div
      className="ff-theme-toggle"
      role="group"
      aria-label="Apariencia"
    >
      {OPTIONS.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            className={active ? "is-active" : ""}
            aria-pressed={active}
            onClick={() => {
              if (!active) onChange(o.value);
            }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
