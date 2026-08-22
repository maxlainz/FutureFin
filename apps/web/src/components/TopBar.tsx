/**
 * Top-bar única del rediseño V1:
 *  - Marca a la izquierda (logo FF + título).
 *  - Pestañas como pills a la derecha (≤720px: ocultas).
 *  - Slot `extras` a la derecha de las pills (usado para anclar el
 *    selector de vista en la esquina superior derecha).
 *  - Botón hamburguesa solo en ≤720px.
 *  - Indicador de salud del API (sin texto) entre marca y nav.
 *
 * El menú de usuario vive en Ajustes; aquí no se muestra.
 */

import type { ReactNode } from "react";
import { MenuIcon } from "./icons";
import { TABS, TAB_PATH, type TabId } from "../lib/navigation";

export function TopBar({
  activeTab,
  navigate,
  health,
  healthError,
  onMobileMenuOpen,
  extras,
  showNav = true,
}: {
  activeTab: TabId | null;
  navigate: (path: string) => void;
  health: { service: string; version: string } | null;
  healthError: string | null;
  onMobileMenuOpen: () => void;
  /** Contenido extra anclado a la derecha (después de las pills). */
  extras?: ReactNode;
  /** `false` oculta pestañas y hamburguesa. Se usa mientras el usuario no tiene acceso al hogar:
   *  antes las nueve pills se pintaban igual y no hacían nada al pulsarlas — navegación muerta. */
  showNav?: boolean;
}) {
  const healthOk = !!health && !healthError;
  const healthTitle = healthError
    ? `API: ${healthError}`
    : health
      ? `API ${health.service} ${health.version}`
      : "Comprobando…";
  return (
    <header className="ff-topbar" role="banner">
      <div className="ff-topbar-brand">
        <span className="logo-mark small" aria-hidden>
          FF
        </span>
        <span className="ff-topbar-title">FutureFin</span>
        <span
          className={`health-dot ${healthOk ? "ok" : "bad"}`}
          title={healthTitle}
          aria-label={healthTitle}
        />
      </div>

      {showNav ? (
      <nav className="ff-topbar-nav" aria-label="Secciones">
        {TABS.map((t) => (
          <a
            key={t.id}
            href={TAB_PATH[t.id]}
            className={`ff-nav-pill ${activeTab === t.id ? "is-active" : ""}`}
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
            }}
          >
            {t.label}
          </a>
        ))}
      </nav>
      ) : null}

      {extras ? <div className="ff-topbar-extras">{extras}</div> : null}

      {showNav ? (
        <button
          type="button"
          className="ff-topbar-mobile-toggle"
          aria-label="Abrir menú"
          title="Menú"
          onClick={onMobileMenuOpen}
        >
          <MenuIcon style={{ width: "1rem", height: "1rem" }} />
        </button>
      ) : null}
    </header>
  );
}
