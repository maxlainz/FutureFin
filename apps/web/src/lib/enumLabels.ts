/**
 * Rótulos en español de los valores de enumeración que devuelve la API.
 *
 * La API habla inglés (`owner`, `member`, `viewer`, `ok`) porque es superficie para
 * desarrolladores; la interfaz no. Antes cada vista decidía por su cuenta: el `<select>` de
 * Ajustes traducía `member` → «Miembro» mientras la píldora de la cuenta y la ficha de la
 * instalación pintaban el valor crudo, así que el mismo usuario se veía como «Miembro» en un
 * sitio y como «member» en otro. Todo rótulo derivado de un enum sale de aquí.
 */

const ROLE_LABELS: Record<string, string> = {
  owner: "Propietario",
  member: "Miembro",
  viewer: "Visor",
};

/** Rol de instalación. Un rol desconocido se muestra tal cual antes que mentir. */
export function roleLabel(role: string | null | undefined): string {
  if (!role) return "";
  return ROLE_LABELS[role] ?? role;
}

const HEALTH_STATUS_LABELS: Record<string, string> = {
  ok: "Correcto",
  degraded: "Degradado",
  down: "Caído",
};

/** Estado del servicio en «Ajustes → Estado del sistema». */
export function healthStatusLabel(status: string | null | undefined): string {
  if (!status) return "";
  return HEALTH_STATUS_LABELS[status] ?? status;
}
