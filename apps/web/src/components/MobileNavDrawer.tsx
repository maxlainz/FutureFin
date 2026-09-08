/**
 * Drawer derecho con la lista completa de secciones, usado solo en móvil.
 * Cierra con backdrop, Escape o tras navegar.
 */

import { useEffect } from "react";
import { TAB_PATH, tabsForScope, type TabId } from "../lib/navigation";
import { appUrl } from "../lib/basePath";
import { XIcon } from "./icons";
import type { LedgerPersonScope } from "../lib/ledger";

export function MobileNavDrawer({
  open,
  onClose,
  activeTab,
  navigate,
  scope,
}: {
  open: boolean;
  onClose: () => void;
  activeTab: TabId | null;
  navigate: (path: string) => void;
  /** Ámbito de datos (Yo | Hogar): filtra qué pestañas se pintan (F1, `tabsForScope`). */
  scope: LedgerPersonScope;
}) {
  useEffect(() => {
    if (!open) return;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  if (!open) return null;

  return (
    <>
      <div
        className="ff-mobile-backdrop"
        onClick={onClose}
        role="presentation"
      />
      <aside
        className="ff-mobile-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="Navegación"
      >
        <div className="ff-mobile-drawer-head">
          <span className="ff-topbar-title">Menú</span>
          <button
            type="button"
            className="btn ghost icon-btn"
            aria-label="Cerrar menú"
            onClick={onClose}
          >
            <XIcon />
          </button>
        </div>
        <nav className="ff-mobile-drawer-nav" aria-label="Secciones">
          {tabsForScope(scope).map((t) => (
            <a
              key={t.id}
              // Con prefijo (Ingress), el `href` crudo mandaría el Cmd-clic fuera del subpath.
              href={appUrl(TAB_PATH[t.id])}
              className={`ff-mobile-drawer-item ${activeTab === t.id ? "is-active" : ""}`}
              aria-current={activeTab === t.id ? "page" : undefined}
              onClick={(e) => {
                if (
                  e.button !== 0 ||
                  e.metaKey ||
                  e.altKey ||
                  e.ctrlKey ||
                  e.shiftKey
                ) {
                  return;
                }
                e.preventDefault();
                navigate(TAB_PATH[t.id]);
                onClose();
              }}
            >
              {t.label}
            </a>
          ))}
        </nav>
      </aside>
    </>
  );
}
