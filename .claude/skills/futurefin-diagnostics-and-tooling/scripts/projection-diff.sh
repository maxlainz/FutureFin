#!/usr/bin/env bash
# projection-diff.sh — fetch /v1/projection/series twice (different params, or
# before/after a code change) and diff the scalars/KPIs/milestones that MUST
# agree, with point-count context.
#
# Usage (from repo root; login env same as smoke-projection-cache.sh):
#   S=".claude/skills/futurefin-diagnostics-and-tooling/scripts/projection-diff.sh"
#   SMOKE_USER=alice SMOKE_PASS=secret bash $S                       # monthly vs density=hybrid
#   SMOKE_USER=... SMOKE_PASS=... bash $S "" "view=mine"             # household vs mine
#   SMOKE_USER=... SMOKE_PASS=... bash $S --save /tmp/base.json      # snapshot BEFORE a change
#   SMOKE_USER=... SMOKE_PASS=... bash $S --compare /tmp/base.json   # AFTER: current vs snapshot
#
# Positional args are raw query strings (without '?'), e.g. "density=hybrid",
# "view=mine", "months=360". Defaults: A="" (monthly household), B="density=hybrid".
#
# Interpretation guide: ../SKILL.md § "Interpreting projection-diff.sh".
# Key invariant (v1.4.0/v1.4.2): milestones, jubilacion_*, compound marker are
# computed on the FULL series — they must be IDENTICAL across densities.
#
# Written against API contract as of 2026-07-02 (v1.4.3), verified by code
# reading (handlers/projection.rs). Errors clearly if the API is not running.
set -euo pipefail
BASE="${BASE:-http://127.0.0.1:8080}"
JAR=$(mktemp); TMPA=$(mktemp); TMPB=$(mktemp)
trap 'rm -f "$JAR" "$TMPA" "$TMPB"' EXIT

if ! curl -sf -o /dev/null --max-time 5 "$BASE/health"; then
  echo "ERROR: no API responding at $BASE/health" >&2
  echo "Start it: 'docker compose up -d' (:8080) or 'cargo run' + BASE=http://127.0.0.1:8081." >&2
  exit 1
fi

login() {
  if [[ -z "${SMOKE_USER:-}" || -z "${SMOKE_PASS:-}" ]]; then
    echo "ERROR: set SMOKE_USER and SMOKE_PASS (existing account)." >&2; exit 1
  fi
  curl -sf -c "$JAR" -X POST "$BASE/v1/auth/login" \
    -H "content-type: application/json" \
    -d "{\"username\":\"$SMOKE_USER\",\"password\":\"$SMOKE_PASS\"}" >/dev/null \
    || { echo "ERROR: login failed (bad credentials?)." >&2; exit 1; }
}

fetch() { # $1 query-string (may be empty), $2 outfile
  local url="$BASE/v1/projection/series"
  [[ -n "$1" ]] && url="$url?$1"
  curl -sf -b "$JAR" --max-time 60 "$url" -o "$2" \
    || { echo "ERROR: GET $url failed." >&2; exit 1; }
}

MODE="fetch-two"; SNAP=""
if [[ "${1:-}" == "--save" ]]; then MODE="save"; SNAP="${2:?--save needs a file path}"; QA="${3:-}"
elif [[ "${1:-}" == "--compare" ]]; then MODE="compare"; SNAP="${2:?--compare needs a file path}"; QA="${3:-}"
else QA="${1:-}"; QB="${2:-density=hybrid}"; fi

login
case "$MODE" in
  save)
    fetch "$QA" "$SNAP"
    echo "Snapshot saved: $SNAP (query: '${QA:-<default monthly household>}')"
    echo "After your change, run: $0 --compare $SNAP"
    exit 0 ;;
  compare)
    [[ -f "$SNAP" ]] || { echo "ERROR: snapshot $SNAP not found (run --save first)." >&2; exit 1; }
    cp "$SNAP" "$TMPA"; LABEL_A="snapshot:$SNAP"
    fetch "$QA" "$TMPB"; LABEL_B="current:'${QA:-<default>}'" ;;
  fetch-two)
    fetch "$QA" "$TMPA"; LABEL_A="A:'${QA:-<default>}'"
    fetch "$QB" "$TMPB"; LABEL_B="B:'$QB'" ;;
esac

python3 - "$TMPA" "$TMPB" "$LABEL_A" "$LABEL_B" <<'PYEOF'
import json, sys

a = json.load(open(sys.argv[1])); b = json.load(open(sys.argv[2]))
la, lb = sys.argv[3], sys.argv[4]

def ms(doc, key):  # milestone summary "target@month, ..."
    return ", ".join(f"{m['target']}@m{m['reached_month_index']}" for m in doc.get(key, [])) or "-"

def row(name, va, vb, must_match=True):
    diff = va != vb
    mark = "DIFF" if diff else "same"
    if diff and not must_match:
        mark = "diff (expected)"
    print(f"  {name:<42} {mark:<16} {va} | {vb}")
    return diff and must_match

# Fields whose divergence is EXPECTED between densities (serialization only):
shape_only = {"density", "points_len", "fire_target_series_len"}
fields = [
    ("density",                       a.get("density"), b.get("density"), False),
    ("months",                        a.get("months"), b.get("months"), True),
    ("horizon_years",                 a.get("horizon_years"), b.get("horizon_years"), True),
    ("horizon_basis",                 a.get("horizon_basis"), b.get("horizon_basis"), True),
    ("starting_net_worth",            a.get("starting_net_worth"), b.get("starting_net_worth"), True),
    ("monthly_delta_assumption",      a.get("monthly_delta_assumption"), b.get("monthly_delta_assumption"), True),
    ("jubilacion_month_index",        a.get("jubilacion_month_index"), b.get("jubilacion_month_index"), True),
    ("jubilacion_target_net_worth",   a.get("jubilacion_target_net_worth"), b.get("jubilacion_target_net_worth"), True),
    ("compound_outpaces..._month_index",
                                      a.get("compound_outpaces_true_savings_month_index"),
                                      b.get("compound_outpaces_true_savings_month_index"), True),
    ("milestones (nominal)",          ms(a, "milestones"), ms(b, "milestones"), True),
    ("milestones_real (deflated)",    ms(a, "milestones_real"), ms(b, "milestones_real"), True),
    ("points_len",                    len(a.get("points", [])), len(b.get("points", [])), False),
    ("fire_target_series_len",        len(a.get("fire_target_series", [])), len(b.get("fire_target_series", [])), False),
    ("asset_series count",            len(a.get("asset_series", [])), len(b.get("asset_series", [])), True),
    ("last point net_worth",          a["points"][-1]["net_worth"] if a.get("points") else None,
                                      b["points"][-1]["net_worth"] if b.get("points") else None, True),
    ("last point month_index",        a["points"][-1]["month_index"] if a.get("points") else None,
                                      b["points"][-1]["month_index"] if b.get("points") else None, True),
]
print(f"Comparing {la}  vs  {lb}")
print(f"  {'field':<42} {'status':<16} A | B")
bad = sum(row(*f) for f in fields)

# When ONLY density differs, everything computed on the full series must match.
if a.get("density") != b.get("density"):
    print("\nNote: densities differ → points_len/fire_target_series_len diffs are")
    print("expected (serialization decimation). Everything marked DIFF above is a")
    print("real divergence: milestones/KPIs are computed on the FULL series and")
    print("must be density-invariant (regression class of v1.4.2).")
sys.exit(1 if bad else 0)
PYEOF
