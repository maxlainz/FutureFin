# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
ENV PATH=/usr/local/cargo/bin:$PATH
WORKDIR /src
COPY Cargo.toml ./
COPY crates/domain ./crates/domain
COPY apps/api ./apps/api
RUN cargo build --release -p futurefin-api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/futurefin-api /app/futurefin-api
ENV PORT=8080
EXPOSE 8080
USER nobody:nogroup
ENTRYPOINT ["/app/futurefin-api"]
