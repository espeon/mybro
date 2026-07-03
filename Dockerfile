# ── Stage 1: Build frontend ───────────────────────────────────────────────────
FROM node:22-slim AS frontend

WORKDIR /app/uman-frontend

RUN corepack enable && corepack prepare pnpm@latest --activate

COPY uman-frontend/package.json uman-frontend/pnpm-lock.yaml uman-frontend/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile

COPY uman-frontend/ ./
RUN pnpm build

# ── Stage 2: Build Rust binary ───────────────────────────────────────────────
FROM rust:1-bookworm AS backend

WORKDIR /app

# Cache dependencies
COPY Cargo.toml ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release 2>/dev/null || true

# Copy source + frontend dist (rust-embed needs dist/ at compile time)
COPY src/ ./src/
COPY --from=frontend /app/uman-frontend/dist/ ./uman-frontend/dist/

RUN cargo build --release

# ── Stage 3: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=backend /app/target/release/uman /app/uman

# Directories for runtime state
RUN mkdir -p /app/.config /app/.logs /app/.cache

VOLUME ["/app/.config", "/app/.logs", "/app/.cache"]

EXPOSE 8084

ENV LISTEN_ADDR=0.0.0.0:8084

ENTRYPOINT ["/app/uman"]
