import { useEffect, useId, type ReactNode } from "react";

export function ModalFormError({
  message,
}: {
  message: string | null | undefined;
}) {
  if (!message) return null;
  return (
    <p className="error compact modal-form-error" role="alert">
      {message}
    </p>
  );
}

/** Tooltip nativo al pasar el cursor o foco. */
export function InlineHint({ title }: { title: string }) {
  return (
    <span
      className="inline-hint-icon"
      title={title}
      role="img"
      aria-label={title}
    >
      i
    </span>
  );
}

export function Modal({
  title,
  open,
  onClose,
  children,
  wide = false,
}: {
  title: string;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        className={
          wide ? "modal-dialog modal-dialog--wide card-elevated" : "modal-dialog card-elevated"
        }
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="modal-header">
          <h3 id={titleId} className="modal-title">
            {title}
          </h3>
          <button
            type="button"
            className="btn ghost modal-close"
            aria-label="Cerrar"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}
