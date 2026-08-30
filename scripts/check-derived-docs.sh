#!/usr/bin/env bash
# Verificador de hechos de las dos superficies documentales DERIVADAS que se publican fuera de
# GitHub y que nadie barre al cambiar la fuente (decisión Q4, consolidación 2026-08-30):
#
#   .github/dockerhub-README.md   → portada de Docker Hub (la publica dockerhub-description.yml)
#   addon/futurefin/DOCS.md       → lo que el usuario lee DENTRO de Home Assistant
#
# Son copias curadas (el tono difiere de sus fuentes a propósito); lo que NO puede divergir son
# los HECHOS: el compose de ejemplo frente al docker-compose.yml real, y la tabla de opciones
# frente al schema del add-on. El incidente que motiva esto: el PR template afirmó durante meses
# que CI no corría la integración — una superficie sin dueño ni barrido miente en silencio.
# Falla con mensaje accionable; CI lo ejecuta (job secrets-scan) y también vale en local.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
err() { echo "check-derived-docs: $*" >&2; fail=1; }

DH=.github/dockerhub-README.md
AD=addon/futurefin/DOCS.md
CY=addon/futurefin/config.yaml

# ── 1. El compose de ejemplo de Docker Hub dice lo mismo que docker-compose.yml ──────────────
# Campos que un self-hoster copia tal cual; si el real cambia, el ejemplo debe cambiar en el
# mismo PR. Se comparan los valores del LADO CONTENEDOR (los ${VAR:-…} del real se normalizan).
compose_fact() { # $1 = etiqueta, $2 = patrón en el compose real, $3 = patrón esperado en DH
  grep -qE "$2" docker-compose.yml || { err "el patrón «$2» ya no está en docker-compose.yml — actualiza este script"; return; }
  grep -qE "$3" "$DH" || err "docker-compose.yml dice «$1» y $DH no lo refleja (esperaba /$3/)"
}
compose_fact "stop_grace_period 60s"      '^\s*stop_grace_period: 60s' 'stop_grace_period: 60s'
compose_fact "puerto contenedor 8080"     ':8080"?$'                   '"8080:8080"'
compose_fact "volumen pgdata"             'pgdata:/var/lib/postgresql/data' 'pgdata:/var/lib/postgresql/data'
compose_fact "volumen ffdata"             'ffdata:/var/lib/futurefin'  'ffdata:/var/lib/futurefin'
compose_fact "healthcheck /v1/ready"      '/v1/ready'                  '/v1/ready'
for kv in "interval: 15s" "timeout: 5s" "retries: 5" "start_period: 120s"; do
  compose_fact "healthcheck $kv" "^\\s*$kv" "$kv"
done

# ── 2. La tabla de opciones del add-on cubre exactamente las claves del schema ───────────────
# Cada clave de options: de config.yaml debe aparecer en DOCS.md como `clave` (fila de la tabla),
# y DOCS.md no debe documentar claves que ya no existen.
keys=$(awk '/^options:/{f=1;next} /^[a-z]/{f=0} f && /^  [a-z_]+:/{sub(/^ +/,"");sub(/:.*$/,"");print}' "$CY")
if [ -z "$keys" ]; then
  err "no pude extraer claves de options: de $CY — actualiza este script"
fi
# bash 3.2 de macOS no tiene mapfile; un while-read es portable.
while IFS= read -r k; do
  [ -n "$k" ] || continue
  grep -q "\`$k\`" "$AD" || err "la opción «$k» existe en $CY y no está documentada en $AD"
done <<EOF
$keys
EOF

# ── 3. El puerto de ingress declarado coincide ───────────────────────────────────────────────
ing=$(grep -m1 '^ingress_port:' "$CY" | awk '{print $2}')
if [ -n "$ing" ] && ! grep -q "$ing" "$AD"; then
  err "ingress_port=$ing en $CY no aparece en $AD"
fi

if [ "$fail" -ne 0 ]; then
  echo "check-derived-docs: las superficies derivadas han divergido de su fuente — arregla la copia (o la fuente) en este mismo PR." >&2
  exit 1
fi
echo "check-derived-docs: OK — Docker Hub y DOCS del add-on siguen diciendo la verdad"
