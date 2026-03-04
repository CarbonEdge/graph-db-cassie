FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release --locked --bin cassie-api

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -m -s /bin/false cassie

COPY --from=builder /app/target/release/cassie-api /usr/local/bin/

USER cassie

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s \
  CMD wget -qO- http://localhost:8080/health || exit 1

ENTRYPOINT ["cassie-api"]
