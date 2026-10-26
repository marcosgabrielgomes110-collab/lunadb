# ── Stage 1: Build ─────────────────────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Cache dependencies by building a dummy project first.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/lib.rs && \
    cargo build --release --locked 2>/dev/null || true && \
    rm -rf src

# Build the real binary.
COPY src/ src/
RUN cargo build --release --locked && \
    strip target/release/lunadb

# ── Stage 2: Runtime ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Only the minimal runtime libs (libc, libgcc); no compilers, no Rust toolchain.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for the daemon.
RUN groupadd --system lunadb && \
    useradd --system --gid lunadb --create-home --home-dir /var/lib/lunadb lunadb

COPY --from=builder /build/target/release/lunadb /usr/local/bin/lunadb

# Data, config, and log directories.
RUN mkdir -p /var/lib/lunadb /etc/lunadb && \
    chown -R lunadb:lunadb /var/lib/lunadb /etc/lunadb

USER lunadb
WORKDIR /var/lib/lunadb

# Default storage and config paths inside the container.
ENV LUNADB_STORAGE_PATH=/var/lib/lunadb \
    LUNADB_CONFIG=/etc/lunadb/config.toml

EXPOSE 7700

HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/lunadb", "shell", "--server", "http://127.0.0.1:7700", "--eval", "health"] || exit 1

ENTRYPOINT ["lunadb"]
CMD ["serve", "--config", "/etc/lunadb/config.toml"]
