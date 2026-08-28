/**
 * Ajustes → MCP: tokens de API (Bearer) para el servidor MCP embebido (`/mcp`).
 *
 * Panel autoservicio (self-fetch al montar, patrón HistorySettingsPanel): lista los tokens
 * del PROPIO usuario, crea nuevos (modal con label + caducidad + permisos) y los revoca. El
 * secreto completo solo existe en la respuesta del POST — se muestra UNA vez con botón de copiar
 * y no vuelve a ser recuperable. Cualquier miembro (viewer incluido) puede crear los
 * suyos: el token hereda su identidad y rol vivos en cada uso — lectura siempre, escritura
 * solo si el rol escribe, el token es `read_write` y `mcp_write_enabled` está activado
 * (Ajustes → MCP). El **scope** es la única de las tres puertas que se elige aquí, y solo resta:
 * un token `read_write` de un `viewer` sigue sin poder escribir.
 */

import { useCallback, useEffect, useState, type FormEvent } from "react";
import { apiDelete, apiGet, apiPost } from "../api/client";
import type { ApiTokenApi, CreateApiTokenResponseApi } from "../api/types";
import { Modal, ModalFormError } from "../components/Modal";
import { RowTrashIcon } from "../components/icons";
import { lastUsedLabel, tokenExpiryLabel } from "../lib/format";

/** Opciones de caducidad del modal de creación (valor = días; "" = sin caducidad). */
const EXPIRY_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "Sin caducidad" },
  { value: "90", label: "90 días" },
  { value: "365", label: "1 año" },
];

/** Opciones de permisos del modal de creación. */
const SCOPE_OPTIONS: { value: "read_write" | "read_only"; label: string }[] = [
  { value: "read_write", label: "Lectura y escritura" },
  { value: "read_only", label: "Solo lectura" },
];

/** Etiqueta legible del scope de un token; un valor desconocido se devuelve tal cual. */
function scopeLabel(scope: string): string {
  switch (scope) {
    case "read_write":
      return "Lectura y escritura";
    case "read_only":
      return "Solo lectura";
    default:
      return scope;
  }
}

export function ApiTokensPanel() {
  const [tokens, setTokens] = useState<ApiTokenApi[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [createOpen, setCreateOpen] = useState(false);
  const [labelDraft, setLabelDraft] = useState("");
  const [expiryDraft, setExpiryDraft] = useState("");
  const [scopeDraft, setScopeDraft] = useState<"read_write" | "read_only">("read_write");
  const [createBusy, setCreateBusy] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  /** Secreto recién creado — visible solo hasta cerrar el aviso. */
  const [freshToken, setFreshToken] = useState<CreateApiTokenResponseApi | null>(null);
  const [copied, setCopied] = useState(false);

  const [revokeTarget, setRevokeTarget] = useState<ApiTokenApi | null>(null);
  const [revoking, setRevoking] = useState(false);
  const [revokeError, setRevokeError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setTokens(await apiGet<ApiTokenApi[]>("/v1/api-tokens"));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "No se pudieron cargar los tokens.");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const openCreate = () => {
    setLabelDraft("");
    setExpiryDraft("");
    setScopeDraft("read_write");
    setCreateError(null);
    setCreateOpen(true);
  };

  const submitCreate = async (e: FormEvent) => {
    e.preventDefault();
    setCreateBusy(true);
    setCreateError(null);
    try {
      const body: { label: string; expires_in_days?: number; scope: "read_write" | "read_only" } = {
        label: labelDraft.trim(),
        scope: scopeDraft,
      };
      if (expiryDraft !== "") body.expires_in_days = Number(expiryDraft);
      const created = await apiPost<CreateApiTokenResponseApi>("/v1/api-tokens", body);
      if (created) {
        setFreshToken(created);
        setCopied(false);
      }
      setCreateOpen(false);
      await reload();
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : "No se pudo crear el token.");
    } finally {
      setCreateBusy(false);
    }
  };

  const copyFreshToken = async () => {
    if (!freshToken) return;
    try {
      await navigator.clipboard.writeText(freshToken.token);
      setCopied(true);
    } catch {
      // Sin permiso de clipboard: el usuario puede seleccionar el <code> a mano.
    }
  };

  const confirmRevoke = async () => {
    if (!revokeTarget) return;
    setRevoking(true);
    setRevokeError(null);
    try {
      await apiDelete(`/v1/api-tokens/${revokeTarget.id}`);
      setRevokeTarget(null);
      await reload();
    } catch (e) {
      setRevokeError(e instanceof Error ? e.message : "No se pudo revocar el token.");
    } finally {
      setRevoking(false);
    }
  };

  const activeTokens = tokens.filter((t) => !t.revoked_at);

  return (
    <section className="panel">
      <div className="panel-head-row">
        <h3 className="panel-title">Tokens de API (MCP)</h3>
        <button type="button" className="btn primary" onClick={openCreate}>
          Crear token
        </button>
      </div>
      <p className="muted">
        Tokens personales para conectar un cliente MCP con la cabecera{" "}
        <code>Authorization: Bearer &lt;token&gt;</code>. Heredan tu rol en cada
        uso y respetan el interruptor de escritura de arriba.
      </p>

      {error ? (
        <p className="error compact" role="alert">
          {error}
        </p>
      ) : null}

      {freshToken ? (
        <div className="token-reveal bordered-top" role="status">
          <p>
            <strong>{freshToken.label}</strong> creado. Cópialo ahora: no volverá a
            mostrarse.
          </p>
          <div className="token-reveal-row">
            <code className="token-secret">{freshToken.token}</code>
            <button type="button" className="btn ghost" onClick={copyFreshToken}>
              {copied ? "Copiado" : "Copiar"}
            </button>
            <button
              type="button"
              className="btn ghost"
              onClick={() => setFreshToken(null)}
            >
              Cerrar
            </button>
          </div>
        </div>
      ) : null}

      {loading ? (
        <p className="muted bordered-top">Cargando…</p>
      ) : activeTokens.length === 0 ? (
        <p className="muted bordered-top">Sin tokens.</p>
      ) : (
        <div className="table-scroll bordered-top">
          <table className="assets-table">
            <thead>
              <tr>
                <th>Nombre</th>
                <th>Token</th>
                <th>Permisos</th>
                <th>Último uso</th>
                <th>Vigencia</th>
                <th className="asset-actions-cell">
                  <span className="sr-only">Acciones</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {activeTokens.map((t) => (
                <tr key={t.id}>
                  <td>{t.label}</td>
                  <td>
                    <code>{t.token_prefix}…</code>
                  </td>
                  <td>{scopeLabel(t.scope)}</td>
                  <td>{lastUsedLabel(t.last_used_at)}</td>
                  <td>{tokenExpiryLabel(t.expires_at, t.revoked_at)}</td>
                  <td className="asset-actions-cell">
                    <button
                      type="button"
                      className="btn ghost danger icon-btn"
                      aria-label={`Revocar ${t.label}`}
                      title="Revocar"
                      onClick={() => {
                        setRevokeError(null);
                        setRevokeTarget(t);
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

      <Modal title="Crear token de API" open={createOpen} onClose={() => setCreateOpen(false)}>
        <form className="stack" onSubmit={submitCreate}>
          <label className="field">
            <span>Nombre</span>
            <input
              value={labelDraft}
              onChange={(e) => setLabelDraft(e.target.value)}
              maxLength={64}
              placeholder="Claude en el portátil"
              autoComplete="off"
              required
            />
          </label>
          <label className="field">
            <span>Caducidad</span>
            <select value={expiryDraft} onChange={(e) => setExpiryDraft(e.target.value)}>
              {EXPIRY_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Permisos</span>
            <select
              value={scopeDraft}
              onChange={(e) => setScopeDraft(e.target.value as "read_write" | "read_only")}
            >
              {SCOPE_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <p className="muted tight">
            Solo lectura: el cliente podrá consultar tus datos pero no crear, editar ni
            borrar nada.
          </p>
          <ModalFormError message={createError} />
          <button type="submit" className="btn primary" disabled={createBusy}>
            Crear token
          </button>
        </form>
      </Modal>

      <Modal
        title="Revocar token"
        open={revokeTarget !== null}
        onClose={() => {
          if (!revoking) setRevokeTarget(null);
        }}
      >
        {revokeTarget ? (
          <div className="stack">
            <ModalFormError message={revokeError} />
            <p className="muted tight">
              ¿Revocar <strong>{revokeTarget.label}</strong> (
              <code>{revokeTarget.token_prefix}…</code>)? Los clientes que lo usen
              perderán el acceso al instante.
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
