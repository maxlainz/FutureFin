#!/usr/bin/env bash
# Arranca Postgres + API con Docker Compose de forma reproducible.
# Por defecto hace `docker compose down --remove-orphans` antes de `up`, para evitar
# errores del tipo «No such container» cuando quedan referencias rotas en el daemon.
#
# Uso (desde la raíz del repo):
#   ./scripts/docker-stack-up.sh           # down limpio + up
#   ./scripts/docker-stack-up.sh --build  # rebuild imagen API + up
#   ./scripts/docker-stack-up.sh --no-down # solo up (rápido; puede fallar si el estado está corrupto)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD=0
NO_DOWN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build | -b) BUILD=1 ;;
    --no-down) NO_DOWN=1 ;;
    -h | --help)
      echo "Usage: $0 [--build|-b] [--no-down]"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
  shift
done

if [[ "$NO_DOWN" -eq 0 ]]; then
  echo "docker compose down --remove-orphans"
  docker compose down --remove-orphans
fi

if [[ "$BUILD" -eq 1 ]]; then
  echo "docker compose build futurefin-api"
  docker compose build futurefin-api
elif [[ -z "$(docker images -q futurefin/futurefin-api:dev 2>/dev/null)" ]]; then
  # Sin imagen local, algunos Compose intentan pull de Docker Hub (repo inexistente) y fallan
  # antes de hacer build; forzamos build explícito.
  echo "Imagen futurefin/futurefin-api:dev ausente; docker compose build futurefin-api"
  docker compose build futurefin-api
fi

echo "docker compose up -d"
docker compose up -d

echo "Waiting for http://127.0.0.1:8080/v1/health ..."
for i in $(seq 1 120); do
  if curl -sf "http://127.0.0.1:8080/v1/health" >/dev/null; then
    echo "OK (after ${i}s)"
    curl -sf "http://127.0.0.1:8080/v1/health"
    echo
    exit 0
  fi
  sleep 1
done

echo "Timeout waiting for API. Recent logs:" >&2
docker compose logs --tail=60 futurefin-api >&2 || true
exit 1
