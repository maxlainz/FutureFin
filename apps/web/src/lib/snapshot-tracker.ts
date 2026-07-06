/**
 * Tracking en memoria (por sesión) de ediciones de valor de activos líquidos, para disparar el
 * modal «¿Guardar snapshot?» una vez que TODOS los activos líquidos propios han recibido una
 * edición dentro de una ventana rodante. Puro: cero React, cero fetch. El estado (la `EditLog`)
 * vive en refs de App.tsx; aquí solo van las decisiones.
 */

/** Mapa `assetId → epochMs` de la última edición de valor registrada. */
export type EditLog = Map<string, number>;

/** Ventana rodante: ~1 h. */
export const SNAPSHOT_EDIT_WINDOW_MS = 60 * 60 * 1000;

/**
 * Devuelve una **copia** de `log` sin las entradas más viejas que `windowMs` respecto a `now`.
 * No muta el original (los callers de App reasignan la ref).
 */
export function pruneEditLog(
  log: EditLog,
  now: number,
  windowMs: number = SNAPSHOT_EDIT_WINDOW_MS,
): EditLog {
  const out: EditLog = new Map();
  for (const [id, ts] of log) {
    if (now - ts <= windowMs) out.set(id, ts);
  }
  return out;
}

/**
 * `true` sólo si hay al menos un activo líquido y **todos** tienen una edición registrada dentro
 * de la ventana. Un id nuevo en el set sin edición reciente (o con edición caducada) → `false`.
 */
export function liquidCoverageComplete(
  log: EditLog,
  liquidAssetIds: string[],
  now: number,
  windowMs: number = SNAPSHOT_EDIT_WINDOW_MS,
): boolean {
  if (liquidAssetIds.length === 0) return false;
  for (const id of liquidAssetIds) {
    const ts = log.get(id);
    if (ts === undefined) return false;
    if (now - ts > windowMs) return false;
  }
  return true;
}
