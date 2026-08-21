#!/usr/bin/env bash
# Imprime por stdout el cuerpo de la sección de CHANGELOG.md de una versión.
#
# Uso:  ./scripts/changelog-section.sh 3.8.0
#       ./scripts/changelog-section.sh v3.8.0     (la "v" inicial es opcional)
#
# Lo consume el workflow `publish-image.yml` para redactar las notas del GitHub
# Release a partir del CHANGELOG, que es la única fuente de verdad de qué llevó
# cada versión. Falla LOUD (exit 1) si la versión no tiene sección: preferimos
# que la publicación se pare a que salga un Release con notas vacías.
set -euo pipefail

version="${1:-}"
if [ -z "$version" ]; then
  echo "uso: $0 <version>   (p. ej. 3.8.0 o v3.8.0)" >&2
  exit 2
fi
version="${version#v}"

# Solo versiones semver. Sin esto, `changelog-section.sh Unreleased` devolvería
# la sección en curso y el workflow publicaría como notas de un Release lo que
# aún no se ha lanzado.
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *)
    echo "versión no semver: '$version' (esperaba X.Y.Z)" >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
changelog="${CHANGELOG_FILE:-$repo_root/CHANGELOG.md}"

if [ ! -f "$changelog" ]; then
  echo "no encuentro el CHANGELOG: $changelog" >&2
  exit 2
fi

# `index($0, want) == 1` exige que la cabecera empiece EXACTAMENTE por
# "## [X.Y.Z]" — comparación literal, así que ni los puntos de la versión se
# interpretan como comodines ni "1.0.1" se come la sección de "1.0.10".
body="$(
  awk -v want="## [$version]" '
    index($0, want) == 1 { found = 1; next }
    found && /^## \[/    { exit }
    found                { print }
  ' "$changelog" | sed -e '/./,$!d'
)"

if [ -z "$body" ]; then
  echo "el CHANGELOG no tiene sección para la versión $version" >&2
  exit 1
fi

# El $(...) de arriba ya recorta los saltos de línea finales.
printf '%s\n' "$body"
