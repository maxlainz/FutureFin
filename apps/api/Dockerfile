# syntax=docker/dockerfile:1
#
# Imagen publicable (convención Docker Hub): futurefin/futurefin:<tag>
# Bases de terceros fijadas por digest — actualizar solo de forma deliberada (véase README).
#
# Desde 3.0.0 la imagen es AUTOCONTENIDA: PostgreSQL corre dentro del propio contenedor
# (16 como versión activa + 15 solo para auto-pg_upgrade de volúmenes antiguos).
# Reglas que NO deben romperse:
#   - NO basar el runtime en `postgres:*`: su `VOLUME` heredado crea volúmenes anónimos
#     en `docker run` sin volumen, y watchtower los pierde al recrear (pérdida silenciosa).
#     Por lo mismo, NO declarar `VOLUME` aquí: el entrypoint detecta con `mountpoint` si
#     hay volumen real y se niega a arrancar sin él (salvo FUTUREFIN_ALLOW_EPHEMERAL_DB=1).
#   - Los digests de pg15/pg16 deben ser los del ÍNDICE multi-arch (manifest list), no los
#     de un manifest de una sola plataforma — si no, el build arm64 falla.

# --- Frontend (Vite) ---
FROM node:26.8-bookworm-slim@sha256:367679cf9792759492a486e4aa4b421764d71a9546a6dae8aab81a99eb797b3e AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/web ./apps/web
RUN npm ci && npm run build:web \
    && test -f apps/web/dist/index.html \
    && test -n "$(ls -A apps/web/dist/assets)"

# --- API (Rust): imagen `rust:bookworm` (stable); digest fija la snapshot exacta del toolchain ---
FROM rust:bookworm@sha256:e70e2eec3d495fd5c8e0be74adda86507dfac7f51a724fbf9813ff59b2b247c7 AS rust-builder
ENV PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/domain ./crates/domain
COPY crates/engine ./crates/engine
COPY apps/api ./apps/api
RUN cargo build --release -p futurefin-api --locked \
    # Manifiestos que consume el entrypoint: versión de la app y lista de migraciones
    # embebidas (para decidir el backup automático pre-migración sin arrancar la API).
    && grep -m1 '^version' apps/api/Cargo.toml | cut -d'"' -f2 > /version.txt \
    && ls apps/api/migrations/*.sql | xargs -n1 basename | cut -d_ -f1 | sort > /migration-versions.txt \
    && test -s /version.txt && test -s /migration-versions.txt

# --- Binarios PostgreSQL (estos stages son solo FUENTE de COPY, nunca la base) ---
FROM postgres:16-bookworm@sha256:60f4761b9035e0b8d5218f701a8c3382f641bf12b1604822574cf5be3baeb537 AS pg16
# La imagen oficial hace dpkg-divert de postgresql.conf.sample al directorio PADRE y deja
# un symlink en la ruta versionada — al copiar solo /usr/share/postgresql/16 el enlace
# quedaría roto e initdb fallaría. Materializar el fichero real (cada major con el SUYO).
RUN rm /usr/share/postgresql/16/postgresql.conf.sample \
    && cp /usr/share/postgresql/postgresql.conf.sample /usr/share/postgresql/16/postgresql.conf.sample \
    # libpq vive en un directorio con triplete de arquitectura; el glob de shell lo
    # resuelve por plataforma sin hardcodear x86_64/aarch64.
    && mkdir /pglib && cp -a /usr/lib/*-linux-gnu/libpq.so.5* /pglib/

FROM postgres:15-bookworm@sha256:cf7f8fb958c63e62875e30645dc4819ff0243a923f3c709e752b99dedd40bfcd AS pg15
RUN rm /usr/share/postgresql/15/postgresql.conf.sample \
    && cp /usr/share/postgresql/postgresql.conf.sample /usr/share/postgresql/15/postgresql.conf.sample

# --- Runtime ---
FROM debian:bookworm-slim@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252 AS runtime
# `jq`: el entrypoint lee /data/options.json cuando la imagen corre como add-on de
# Home Assistant (parsear JSON con sed/grep sería frágil y silenciosamente incorrecto).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl gosu locales jq \
        libicu72 libssl3 libxml2 libxslt1.1 liblz4-1 liblzma5 libzstd1 zlib1g \
        libgssapi-krb5-2 libldap-2.5-0 libsasl2-2 libpam0g libselinux1 \
        libedit2 libreadline8 libuuid1 libsystemd0 libffi8 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    # Los volúmenes creados por la imagen oficial (2.x) tienen datcollate=en_US.utf8:
    # sin esta locale generada, glibc no puede abrir esas bases ("could not create locale").
    && localedef -i en_US -c -f UTF-8 -A /usr/share/locale/locale.alias en_US.UTF-8

COPY --from=pg16 /usr/lib/postgresql/16 /usr/lib/postgresql/16
COPY --from=pg16 /usr/share/postgresql/16 /usr/share/postgresql/16
COPY --from=pg15 /usr/lib/postgresql/15 /usr/lib/postgresql/15
COPY --from=pg15 /usr/share/postgresql/15 /usr/share/postgresql/15
COPY --from=pg16 /pglib/ /usr/lib/

# JIT fuera: llvmjit arrastra libLLVM (~120 MB) y no aporta nada a esta carga de trabajo.
RUN rm -f /usr/lib/postgresql/*/lib/llvmjit.so /usr/lib/postgresql/*/lib/llvmjit_types.bc

# Gate de enlazado y de instalación: el build FALLA si a algún binario o librería de PG
# le falta una .so, o si quedó algún symlink roto en los árboles copiados (la causa del
# incidente del postgresql.conf.sample divertido).
RUN set -eu; fail=0; \
    for f in /usr/lib/postgresql/1?/bin/* /usr/lib/postgresql/1?/lib/*.so /usr/lib/libpq.so.5; do \
        if ldd "$f" 2>/dev/null | grep -q 'not found'; then \
            echo "MISSING LIBS in $f:"; ldd "$f" | grep 'not found'; fail=1; \
        fi; \
    done; \
    [ "$fail" -eq 0 ]; \
    broken="$(find /usr/lib/postgresql /usr/share/postgresql -xtype l)"; \
    if [ -n "$broken" ]; then echo "BROKEN SYMLINKS:"; echo "$broken"; exit 1; fi; \
    test -s /usr/share/postgresql/16/postgresql.conf.sample; \
    test -s /usr/share/postgresql/15/postgresql.conf.sample; \
    /usr/lib/postgresql/16/bin/postgres --version; \
    /usr/lib/postgresql/15/bin/postgres --version; \
    /usr/lib/postgresql/16/bin/pg_upgrade --version

# Dos usuarios sin privilegios: `postgres` (uid 999, como la imagen oficial Debian) para el
# postmaster y `futurefin` (uid 10001) para la API. El PID 1 es root solo para chown/gosu.
RUN groupadd -r -g 999 postgres \
    && useradd -r -u 999 -g postgres -d /var/lib/postgresql -s /bin/bash postgres \
    && groupadd -r -g 10001 futurefin \
    && useradd -r -u 10001 -g futurefin -d /nonexistent -s /usr/sbin/nologin futurefin \
    && mkdir -p /var/lib/postgresql/data /var/lib/futurefin /var/run/postgresql \
    && chown -R postgres:postgres /var/lib/postgresql /var/lib/futurefin /var/run/postgresql \
    && chmod 0700 /var/lib/postgresql/data \
    && chmod 2775 /var/run/postgresql

# Smoke de initdb en BUILD: ejercita binarios + ficheros share + locales de verdad.
# C.UTF-8 = camino de instalación nueva; en_US.utf8 = camino de adopción 2.x/pg_upgrade
# (valida también el localedef de arriba). Falla el build si initdb no puede crear un cluster.
RUN set -eu; \
    gosu postgres /usr/lib/postgresql/16/bin/initdb -D /tmp/_smoke1 --username=smoke --encoding=UTF8 --locale=C.UTF-8 >/dev/null; \
    gosu postgres /usr/lib/postgresql/16/bin/initdb -D /tmp/_smoke2 --username=smoke --encoding=UTF8 --locale=en_US.utf8 >/dev/null; \
    gosu postgres /usr/lib/postgresql/15/bin/initdb -D /tmp/_smoke3 --username=smoke --encoding=UTF8 --locale=en_US.utf8 >/dev/null; \
    rm -rf /tmp/_smoke1 /tmp/_smoke2 /tmp/_smoke3

WORKDIR /app
COPY --from=rust-builder --chown=root:root /src/target/release/futurefin-api /app/futurefin-api
COPY --from=rust-builder /version.txt /app/VERSION
COPY --from=rust-builder /migration-versions.txt /app/migration-versions.txt
COPY --from=web --chown=root:root /build/apps/web/dist /app/web
COPY apps/api/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod 0555 /docker-entrypoint.sh /app/futurefin-api

ENV PORT=8080 \
    WEB_STATIC_ROOT=/app/web \
    PGDATA=/var/lib/postgresql/data \
    FUTUREFIN_STATE_DIR=/var/lib/futurefin \
    PG_MAJOR=16 \
    PATH=/usr/lib/postgresql/16/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

LABEL com.futurefin.postgres.majors="15,16"

EXPOSE 8080
# Forma shell (resuelve PATH — incidente v1.0.2). /v1/ready valida también el PG embebido;
# NO añadir un fallback `</dev/tcp/...`: enmascararía un 503 con la base caída.
HEALTHCHECK --interval=15s --timeout=5s --start-period=120s --retries=5 \
    CMD curl -fsS http://127.0.0.1:8080/v1/ready >/dev/null || exit 1

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["serve"]
