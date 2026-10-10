# syntax=docker/dockerfile:1.7
#
# Lineal – Container-Images (docs/15 §3 Betrieb).
#
#   docker build --target server -t lineal-server .   # Binary `lineal` (serve, worker, migrate, seed, import)
#   docker build --target web    -t lineal-web .      # nginx: Web-App + Reverse Proxy auf den Server
#
# Zusammen gestartet über `docker compose --profile app up -d` (siehe docker-compose.yml).

ARG RUST_VERSION=1
ARG NODE_VERSION=22

# ---------------------------------------------------------------------------------------------
# Rust: Release-Build des Binaries. rust-toolchain.toml wird bewusst nicht kopiert (das Image
# bringt stable mit; rustfmt/clippy werden für den Build nicht gebraucht).
FROM rust:${RUST_VERSION}-bookworm AS rust-build
WORKDIR /src
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY crates ./crates
# lineal-mcp bettet die Fachdoku und den Tool-Snapshot ein (include_str!).
COPY docs ./docs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p lineal-server \
    && install -D -m 0755 target/release/lineal /out/lineal

# ---------------------------------------------------------------------------------------------
# Web: Vite-Build (Codegen läuft gegen das eingecheckte web/schema.graphql).
FROM node:${NODE_VERSION}-bookworm-slim AS web-build
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY web ./
RUN npm run build

# ---------------------------------------------------------------------------------------------
# Laufzeit Server: schlankes Debian, kein Root, Daten unter /data.
FROM debian:bookworm-slim AS server
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin lineal \
    && install -d -o lineal -g lineal /data /data/blobs
COPY --from=rust-build /out/lineal /usr/local/bin/lineal
USER lineal
WORKDIR /data
ENV LINEAL_BIND=0.0.0.0:8080 \
    LINEAL_BLOB_DIR=/data/blobs \
    RUST_LOG=info
VOLUME ["/data"]
EXPOSE 8080
# /healthz ohne curl: Bash-/dev/tcp (nur für `serve` sinnvoll; Compose setzt es je Dienst).
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=5 \
    CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && head -1 <&3 | grep -q ' 200 '"]
ENTRYPOINT ["lineal"]
CMD ["serve"]

# ---------------------------------------------------------------------------------------------
# Laufzeit Web: nginx (unprivilegiert, Port 8080) liefert die SPA aus und leitet API-Pfade weiter.
FROM nginxinc/nginx-unprivileged:1.27-alpine AS web
COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=web-build /web/dist /usr/share/nginx/html
EXPOSE 8080
