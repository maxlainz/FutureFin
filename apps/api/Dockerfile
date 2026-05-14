# syntax=docker/dockerfile:1
#
# Imagen publicable (convención Docker Hub): futurefin/futurefin:<tag>
# Bases de terceros fijadas por digest — actualizar solo de forma deliberada (véase README).

# --- Frontend (Vite) ---
FROM node:24.15-bookworm-slim@sha256:24dc26ef1e3c3690f27ebc4136c9c186c3133b25563ae4d7f0692e4d1fe5db0e AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/web ./apps/web
RUN npm ci && npm run build:web \
    && test -f apps/web/dist/index.html \
    && test -n "$(ls -A apps/web/dist/assets)"

# --- API (Rust): imagen `rust:bookworm` (stable); digest fija la snapshot exacta del toolchain ---
FROM rust:bookworm@sha256:adab7941580c74513aa3347f2d2a1f975498280743d29ec62978ba12e3540d3a AS rust-builder
ENV PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/domain ./crates/domain
COPY crates/engine ./crates/engine
COPY apps/api ./apps/api
RUN cargo build --release -p futurefin-api --locked

# --- Runtime ---
FROM debian:bookworm-slim@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252 AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust-builder --chown=root:root /src/target/release/futurefin-api /app/futurefin-api
COPY apps/api/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod 0555 /docker-entrypoint.sh /app/futurefin-api
COPY --from=web --chown=nobody:nogroup /build/apps/web/dist /app/web
ENV PORT=8080
ENV WEB_STATIC_ROOT=/app/web
USER nobody:nogroup
EXPOSE 8080
ENTRYPOINT ["/docker-entrypoint.sh"]
