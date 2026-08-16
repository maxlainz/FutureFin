#!/usr/bin/env bash
# FutureFin all-in-one entrypoint (desde 3.0.0).
#
# Supervisa DOS procesos: el PostgreSQL embebido y la API. Reglas de oro:
#   - NUNCA borra un cluster: los clusters viejos/parciales se mueven a un lado (mv),
#     jamás rm. Lo único que se borra son backups propios según retención y el staging
#     de pg_upgrade una vez copiado.
#   - Al postmaster se le para con SIGINT (fast shutdown), NO con SIGTERM (smart, puede
#     colgarse esperando clientes). Ver failure-archaeology antes de cambiar esto.
#   - Sin volumen montado en $PGDATA se ABORTA (los datos morirían con el contenedor),
#     salvo FUTUREFIN_ALLOW_EPHEMERAL_DB=1.
#
# Modos (FUTUREFIN_MODE o argv[1]): serve (default) | db-only (rescate: solo PostgreSQL).
# Cualquier otro argv se ejecuta tal cual (p.ej. `docker run … pg_dump --version`).
set -Eeuo pipefail

# ── Configuración ────────────────────────────────────────────────────────────
PGDATA="${PGDATA:-/var/lib/postgresql/data}"
PG_MAJOR="${PG_MAJOR:-16}"
PG_BINROOT=/usr/lib/postgresql
STATE_DIR="${FUTUREFIN_STATE_DIR:-/var/lib/futurefin}"
BACKUP_DIR="${FUTUREFIN_BACKUP_DIR:-$STATE_DIR/backups}"
SOCK_DIR=/var/run/postgresql

DB_MODE="${FUTUREFIN_DB_MODE:-auto}"                  # auto | embedded | external
BACKUP_KEEP="${FUTUREFIN_BACKUP_KEEP:-10}"
BACKUP_KEEP_DAYS="${FUTUREFIN_BACKUP_KEEP_DAYS:-90}"
PREMIGRATION_BACKUP="${FUTUREFIN_PREMIGRATION_BACKUP:-on}"
ALLOW_EPHEMERAL="${FUTUREFIN_ALLOW_EPHEMERAL_DB:-0}"
EXTERNAL_WAIT_SECS="${FUTUREFIN_EXTERNAL_WAIT_SECS:-60}"
API_STOP_TIMEOUT="${FUTUREFIN_API_STOP_TIMEOUT:-15}"
PG_STOP_TIMEOUT="${FUTUREFIN_PG_STOP_TIMEOUT:-30}"
POSTGRES_USER="${POSTGRES_USER:-futurefin}"
POSTGRES_DB="${POSTGRES_DB:-futurefin}"

APP_VERSION="$(cat /app/VERSION 2>/dev/null || echo unknown)"
API_PID=""
PG_PID=""
SHUTTING_DOWN=0
AUTOMIGRATE_PENDING=0
SKIP_PREMIGRATION=0
PGUPGRADE_JUST_RAN=0

# ── Utilidades ───────────────────────────────────────────────────────────────
log()  { echo "[futurefin-entrypoint] $*"; }
warn() { echo "[futurefin-entrypoint] WARN: $*" >&2; }
die()  { echo "[futurefin-entrypoint] FATAL: $*" >&2; exit 1; }

is_root() { [ "$(id -u)" = 0 ]; }

run_as_pg() {
  if is_root; then gosu postgres "$@"; else "$@"; fi
}

is_mounted() {
  if command -v mountpoint >/dev/null 2>&1; then
    mountpoint -q "$1"
  else
    grep -qs " $1 " /proc/self/mountinfo
  fi
}

has_cluster() { [ -s "$PGDATA/PG_VERSION" ]; }

pgdata_empty() {
  [ -z "$(find "$PGDATA" -mindepth 1 -maxdepth 1 ! -name 'lost+found' -print -quit 2>/dev/null)" ]
}

# Ficheros de estado KEY=VALUE en $STATE_DIR/state/<nombre>.env
state_get() { # $1=file $2=key — vacío (exit 0) si el fichero aún no existe.
  # OJO: con `set -Eeuo pipefail`, un sed que falla dentro de una sustitución de comando
  # en una asignación MATA el script sin mensaje (así murió el segundo arranque en CI).
  local f="$STATE_DIR/state/$1.env"
  [ -f "$f" ] || return 0
  sed -n "s/^$2=//p" "$f" | head -n1
}
state_set() { # $1=file $2=key $3=value
  local f="$STATE_DIR/state/$1.env" tmp
  mkdir -p "$STATE_DIR/state"
  tmp="$(grep -v "^$2=" "$f" 2>/dev/null || true)"
  printf '%s\n%s=%s\n' "$tmp" "$2" "$3" | sed '/^$/d' > "$f"
}

psql_local() { # $1=db, resto args extra — vía socket, sin .psqlrc
  local db="$1"; shift
  psql -X -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$db" -v ON_ERROR_STOP=1 "$@"
}

# Censo dinámico de tablas públicas (tabla:filas por línea, ordenado) — sirve para
# verificar automigración y pg_upgrade sin listas hardcodeadas que deriven.
census() { # $@ = args completos de psql (URL o -h/-U/-d)
  local tables t c
  tables=$(psql -X -tA "$@" -c "SELECT table_name FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE' ORDER BY table_name") || return 1
  for t in $tables; do
    c=$(psql -X -tA "$@" -c "SELECT count(*) FROM \"${t}\"") || c=ERR
    printf '%s:%s\n' "$t" "$c"
  done
}

controldata_field() { # $1=bindir $2=datadir $3=patrón
  run_as_pg "$1/pg_controldata" "$2" | sed -n "s/^$3:[[:space:]]*//p" | head -n1
}

# ── Apagado ordenado ─────────────────────────────────────────────────────────
stop_pid() { # $1=pid $2=señal $3=timeout_s [$4=señal_escalada]
  local pid="$1" sig="$2" timeout="$3" esc="${4:-KILL}" waited=0
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || return 0
  kill -s "$sig" "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge $((timeout * 5)) ]; then
      warn "process $pid did not stop after ${timeout}s ($sig) — escalating to $esc"
      kill -s "$esc" "$pid" 2>/dev/null || true
      # La escalada también está acotada: si en 10 s sigue vivo, SIGKILL. Un `wait`
      # sin límite aquí puede bloquear el apagado para siempre.
      local w2=0
      while kill -0 "$pid" 2>/dev/null && [ "$w2" -lt 50 ]; do sleep 0.2; w2=$((w2 + 1)); done
      if kill -0 "$pid" 2>/dev/null; then
        warn "process $pid survived $esc — sending KILL"
        kill -s KILL "$pid" 2>/dev/null || true
      fi
      wait "$pid" 2>/dev/null || true
      return 0
    fi
    sleep 0.2; waited=$((waited + 1))
  done
  wait "$pid" 2>/dev/null || true
}

on_term() {
  [ "$SHUTTING_DOWN" = 1 ] && return
  SHUTTING_DOWN=1
  log "shutdown signal received — stopping API first, then PostgreSQL (fast)"
  stop_pid "$API_PID" TERM "$API_STOP_TIMEOUT"
  # SIGINT = fast shutdown (checkpoint y salir). SIGQUIT = immediate como escalada.
  stop_pid "$PG_PID" INT "$PG_STOP_TIMEOUT" QUIT
  log "clean shutdown complete"
  exit 0
}
trap on_term TERM INT

# ── PostgreSQL embebido ──────────────────────────────────────────────────────
ensure_runtime_dirs() {
  mkdir -p "$STATE_DIR/state" "$BACKUP_DIR" "$SOCK_DIR"
  if is_root; then
    chown postgres:postgres "$STATE_DIR" "$STATE_DIR/state" "$BACKUP_DIR" "$SOCK_DIR" || true
    chmod 2775 "$SOCK_DIR" || true
  fi
}

init_fresh_cluster() {
  log "initializing fresh PostgreSQL $PG_MAJOR cluster in $PGDATA"
  if is_root; then chown postgres:postgres "$PGDATA"; chmod 0700 "$PGDATA"; fi
  run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/initdb" -D "$PGDATA" \
    --username="$POSTGRES_USER" \
    --encoding=UTF8 --locale=C.UTF-8 \
    --data-checksums \
    --auth-local=trust --auth-host=scram-sha-256 >/dev/null
  local sysid
  sysid="$(controldata_field "$PG_BINROOT/$PG_MAJOR/bin" "$PGDATA" 'Database system identifier')"
  state_set cluster ORIGIN initdb
  state_set cluster CREATED_BY_VERSION "$APP_VERSION"
  state_set cluster REINDEXED_SYSID "$sysid"
}

adopt_cluster() {
  local owner pguid
  owner="$(stat -c %u "$PGDATA")"
  pguid="$(id -u postgres 2>/dev/null || id -u)"
  if is_root && [ "$owner" != "$pguid" ]; then
    log "adopting ownership of PGDATA (uid $owner -> $pguid)"
    chown -R postgres:postgres "$PGDATA"
    chmod 0700 "$PGDATA"
  fi
  # pg_hba del usuario: no la reescribimos, pero avisamos si el socket no va a entrar.
  if ! grep -Eqs '^[[:space:]]*local[[:space:]]+all[[:space:]]+all[[:space:]]+trust' "$PGDATA/pg_hba.conf"; then
    warn "pg_hba.conf has no 'local all all trust' line; if startup fails to connect via socket, add it (see README «Actualizar a 3.x»)"
  fi
}

# REINDEX de adopción: los índices de texto de un cluster creado por otra libc (musl,
# postgres:16.4-alpine de 2.x) tienen orden de colación distinto — sin esto hay
# índices únicos silenciosamente corruptos. Solo una vez por cluster (system identifier).
maybe_adoption_reindex() {
  local sysid marked
  sysid="$(controldata_field "$PG_BINROOT/$PG_MAJOR/bin" "$PGDATA" 'Database system identifier')"
  marked="$(state_get cluster REINDEXED_SYSID)"
  [ "$marked" = "$sysid" ] && return 0
  log "reindexing database after adoption (musl->glibc collation) — one-time, may take a moment"
  local t0 t1
  t0=$(date +%s)
  psql_local "$POSTGRES_DB" -c "REINDEX DATABASE \"$POSTGRES_DB\";" >/dev/null
  psql_local "$POSTGRES_DB" -c "ALTER DATABASE \"$POSTGRES_DB\" REFRESH COLLATION VERSION;" >/dev/null 2>&1 || true
  t1=$(date +%s)
  log "reindex complete in $((t1 - t0))s"
  state_set cluster ORIGIN "${ORIGIN_OVERRIDE:-adopted}"
  state_set cluster REINDEXED_SYSID "$sysid"
}

start_postgres() {
  local extra=()
  [ -n "${FUTUREFIN_PG_LOG_LEVEL:-}" ] && extra+=(-c "log_min_messages=$FUTUREFIN_PG_LOG_LEVEL")
  log "starting embedded PostgreSQL $PG_MAJOR (socket-only at $SOCK_DIR)"
  # OJO: el postmaster se lanza INLINE, nunca vía una función en background
  # (`run_as_pg … &` crea un subshell bash intermedio: $! sería el subshell, que
  # además IGNORA SIGINT/SIGQUIT en jobs de background — el apagado ordenado nunca
  # llegaría al postmaster; así se produjo el stop colgado de 60 s en CI).
  # gosu hace exec, de modo que $! ES el postmaster.
  if is_root; then
    gosu postgres "$PG_BINROOT/$PG_MAJOR/bin/postgres" -D "$PGDATA" \
      -c listen_addresses="${FUTUREFIN_PG_LISTEN:-}" \
      -c unix_socket_directories="$SOCK_DIR" \
      -c logging_collector=off \
      "${extra[@]}" &
  else
    "$PG_BINROOT/$PG_MAJOR/bin/postgres" -D "$PGDATA" \
      -c listen_addresses="${FUTUREFIN_PG_LISTEN:-}" \
      -c unix_socket_directories="$SOCK_DIR" \
      -c logging_collector=off \
      "${extra[@]}" &
  fi
  PG_PID=$!
}

wait_pg_ready() { # $1=db
  local _unused
  for _ in $(seq 1 120); do
    if ! kill -0 "$PG_PID" 2>/dev/null; then
      die "PostgreSQL exited during startup — check the log lines above"
    fi
    if pg_isready -q -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$1" 2>/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  die "PostgreSQL did not become ready within 60s"
}

post_start_maintenance() {
  # Rol: en cluster propio initdb ya creó a $POSTGRES_USER como superusuario; en uno
  # adoptado de 2.x el superusuario ES $POSTGRES_USER. Si no existe, el usuario tiene
  # un POSTGRES_USER distinto al de su instalación 2.x.
  if ! psql -X -tA -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -c 'SELECT 1' >/dev/null 2>&1; then
    die "cannot connect as role '$POSTGRES_USER'. If your 2.x install used a custom POSTGRES_USER, set the same value now."
  fi
  if [ "$(psql -X -tA -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -c "SELECT 1 FROM pg_database WHERE datname = '$POSTGRES_DB'")" != 1 ]; then
    log "creating database $POSTGRES_DB"
    psql -X -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -v ON_ERROR_STOP=1 \
      -c "CREATE DATABASE \"$POSTGRES_DB\" OWNER \"$POSTGRES_USER\";" >/dev/null
  fi
  if [ -n "${POSTGRES_PASSWORD:-}" ]; then
    # Compat 2.x: la contraseña ya no es necesaria (socket local trust), pero si viene
    # la aplicamos al rol para no sorprender a nadie que la use externamente.
    psql -X -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -v ON_ERROR_STOP=1 \
      -v pw="$POSTGRES_PASSWORD" -c "ALTER ROLE \"$POSTGRES_USER\" PASSWORD :'pw';" >/dev/null
  fi
  maybe_adoption_reindex
}

# ── Backup automático pre-migración ─────────────────────────────────────────
premigration_backup() {
  [ "$PREMIGRATION_BACKUP" = on ] || { log "pre-migration backup disabled (FUTUREFIN_PREMIGRATION_BACKUP=$PREMIGRATION_BACKUP)"; return 0; }
  [ "$SKIP_PREMIGRATION" = 1 ] && return 0

  local last applied pending=0 out ts
  last="$(cat "$STATE_DIR/state/last-version" 2>/dev/null || true)"
  applied="$(psql_local "$POSTGRES_DB" -tA -c 'SELECT version FROM _sqlx_migrations ORDER BY version' 2>/dev/null || true)"
  if [ -z "$applied" ]; then
    # Base recién creada (o sin migrar): no hay nada que perder todavía.
    echo "$APP_VERSION" > "$STATE_DIR/state/last-version"
    return 0
  fi
  if [ "$last" != "$APP_VERSION" ]; then pending=1; fi
  if [ "$pending" = 0 ] && ! comm -13 <(printf '%s\n' "$applied" | sort) <(sort /app/migration-versions.txt) | grep -q .; then
    : # ni cambio de versión ni migraciones pendientes
  else
    pending=1
  fi
  if [ "$pending" = 1 ]; then
    ts="$(date -u +%Y%m%dT%H%M%SZ)"
    out="$BACKUP_DIR/pre-migration-${last:-unknown}-to-${APP_VERSION}-${ts}.sql.gz"
    log "app version change or pending migrations detected — writing pre-migration backup"
    if ! pg_dump -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" | gzip -6 > "$out"; then
      rm -f "$out"
      die "pre-migration backup FAILED — refusing to start with pending migrations and no safety net. Fix the cause or set FUTUREFIN_PREMIGRATION_BACKUP=off to bypass deliberately."
    fi
    log "pre-migration backup written: $out ($(du -h "$out" | cut -f1))"
    prune_backups
  fi
  echo "$APP_VERSION" > "$STATE_DIR/state/last-version"
}

prune_backups() {
  # Los `ls` van protegidos con `|| true`: con pipefail, un glob sin coincidencias
  # mataría la función entera en silencio (misma clase de bug que state_get).
  local f avail n
  cd "$BACKUP_DIR" || return 0
  # Los $BACKUP_KEEP más recientes son intocables; del resto, fuera los de > KEEP_DAYS.
  (ls -1t pre-*.sql.gz 2>/dev/null || true) | tail -n +"$((BACKUP_KEEP + 1))" | while read -r f; do
    if [ -n "$(find "$f" -maxdepth 0 -mtime +"$BACKUP_KEEP_DAYS" 2>/dev/null)" ]; then
      log "pruning old automatic backup: $f"
      rm -f "$f"
    fi
  done
  # Presión de disco: por debajo de 256 MB libres seguimos podando (nunca los 3 últimos).
  avail="$(df -Pm "$BACKUP_DIR" | awk 'NR==2 {print $4}')"
  if [ "${avail:-999999}" -lt 256 ]; then
    n=$( (ls -1 pre-*.sql.gz 2>/dev/null || true) | wc -l)
    while [ "$n" -gt 3 ] && [ "$avail" -lt 256 ]; do
      f="$( (ls -1tr pre-*.sql.gz 2>/dev/null || true) | head -n1)"
      [ -n "$f" ] || break
      warn "low disk space (${avail}MB) — pruning $f"
      rm -f "$f"
      n=$((n - 1))
      avail="$(df -Pm "$BACKUP_DIR" | awk 'NR==2 {print $4}')"
    done
  fi
  cd - >/dev/null || true
}

# ── pg_upgrade automático (cluster de un major anterior) ─────────────────────
maybe_pg_upgrade() {
  local old resume
  old="$(cat "$PGDATA/PG_VERSION")"
  resume="$(state_get pgupgrade STATE)"
  if [ -n "$resume" ] && [ "$resume" != "done" ]; then
    log "resuming interrupted pg_upgrade (state=$resume)"
    pgupgrade_swap_resume "$resume"
    return 0
  fi
  [ "$old" = "$PG_MAJOR" ] && return 0
  if [ "$old" -gt "$PG_MAJOR" ] 2>/dev/null; then
    die "PGDATA was created by PostgreSQL $old, NEWER than this image's $PG_MAJOR. Do not start an older FutureFin over it."
  fi
  if [ ! -x "$PG_BINROOT/$old/bin/postgres" ]; then
    die "PGDATA was created by PostgreSQL $old and this image only bundles $(ls "$PG_BINROOT" | tr '\n' ',' | sed 's/,$//'). Options: (1) start an older FutureFin release that bundles $old to upgrade stepwise; (2) dump with the official postgres:$old image and use external automigration."
  fi

  log "pg_upgrade needed: PostgreSQL $old -> $PG_MAJOR"
  # Espacio: dump + cluster nuevo + copia ≈ 3× el cluster actual.
  local need avail
  need=$(( $(du -sm "$PGDATA" | cut -f1) * 3 ))
  avail="$(df -Pm "$STATE_DIR" | awk 'NR==2 {print $4}')"
  if [ "$avail" -lt "$need" ]; then
    die "not enough free space for pg_upgrade: need ~${need}MB in $STATE_DIR, have ${avail}MB"
  fi

  # 1. Arranque limpio del cluster viejo (pg_upgrade rechaza clusters en recuperación),
  #    lectura de locale/encoding/checksums, backup obligatorio, censo, parada.
  local datcollate datctype encoding checksums old_census
  run_as_pg "$PG_BINROOT/$old/bin/pg_ctl" -D "$PGDATA" -w \
    -o "-c listen_addresses='' -c unix_socket_directories=$SOCK_DIR" start >/dev/null
  datcollate=$(psql -X -tA -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -c "SELECT datcollate FROM pg_database WHERE datname='template0'")
  datctype=$(psql -X -tA -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -c "SELECT datctype FROM pg_database WHERE datname='template0'")
  encoding=$(psql -X -tA -h "$SOCK_DIR" -U "$POSTGRES_USER" -d postgres -c "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname='template0'")
  old_census="$(census -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" 2>/dev/null || true)"
  local ts out
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  out="$BACKUP_DIR/pre-pgupgrade-${old}-to-${PG_MAJOR}-${ts}.sql.gz"
  log "writing mandatory pre-pg_upgrade backup (pg_dumpall)"
  if ! pg_dumpall -h "$SOCK_DIR" -U "$POSTGRES_USER" | gzip -6 > "$out"; then
    run_as_pg "$PG_BINROOT/$old/bin/pg_ctl" -D "$PGDATA" -m fast -w stop >/dev/null || true
    rm -f "$out"
    die "pre-pg_upgrade backup failed — aborting before touching anything"
  fi
  run_as_pg "$PG_BINROOT/$old/bin/pg_ctl" -D "$PGDATA" -m fast -w stop >/dev/null
  checksums="$(controldata_field "$PG_BINROOT/$old/bin" "$PGDATA" 'Data page checksum version')"

  # 2. Cluster nuevo en staging con locale/encoding/checksums IDÉNTICOS.
  local staging="$STATE_DIR/pgupgrade"
  rm -rf "$staging"
  mkdir -p "$staging/new" "$staging/logs"
  if is_root; then chown -R postgres:postgres "$staging"; fi
  local initflags=(--username="$POSTGRES_USER" --encoding="$encoding" --lc-collate="$datcollate" --lc-ctype="$datctype" --auth-local=trust --auth-host=scram-sha-256)
  [ "${checksums:-0}" != 0 ] && initflags+=(--data-checksums)
  run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/initdb" -D "$staging/new" "${initflags[@]}" >/dev/null

  # 3. pg_upgrade en modo copia (no --link: el cluster viejo queda utilizable si algo falla).
  log "running pg_upgrade $old -> $PG_MAJOR (copy mode)"
  (
    cd "$staging/logs"
    run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/pg_upgrade" \
      --old-datadir="$PGDATA" --new-datadir="$staging/new" \
      --old-bindir="$PG_BINROOT/$old/bin" --new-bindir="$PG_BINROOT/$PG_MAJOR/bin" \
      --username="$POSTGRES_USER" --socketdir="$SOCK_DIR"
  ) || die "pg_upgrade failed — old cluster untouched in $PGDATA; logs in $staging/logs"

  # 4. Verificación del cluster nuevo por censo, aún en staging.
  run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/pg_ctl" -D "$staging/new" -w \
    -o "-c listen_addresses='' -c unix_socket_directories=$SOCK_DIR" start >/dev/null
  local new_census
  new_census="$(census -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" 2>/dev/null || true)"
  run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/pg_ctl" -D "$staging/new" -m fast -w stop >/dev/null
  if [ "$old_census" != "$new_census" ]; then
    die "pg_upgrade verification FAILED (row census differs). Old cluster untouched in $PGDATA. old=[$old_census] new=[$new_census]"
  fi

  # 5. Swap reanudable: viejo a un lado (mismo fs, instantáneo), nuevo copiado dentro.
  state_set pgupgrade OLD_MAJOR "$old"
  state_set pgupgrade STATE staged
  pgupgrade_swap_resume staged
  log "pg_upgrade $old -> $PG_MAJOR completed; old cluster preserved at $PGDATA/pgdata_old_$old (delete manually when satisfied)"
  PGUPGRADE_JUST_RAN=1
}

pgupgrade_swap_resume() { # $1=estado desde el que reanudar
  local st="$1" old staging="$STATE_DIR/pgupgrade" f
  old="$(state_get pgupgrade OLD_MAJOR)"
  case "$st" in
    staged|swapping)
      state_set pgupgrade STATE swapping
      mkdir -p "$PGDATA/pgdata_old_$old"
      if is_root; then chown postgres:postgres "$PGDATA/pgdata_old_$old"; fi
      find "$PGDATA" -mindepth 1 -maxdepth 1 ! -name "pgdata_old_*" ! -name 'lost+found' \
        -exec mv -t "$PGDATA/pgdata_old_$old/" {} +
      ;&
    copying)
      state_set pgupgrade STATE copying
      [ -s "$staging/new/PG_VERSION" ] || die "pg_upgrade staging missing at $staging/new; old cluster preserved at $PGDATA/pgdata_old_$old — restore it manually (mv contents back) and retry"
      # Copia parcial previa (si la hubo) fuera — el staging y el cluster viejo están intactos.
      find "$PGDATA" -mindepth 1 -maxdepth 1 ! -name "pgdata_old_*" ! -name 'lost+found' \
        -exec rm -rf {} +
      cp -a "$staging/new/." "$PGDATA/"
      if is_root; then chown -R postgres:postgres "$PGDATA"; chmod 0700 "$PGDATA"; fi
      ;&
    *)
      state_set pgupgrade STATE "done"
      rm -rf "$staging"
      ;;
  esac
}

# ── Automigración one-shot desde base externa ────────────────────────────────
automigrate_prepare() { # deja el dump listo y el cluster embebido creado
  local st attempts ts
  st="$(state_get automigration STATE)"
  attempts="$(state_get automigration ATTEMPTS)"; attempts="${attempts:-0}"
  case "$st" in
    done)
      die "automigration already completed once, but $PGDATA is now empty (the migrated cluster was removed). To re-migrate from the external database, delete $STATE_DIR/state/automigration.env and restart; for a fresh empty install, unset DATABASE_URL." ;;
    failed)
      die "a previous automigration FAILED. Your data is intact in the external database. Inspect $BACKUP_DIR (dump preserved) and $STATE_DIR/state/automigration.env, fix the cause, then delete that file to retry." ;;
    in-progress)
      if [ "$attempts" -ge 3 ]; then
        state_set automigration STATE failed
        die "automigration failed 3 times — marking as failed. Your data is intact in the external database."
      fi
      warn "previous automigration attempt died midway — moving partial cluster aside and retrying"
      ts="$(date -u +%Y%m%dT%H%M%SZ)"
      mkdir -p "$STATE_DIR/failed-automigration-$ts"
      find "$PGDATA" -mindepth 1 -maxdepth 1 ! -name 'lost+found' \
        -exec mv -t "$STATE_DIR/failed-automigration-$ts/" {} + 2>/dev/null || true
      ;;
  esac

  log "waiting for external database to answer (max ${EXTERNAL_WAIT_SECS}s): migration source"
  local ok=0
  for _ in $(seq 1 "$EXTERNAL_WAIT_SECS"); do
    if pg_isready -q -d "$EXTERNAL_URL" 2>/dev/null; then ok=1; break; fi
    sleep 1
  done
  if [ "$ok" != 1 ]; then
    die "DATABASE_URL is set and the embedded volume is empty, but the external database does not answer. Refusing to start empty. Either bring the external DB up (one last time, to migrate), or unset DATABASE_URL for a fresh install."
  fi

  state_set automigration STATE in-progress
  state_set automigration ATTEMPTS "$((attempts + 1))"
  state_set automigration SOURCE "$(echo "$EXTERNAL_URL" | sed 's#//[^@]*@#//***@#')"

  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  AUTOMIGRATION_DUMP="$BACKUP_DIR/pre-automigration-${ts}.sql.gz"
  log "dumping external database (read-only) to $AUTOMIGRATION_DUMP"
  if ! pg_dump --no-owner --no-privileges -d "$EXTERNAL_URL" | gzip -6 > "$AUTOMIGRATION_DUMP"; then
    rm -f "$AUTOMIGRATION_DUMP"
    die "pg_dump from the external database failed — nothing was written locally, your data is intact"
  fi
  census -d "$EXTERNAL_URL" > "$STATE_DIR/state/automigration-source-census.txt" || true

  init_fresh_cluster
  AUTOMIGRATE_PENDING=1
}

automigrate_restore() { # con el PG embebido ya arrancado
  log "restoring dump into the embedded database"
  if ! gunzip -c "$AUTOMIGRATION_DUMP" | psql_local "$POSTGRES_DB" -q; then
    die "restore into the embedded database FAILED. Dump preserved at $AUTOMIGRATION_DUMP; your data is intact in the external database. Manual restore: gunzip -c <dump> | psql -h $SOCK_DIR -U $POSTGRES_USER -d $POSTGRES_DB"
  fi
  local src dst
  src="$(cat "$STATE_DIR/state/automigration-source-census.txt" 2>/dev/null || true)"
  dst="$(census -h "$SOCK_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" || true)"
  if [ -n "$src" ] && [ "$src" != "$dst" ]; then
    state_set automigration STATE failed
    die "automigration verification FAILED (row census differs). Dump: $AUTOMIGRATION_DUMP. External database untouched. source=[$src] embedded=[$dst]"
  fi
  state_set automigration STATE "done"
  local rows
  rows="$(printf '%s\n' "$dst" | awk -F: '{s+=$2} END {print s+0}')"
  log "automigration completed: $rows rows across $(printf '%s\n' "$dst" | grep -c . ) tables. The external database is NO LONGER USED — you can retire it and remove DATABASE_URL."
  SKIP_PREMIGRATION=1
  echo "$APP_VERSION" > "$STATE_DIR/state/last-version"
}

# ── Modo externo (deprecado) ─────────────────────────────────────────────────
warn_external_deprecated() {
  cat >&2 <<EOF
[futurefin-entrypoint] ============================ DEPRECATED ============================
[futurefin-entrypoint]  Estás usando una base de datos EXTERNA (DATABASE_URL definida).
[futurefin-entrypoint]  Desde 3.0.0 FutureFin incluye PostgreSQL en la propia imagen y este
[futurefin-entrypoint]  modo esta DEPRECADO: se eliminara en 4.0.0.
[futurefin-entrypoint]  Para migrar: monta un volumen en $PGDATA y reinicia;
[futurefin-entrypoint]  FutureFin copiara los datos automaticamente una sola vez.
[futurefin-entrypoint]  Ver: README seccion «Actualizar a 3.x».
[futurefin-entrypoint] ====================================================================
EOF
}

exec_api_external() {
  warn_external_deprecated
  export DATABASE_URL="$EXTERNAL_URL"
  log "starting FutureFin $APP_VERSION against the external database"
  if is_root; then
    exec gosu futurefin /app/futurefin-api
  else
    exec /app/futurefin-api
  fi
}

# ── API ──────────────────────────────────────────────────────────────────────
start_api() {
  export DATABASE_URL="postgres:///$POSTGRES_DB?host=$SOCK_DIR&user=$POSTGRES_USER"
  log "starting FutureFin API $APP_VERSION"
  if is_root; then
    gosu futurefin /app/futurefin-api &
  else
    /app/futurefin-api &
  fi
  API_PID=$!
}

supervise() {
  while :; do
    wait -n 2>/dev/null || true
    [ "$SHUTTING_DOWN" = 1 ] && exit 0
    if [ -n "$PG_PID" ] && ! kill -0 "$PG_PID" 2>/dev/null; then
      warn "PostgreSQL exited unexpectedly — shutting down"
      break
    fi
    if [ -n "$API_PID" ] && ! kill -0 "$API_PID" 2>/dev/null; then
      warn "API exited unexpectedly — shutting down"
      break
    fi
  done
  SHUTTING_DOWN=1
  stop_pid "$API_PID" TERM "$API_STOP_TIMEOUT"
  stop_pid "$PG_PID" INT "$PG_STOP_TIMEOUT" QUIT
  exit 1   # restart: unless-stopped nos recupera
}

print_db_only_help() {
  cat <<EOF
[futurefin-entrypoint] ── db-only mode ─────────────────────────────────────────
[futurefin-entrypoint] PostgreSQL is up (socket $SOCK_DIR); the API is NOT running.
[futurefin-entrypoint] Restore a backup from another terminal:
[futurefin-entrypoint]   docker exec -i <container> psql -h $SOCK_DIR -U $POSTGRES_USER -d postgres \\
[futurefin-entrypoint]     -c 'DROP DATABASE IF EXISTS "$POSTGRES_DB";' -c 'CREATE DATABASE "$POSTGRES_DB" OWNER "$POSTGRES_USER";'
[futurefin-entrypoint]   gunzip -c backup.sql.gz | docker exec -i <container> psql -h $SOCK_DIR -U $POSTGRES_USER -d $POSTGRES_DB
[futurefin-entrypoint] Stop with: docker stop <container>
[futurefin-entrypoint] ─────────────────────────────────────────────────────────
EOF
}

# ── main ─────────────────────────────────────────────────────────────────────
main() {
  local cmd="${1:-serve}"
  case "$cmd" in
    serve|db-only) ;;
    *) exec "$@" ;;
  esac
  MODE="${FUTUREFIN_MODE:-$cmd}"

  case "$DB_MODE" in auto|embedded|external) ;; *) die "invalid FUTUREFIN_DB_MODE='$DB_MODE' (auto|embedded|external)";; esac

  log "FutureFin $APP_VERSION — mode=$MODE db_mode=$DB_MODE postgres_majors=$(ls "$PG_BINROOT" | tr '\n' ' ')"
  ensure_runtime_dirs

  local MOUNTED=0
  is_mounted "$PGDATA" && MOUNTED=1

  EXTERNAL_URL=""
  if [ -n "${DATABASE_URL:-}" ] && ! printf '%s' "$DATABASE_URL" | grep -q "$SOCK_DIR"; then
    EXTERNAL_URL="$DATABASE_URL"
  fi

  # ── Decisión de modo de base de datos ──
  if [ "$DB_MODE" = external ]; then
    [ -n "$EXTERNAL_URL" ] || die "FUTUREFIN_DB_MODE=external but DATABASE_URL is not set"
    [ "$MODE" = db-only ] && die "db-only mode makes no sense with an external database"
    exec_api_external            # no vuelve
  fi

  if [ "$DB_MODE" = auto ] && [ -n "$EXTERNAL_URL" ]; then
    if [ "$MOUNTED" != 1 ]; then
      # Compose 2.x recreado con imagen 3.x (watchtower): sin volumen en el contenedor
      # de la app. Migrar aqui escribiria en la capa efimera → NUNCA. Modo compat.
      [ "$MODE" = db-only ] && die "db-only mode needs a volume mounted at $PGDATA"
      exec_api_external          # no vuelve
    fi
    # in-progress se comprueba ANTES que has_cluster: un cluster a medio restaurar no
    # debe adoptarse jamás — automigrate_prepare lo aparta y reintenta.
    if [ "$(state_get automigration STATE)" = in-progress ]; then
      automigrate_prepare
    elif has_cluster; then
      if [ "$(state_get automigration STATE)" = "done" ]; then
        warn "DATABASE_URL is set but data was already migrated to the embedded database — ignoring it. Remove DATABASE_URL from your compose."
      else
        warn "DATABASE_URL is set but $PGDATA already contains an embedded cluster with data — the EMBEDDED database wins. Set FUTUREFIN_DB_MODE=external if you really want the external one."
      fi
    elif pgdata_empty; then
      automigrate_prepare
    fi
  fi

  # ── Camino embebido ──
  if [ "$MOUNTED" != 1 ] && [ "$ALLOW_EPHEMERAL" != 1 ]; then
    die "no persistent volume is mounted at $PGDATA — your data would be LOST when the container is recreated. Mount a volume (see docker-compose.yml) or set FUTUREFIN_ALLOW_EPHEMERAL_DB=1 for throwaway use."
  fi
  if [ "$MOUNTED" != 1 ] && [ "$ALLOW_EPHEMERAL" = 1 ]; then
    warn "running with an EPHEMERAL database (no volume at $PGDATA) — all data is lost when this container is removed"
  fi

  if has_cluster; then
    adopt_cluster
    maybe_pg_upgrade
  elif [ "$AUTOMIGRATE_PENDING" != 1 ]; then
    if ! pgdata_empty; then
      die "$PGDATA is not empty but contains no PG_VERSION — refusing to touch it. Inspect the volume manually."
    fi
    init_fresh_cluster
  fi

  start_postgres
  wait_pg_ready postgres
  post_start_maintenance
  if [ "$PGUPGRADE_JUST_RAN" = 1 ]; then
    run_as_pg "$PG_BINROOT/$PG_MAJOR/bin/vacuumdb" \
      -h "$SOCK_DIR" -U "$POSTGRES_USER" --all --analyze-in-stages >/dev/null 2>&1 || true
  fi
  if [ "$AUTOMIGRATE_PENDING" = 1 ]; then
    automigrate_restore
  fi
  premigration_backup

  if [ "$MODE" = db-only ]; then
    print_db_only_help
    supervise
  fi

  start_api
  supervise
}

main "$@"
