import { useEffect, useRef, useState } from "react";
import type { HistorySnapshotKindApi } from "../api/types";

/**
 * Botón presentacional «Guardar snapshot» para las vistas Activos/Pasivos. Máquina de estados
 * interna idle → busy → success/error. El guardado real lo hace `onSave` (App): resuelve (void)
 * si se guardó, y **lanza** un `Error` (con el mensaje de la API) si falla — este componente lo
 * captura y lo muestra inline. La captura hace upsert silencioso, así que no hay flujo de
 * confirmación.
 */

type SnapshotButtonState = "idle" | "busy" | "success" | "error";

/** Cuánto tiempo se muestra «Snapshot guardado» antes de volver a idle. */
const SUCCESS_VISIBLE_MS = 2500;

export function SnapshotButton({
  kind,
  disabled = false,
  onSave,
}: {
  kind: HistorySnapshotKindApi;
  disabled?: boolean;
  /** Resuelve (void) si se guardó; lanza `Error` con el mensaje de la API si falla. */
  onSave: () => Promise<void>;
}) {
  const [state, setState] = useState<SnapshotButtonState>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const successTimerRef = useRef<number | null>(null);

  const clearSuccessTimer = () => {
    if (successTimerRef.current !== null) {
      window.clearTimeout(successTimerRef.current);
      successTimerRef.current = null;
    }
  };

  useEffect(() => clearSuccessTimer, []);

  async function handleClick() {
    if (state === "busy") return;
    clearSuccessTimer();
    setState("busy");
    setErrorMsg(null);
    try {
      // onSave resuelve (void) en éxito y lanza en fallo → un catch basta; no hay rama "false".
      await onSave();
      setState("success");
      successTimerRef.current = window.setTimeout(() => {
        successTimerRef.current = null;
        setState("idle");
      }, SUCCESS_VISIBLE_MS);
    } catch (e: unknown) {
      setErrorMsg(e instanceof Error ? e.message : String(e));
      setState("error");
    }
  }

  const label =
    state === "busy"
      ? "Guardando…"
      : state === "success"
        ? "Snapshot guardado"
        : "Guardar snapshot";

  return (
    <span className="snapshot-button">
      <button
        type="button"
        className="btn ghost"
        title="Guarda un snapshot con tus ítems de hoy"
        aria-label={
          kind === "asset"
            ? "Guardar snapshot de activos"
            : "Guardar snapshot de pasivos"
        }
        disabled={disabled || state === "busy"}
        onClick={() => void handleClick()}
      >
        {label}
      </button>
      {state === "error" && errorMsg ? (
        <span className="error snapshot-button-error" role="alert">
          {errorMsg}
        </span>
      ) : null}
    </span>
  );
}
