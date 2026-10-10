#!/usr/bin/env bash
# Entwicklungsumgebung: DB starten, migrieren, Server und Web-Dev-Server starten.
# Ctrl-C beendet beide Prozesse. Optional: SEED=1 scripts/dev.sh legt Beispieldaten an.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env ] || cp .env.example .env
set -a
# shellcheck disable=SC1091
. ./.env
set +a
export LINEAL_DEV=1

docker compose up -d --wait
cargo run -q -p lineal-server -- migrate
if [ "${SEED:-0}" = "1" ]; then
  cargo run -q -p lineal-server -- seed
fi

[ -d web/node_modules ] || (cd web && npm install)

cargo run -q -p lineal-server -- serve &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT INT TERM

echo "Server: ${LINEAL_BIND:-127.0.0.1:8080}   Web: http://localhost:5173"
(cd web && npm run dev)
