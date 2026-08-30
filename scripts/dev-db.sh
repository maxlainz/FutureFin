#!/usr/bin/env bash
# Levanta (o reutiliza) el Postgres de DESARROLLO de docker-compose.dev.yml en 127.0.0.1:5432 y
# espera a que acepte conexiones — el prerequisito de `cargo run`. Nombre citable para que los
# docs no dupliquen el bloque (decisión Q1, consolidación 2026-08-30).
set -euo pipefail
cd "$(dirname "$0")/.."

docker compose -f docker-compose.dev.yml up -d

for _ in $(seq 1 30); do
  if docker compose -f docker-compose.dev.yml exec -T db pg_isready -U futurefin >/dev/null 2>&1; then
    echo "dev-db: PostgreSQL listo en 127.0.0.1:5432"
    exit 0
  fi
  sleep 2
done

echo "dev-db: PostgreSQL no respondió tras 60s — mira 'docker compose -f docker-compose.dev.yml logs'" >&2
exit 1
