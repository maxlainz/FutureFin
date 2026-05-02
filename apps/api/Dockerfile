# syntax=docker/dockerfile:1
#
# Imagen publicable (convención Docker Hub): futurefin/futurefin-api:<tag>
# Bases de terceros fijadas por digest — actualizar solo de forma deliberada (véase README).

# --- Frontend (Vite) ---
FROM node:22.14-bookworm-slim@sha256:1c18d9ab3af4585870b92e4dbc5cac5a0dc77dd13df1a5905cea89fc720eb05b AS web
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/web ./apps/web
RUN npm ci && npm run build:web

# --- API (Rust): imagen `rust:bookworm` (stable); digest fija la snapshot exacta del toolchain ---
FROM rust:bookworm@sha256:adab7941580c74513aa3347f2d2a1f975498280743d29ec62978ba12e3540d3a AS rust-builder
ENV PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/domain ./crates/domain
COPY apps/api ./apps/api
RUN cargo build --release -p futurefin-api --locked

# --- Runtime ---
FROM debian:bookworm-slim@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252 AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust-builder --chown=nobody:nogroup /src/target/release/futurefin-api /app/futurefin-api
COPY --from=web --chown=nobody:nogroup /build/apps/web/dist /app/web
ENV PORT=8080
ENV WEB_STATIC_ROOT=/app/web
USER nobody:nogroup
EXPOSE 8080
ENTRYPOINT ["/app/futurefin-api"]
