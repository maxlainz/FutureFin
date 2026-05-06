#!/usr/bin/env bash
set -euo pipefail

# Backup de PostgreSQL usando el stack de producción (Compose).
#
# Uso (desde la raíz del repo):
#   ./scripts/backup-postgres.sh
#
# Variables opcionales:
#   ENV_FILE=.env.prod
#   BACKUP_DIR=./backups
#   KEEP_BACKUPS=30

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENV_FILE="${ENV_FILE:-.env.prod}"
BACKUP_DIR="${BACKUP_DIR:-./backups}"
KEEP_BACKUPS="${KEEP_BACKUPS:-30}"

get_env() {
  # Lee KEY=VALUE desde el fichero ENV_FILE sin intentar "source" (para evitar fallar con placeholders tipo <...>).
  local key="$1"
  awk -F= -v k="$key" '$1 == k {sub(/^[ \t]+/, "", $2); print $2; exit}' "$ENV_FILE"
}

POSTGRES_USER="$(get_env "POSTGRES_USER")"
POSTGRES_DB="$(get_env "POSTGRES_DB")"

if [[ -z "$POSTGRES_USER" || -z "$POSTGRES_DB" ]]; then
  echo "Faltan POSTGRES_USER/POSTGRES_DB en $ENV_FILE" >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR"

ts="$(date -u +"%Y%m%dT%H%M%SZ")"
outfile="${BACKUP_DIR}/futurefin-postgres-${ts}.sql.gz"

echo "Creando backup PostgreSQL -> $outfile"

# -T: no asigna TTY (evita errores en CI / pipes)
# pg_dump existe en la imagen oficial de postgres.
docker compose --env-file "$ENV_FILE" -f docker-compose.prod.yml exec -T futurefin-database \
  pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" | gzip -9 > "$outfile"

echo "Backup OK"

echo "Retención: conservar últimos ${KEEP_BACKUPS} backups"
mapfile -t files < <(ls -1t "${BACKUP_DIR}"/futurefin-postgres-*.sql.gz 2>/dev/null || true)
if (( ${#files[@]} > KEEP_BACKUPS )); then
  for ((i=KEEP_BACKUPS; i<${#files[@]}; i++)); do
    rm -f "${files[$i]}"
  done
fi

echo "Retención OK"

