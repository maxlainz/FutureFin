#!/usr/bin/env bash
# api-timing.sh — login + timed hits on key FutureFin endpoints (cold vs warm).
#
# Usage (from repo root):
#   SMOKE_USER=alice SMOKE_PASS=secret \
#     bash .claude/skills/futurefin-diagnostics-and-tooling/scripts/api-timing.sh
#   # split-dev (API on :8081):
#   BASE=http://127.0.0.1:8081 SMOKE_USER=... SMOKE_PASS=... bash .../api-timing.sh
#
# Env vars (same convention as scripts/smoke-projection-cache.sh):
#   BASE        API base URL (default http://127.0.0.1:8080)
#   SMOKE_USER  existing username with installation membership. If unset,
#   SMOKE_PASS  registers a throwaway user — only useful on a VIRGIN database
#               (on a non-virgin DB the throwaway stays "pending" with no
#               membership and every data endpoint would 403; the script
#               detects this via /v1/installation/session-context and aborts).
#
# What it prints: wall-clock ms per endpoint (curl %{time_total}), two hits on
# each projection density so you can see MISS→HIT, and gzip vs raw payload
# sizes. Interpretation guide: ../SKILL.md § "Interpreting api-timing.sh".
#
# Written against API contract as of 2026-07-02 (v1.4.3), verified by code
# reading (apps/api/src/routes/mod.rs, handlers/projection.rs). Degrades with
# a clear error if the API is not running.
set -euo pipefail
BASE="${BASE:-http://127.0.0.1:8080}"
JAR=$(mktemp)
trap 'rm -f "$JAR"' EXIT

if ! curl -sf -o /dev/null --max-time 5 "$BASE/health"; then
  echo "ERROR: no API responding at $BASE/health" >&2
  echo "Start it first: 'docker compose up -d' (prod stack, :8080)" >&2
  echo "or 'cd apps/api && cargo run' (split-dev → rerun with BASE=http://127.0.0.1:8081)." >&2
  exit 1
fi

timed() { # $1 label, $2 url — prints one aligned "label  N ms" line
  local t
  if ! t=$(curl -sf -b "$JAR" -o /dev/null --max-time 30 -w '%{time_total}' "$2"); then
    printf '  %-34s FAILED (non-2xx or timeout)\n' "$1"
    return 0
  fi
  awk -v l="$1" -v t="$t" 'BEGIN{printf "  %-34s %8.1f ms\n", l, t*1000}'
}

sizes() { # $1 label, $2 url — raw vs gzip size_download
  local raw gz
  raw=$(curl -sf -b "$JAR" -o /dev/null -w '%{size_download}' "$2" || echo "?")
  gz=$(curl -sf -b "$JAR" -H 'Accept-Encoding: gzip' -o /dev/null -w '%{size_download}' "$2" || echo "?")
  printf '  %-34s raw %8s B   gzip %8s B\n' "$1" "$raw" "$gz"
}

echo "== Unauthenticated =="
timed "GET /health (process up)"      "$BASE/health"
timed "GET /v1/health"                "$BASE/v1/health"
timed "GET /v1/ready (DB ping)"       "$BASE/v1/ready"

echo "== Login =="
if [[ -n "${SMOKE_USER:-}" && -n "${SMOKE_PASS:-}" ]]; then
  USER="$SMOKE_USER"; PASS="$SMOKE_PASS"
else
  USER="timing_$(date +%s)_$$"; PASS="timing-pass-1234"
  echo "  SMOKE_USER unset → registering throwaway $USER (only useful on a virgin DB)"
  curl -sf -c "$JAR" -X POST "$BASE/v1/auth/register" \
    -H "content-type: application/json" \
    -d "{\"username\":\"$USER\",\"password\":\"$PASS\",\"birth_date\":\"1990-01-01\"}" >/dev/null \
    || { echo "ERROR: register failed (validation error or username taken)." >&2; exit 1; }
fi
t0=$(date +%s%N)
curl -sf -c "$JAR" -X POST "$BASE/v1/auth/login" \
  -H "content-type: application/json" \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}" >/dev/null \
  || { echo "ERROR: login failed (bad credentials?)." >&2; exit 1; }
awk -v ms=$(( ($(date +%s%N) - t0) / 1000000 )) \
  'BEGIN{printf "  login: %d ms (Argon2id verify dominates; 100-500 ms is normal)\n", ms}'

# Gate: a session alone is not enough — data endpoints require installation
# membership (pending users get 403 everywhere). session-context returns
# "access":null for pending users, "access":{...} for members.
CTX=$(curl -sf -b "$JAR" "$BASE/v1/installation/session-context" || echo "")
if ! printf '%s' "$CTX" | grep -q '"access":{'; then
  echo "ERROR: user '$USER' has a session but NO installation membership" >&2
  echo "(pending approval, or installation not set up). Every data endpoint" >&2
  echo "would return 403. Pass SMOKE_USER/SMOKE_PASS of an approved member." >&2
  exit 1
fi
echo "  waiting 1500 ms for post-login cache warm-up (tokio::spawn, household both densities)…"
sleep 1.5

echo "== Projection (two hits each: #1 may be MISS, #2 must be HIT) =="
timed "series?density=hybrid  #1"  "$BASE/v1/projection/series?density=hybrid"
timed "series?density=hybrid  #2"  "$BASE/v1/projection/series?density=hybrid"
timed "series (monthly)       #1"  "$BASE/v1/projection/series"
timed "series (monthly)       #2"  "$BASE/v1/projection/series"
timed "series?months=840 (cache BYPASS = true compute)" "$BASE/v1/projection/series?months=840"
timed "series?view=mine (own cache key)" "$BASE/v1/projection/series?view=mine"

echo "== Other data endpoints (no cache; DB query cost) =="
timed "GET /v1/summary"            "$BASE/v1/summary"
timed "GET /v1/assets"             "$BASE/v1/assets"
timed "GET /v1/budget"             "$BASE/v1/budget"
timed "GET /v1/liabilities"        "$BASE/v1/liabilities"

echo "== Payload sizes (raw vs Content-Encoding: gzip) =="
sizes "series (monthly)"           "$BASE/v1/projection/series"
sizes "series?density=hybrid"      "$BASE/v1/projection/series?density=hybrid"
echo "done"
