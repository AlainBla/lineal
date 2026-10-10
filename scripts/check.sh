#!/usr/bin/env bash
# Komplette Prüfung (CI und lokal): fmt, clippy, tests, Schema-Export, Web.
# Voraussetzungen: Rust stable mit rustfmt + clippy, Node 22+/npm, Docker (Postgres auf :5440).
set -euo pipefail
cd "$(dirname "$0")/.."

export DATABASE_URL="${DATABASE_URL:-postgres://lineal:lineal@localhost:5440/lineal}"

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

if ! command -v cargo-fmt >/dev/null || ! command -v cargo-clippy >/dev/null; then
  echo "rustfmt/clippy fehlen: 'sudo apt install rustfmt rust-clippy' oder 'rustup component add rustfmt clippy'" >&2
  exit 1
fi

step "PostgreSQL starten (docker compose, Port 5440)"
docker compose up -d --wait

step "cargo fmt --check"
cargo fmt --all -- --check

step "cargo clippy -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --workspace

step "cargo deny: advisories, bans, licenses (optional)"
if cargo deny --version >/dev/null 2>&1; then
  cargo deny check advisories bans licenses
else
  echo "übersprungen: cargo-deny fehlt (cargo install cargo-deny --locked)"
fi

step "GraphQL-Schema exportieren (web/schema.graphql wird neu erzeugt)"
cargo run -q -p lineal-server -- schema-export --out web/schema.graphql

step "web: npm ci, typecheck, lint, test"
(
  cd web
  npm ci --no-audit --no-fund
  npm run typecheck
  npm run lint
  npm test
  # Laufzeit-Abhängigkeiten müssen frei von hohen Schwachstellen sein (bricht ab).
  npm audit --omit=dev --audit-level=high
  # Dev-Werkzeuge: Funde nur anzeigen. Bekannt: braces (ReDoS) in der @graphql-codegen-Kette,
  # es gibt noch keine gepatchte Version; betrifft nur den Build mit eigenen Dateien (docs/15 §4.6).
  npm audit --audit-level=high >/dev/null 2>&1 || echo "Hinweis: npm audit meldet Funde in Dev-Werkzeugen (bekannt, siehe docs/15 Abschnitt 4.6)"
)

printf '\n\033[1;32mcheck.sh: alles grün\033[0m\n'
