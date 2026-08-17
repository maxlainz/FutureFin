/**
 * Ajustes → Acceso: conexiones OAuth (apps autorizadas vía `/oauth/authorize`, p. ej.
 * el conector de claude.ai). Calco del patrón de ApiTokensPanel: self-fetch al montar,
 * lista del PROPIO usuario y revocación con confirmación (corte inmediato — el access
 * token muere con el grant sin esperar a caducar). Disponible incluso con
 * FUTUREFIN_MCP_ENABLED=0: apagar MCP no puede dejarte sin poder revocar.
 */

import { useCallback, useEffect, useState } from "react";
import { apiDelete, apiGet } from "../api/client";
import type { OAuthConnectionApi } from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { LinkIcon, RowTrashIcon } from "../components/icons";
import { isoTimestampDmy, lastUsedLabel } from "../lib/format";

export function OAuthConnectionsPanel() {
  const [connections, setConnections] = useState<OAuthConnectionApi[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [revokeTarget, setRevokeTarget] = useState<OAuthConnectionApi | null>(null);
  const [revoking, setRevoking] = useState(false);
  const [revokeError, setRevokeError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setConnections(await apiGet<OAuthConnectionApi[]>("/v1/oauth/connections"));
      setError(null);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "No se pudieron cargar las conexiones.",
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const confirmRevoke = async () => {
    if (!revokeTarget) return;
    setRevoking(true);
    setRevokeError(null);
    try {
      await apiDelete(`/v1/oauth/connections/${revokeTarget.id}`);
      setRevokeTarget(null);
      await reload();
    } catch (e) {
      setRevokeError(
        e instanceof Error ? e.message : "No se pudo revocar la conexión.",
      );
    } finally {
      setRevoking(false);
    }
  };

  return (
    <section className="panel">
      <div className="panel-head-row">
        <h3 className="panel-title">
          <LinkIcon className="panel-title-icon" /> Conexiones
        </h3>
      </div>
      <p className="muted">
        Aplicaciones autorizadas por OAuth (p. ej. el conector de claude.ai). Solo
        lectura; revocar corta el acceso al instante.
      </p>

      {error ? (
        <p className="error compact" role="alert">
          {error}
        </p>
      ) : null}

      {loading ? (
        <p className="muted bordered-top">Cargando…</p>
      ) : connections.length === 0 ? (
        <p className="muted bordered-top">Sin aplicaciones conectadas.</p>
      ) : (
        <div className="table-scroll bordered-top">
          <table className="assets-table">
            <thead>
              <tr>
                <th>Aplicación</th>
                <th>Conectada</th>
                <th>Último uso</th>
                <th className="asset-actions-cell">
                  <span className="sr-only">Acciones</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {connections.map((c) => (
                <tr key={c.id}>
                  <td>
                    {c.client_name}
                    {c.redirect_host ? (
                      <>
                        {" "}
                        <span className="muted">({c.redirect_host})</span>
                      </>
                    ) : null}
                  </td>
                  <td>{isoTimestampDmy(c.created_at)}</td>
                  <td>{lastUsedLabel(c.last_used_at)}</td>
                  <td className="asset-actions-cell">
                    <button
                      type="button"
                      className="btn ghost danger icon-btn"
                      aria-label={`Revocar ${c.client_name}`}
                      title="Revocar"
                      onClick={() => {
                        setRevokeError(null);
                        setRevokeTarget(c);
                      }}
                    >
                      <RowTrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <Modal
        title="Revocar conexión"
        open={revokeTarget !== null}
        onClose={() => {
          if (!revoking) setRevokeTarget(null);
        }}
      >
        {revokeTarget ? (
          <div className="stack">
            <ModalFormError message={revokeError} />
            <p className="muted tight">
              ¿Revocar el acceso de <strong>{revokeTarget.client_name}</strong>? La
              aplicación perderá el acceso al instante y tendrá que volver a pedir
              permiso.
            </p>
            <div className="asset-form-actions">
              <button
                type="button"
                className="btn ghost danger"
                disabled={revoking}
                onClick={() => void confirmRevoke()}
              >
                {revoking ? "Revocando…" : "Revocar"}
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={revoking}
                onClick={() => setRevokeTarget(null)}
              >
                Cancelar
              </button>
            </div>
          </div>
        ) : null}
      </Modal>
    </section>
  );
}
