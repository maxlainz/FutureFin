#!/usr/bin/env bash
set -euo pipefail

# Restaura un backup .sql.gz (de backup-postgres.sh o de los automáticos pre-migración)
# en el stack de producción (contenedor único desde 3.0.0).
#
# Cómo funciona: para el servicio normal, arranca un contenedor temporal en modo
# `db-only` (solo PostgreSQL, sin API), restaura por socket, lo para limpiamente y
# vuelve a levantar el stack — que re-aplica migraciones si el dump era de una
# versión anterior.
#
# Uso (desde la raíz del repo):
#   ./scripts/restore-postgres.sh backups/futurefin-postgres-<ts>.sql.gz [--yes]
#
# Variables opcionales: SERVICE=futurefin  ENV_FILE=  POSTGRES_USER/DB=futurefin

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SERVICE="${SERVICE:-futurefin}"
ENV_FILE="${ENV_FILE:-}"
POSTGRES_USER="${POSTGRES_USER:-futurefin}"
POSTGRES_DB="${POSTGRES_DB:-futurefin}"
RESTORE_NAME=futurefin-restore
SOCK=/var/run/postgresql

DUMP="${1:-}"
CONFIRM="${2:-}"

if [[ -z "$DUMP" || ! -f "$DUMP" ]]; then
  echo "Uso: $0 <backup.sql.gz> [--yes]" >&2
  exit 1
fi

compose() {
  if [[ -n "$ENV_FILE" ]]; then
    docker compose --env-file "$ENV_FILE" "$@"
  else
    docker compose "$@"
  fi
}

if [[ "$CONFIRM" != "--yes" ]]; then
  echo "Esto BORRARÁ la base '$POSTGRES_DB' actual y la sustituirá por: $DUMP"
  read -r -p "¿Continuar? (escribe 'si'): " answer
  [[ "$answer" == "si" ]] || { echo "Cancelado."; exit 1; }
fi

# Llaves obligatorias antes de «…»: el bash 3.2 de macOS traga los bytes del
# carácter multibyte dentro del nombre de la variable ("SERVICE…: unbound
# variable" con set -u) y aborta el restore antes de tocar nada.
echo "1/6 Parando el servicio ${SERVICE}…"
compose stop "$SERVICE"

echo "2/6 Arrancando PostgreSQL en modo rescate (db-only)…"
docker rm -f "$RESTORE_NAME" >/dev/null 2>&1 || true
compose run -d --rm --no-deps --name "$RESTORE_NAME" \
  -e FUTUREFIN_MODE=db-only "$SERVICE" >/dev/null

echo -n "   esperando a PostgreSQL"
for _ in $(seq 1 60); do
  if docker exec "$RESTORE_NAME" pg_isready -q -h "$SOCK" -U "$POSTGRES_USER" 2>/dev/null; then
    break
  fi
  echo -n "."; sleep 1
done
echo
docker exec "$RESTORE_NAME" pg_isready -q -h "$SOCK" -U "$POSTGRES_USER" \
  || { echo "PostgreSQL no llegó a estar listo — abortando (nada se ha borrado)"; docker stop "$RESTORE_NAME" >/dev/null; exit 1; }

echo "   censo ANTES:"
docker exec "$RESTORE_NAME" psql -X -tA -h "$SOCK" -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  -c "SELECT count(*) || ' filas en ' || count(distinct table_name) || ' tablas (aprox)' FROM information_schema.columns WHERE table_schema='public'" 2>/dev/null || echo "   (base vacía o inexistente)"

echo "3/6 Recreando la base ${POSTGRES_DB}…"
docker exec "$RESTORE_NAME" psql -X -h "$SOCK" -U "$POSTGRES_USER" -d postgres -v ON_ERROR_STOP=1 \
  -c "DROP DATABASE IF EXISTS \"$POSTGRES_DB\";" \
  -c "CREATE DATABASE \"$POSTGRES_DB\" OWNER \"$POSTGRES_USER\";"

echo "4/6 Restaurando el dump…"
gunzip -c "$DUMP" | docker exec -i "$RESTORE_NAME" \
  psql -X -q -h "$SOCK" -U "$POSTGRES_USER" -d "$POSTGRES_DB" -v ON_ERROR_STOP=1

echo "   censo DESPUÉS:"
docker exec "$RESTORE_NAME" psql -X -tA -h "$SOCK" -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  -c "SELECT string_agg(table_name || ':' || (xpath('/row/cnt/text()', query_to_xml('SELECT count(*) AS cnt FROM \"' || table_name || '\"', false, true, '')))[1]::text::text, ' ') FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE'"

echo "5/6 Parando el contenedor de rescate (apagado limpio)…"
docker stop "$RESTORE_NAME" >/dev/null

echo "6/6 Levantando el stack…"
compose up -d "$SERVICE"

echo -n "   esperando /v1/ready"
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${APP_PORT:-8080}/v1/ready" >/dev/null 2>&1; then
    echo; echo "Restore COMPLETADO. Verifica con: docker compose logs $SERVICE | grep 'migrations applied'"
    exit 0
  fi
  echo -n "."; sleep 2
done
echo
echo "El stack no respondió a /v1/ready en 120s — revisa: docker compose logs $SERVICE" >&2
exit 1
