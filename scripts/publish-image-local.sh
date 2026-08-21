#!/usr/bin/env bash
# Publica la imagen multi-arch DESDE ESTA MÁQUINA, sin gastar minutos de GitHub Actions.
#
# Uso:
#   ./scripts/publish-image-local.sh                 # versión = la de apps/api/Cargo.toml
#   ./scripts/publish-image-local.sh 3.8.1           # versión explícita
#   ./scripts/publish-image-local.sh --dry-run       # enseña qué haría y no publica
#   ./scripts/publish-image-local.sh --yes           # sin confirmación interactiva
#
# Por qué existe: `publish-image.yml` construye en runners amd64, así que la mitad arm64 va
# EMULADA bajo QEMU — de ahí que un release tarde ~1,5-2 h y se coma el 66 % de la cuota mensual
# (958 de 1460 min en agosto de 2026). El cómputo real del build son ~3,5 min; el resto es
# emulación y descarga de bases. En local con containerd image store se publican las dos
# arquitecturas de una vez y el gasto en Actions es CERO.
#
# Hace exactamente lo mismo que el workflow, para que no haya deriva:
#   - mismas dos registries (GHCR + Docker Hub) y mismos tags (:X.Y.Z, :X.Y, :X, :latest)
#   - mismo Release de GitHub, con las notas sacadas del CHANGELOG por `changelog-section.sh`
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

DRY_RUN=0
ASSUME_YES=0
VERSION=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --yes|-y)  ASSUME_YES=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    -*)        echo "opción desconocida: $arg" >&2; exit 2 ;;
    *)         VERSION="$arg" ;;
  esac
done

die() { echo "ERROR: $*" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

# --- 1. Versión y coherencia con el árbol -------------------------------------
cargo_version="$(grep -m1 '^version' apps/api/Cargo.toml | cut -d'"' -f2)"
VERSION="${VERSION:-$cargo_version}"
VERSION="${VERSION#v}"
TAG="v$VERSION"

case "$VERSION" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) die "versión no semver: '$VERSION'" ;;
esac

[ "$VERSION" = "$cargo_version" ] || die \
  "pediste $VERSION pero apps/api/Cargo.toml está en $cargo_version. La imagen sirve la versión
       del Cargo.toml del árbol que construyes: publicar con otro tag mentiría sobre su contenido."

# El CHANGELOG es la fuente de las notas del Release; si falta, mejor pararse ahora que a mitad.
./scripts/changelog-section.sh "$VERSION" >/dev/null \
  || die "CHANGELOG.md no tiene sección para $VERSION"

[ -z "$(git status --porcelain)" ] || die \
  "el árbol de trabajo tiene cambios sin commitear — publicarías algo que no está en git"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
  || die "el tag $TAG no existe. Créalo desde main antes de publicar (CLAUDE.md §Releases)."

tag_sha="$(git rev-parse "$TAG^{commit}")"
head_sha="$(git rev-parse HEAD)"
[ "$tag_sha" = "$head_sha" ] || die \
  "HEAD ($(git rev-parse --short HEAD)) no es el commit de $TAG ($(git rev-parse --short "$TAG")).
       Haz 'git checkout $TAG' para construir exactamente lo tagueado."

# --- 2. ¿Este tag es el más nuevo? Decide si mueve :latest ---------------------
newest="$(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' | sed 's/^v//' | sort -V | tail -1)"
if [ "$VERSION" = "$newest" ]; then
  MOVE_LATEST=1
else
  MOVE_LATEST=0
  echo "AVISO: $VERSION no es la versión más alta ($newest) → NO se moverá :latest."
fi

# --- 3. Tags a publicar (mismos que docker/metadata-action en el workflow) -----
major="${VERSION%%.*}"
rest="${VERSION#*.}"
minor="${rest%%.*}"

IMAGES=("ghcr.io/maxlainz/futurefin" "docker.io/maxlainz/futurefin")
TAG_ARGS=()
for img in "${IMAGES[@]}"; do
  TAG_ARGS+=(-t "$img:$VERSION" -t "$img:$major.$minor" -t "$img:$major")
  [ "$MOVE_LATEST" -eq 1 ] && TAG_ARGS+=(-t "$img:latest")
done

step "Publicación local de FutureFin $TAG"
cat <<EOF
  commit      : $(git rev-parse --short HEAD)
  plataformas : linux/amd64, linux/arm64
  registries  : ${IMAGES[*]}
  tags        : $VERSION, $major.$minor, $major$([ "$MOVE_LATEST" -eq 1 ] && echo ", latest")
  Release     : se creará si no existe, con las notas del CHANGELOG
EOF

if [ "$DRY_RUN" -eq 1 ]; then
  echo
  echo "(--dry-run: no se publica nada)"
  printf '  docker buildx build --platform linux/amd64,linux/arm64 --push \\\n'
  printf '    %s \\\n' "${TAG_ARGS[@]}"
  printf '    -f apps/api/Dockerfile .\n'
  exit 0
fi

if [ "$ASSUME_YES" -eq 0 ]; then
  printf '\n¿Publicar? Esto sube imágenes públicas a dos registries. [escribe "si"]: '
  read -r answer
  [ "$answer" = "si" ] || { echo "cancelado"; exit 1; }
fi

# --- 4. Build multi-arch + push ----------------------------------------------
# Requiere el containerd image store de Docker Desktop (Settings → General). Sin él, el driver
# `docker` no sabe producir un índice multi-plataforma y el push falla con "multiple platforms
# feature is currently not supported".
step "Construyendo y publicando (amd64 emulado sobre arm64: es el paso lento)"
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --push \
  "${TAG_ARGS[@]}" \
  -f apps/api/Dockerfile \
  .

# --- 5. Verificar lo publicado, no solo que el comando saliera 0 --------------
step "Verificando el manifiesto publicado"
for img in "${IMAGES[@]}"; do
  echo "--- $img:$VERSION"
  docker buildx imagetools inspect "$img:$VERSION" \
    | grep -E 'Name:|Platform:|MediaType: application/vnd.oci.image.index' || true
done

# --- 6. GitHub Release (lo mismo que hace el workflow) ------------------------
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  if gh release view "$TAG" >/dev/null 2>&1; then
    step "El Release $TAG ya existe — no lo toco"
  else
    step "Creando el GitHub Release $TAG"
    notes="$(mktemp)"
    trap 'rm -f "$notes"' EXIT
    ./scripts/changelog-section.sh "$VERSION" > "$notes"
    if [ "$MOVE_LATEST" -eq 1 ]; then
      gh release create "$TAG" --title "$TAG" --notes-file "$notes" --verify-tag --latest
    else
      gh release create "$TAG" --title "$TAG" --notes-file "$notes" --verify-tag --latest=false
    fi
  fi
else
  echo "AVISO: sin 'gh' autenticado — el GitHub Release NO se ha creado."
fi

step "Listo: $TAG publicado sin gastar minutos de Actions"
