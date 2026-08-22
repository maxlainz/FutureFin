#!/usr/bin/env bash
# Publica `dev` en `main` para una release.
#
# Existe porque el merge a `main` tiene dos trampas que se repiten en CADA release y que ya
# mordieron en la 4.0.0:
#
#   1. `git merge dev` a secas escribe «Merge branch 'dev'», que incumple la convención de commits
#      del repo y es lo primero que ve quien abre la rama pública.
#   2. `main` no publica `CLAUDE.md` ni `.claude/` (documentación de desarrollo, vive solo en
#      `dev`), así que cada merge produce conflictos «modificado/borrado» en esas rutas. Se
#      resuelven SIEMPRE borrando — pero hacerlo a mano invita a resolver mal el día que haya
#      prisa, y el job `main-guard` de CI solo avisa después de empujar.
#
# Uso:
#   ./scripts/release-to-main.sh 4.1.0 "una frase sobre la versión"
#
# No taguea ni empuja: eso lo decides tú después de mirar el resultado.
set -euo pipefail

VERSION="${1:?falta la versión, p. ej. 4.1.0}"
SUMMARY="${2:?falta una frase que resuma la versión}"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# Rutas que `main` nunca publica. Si añades otra, añádela también al job `main-guard` de ci.yml.
PRIVATE_PATHS=(CLAUDE.md .claude)

[ -z "$(git status --porcelain)" ] || {
  echo "ERROR: el árbol tiene cambios sin commitear. Guárdalos antes de publicar." >&2
  exit 1
}

echo "== comprobaciones previas =="
./scripts/audit-releases.sh --version
./scripts/scan-sensitive.sh

cargo_version="$(grep -m1 '^version' apps/api/Cargo.toml | cut -d'"' -f2)"
[ "$cargo_version" = "$VERSION" ] || {
  echo "ERROR: apps/api/Cargo.toml está en $cargo_version y pides publicar $VERSION." >&2
  exit 1
}

# Sin esto se publica lo que haya en local: si `dev` o `main` están atrasados respecto al
# remoto, el merge sale parcial y el tag apunta a un árbol que nadie ha revisado.
echo "== sincronía con origin =="
git fetch --quiet origin
for rama in dev main; do
  local_sha="$(git rev-parse "$rama")"
  remote_sha="$(git rev-parse "origin/$rama" 2>/dev/null || echo "")"
  if [ -n "$remote_sha" ] && [ "$local_sha" != "$remote_sha" ]; then
    echo "ERROR: $rama local ($local_sha) difiere de origin/$rama ($remote_sha). Sincroniza antes de publicar." >&2
    exit 1
  fi
done

git checkout dev
git checkout main

echo "== merge =="
# Los conflictos de PRIVATE_PATHS son esperados y se resuelven abajo, así que un exit 1 es
# normal. Cualquier OTRO código no lo es: con un `|| true` a secas, un merge que ni llegara a
# empezar (índice sucio, referencia mala) dejaba cero ficheros en conflicto, la comprobación
# de abajo pasaba, y el script hacía un commit normal que BORRABA CLAUDE.md y .claude de main
# sin haber mergeado nada.
rc=0
git merge --no-ff --no-commit dev -m "chore(release): $VERSION — $SUMMARY" || rc=$?
if [ "$rc" -gt 1 ]; then
  echo "ERROR: git merge falló con código $rc (no es un conflicto). Revísalo a mano." >&2
  git merge --abort 2>/dev/null || true
  exit 1
fi

# Solo lo TRACKEADO. `rm -rf .claude` se llevaba también lo ignorado que vive dentro
# (settings.local.json, cachés, notas), que `git status --porcelain` no ve y `git checkout dev`
# no restaura: desaparecía sin aviso y sin vuelta atrás.
#
# La lista se captura ANTES del `git rm --cached`, y ese orden es todo el truco: al revés
# —que es como salió la primera versión de este guard— el índice ya no contiene esas rutas
# cuando `git ls-files` las busca, así que devuelve vacío y el borrado es un NO-OP silencioso.
# El commit sale bien igual (las rutas ya no están en el índice), pero el árbol de trabajo se
# queda con los ficheros sueltos y sin trackear en `main`: inofensivos hasta que alguien haga
# un `git add -A` ahí y los publique. Ocurrió el 2026-08-22, en la primera ejecución.
private_tracked=()
while IFS= read -r -d '' f; do private_tracked+=("$f"); done \
  < <(git ls-files -z -- "${PRIVATE_PATHS[@]}")

git rm -r -q --cached --ignore-unmatch "${PRIVATE_PATHS[@]}"

if [ ${#private_tracked[@]} -gt 0 ]; then
  rm -f -- "${private_tracked[@]}"
  # Los directorios vacíos que quedan no los ve git, pero ensucian el árbol de `main`.
  find "${PRIVATE_PATHS[@]}" -type d -empty -delete 2>/dev/null || true
fi

# Cualquier conflicto FUERA de esas rutas es real y hay que mirarlo a mano.
remaining="$(git diff --name-only --diff-filter=U || true)"
if [ -n "$remaining" ]; then
  echo "ERROR: conflictos que no son documentación interna — resuélvelos a mano:" >&2
  printf '  %s\n' $remaining >&2
  exit 1
fi

# El cuerpo lleva los TITULARES de lo que entra en la rama. Un merge que solo dice su versión
# obliga a ir al historial para saber qué trae; con los asuntos delante, el commit se lee solo.
titulares="$(git log --no-merges --pretty='- %s' HEAD..dev)"
[ -n "$titulares" ] || titulares="- (sin commits nuevos respecto a main)"

git commit -m "chore(release): $VERSION — $SUMMARY

$titulares

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"

offenders="$(git ls-files -- "${PRIVATE_PATHS[@]}")"
[ -z "$offenders" ] || {
  echo "ERROR: main seguiría publicando documentación interna:" >&2
  printf '  %s\n' $offenders >&2
  exit 1
}

echo
echo "main listo en $(git rev-parse --short HEAD). Ahora, si te convence:"
echo "  git push origin main"
echo "  git push origin v$VERSION      # después de crear el tag"
echo "  git checkout dev"
