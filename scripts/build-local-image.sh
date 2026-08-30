#!/usr/bin/env bash
# Construye la imagen local y levanta el stack completo de producción con ella — el drill A de
# futurefin-change-control §4.2, con nombre citable (decisión Q1, consolidación 2026-08-30).
#
# Requisitos en .env:  FUTUREFIN_IMAGE=futurefin-local  y  FUTUREFIN_TAG=dev
# TRAMPA cubierta por los compose: ningún fichero declara env_file ni DATABASE_URL, así que la
# DATABASE_URL de desarrollo del .env NO llega al contenedor (docs/configuracion.md, «Cuidado con
# este pie»). No la inyectes tú con -e.
set -euo pipefail
cd "$(dirname "$0")/.."

# --load es obligatorio con BuildKit para que la imagen quede en el store local de Docker.
docker build --load -f apps/api/Dockerfile -t futurefin-local:dev .

docker compose -f docker-compose.yml -f docker-compose.local.yml --env-file .env up -d

for _ in $(seq 1 40); do
  if curl -sf http://127.0.0.1:8080/v1/ready >/dev/null 2>&1; then
    echo "build-local-image: /v1/ready OK — versión servida: $(curl -s http://127.0.0.1:8080/v1/health)"
    echo "Siguientes pasos del drill (change-control §4.2 A): click por la app en claro Y oscuro,"
    echo "y el apagado limpio:  docker compose stop -t 60 futurefin  (exit code debe ser 0)."
    exit 0
  fi
  sleep 4
done

echo "build-local-image: /v1/ready no respondió — mira 'docker compose logs futurefin'" >&2
exit 1
