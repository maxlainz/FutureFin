#!/usr/bin/env bash
set -euo pipefail

# Backup de PostgreSQL del stack de producción (contenedor único desde 3.0.0):
# ejecuta pg_dump DENTRO del contenedor `futurefin` vía socket Unix y lo comprime al host.
#
# Uso (desde la raíz del repo):
#   ./scripts/backup-postgres.sh
#
# Variables opcionales:
#   SERVICE=futurefin        # nombre del servicio compose
#   ENV_FILE=                # --env-file para compose (ya no es obligatorio)
#   BACKUP_DIR=./backups
#   KEEP_BACKUPS=30
#   POSTGRES_USER=futurefin
#   POSTGRES_DB=futurefin
#
# Nota: los backups AUTOMÁTICOS pre-migración viven dentro del volumen ffdata
# (/var/lib/futurefin/backups). Para copiarlos al host:
#   docker compose cp futurefin:/var/lib/futurefin/backups ./backups-auto

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SERVICE="${SERVICE:-futurefin}"
ENV_FILE="${ENV_FILE:-}"
BACKUP_DIR="${BACKUP_DIR:-./backups}"
KEEP_BACKUPS="${KEEP_BACKUPS:-30}"
POSTGRES_USER="${POSTGRES_USER:-futurefin}"
POSTGRES_DB="${POSTGRES_DB:-futurefin}"

compose() {
  if [[ -n "$ENV_FILE" ]]; then
    docker compose --env-file "$ENV_FILE" "$@"
  else
    docker compose "$@"
  fi
}

if ! compose ps --status running "$SERVICE" 2>/dev/null | grep -q "$SERVICE"; then
  echo "El servicio '$SERVICE' no está corriendo. Arráncalo con: docker compose up -d" >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR"

ts="$(date -u +"%Y%m%dT%H%M%SZ")"
outfile="${BACKUP_DIR}/futurefin-postgres-${ts}.sql.gz"

echo "Creando backup PostgreSQL -> $outfile"

# -T: no asigna TTY (evita errores en CI / pipes). pg_dump va dentro de la imagen 3.x.
compose exec -T "$SERVICE" \
  pg_dump -h /var/run/postgresql -U "$POSTGRES_USER" -d "$POSTGRES_DB" | gzip -9 > "$outfile"

echo "Backup OK ($(du -h "$outfile" | cut -f1))"

echo "Retención: conservar últimos ${KEEP_BACKUPS} backups"
# `mapfile` es de bash 4+: en el bash 3.2 que trae macOS la retención moría con
# "mapfile: command not found" y los backups viejos se acumulaban en silencio.
files=()
while IFS= read -r f; do
  [[ -n "$f" ]] && files+=("$f")
done < <(ls -1t "${BACKUP_DIR}"/futurefin-postgres-*.sql.gz 2>/dev/null || true)
if (( ${#files[@]} > KEEP_BACKUPS )); then
  for ((i=KEEP_BACKUPS; i<${#files[@]}; i++)); do
    rm -f "${files[$i]}"
  done
fi

echo "Retención OK"
