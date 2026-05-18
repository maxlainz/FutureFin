/**
 * Barra apilada simple comparando inflow vs outflow planificado. Devuelve `null` cuando no hay
 * datos positivos o los valores son no-finitos.
 */
export function PlanningDirectionChart({
  inflow,
  outflow,
}: {
  inflow: number;
  outflow: number;
}) {
  const sum = inflow + outflow;
  if (!(sum > 0) || !(Number.isFinite(inflow) && Number.isFinite(outflow))) {
    return null;
  }
  const wi = (inflow / sum) * 100;
  const wo = (outflow / sum) * 100;
  return (
    <div className="planning-dir-chart bordered-top">
      <svg
        viewBox="0 0 100 12"
        preserveAspectRatio="none"
        className="planning-dir-svg"
        role="img"
        aria-label="Comparación entradas y salidas planificadas"
      >
        <title>Entradas y salidas</title>
        <rect x="0" y="0" width={wi} height="12" className="planning-dir-bar-in" />
        <rect
          x={wi}
          y="0"
          width={wo}
          height="12"
          className="planning-dir-bar-out"
        />
      </svg>
    </div>
  );
}
