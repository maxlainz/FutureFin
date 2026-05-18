/**
 * Tarjeta de cuenta destacada — se renderiza arriba de Ajustes, en todas las
 * sub-tabs. Reemplaza el chip de usuario + botón "Salir" que estaba en la
 * cabecera del shell antiguo.
 */

import { UserIcon, XIcon } from "./icons";

export type AccountCardRole = string | null;

export function AccountCard({
  username,
  role,
  installationName,
  onEditAccount,
  onLogout,
  busy,
}: {
  username: string;
  role: AccountCardRole;
  installationName: string | null;
  onEditAccount: () => void;
  onLogout: () => void;
  busy: boolean;
}) {
  const initials = username.slice(0, 2);
  return (
    <section className="ff-account-card" aria-label="Tu cuenta">
      <div className="ff-account-main">
        <span className="ff-avatar" aria-hidden>
          {initials}
        </span>
        <div className="ff-account-info">
          <div className="ff-account-name">{username}</div>
          <div className="ff-account-meta">
            {role ? <span className="role-pill subtle">{role}</span> : null}
            {installationName ? <span>{installationName}</span> : null}
          </div>
        </div>
      </div>
      <div className="ff-account-actions">
        <button
          type="button"
          className="btn ghost"
          onClick={onEditAccount}
          disabled={busy}
        >
          <UserIcon />
          Editar cuenta
        </button>
        <button
          type="button"
          className="btn"
          onClick={onLogout}
          disabled={busy}
        >
          <XIcon />
          Cerrar sesión
        </button>
      </div>
    </section>
  );
}
