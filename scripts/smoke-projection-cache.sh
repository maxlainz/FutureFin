#!/usr/bin/env bash
# Smoke E2E del cache de proyección.
# Asume API corriendo en localhost:8080 (Docker compose o cargo run).
#
# Si la BD del Docker ya tiene un installation con usuarios reales (caso típico
# en local), pasa SMOKE_USER y SMOKE_PASS con credenciales válidas:
#   SMOKE_USER=alice SMOKE_PASS=... bash scripts/smoke-projection-cache.sh
#
# Si la BD está virgen, registra un usuario throwaway y hace setup automático.

set -euo pipefail
BASE="${BASE:-http://127.0.0.1:8080}"
JAR=$(mktemp)
trap 'rm -f "$JAR"' EXIT

if [[ -n "${SMOKE_USER:-}" && -n "${SMOKE_PASS:-}" ]]; then
  USER="$SMOKE_USER"
  PASS="$SMOKE_PASS"
  echo "→ Login como usuario existente $USER"
  curl -sf -c "$JAR" -X POST "$BASE/v1/auth/login" \
    -H "content-type: application/json" \
    -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}" >/dev/null
else
  USER="smoke_$(date +%s)_$$"
  PASS="smoke-pass-1234"
  echo "→ Register $USER (BD debe estar virgen — si falla, pasa SMOKE_USER/SMOKE_PASS)"
  curl -sf -c "$JAR" -X POST "$BASE/v1/auth/register" \
    -H "content-type: application/json" \
    -d "{\"username\":\"$USER\",\"password\":\"$PASS\",\"birth_date\":\"1990-01-01\"}" >/dev/null
  echo "→ Login"
  curl -sf -c "$JAR" -X POST "$BASE/v1/auth/login" \
    -H "content-type: application/json" \
    -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}" >/dev/null
  echo "→ Setup installation (skipea si ya existe)"
  curl -s -b "$JAR" -X POST "$BASE/v1/installation/setup" \
    -H "content-type: application/json" \
    -d '{"base_currency":"EUR","calendar_tz":"Europe/Madrid"}' >/dev/null || true
fi

# Esperar warm-up (background tokio::spawn). El compute del engine en dev
# puede tardar 200-500 ms en máquina lenta; damos margen suficiente.
WARMUP_MS=1500
echo "→ Esperando warm-up ${WARMUP_MS}ms…"
sleep "$(awk "BEGIN{print $WARMUP_MS/1000}")"

echo "→ GET density=hybrid (esperado ~5 KB, render rápido)"
for i in 1 2; do
  t=$(curl -sf -b "$JAR" -o /dev/null -w "%{time_total}\n" \
        "$BASE/v1/projection/series?density=hybrid")
  ms=$(awk "BEGIN{printf \"%d\", $t*1000}")
  printf "  hybrid #%d: %s ms\n" "$i" "$ms"
done

echo "→ GET density=monthly (full ~30 KB)"
for i in 1 2; do
  t=$(curl -sf -b "$JAR" -o /dev/null -w "%{time_total}\n" \
        "$BASE/v1/projection/series")
  ms=$(awk "BEGIN{printf \"%d\", $t*1000}")
  printf "  monthly #%d: %s ms\n" "$i" "$ms"
done

echo
echo "→ Mutación (crear asset, debería invalidar cache)"
# Buscar o crear categoría "Cash" (asset). Si ya existe, 409 conflict → reusa
# la que está. Lee la lista y filtra por nombre.
CAT_NAME="Smoke Cache Cash"
CREATE_RESP=$(curl -s -b "$JAR" -X POST "$BASE/v1/categories" \
  -H "content-type: application/json" \
  -d "{\"scope\":\"asset\",\"name\":\"$CAT_NAME\"}")
CAT_ID=$(echo "$CREATE_RESP" | python3 -c "import sys, json
try:
    print(json.load(sys.stdin).get('id', ''))
except Exception:
    print('')")
if [[ -z "$CAT_ID" ]]; then
  # No se pudo crear (probablemente ya existe); buscarla en la lista.
  CAT_ID=$(curl -sf -b "$JAR" "$BASE/v1/categories" | python3 -c "import sys, json
cats = json.load(sys.stdin)
for c in cats:
    if c.get('scope') == 'asset' and c.get('name') == '$CAT_NAME':
        print(c['id']); break")
fi
if [[ -z "$CAT_ID" ]]; then
  echo "  ! no se pudo obtener category_id; abortando mutación"
  echo "done"
  exit 0
fi

curl -sf -b "$JAR" -X POST "$BASE/v1/assets" \
  -H "content-type: application/json" \
  -d "{\"category_id\":\"$CAT_ID\",\"name\":\"Smoke Cache Asset $(date +%s)\",\"current_value\":\"10000\"}" >/dev/null

# Pequeña espera para que el tokio::spawn de invalidación termine.
sleep 0.2

echo "→ GET tras invalidación (esperado MISS, luego HIT)"
for i in 1 2; do
  t=$(curl -sf -b "$JAR" -o /dev/null -w "%{time_total}\n" \
        "$BASE/v1/projection/series")
  ms=$(awk "BEGIN{printf \"%d\", $t*1000}")
  printf "  GET #%d: %s ms\n" "$i" "$ms"
done

echo
echo "→ Logout"
curl -sf -b "$JAR" -X POST "$BASE/v1/auth/logout" >/dev/null
echo "done"
