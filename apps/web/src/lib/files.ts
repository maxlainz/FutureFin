/**
 * Helpers de lectura de ficheros del navegador. Sin React, sin fetch (solo APIs DOM:
 * `File`, `window.btoa`). Compartido por el flujo de import `.ffbackup` (App.tsx) y por el
 * wizard de import de CSV bancarios (transacciones).
 */

/**
 * Lee un `File` y devuelve su contenido en base64 (sin prefijo `data:`). Procesa en trozos de
 * 32 KiB para no desbordar la pila con `String.fromCharCode(...spread)` en archivos grandes.
 */
export async function readFileAsBase64(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(
      ...bytes.subarray(i, Math.min(i + chunk, bytes.length)),
    );
  }
  return window.btoa(binary);
}
