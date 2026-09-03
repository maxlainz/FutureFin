#!/usr/bin/env bash
# Siembra una instalación de DEMO con datos sintéticos y creíbles.
#
# Para qué: las capturas del README y de la documentación salen de aquí, nunca de una instalación
# real — una captura del Resumen es el patrimonio neto de alguien (skill `futurefin-data-hygiene`).
# También sirve para enseñar la app a alguien, o para probar un cambio contra datos con forma.
#
# TODOS los datos son inventados. Si reconoces alguno, es casualidad.
#
# Uso:
#   ./scripts/seed-demo.sh [URL_BASE] [USUARIO] [CONTRASEÑA]
#   ./scripts/seed-demo.sh                      # http://127.0.0.1:8080, demo / demo-futurefin-2026
#
# Requisitos: la instalación tiene que estar VACÍA (sin usuarios), porque el primer registro es el
# que crea el hogar y se convierte en propietario. Contra una instalación con datos, aborta.
set -euo pipefail

BASE="${1:-http://127.0.0.1:8080}"
USER="${2:-demo}"
PASS="${3:-demo-futurefin-2026}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

say() { printf '  %s\n' "$*"; }

api() { # api METHOD PATH [JSON]
  local method="$1" path="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -fsS -b "$JAR" -c "$JAR" -X "$method" "$BASE$path" \
      -H 'content-type: application/json' -d "$body"
  else
    curl -fsS -b "$JAR" -c "$JAR" -X "$method" "$BASE$path"
  fi
}

# `id` del primer objeto del array cuyo campo `name` coincide. jq no está garantizado en un Mac
# recién estrenado, así que se resuelve con python3, que sí.
pick_id() { # pick_id JSON NOMBRE
  python3 -c '
import json, sys
rows = json.loads(sys.argv[1])
name = sys.argv[2]
for r in rows:
    if r.get("name") == name:
        print(r["id"]); break
else:
    sys.exit(f"no encuentro la categoría {name!r}")
' "$1" "$2"
}

json_id() { python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }

# Fechas relativas a hoy, para que la demo no envejezca.
ymd() { python3 -c "
import datetime as d
print((d.date.today() + d.timedelta(days=int('$1'))).isoformat())
"; }
month_ago() { python3 -c "
import datetime as d
t = d.date.today().replace(day=15)
m = t.month - int('$1'); y = t.year
while m <= 0: m += 12; y -= 1
print(d.date(y, m, 15).isoformat())
"; }

echo "Sembrando datos de demo en $BASE"

if ! curl -fsS "$BASE/v1/health" >/dev/null 2>&1; then
  echo "ERROR: no responde $BASE/v1/health. ¿Está la app en marcha?" >&2
  exit 1
fi

# ── Propietario ───────────────────────────────────────────────────────────────
say "usuario $USER"
if ! api POST /v1/auth/register \
    "{\"username\":\"$USER\",\"password\":\"$PASS\",\"birth_date\":\"1990-06-15\"}" >/dev/null 2>&1; then
  echo "ERROR: no se ha podido registrar '$USER'." >&2
  echo "       La instalación debe estar vacía: el primer registro es el que crea el hogar." >&2
  exit 1
fi
api POST /v1/auth/login "{\"username\":\"$USER\",\"password\":\"$PASS\"}" >/dev/null

# ── Ajustes del hogar ─────────────────────────────────────────────────────────
say "ajustes: zona horaria, inflación y plan FIRE"
api PATCH /v1/installation '{
  "calendar_tz": "Europe/Madrid",
  "base_currency": "EUR",
  "annual_inflation_assumption_percent": "2.5",
  "onboarding_completed": true,
  "fire_settings": {
    "taxes_enabled": true,
    "tax_brackets": [
      {"up_to": "6000", "pct": "19"},
      {"up_to": "50000", "pct": "21"},
      {"up_to": "200000", "pct": "23"},
      {"up_to": "300000", "pct": "27"},
      {"up_to": null, "pct": "30"}
    ],
    "savings_source": "budget"
  }
}' >/dev/null

CATS="$(api GET /v1/categories)"
CAT_CUENTA="$(pick_id "$CATS" 'Cuenta corriente')"
CAT_AHORRO="$(pick_id "$CATS" 'Ahorro')"
CAT_INVERSION="$(pick_id "$CATS" 'Inversión')"
CAT_INMUEBLE="$(pick_id "$CATS" 'Inmuebles')"
CAT_HIPOTECA="$(pick_id "$CATS" 'Hipoteca')"
CAT_NOMINA="$(pick_id "$CATS" 'Nómina')"
CAT_VIVIENDA="$(pick_id "$CATS" 'Vivienda')"
CAT_SUPER="$(pick_id "$CATS" 'Supermercado')"
CAT_SUMIN="$(pick_id "$CATS" 'Suministros')"
CAT_TRANSPORTE="$(pick_id "$CATS" 'Transporte')"
CAT_OCIO="$(pick_id "$CATS" 'Ocio')"

# ── Activos ───────────────────────────────────────────────────────────────────

# 5.0.0: el plan de jubilación es de cada persona, no del hogar (perfil por usuario). Modo del
# objetivo, SWR y edad límite viven aquí; la pensión con fecha alimenta el objetivo puente y las
# bandas de Monte Carlo la leen. Cifras inventadas, como todo lo demás.
say "perfil de jubilación: estrategia, SWR y pensión con fecha"
api PATCH /v1/auth/me/retirement-profile '{
  "strategy": "asap",
  "fire_number_mode": "annual_expense",
  "swr_pct": "3.5",
  "horizon_lifespan_age": 90,
  "pension": {"monthly_amount_today": "1200", "starts_at_age": 67, "indexed": true}
}' >/dev/null
say "activos"
A_CUENTA="$(api POST /v1/assets "{\"category_id\":\"$CAT_CUENTA\",\"name\":\"Cuenta corriente\",\"current_value\":\"4200\",\"is_liquid\":true,\"expected_annual_return_percent\":\"0\"}" | json_id)"
api POST /v1/assets "{\"category_id\":\"$CAT_AHORRO\",\"name\":\"Cuenta remunerada\",\"current_value\":\"11500\",\"is_liquid\":true,\"expected_annual_return_percent\":\"2.25\"}" >/dev/null
api POST /v1/assets "{\"category_id\":\"$CAT_INVERSION\",\"name\":\"Fondo indexado global\",\"current_value\":\"38600\",\"purchase_price\":\"31000\",\"is_liquid\":true,\"expected_annual_return_percent\":\"7\",\"annual_volatility_percent\":\"17\"}" >/dev/null
api POST /v1/assets "{\"category_id\":\"$CAT_INVERSION\",\"name\":\"Plan de pensiones\",\"current_value\":\"9800\",\"purchase_price\":\"9000\",\"is_liquid\":false,\"expected_annual_return_percent\":\"5.5\",\"annual_volatility_percent\":\"10\"}" >/dev/null
api POST /v1/assets "{\"category_id\":\"$CAT_INMUEBLE\",\"name\":\"Vivienda habitual\",\"current_value\":\"210000\",\"purchase_price\":\"185000\",\"is_liquid\":false,\"expected_annual_return_percent\":\"2\"}" >/dev/null

# ── Pasivos ───────────────────────────────────────────────────────────────────
say "pasivos"
END_HIPOTECA="$(python3 -c "
import datetime as d
print(d.date(d.date.today().year + 18, 6, 1).isoformat())
")"
api POST /v1/liabilities "{\"category_id\":\"$CAT_HIPOTECA\",\"expense_category_id\":\"$CAT_VIVIENDA\",\"label\":\"Hipoteca vivienda\",\"principal\":\"126400\",\"apr_percent\":\"2.35\",\"payment_amount\":\"742\",\"payment_frequency\":\"monthly\",\"payment_end_date\":\"$END_HIPOTECA\",\"repayment_model\":\"french\",\"derive_principal_from_plan\":false}" >/dev/null

# ── Presupuesto ───────────────────────────────────────────────────────────────
say "presupuesto"
# La cuota de la hipoteca (742 €) NO va aquí: sale del pasivo y el motor la suma sola. Con estos
# números el hogar ahorra ~25 % de lo que ingresa, que es una cifra creíble; una demo que ahorra el
# 40 % enseña una proyección demasiado bonita para ser útil como ejemplo.
api POST /v1/budget/entries "{\"category_id\":\"$CAT_NOMINA\",\"amount\":\"2650\"}" >/dev/null
api POST /v1/budget/entries "{\"category_id\":\"$CAT_SUPER\",\"amount\":\"520\"}" >/dev/null
api POST /v1/budget/entries "{\"category_id\":\"$CAT_VIVIENDA\",\"amount\":\"165\"}" >/dev/null
api POST /v1/budget/entries "{\"category_id\":\"$CAT_SUMIN\",\"amount\":\"175\"}" >/dev/null
api POST /v1/budget/entries "{\"category_id\":\"$CAT_TRANSPORTE\",\"amount\":\"130\"}" >/dev/null
api POST /v1/budget/entries "{\"category_id\":\"$CAT_OCIO\",\"amount\":\"260\"}" >/dev/null

# ── Movimientos previstos ─────────────────────────────────────────────────────
say "movimientos previstos"
api POST /v1/planning/flows "{\"category_id\":\"$CAT_OCIO\",\"title\":\"Viaje de verano\",\"expected_amount\":\"1200\",\"due_date\":\"$(ymd 95)\"}" >/dev/null
api POST /v1/planning/flows "{\"category_id\":\"$CAT_TRANSPORTE\",\"title\":\"Revisión del coche\",\"expected_amount\":\"380\",\"due_date\":\"$(ymd 40)\"}" >/dev/null

# ── Movimientos reales (cuatro meses) ─────────────────────────────────────────
say "movimientos de los últimos meses"
for m in 3 2 1 0; do
  fecha="$(month_ago "$m")"
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Nómina\",\"amount\":\"2650\",\"kind\":\"income\",\"category_id\":\"$CAT_NOMINA\",\"linked_asset_id\":\"$A_CUENTA\"}" >/dev/null
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Compra semanal\",\"amount\":\"-$((505 + m * 14))\",\"kind\":\"expense\",\"category_id\":\"$CAT_SUPER\"}" >/dev/null
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Luz y agua\",\"amount\":\"-$((168 + m * 6))\",\"kind\":\"expense\",\"category_id\":\"$CAT_SUMIN\"}" >/dev/null
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Transporte público\",\"amount\":\"-128\",\"kind\":\"expense\",\"category_id\":\"$CAT_TRANSPORTE\"}" >/dev/null
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Cine y restaurantes\",\"amount\":\"-$((235 + m * 18))\",\"kind\":\"expense\",\"category_id\":\"$CAT_OCIO\"}" >/dev/null
  api POST /v1/transactions "{\"op_date\":\"$fecha\",\"concept\":\"Aportación al fondo\",\"amount\":\"-400\",\"kind\":\"savings\"}" >/dev/null
done

# ── Histórico ─────────────────────────────────────────────────────────────────
# Un snapshot por `kind`: la API los guarda por separado (un pasivo lleva TAE y cuota, un activo
# no). El JSON se genera con python leyendo argv — incrustarlo en la cadena del shell mezcla las
# comillas de los dos lenguajes y se rompe de formas creativas.
snapshot_json() { # snapshot_json KIND FECHA MESES_ATRAS
  python3 - "$1" "$2" "$3" <<'PY'
import json, sys

kind, fecha, meses = sys.argv[1], sys.argv[2], int(sys.argv[3])
# El patrimonio crece ~1,2 % al mes hacia hoy; la hipoteca baja 620 €/mes.
f = 1 - meses * 0.012

if kind == "asset":
    items = [
        {"label": "Cuenta corriente", "value": str(round(4200 * f, 2))},
        {"label": "Cuenta remunerada", "value": str(round(11500 * f, 2))},
        {"label": "Fondo indexado global", "value": str(round(38600 * f, 2))},
        {"label": "Plan de pensiones", "value": str(round(9800 * f, 2))},
        {"label": "Vivienda habitual", "value": "210000"},
    ]
else:
    items = [
        {
            "label": "Hipoteca vivienda",
            "value": str(round(126400 + meses * 620, 2)),
            "apr_percent": "2.35",
            "payment_amount": "742",
            "payment_frequency": "monthly",
        }
    ]

print(json.dumps({"kind": kind, "snapshot_date": fecha, "items": items}))
PY
}

say "snapshots del histórico"
for m in 12 9 6 3; do
  fecha="$(month_ago "$m")"
  api POST /v1/history/snapshots "$(snapshot_json asset "$fecha" "$m")" >/dev/null
  api POST /v1/history/snapshots "$(snapshot_json liability "$fecha" "$m")" >/dev/null
done

echo
echo "Listo. Entra en $BASE con:"
echo "  usuario:    $USER"
echo "  contraseña: $PASS"
