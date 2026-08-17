/**
 * Helpers puros del flujo OAuth (`/oauth/authorize`): parseo de los query params del
 * authorization request, etiqueta del host del redirect y mensajes de error en español.
 * Sin fetch ni estado — testeables con Vitest.
 */

export type AuthorizeParams = {
  client_id: string;
  redirect_uri: string;
  response_type: string;
  code_challenge: string;
  code_challenge_method: string;
  state?: string;
  scope?: string;
  resource?: string;
};

/**
 * Extrae los parámetros OAuth de `window.location.search`. Devuelve null si falta
 * alguno de los cinco obligatorios — la validación REAL (cliente registrado, redirect
 * exacto, S256…) la hace el servidor; aquí solo se decide si hay algo que enviarle.
 */
export function parseAuthorizeParams(search: string): AuthorizeParams | null {
  const q = new URLSearchParams(search);
  const required = [
    "client_id",
    "redirect_uri",
    "response_type",
    "code_challenge",
    "code_challenge_method",
  ] as const;
  const values: Record<string, string> = {};
  for (const key of required) {
    const v = q.get(key);
    if (!v) return null;
    values[key] = v;
  }
  const params: AuthorizeParams = {
    client_id: values.client_id,
    redirect_uri: values.redirect_uri,
    response_type: values.response_type,
    code_challenge: values.code_challenge,
    code_challenge_method: values.code_challenge_method,
  };
  const state = q.get("state");
  if (state) params.state = state;
  const scope = q.get("scope");
  if (scope) params.scope = scope;
  const resource = q.get("resource");
  if (resource) params.resource = resource;
  return params;
}

/** Host legible del redirect_uri (`claude.ai`); el string original si no parsea. */
export function redirectHostLabel(redirectUri: string): string {
  try {
    return new URL(redirectUri).host || redirectUri;
  } catch {
    return redirectUri;
  }
}

const ERROR_MESSAGES: Record<string, string> = {
  missing_client_id: "La solicitud no identifica a ninguna aplicación (falta client_id).",
  unknown_client:
    "La aplicación no está registrada en esta instalación. Vuelve a intentar la conexión desde el cliente.",
  missing_redirect_uri: "La solicitud no incluye una URL de retorno (redirect_uri).",
  redirect_uri_mismatch:
    "La URL de retorno no coincide con la registrada por la aplicación. Por seguridad, no se continuará.",
  unsupported_response_type: "La aplicación pide un tipo de respuesta no soportado.",
  invalid_request: "La solicitud de autorización es inválida (PKCE S256 es obligatorio).",
  invalid_target: "La aplicación pide acceso a un recurso que no es esta instalación.",
  server_error: "Error interno validando la solicitud. Inténtalo de nuevo.",
};

/** Mensaje en español para un código de error de autorización; default genérico. */
export function authorizeErrorMessage(code: string): string {
  return (
    ERROR_MESSAGES[code] ??
    `La solicitud de autorización no es válida (${code}).`
  );
}
