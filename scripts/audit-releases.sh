#!/usr/bin/env bash
# Audita la coherencia entre las CUATRO listas de versiones del proyecto:
#
#   1. secciones `## [X.Y.Z]` de CHANGELOG.md
#   2. tags git `vX.Y.Z`
#   3. GitHub Releases            (solo si hay `gh` autenticado)
#   4. secciones `## X.Y.Z` de addon/futurefin/CHANGELOG.md (lo que renderiza la
#      tienda de Home Assistant; se quedó clavado en 4.3.1 durante cuatro releases)
#
# Existe porque las tres se desincronizaron sin que nadie lo viera: en agosto de
# 2026 había 38 tags, 48 secciones y solo DOS Releases, y dos secciones
# (`1.0.5` y `2.2.0`) se habían perdido del CHANGELOG pese a estar publicadas.
# Nada avisaba: cada lista la mantenía un mecanismo distinto.
#
# Uso:
#   ./scripts/audit-releases.sh            # informe legible; sale 1 si hay deriva bloqueante
#   ./scripts/audit-releases.sh --version  # solo comprueba la versión de Cargo.toml (modo CI)
#   ./scripts/audit-releases.sh --addon    # el add-on de HA apunta a la versión del binario
#
# Deriva BLOQUEANTE (exit 1): un tag sin sección en el CHANGELOG. Eso rompe el
# paso de publicación de `publish-image.yml`, que redacta las notas desde ahí.
# Deriva INFORMATIVA (exit 0): secciones sin tag (versiones documentadas que
# nunca se publicaron) y tags sin Release (recuperable con `gh release create`).
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

changelog_versions() {
  grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | tr -d '#[] ' | sort -V -u
}

# Formato del add-on: "## X.Y.Z" SIN corchetes ni fecha (distinto del raíz a propósito —
# ver addon/futurefin/CHANGELOG.md; no se homogeneiza el formato de las secciones ya escritas).
addon_changelog_versions() {
  grep -oE '^## [0-9]+\.[0-9]+\.[0-9]+$' addon/futurefin/CHANGELOG.md | tr -d '# ' | sort -V -u
}

cargo_version() {
  grep -m1 '^version' apps/api/Cargo.toml | cut -d'"' -f2
}

addon_version() {
  grep -m1 '^version:' addon/futurefin/config.yaml | cut -d'"' -f2
}

# --- modo add-on: la tienda de HA y el binario deben decir lo mismo -------------
# El Supervisor usa el `version:` del add-on como TAG de la imagen. Si diverge del
# Cargo.toml, o pide una imagen que no existe todavía, o sirve una vieja en silencio.
# Lo sincroniza el paso final de publish-image.yml; este modo es la comprobación
# manual (durante una release el add-on va por detrás hasta que la imagen se publica,
# así que NO está cableado en la CI a propósito).
if [ "${1:-}" = "--addon" ]; then
  cargo_v="$(cargo_version)"
  addon_v="$(addon_version)"
  if [ "$cargo_v" != "$addon_v" ]; then
    echo "FALLO: el add-on declara $addon_v y apps/api/Cargo.toml está en $cargo_v." >&2
    echo "       El Supervisor de Home Assistant usa ese número como tag de la imagen." >&2
    echo "       Sincroniza addon/futurefin/config.yaml (lo hace publish-image.yml tras" >&2
    echo "       publicar la imagen; a mano, con un PR normal)." >&2
    exit 1
  fi
  echo "OK: add-on y binario coinciden en $cargo_v."
  exit 0
fi

# --- modo CI: la versión que se va a publicar DEBE tener sección ---------------
# Es el guard barato que habría evitado el agujero de la 2.2.0 (se tagueó y se
# publicó imagen, pero su sección desapareció del CHANGELOG sin que nada chistara).
if [ "${1:-}" = "--version" ]; then
  v="$(cargo_version)"
  if ! ./scripts/changelog-section.sh "$v" >/dev/null 2>&1; then
    echo "FALLO: apps/api/Cargo.toml está en $v y CHANGELOG.md no tiene su sección '## [$v]'." >&2
    echo "       Sin ella, publish-image.yml no puede redactar las notas del Release." >&2
    exit 1
  fi
  # Mismo guard, aplicado al changelog del add-on de Home Assistant: sin sección aquí, la
  # tienda de HA vuelve a quedarse «clavada» en la última versión documentada aunque el
  # `version:` de config.yaml sí avance (incidente: nada visible más allá de 4.3.1 pese a
  # cuatro releases reales — 4.4.0, 4.4.1, 4.5.0, 4.6.0 — publicadas de por medio).
  if ! addon_changelog_versions | grep -qx "$v"; then
    echo "FALLO: apps/api/Cargo.toml está en $v y addon/futurefin/CHANGELOG.md no tiene su" >&2
    echo "       sección '## $v'. Sin ella, la tienda de Home Assistant se queda clavada en" >&2
    echo "       la última versión documentada aunque el add-on sí actualice el binario." >&2
    echo "       Añade la sección (puede ser una línea si esta versión no cambia nada" >&2
    echo "       visible desde el add-on) antes de mergear el bump." >&2
    exit 1
  fi
  echo "OK: la versión $v tiene sección en el CHANGELOG y en el changelog del add-on."
  exit 0
fi

# --- informe completo ---------------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

changelog_versions > "$tmp/changelog"
git tag -l 'v[0-9]*.[0-9]*.[0-9]*' | sed 's/^v//' | sort -V -u > "$tmp/tags"

echo "CHANGELOG: $(wc -l < "$tmp/changelog" | tr -d ' ')   tags: $(wc -l < "$tmp/tags" | tr -d ' ')"

status=0

echo
echo "== Tags SIN sección en el CHANGELOG (bloqueante) =="
if comm -23 "$tmp/tags" "$tmp/changelog" | grep -q .; then
  comm -23 "$tmp/tags" "$tmp/changelog" | sed 's/^/  v/'
  status=1
else
  echo "  (ninguno)"
fi

echo
echo "== Secciones SIN tag (documentadas, nunca publicadas — informativo) =="
comm -13 "$tmp/tags" "$tmp/changelog" | sed 's/^/  /' | grep . || echo "  (ninguna)"

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  gh release list --limit 200 --json tagName --jq '.[].tagName' \
    | sed 's/^v//' | sort -V -u > "$tmp/releases"
  echo
  echo "== Tags SIN GitHub Release (informativo: 'gh release create') =="
  comm -23 "$tmp/tags" "$tmp/releases" | sed 's/^/  v/' | grep . || echo "  (ninguno)"
else
  echo
  echo "== GitHub Releases: omitido (sin 'gh' autenticado) =="
fi

echo
if [ "$status" -eq 0 ]; then
  echo "Sin deriva bloqueante."
else
  echo "DERIVA BLOQUEANTE: hay tags cuya versión no está documentada." >&2
fi
exit "$status"
