/**
 * Popover de ayuda anclado a un icono de interrogante.
 *
 * Por qué un componente nuevo y no el `title=` nativo (`InlineHint`) ni el `Modal`: el nativo es
 * invisible al tocar en móvil y no se puede estilar; el `Modal` es a pantalla completa con
 * backdrop, desproporcionado para explicar un KPI de dos líneas y corta el flujo de lectura.
 *
 * Es un diálogo NO modal: no atrapa el foco ni bloquea el scroll de la página. Se cierra con
 * Escape, con un clic fuera y al perder el foco. El panel se clampa al viewport (misma técnica
 * que el tooltip del chart de proyección) para no romper la regla de oro responsive de cero
 * scroll horizontal.
 */

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { QuestionIcon } from "./icons";

export function HelpPopover({ title, body }: { title: string; body: string }) {
  const [open, setOpen] = useState(false);
  const [offsetX, setOffsetX] = useState(0);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const panelId = useId();

  // Escape + clic fuera. Se registran solo mientras está abierto: un listener global permanente
  // por cada icono de la página sería caro (hay ~50).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [open]);

  // Clamp al viewport ANTES de pintar (useLayoutEffect): con useEffect el panel se vería un
  // frame desbordado antes de recolocarse.
  useLayoutEffect(() => {
    if (!open || !panelRef.current) return;
    setOffsetX(0);
    const r = panelRef.current.getBoundingClientRect();
    const margin = 8;
    if (r.right > window.innerWidth - margin) {
      setOffsetX(window.innerWidth - margin - r.right);
    } else if (r.left < margin) {
      setOffsetX(margin - r.left);
    }
  }, [open]);

  return (
    <span className="help-popover" ref={wrapRef}>
      <button
        type="button"
        className="inline-hint-icon help-popover__trigger"
        aria-label={`Qué significa: ${title}`}
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <QuestionIcon />
      </button>
      {open ? (
        <div
          id={panelId}
          ref={panelRef}
          className="help-popover__panel"
          role="dialog"
          aria-label={title}
          style={offsetX ? { transform: `translateX(${offsetX}px)` } : undefined}
        >
          <p className="help-popover__title">{title}</p>
          <p className="help-popover__body">{body}</p>
        </div>
      ) : null}
    </span>
  );
}
