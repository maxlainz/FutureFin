#!/usr/bin/env bash
# db-stats.sh — DB-level ground truth: exact row counts per table,
# _sqlx_migrations state, session/liability/pending-user diagnostics.
#
# Runs psql INSIDE the compose container because the prod stack does NOT
# expose Postgres AT ALL (since 3.0.0 PostgreSQL is embedded in the single
# `futurefin` container, socket-only — no TCP listener even inside; the dev
# database is a separate compose, docker-compose.dev.yml, on 127.0.0.1:5432).
#
# Usage (from repo root, where docker-compose.yml lives):
#   bash scripts/diagnostics/db-stats.sh
#   POSTGRES_USER=futurefin POSTGRES_DB=futurefin bash .../db-stats.sh
#
# Env vars (defaults match docker-compose.yml):
#   DB_SERVICE     compose service name (default futurefin)
#   POSTGRES_USER  psql -U (default futurefin)
#   POSTGRES_DB    psql -d (default futurefin)
#
# Read-only: only SELECTs. Interpretation guide: ../SKILL.md § "db-stats.sh".
# Written against the schema as of 2026-07-02 (v1.4.3, 31 migration files),
# updated 3.0.0 for the single-container stack. Degrades with a clear error
# if Docker or the container is not up.
set -euo pipefail
DB_SERVICE="${DB_SERVICE:-futurefin}"
PGUSER="${POSTGRES_USER:-futurefin}"
PGDB="${POSTGRES_DB:-futurefin}"

command -v docker >/dev/null \
  || { echo "ERROR: docker not found in PATH." >&2; exit 1; }
[[ -f docker-compose.yml ]] \
  || echo "WARN: no docker-compose.yml in $(pwd) — run from the repo root." >&2
if ! docker compose ps --status running "$DB_SERVICE" 2>/dev/null | grep -q "$DB_SERVICE"; then
  echo "ERROR: compose service '$DB_SERVICE' is not running." >&2
  echo "Start it: 'docker compose up -d $DB_SERVICE' (from the repo root)." >&2
  exit 1
fi

psql_run() { # runs psql in the container via Unix socket; -T because there is no TTY here
  docker compose exec -T "$DB_SERVICE" \
    psql -h /var/run/postgresql -U "$PGUSER" -d "$PGDB" -v ON_ERROR_STOP=1 -P pager=off "$@"
}

echo "== Server version and collation (adopted clusters show datcollate=en_US.utf8) =="
psql_run -c "SHOW server_version;"
psql_run -c "SELECT datname, datcollate, datcollversion FROM pg_database WHERE datname = current_database();"

echo "== _sqlx_migrations (must equal the repo's apps/api/migrations/*.sql count) =="
if [[ -d apps/api/migrations ]]; then
  echo "  repo migration files: $(ls apps/api/migrations/*.sql | wc -l | tr -d ' ')"
fi
psql_run -c "SELECT count(*) AS applied,
                    max(version) AS latest_version,
                    bool_and(success) AS all_success
             FROM _sqlx_migrations;"
psql_run -c "SELECT version, description, success, installed_on
             FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;"

echo "== Exact row counts per table (public schema) =="
# query_to_xml trick: one dynamic count(*) per table, single result set.
psql_run -c "
  SELECT c.relname AS table_name,
         (xpath('/row/cnt/text()',
                query_to_xml(format('SELECT count(*) AS cnt FROM public.%I', c.relname),
                             false, true, '')))[1]::text::bigint AS rows
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE n.nspname = 'public' AND c.relkind = 'r'
  ORDER BY c.relname;"

echo "== Diagnostics =="
psql_run -c "SELECT count(*) FILTER (WHERE expires_at >  now()) AS active_sessions,
                    count(*) FILTER (WHERE expires_at <= now()) AS expired_sessions
             FROM sessions;"
# NORMAL to be > 0: reads never mutate — GET handlers FILTER expired
# liabilities (payment_end_date < today), they never DELETE them (v1.3.0).
psql_run -c "SELECT count(*) AS expired_liabilities_still_stored
             FROM liabilities WHERE payment_end_date < CURRENT_DATE;"
# Users with no installation membership = 'pending' (see no data, 403 on
# ledger endpoints) — expected only while the owner hasn't approved them.
psql_run -c "SELECT count(*) AS pending_users
             FROM users u
             WHERE NOT EXISTS (SELECT 1 FROM installation_memberships m
                               WHERE m.user_id = u.id);"
psql_run -c "SELECT role, count(*) FROM installation_memberships GROUP BY role ORDER BY role;"
echo "done"
