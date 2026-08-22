#!/usr/bin/env bash
# projection-diff.sh — fetch /v1/projection/series twice (different params, or
# before/after a code change) and diff the scalars/KPIs/milestones that MUST
# agree, with point-count context.
#
# Usage (from repo root; login env same as smoke-projection-cache.sh):
#   S="scripts/diagnostics/projection-diff.sh"
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

command -v python3 >/dev/null \
  || { echo "ERROR: python3 is required (JSON comparison)." >&2; exit 1; }

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
same_density = a.get("density") == b.get("density")

def ms(doc, key):  # milestone summary "target@month, ..."
    return ", ".join(f"{m['target']}@m{m['reached_month_index']}" for m in doc.get(key, [])) or "-"

def row(name, va, vb, must_match=True):
    diff = va != vb
    mark = "DIFF" if diff else "same"
    if diff and not must_match:
        mark = "diff (expected)"
    print(f"  {name:<42} {mark:<16} {va} | {vb}")
    return diff and must_match

# Scalars/KPIs/milestones are computed on the FULL series server-side
# (handlers/projection.rs) → must match regardless of density. Array shapes
# can legitimately differ between densities. The series is months+1 points
# long (index 0 = today, then one per month): monthly ends at month_index =
# months (e.g. 840 at the default horizon), hybrid's last kept index is the
# last multiple of 12 ≤ months — identical (840) at the default horizon;
# they only diverge with a non-multiple-of-12 ?months= override
# (e.g. months=839 → monthly last 839, hybrid last 828).
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
    ("points_len",                    len(a.get("points", [])), len(b.get("points", [])), same_density),
    ("fire_target_series_len",        len(a.get("fire_target_series", [])), len(b.get("fire_target_series", [])), same_density),
    ("asset_series count",            len(a.get("asset_series", [])), len(b.get("asset_series", [])), True),
    ("last point month_index",        a["points"][-1]["month_index"] if a.get("points") else None,
                                      b["points"][-1]["month_index"] if b.get("points") else None, same_density),
]
print(f"Comparing {la}  vs  {lb}")
print(f"  {'field':<42} {'status':<16} A | B")
bad = sum(row(*f) for f in fields)

# Point-level check at SHARED month_index values. Hybrid indices are a strict
# subset of monthly indices, and both densities serialize from the same
# deterministic compute → net_worth/contributed_capital/fire_target must be
# bit-identical (f64) at every shared index.
def by_month(doc):
    pts = {p["month_index"]: p for p in doc.get("points", [])}
    ft = doc.get("fire_target_series", [])
    ftm = {p["month_index"]: ft[i] for i, p in enumerate(doc.get("points", []))} if len(ft) == len(pts) else {}
    return pts, ftm

pa, fta = by_month(a); pb, ftb = by_month(b)
shared = sorted(set(pa) & set(pb))
mismatches = []
for m in shared:
    for field in ("net_worth", "contributed_capital"):
        if pa[m][field] != pb[m][field]:
            mismatches.append((m, field, pa[m][field], pb[m][field]))
    if fta and ftb and fta.get(m) != ftb.get(m):
        mismatches.append((m, "fire_target", fta.get(m), ftb.get(m)))
print(f"\nShared month_index points: {len(shared)} "
      f"(A has {len(pa)}, B has {len(pb)})")
if mismatches:
    bad += 1
    print(f"  DIFF at {len(mismatches)} shared-index values (first 3):")
    for m, field, va, vb in mismatches[:3]:
        print(f"    month {m} {field}: {va} | {vb}")
else:
    print("  same: all shared-index values identical")

if not same_density:
    print("\nNote: densities differ → points_len/fire_target_series_len (and the")
    print("last serialized month_index) diffs are expected decimation artifacts.")
    print("Everything marked DIFF above is a real divergence: KPIs/milestones are")
    print("computed on the FULL series server-side and must be density-invariant")
    print("(regression class of v1.4.2).")
sys.exit(1 if bad else 0)
PYEOF
