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

DB_MODE="${FUTUREFIN_DB_MODE:-auto}"                  # auto | embedded (external retirado en 4.0.0)
BACKUP_KEEP="${FUTUREFIN_BACKUP_KEEP:-10}"
BACKUP_KEEP_DAYS="${FUTUREFIN_BACKUP_KEEP_DAYS:-90}"
PREMIGRATION_BACKUP="${FUTUREFIN_PREMIGRATION_BACKUP:-on}"
ALLOW_EPHEMERAL="${FUTUREFIN_ALLOW_EPHEMERAL_DB:-0}"
API_STOP_TIMEOUT="${FUTUREFIN_API_STOP_TIMEOUT:-15}"
PG_STOP_TIMEOUT="${FUTUREFIN_PG_STOP_TIMEOUT:-30}"
POSTGRES_USER="${POSTGRES_USER:-futurefin}"
POSTGRES_DB="${POSTGRES_DB:-futurefin}"

APP_VERSION="$(cat /app/VERSION 2>/dev/null || echo unknown)"
API_PID=""
PG_PID=""
SHUTTING_DOWN=0
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
    die "PGDATA was created by PostgreSQL $old and this image only bundles $(ls "$PG_BINROOT" | tr '\n' ',' | sed 's/,$//'). Options: (1) start an older FutureFin release that bundles $old to upgrade stepwise; (2) dump with the official postgres:$old image and restore into a fresh volume with scripts/restore-postgres.sh."
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
# ── Base de datos externa: retirada en 4.0.0 ────────────────────────────────────
# Hasta la 3.x esta imagen sabía tres cosas más: hablar con una base externa
# (`exec_api_external`), avisar de que estaba deprecada, y migrar sus datos a la base
# embebida una sola vez (`automigrate_prepare`/`automigrate_restore`). Se anunció su
# eliminación en el README, en `.env.example` y en el propio aviso de deprecación desde
# la 3.0.0, y aquí está.
#
# Lo único que queda es la puerta de abajo: alguien que actualice a 4.0.0 con
# `DATABASE_URL` todavía puesta y sin cluster embebido NO debe arrancar con una base
# vacía. Eso se leería como pérdida de datos aunque sus datos estén intactos al otro
# lado. Se para y se le dice exactamente qué hacer.
refuse_external_database() {
  cat >&2 <<'MSG'

[futurefin-entrypoint] ─────────────────────────────────────────────────────────
  DATABASE_URL apunta a una base de datos EXTERNA y este volumen no contiene
  todavía una base embebida.

  FutureFin 4.0.0 ya no habla con bases de datos externas: PostgreSQL va dentro
  de la imagen. Tus datos NO se han tocado y siguen intactos donde están.

  Para migrarlos:
    1. Arranca UNA VEZ FutureFin 3.9.0 con esta misma DATABASE_URL y este mismo
       volumen. Copiará tus datos a la base embebida y te lo dirá en los logs
       ("automigration completed").
    2. Quita DATABASE_URL de tu compose.
    3. Vuelve a 4.0.0.

  Documentación: https://github.com/maxlainz/FutureFin/blob/main/docs/actualizar.md
─────────────────────────────────────────────────────────────────────────────────
MSG
  exit 1
}

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

  # `external` se acepta como valor para poder dar un mensaje útil en vez de un error
  # críptico a quien lo arrastre en su compose desde la 3.x.
  case "$DB_MODE" in
    auto|embedded) ;;
    external) die "FUTUREFIN_DB_MODE=external ya no existe: FutureFin 4.0.0 solo usa la base embebida. Quita esa variable (y DATABASE_URL) de tu compose; si aún no has migrado, arranca una vez la 3.9.0 para hacerlo." ;;
    *) die "invalid FUTUREFIN_DB_MODE='$DB_MODE' (auto|embedded)" ;;
  esac

  log "FutureFin $APP_VERSION — mode=$MODE db_mode=$DB_MODE postgres_majors=$(ls "$PG_BINROOT" | tr '\n' ' ')"
  ensure_runtime_dirs

  local MOUNTED=0
  is_mounted "$PGDATA" && MOUNTED=1

  EXTERNAL_URL=""
  if [ -n "${DATABASE_URL:-}" ] && ! printf '%s' "$DATABASE_URL" | grep -q "$SOCK_DIR"; then
    EXTERNAL_URL="$DATABASE_URL"
  fi

  # ── DATABASE_URL heredada de la 3.x ──
  # Con cluster embebido ya presente, la externa se ignora: quien migró en la 3.x tiene sus
  # datos aquí y solo le sobra una variable en el compose. Sin cluster, se para (arrancar
  # con una base vacía se leería como pérdida de datos).
  if [ -n "$EXTERNAL_URL" ]; then
    if has_cluster; then
      warn "DATABASE_URL está definida pero FutureFin 4.0.0 solo usa la base embebida, que ya tiene tus datos — se ignora. Quítala de tu compose."
    else
      refuse_external_database   # no vuelve
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
  else
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
  premigration_backup

  if [ "$MODE" = db-only ]; then
    print_db_only_help
    supervise
  fi

  start_api
  supervise
}

main "$@"
