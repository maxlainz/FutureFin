import { Modal } from "./Modal";

/**
 * Modal «¿Guardar snapshot?». Componente tonto: toda la lógica (cuándo abrir, qué guardar, si
 * ofrecer el paso de pasivos) vive en App.tsx. Dos pasos secuenciales:
 *  - `assets`: se ofrece al detectar que todos los activos líquidos propios se editaron.
 *  - `liabilities`: se ofrece tras guardar activos si hay cambios de pasivos sin snapshotear.
 * `closed` → no se renderiza nada.
 */
export function SnapshotPromptModal({
  step,
  busy,
  onSaveAssets,
  onSaveLiabilities,
  onDismiss,
}: {
  step: "closed" | "assets" | "liabilities";
  busy: boolean;
  onSaveAssets: () => void;
  onSaveLiabilities: () => void;
  onDismiss: () => void;
}) {
  return (
    <Modal title="¿Guardar snapshot?" open={step !== "closed"} onClose={onDismiss}>
      {step === "assets" ? (
        <div className="stack">
          <p className="muted tight">
            Has actualizado todos tus activos líquidos. Guarda un snapshot para
            conservar el histórico de hoy.
          </p>
          <div className="asset-form-actions">
            <button
              type="button"
              className="btn primary"
              disabled={busy}
              onClick={onSaveAssets}
            >
              Guardar snapshot
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={busy}
              onClick={onDismiss}
            >
              Ahora no
            </button>
          </div>
        </div>
      ) : step === "liabilities" ? (
        <div className="stack">
          <p className="muted tight">
            Snapshot de activos guardado. También hay cambios en pasivos desde el
            último snapshot. ¿Guardar snapshot de pasivos?
          </p>
          <div className="asset-form-actions">
            <button
              type="button"
              className="btn primary"
              disabled={busy}
              onClick={onSaveLiabilities}
            >
              Guardar
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={busy}
              onClick={onDismiss}
            >
              No, gracias
            </button>
          </div>
        </div>
      ) : null}
    </Modal>
  );
}
