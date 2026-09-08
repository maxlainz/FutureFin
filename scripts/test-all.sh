#!/usr/bin/env bash
# La puerta local completa, con nombre citable: los mismos gates que CI (jobs rust/web/integration)
# en el orden del checklist de futurefin-change-control §6. Los docs citan ESTE script en vez de
# duplicar el bloque de comandos (decisión Q1 de la consolidación 2026-08-30: un comando ejecutable
# no puede derivar — o corre o falla).
#
# Uso:   ./scripts/test-all.sh            # todo (necesita la DB de test en :5433)
#        SKIP_DB=1 ./scripts/test-all.sh  # solo los gates sin base de datos
#
# TEST_DATABASE_URL es sobreescribible; el default es el ff-test-db documentado.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${TEST_DATABASE_URL:=postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test}"

step() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

step "cargo build -p futurefin-api --locked"
cargo build -p futurefin-api --locked

step "cargo test -p futurefin-engine"
cargo test -p futurefin-engine

# La puerta de degeneración (`crates/engine-stochastic`) no necesita base de datos y es el gate que
# dice que el camino de coma flotante y el exacto siguen siendo la MISMA simulación. Corre aquí y
# no solo dentro de `cargo test --workspace`, para que también la vea `SKIP_DB=1`.
step "cargo test -p futurefin-engine-stochastic"
cargo test -p futurefin-engine-stochastic

if [ "${SKIP_DB:-0}" = "1" ]; then
  step "integración SALTADA (SKIP_DB=1)"
else
  # Aviso temprano y accionable si la DB de test no está: el fallo de sqlx tarda y confunde.
  if ! (exec 3<>/dev/tcp/127.0.0.1/5433) 2>/dev/null; then
    echo "ERROR: no hay Postgres de test en 127.0.0.1:5433." >&2
    echo "Arráncalo una vez (futurefin-validation-and-qa §2 / docs/desarrollo.md):" >&2
    echo "  docker run -d --name ff-test-db --shm-size=1g -e POSTGRES_USER=futurefin \\" >&2
    echo "    -e POSTGRES_PASSWORD=futurefin_test -e POSTGRES_DB=futurefin_test \\" >&2
    echo "    -p 5433:5432 postgres:16.4-alpine" >&2
    echo "O corre solo los gates sin DB:  SKIP_DB=1 ./scripts/test-all.sh" >&2
    exit 2
  fi
  exec 3>&- || true
  step "cargo test --workspace (integración contra ${TEST_DATABASE_URL%%@*}@…)"
  TEST_DATABASE_URL="$TEST_DATABASE_URL" cargo test --workspace
fi

step "npm run typecheck:web"
npm run typecheck:web

step "npm run lint:web"
npm run lint:web

step "npm test --workspace futurefin-web"
npm test --workspace futurefin-web

step "npm run build:web"
npm run build:web

step "test-all: todo en verde"
