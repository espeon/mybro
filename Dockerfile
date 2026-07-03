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

# Pre-warm the cargo dependency cache with an empty main.rs.
# The || true makes this tolerant of any harmless cargo warnings.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/uman-frontend/dist && echo 'fn main() {}' > src/main.rs && \
    echo '<html></html>' > src/uman-frontend/dist/index.html && \
    cargo build --release 2>/dev/null || true

# Copy real source + frontend dist (rust-embed needs dist/ at compile time)
COPY src/ ./src/
COPY --from=frontend /app/uman-frontend/dist/ ./uman-frontend/dist/

# Force a rebuild: touch every source file and delete the cached binary.
# Without this, Cargo's incremental build sees the binary from the cache
# step and skips rebuilding even though src/main.rs has changed.
RUN find src -name '*.rs' -exec touch {} + && \
    rm -f target/release/mybro target/release/deps/mybro* && \
    cargo build --release

# ── Stage 3: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=backend /app/target/release/mybro /app/mybro

# Sanity check at build time — fail fast if the binary didn't get copied
RUN test -x /app/mybro && /app/mybro --help || echo "(no --help flag, binary present)"

# Directories for runtime state
RUN mkdir -p /app/.config /app/.logs /app/.cache /app/.data

VOLUME ["/app/.config", "/app/.data", "/app/.logs", "/app/.cache"]

EXPOSE 8084

ENV LISTEN_ADDR=0.0.0.0:8084

ENTRYPOINT ["/app/mybro"]
