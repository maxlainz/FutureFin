#!/usr/bin/env bash
# Escáner de datos sensibles sobre los ficheros TRACKEADOS del repositorio.
#
# Existe por un incidente concreto: hasta agosto de 2026 `apps/api/tests/fixtures/`
# contenía extractos bancarios REALES —IBAN español completo, nombre y apellidos de una
# persona, nómina al céntimo— y el CHANGELOG narraba las cifras de una instalación real.
# El repositorio iba a hacerse público con todo eso dentro y en 109 commits del historial.
# Nada lo detectaba: cada fixture lo revisaba quien lo escribía.
#
# Uso:
#   ./scripts/scan-sensitive.sh          # sale 1 si encuentra algo
#   ./scripts/scan-sensitive.sh --list   # solo lista los patrones que aplica
#
# Excepciones legítimas: añádelas a scripts/sensitive-allowlist.txt, una expresión
# regular extendida por línea, SIEMPRE con un comentario encima explicando por qué.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

allowlist="scripts/sensitive-allowlist.txt"

# nombre|regex extendida (ERE). Deliberadamente estrechas: un falso positivo que nadie
# sabe silenciar acaba en `|| true` y entonces el gate deja de existir.
PATTERNS=(
  "IBAN|(^|[^A-Za-z0-9])[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}([^A-Za-z0-9]|$)"
  "Tarjeta de crédito|(^|[^0-9.])(4[0-9]{12}([0-9]{3})?|5[1-5][0-9]{14}|3[47][0-9]{13})([^0-9.]|$)"
  "Clave privada|-----BEGIN [A-Z ]*PRIVATE KEY-----"
  "Token de GitHub|gh[pousr]_[A-Za-z0-9]{36}"
  "Clave de AWS|AKIA[0-9A-Z]{16}"
  "Token de Slack|xox[baprs]-[A-Za-z0-9-]{10,}"
  "Clave de OpenAI/Anthropic|sk-(ant-)?[A-Za-z0-9_-]{24,}"
  "Token de API de FutureFin|ff[po]_[A-Za-z0-9]{24,}"
)

if [ "${1:-}" = "--list" ]; then
  printf '%s\n' "${PATTERNS[@]%%|*}"
  exit 0
fi

# Los ficheros de bloqueo son ruido puro: hashes y metadatos de terceros.
files="$(git ls-files -- . ':!package-lock.json' ':!Cargo.lock' ":!$allowlist")"

status=0
for entry in "${PATTERNS[@]}"; do
  name="${entry%%|*}"
  regex="${entry#*|}"
  # -I: ignora binarios. Filtramos por la allowlist línea a línea.
  hits="$(printf '%s\n' "$files" | tr '\n' '\0' \
    | xargs -0 grep -InE "$regex" 2>/dev/null || true)"
  if [ -f "$allowlist" ] && [ -n "$hits" ]; then
    while IFS= read -r rule; do
      case "$rule" in ''|'#'*) continue ;; esac
      hits="$(printf '%s\n' "$hits" | grep -vE "$rule" || true)"
    done < "$allowlist"
  fi
  if [ -n "$hits" ]; then
    echo "FALLO [$name]:" >&2
    printf '%s\n' "$hits" | sed 's/^/  /' >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  cat >&2 <<'MSG'

Datos sensibles en ficheros trackeados. Antes de commitear:
  - los fixtures de test se FABRICAN, no se exportan del banco (skill futurefin-data-hygiene);
  - el CHANGELOG usa cifras de ejemplo, nunca las de una instalación real;
  - si el dato ya está en el historial, avisa: hace falta reescribirlo, no basta con borrarlo.
MSG
  exit 1
fi

echo "OK: sin datos sensibles en los ficheros trackeados."
