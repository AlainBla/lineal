#!/usr/bin/env bash
# End-to-End-Tests (Playwright) gegen einen echten Server. Bewusst NICHT Teil von check.sh
# (dauert deutlich länger). Argumente werden an `playwright test` durchgereicht,
# z. B. `scripts/e2e.sh -g "F3"`.
#
# Ablauf: Datenbank `lineal_e2e` im Container (Port 5440) neu anlegen, migrieren,
# `lineal seed --demo`, Web bauen, dann startet Playwright `lineal serve`, `lineal worker` und
# `vite preview` (Proxy auf den Server) auf freien Ports. Browser: Chromium, headless.
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

free_port() { # erster freier TCP-Port ab $1
  python3 - "$1" <<'PY'
import socket, sys
port = int(sys.argv[1])
while True:
    with socket.socket() as s:
        try:
            s.bind(("127.0.0.1", port))
            print(port)
            break
        except OSError:
            port += 1
PY
}

E2E_DB_URL="postgres://lineal:lineal@127.0.0.1:5440/lineal_e2e"

step "PostgreSQL starten und Datenbank lineal_e2e neu anlegen"
docker compose up -d --wait
docker compose exec -T postgres psql -U lineal -d postgres -v ON_ERROR_STOP=1 \
  -c "DROP DATABASE IF EXISTS lineal_e2e WITH (FORCE)" -c "CREATE DATABASE lineal_e2e"

step "lineal bauen, migrieren, Demo-Daten anlegen"
cargo build -p lineal-server
export LINEAL_BIN="$PWD/target/debug/lineal"
(
  export DATABASE_URL="$E2E_DB_URL" LINEAL_DEV=1 RUST_LOG=warn
  "$LINEAL_BIN" migrate
  "$LINEAL_BIN" seed --demo
)
rm -rf /tmp/lineal-e2e-blobs

step "API-Keys für den MCP-Flow (F17) anlegen: Bob, einmal frei, einmal nur ENG"
# API-Keys lassen sich (noch) nicht über GraphQL anlegen; der Test bekommt feste, erfundene Tokens.
export LINEAL_E2E_API_KEY="lin_api_e2e_bob_full_0000000000000001"
export LINEAL_E2E_API_KEY_ENG="lin_api_e2e_bob_eng_0000000000000002"
key_sql() { # $1 Token, $2 Label, $3 team_ids-Ausdruck
  local hash
  hash="$(printf '%s' "$1" | sha256sum | cut -d' ' -f1)"
  printf "INSERT INTO api_key (organization_id, user_id, label, key_prefix, key_hash, scopes, team_ids) SELECT u.organization_id, u.id, '%s', left('%s', 12), '%s', ARRAY['read','write'], %s FROM \"user\" u WHERE u.email = 'bob@acme.example';" "$2" "$1" "$hash" "$3"
}
{
  key_sql "$LINEAL_E2E_API_KEY" "e2e full" "NULL"
  key_sql "$LINEAL_E2E_API_KEY_ENG" "e2e eng only" "ARRAY(SELECT id FROM team WHERE key = 'ENG')"
} | docker compose exec -T postgres psql -U lineal -d lineal_e2e -v ON_ERROR_STOP=1

export LINEAL_E2E_API_PORT="$(free_port 18741)"
export LINEAL_E2E_WEB_PORT="$(free_port 4173)"
export LINEAL_E2E_DATABASE_URL="$E2E_DB_URL"
echo "API :$LINEAL_E2E_API_PORT, Web :$LINEAL_E2E_WEB_PORT"

step "Web bauen und Chromium bereitstellen"
cd web
npm ci --no-audit --no-fund
npm run build
npx playwright install chromium

step "Playwright"
npx playwright test "$@"

printf '\n\033[1;32me2e.sh: alles grün\033[0m\n'
