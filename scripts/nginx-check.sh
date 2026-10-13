#!/usr/bin/env bash
# Prüft deploy/nginx.conf ohne Anwendungs-Stack: Syntax (nginx -t) und das Verhalten der Web-
# Schicht gegen einen NICHT vorhandenen Upstream (CSP, Favicon, 404 statt Soft-404, JSON-Fehler
# mit Retry-After bei Server-Ausfall, schnelles Scheitern).
#
# Läuft mit einem eigenen kurzlebigen Docker-Netz und Container (Namen lineal-nginxcheck-*), fasst
# keine Compose-Projekte an. Nutzung: scripts/nginx-check.sh [<nginx-image>]
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE="${1:-nginxinc/nginx-unprivileged:1.27-alpine}"
NET="lineal-nginxcheck-net"
NAME="lineal-nginxcheck-web"
TMP="$(mktemp -d)"
fail=0

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

check() { # Beschreibung, Bedingung (Exit-Code eines Befehls)
  local desc="$1"; shift
  if "$@"; then echo "ok    $desc"; else echo "FAIL  $desc"; fail=1; fi
}

mkdir -p "$TMP/html/assets"
echo '<!doctype html><title>x</title><div id=root></div>' > "$TMP/html/index.html"
echo 'console.log(1)' > "$TMP/html/assets/app-abc.js"
chmod -R a+rX "$TMP"

echo "== nginx -t"
docker run --rm -v "$PWD/deploy/nginx.conf:/etc/nginx/conf.d/default.conf:ro" "$IMAGE" nginx -t 2>&1 | tail -2

docker network create "$NET" >/dev/null
docker run -d --name "$NAME" --network "$NET" \
  -v "$PWD/deploy/nginx.conf:/etc/nginx/conf.d/default.conf:ro" \
  -v "$TMP/html:/usr/share/nginx/html:ro" "$IMAGE" >/dev/null

curl_in() { docker run --rm --network "$NET" curlimages/curl:8.10.1 -s -m 15 "$@"; }
for _ in $(seq 1 30); do
  curl_in -o /dev/null "http://$NAME:8080/" 2>/dev/null && break
  sleep 0.5
done

BASE="http://$NAME:8080"
hdr() { curl_in -D - -o /dev/null "$BASE$1"; }

root="$(hdr /)"
check "CSP auf / (Skripte nur self, frame-ancestors none)" \
  bash -c "grep -qi \"content-security-policy:.*script-src 'self'\" <<<\"\$1\" && grep -qi \"frame-ancestors 'none'\" <<<\"\$1\"" _ "$root"
check "CSP erlaubt WebSocket zum eigenen Ursprung und Bilder aus /files (connect-src/img-src)" \
  bash -c "grep -qi \"connect-src 'self'\" <<<\"\$1\" && grep -qi \"img-src 'self' data: blob:\" <<<\"\$1\"" _ "$root"
check "CSP auf Client-Route (SPA-Fallback)" \
  bash -c "grep -qi 'content-security-policy' <<<\"\$1\"" _ "$(hdr /acme/team/ENG/active)"
check "CSP auf /assets/" \
  bash -c "grep -qi 'content-security-policy' <<<\"\$1\" && grep -qi 'immutable' <<<\"\$1\"" _ "$(hdr /assets/app-abc.js)"
check "/favicon.ico liefert 200 image/svg+xml" \
  bash -c "head -1 <<<\"\$1\" | grep -q ' 200' && grep -qi 'content-type: image/svg+xml' <<<\"\$1\"" _ "$(hdr /favicon.ico)"
for p in /robots.txt /.env /missing.js /assets/missing.js; do
  check "$p -> 404 statt Soft-404" bash -c "head -1 <<<\"\$1\" | grep -q ' 404'" _ "$(hdr "$p")"
done
check "Client-Route ohne Endung liefert index.html (200)" \
  bash -c "head -1 <<<\"\$1\" | grep -q ' 200'" _ "$(hdr /acme/inbox)"

# Server-Ausfall: kein Container "server" im Netz -> schnell 503 als JSON mit Retry-After.
start=$(date +%s)
down="$(curl_in -D - "$BASE/auth/me")"
elapsed=$(( $(date +%s) - start ))
check "Server aus: /auth/me -> 503 JSON mit Retry-After" \
  bash -c "head -1 <<<\"\$1\" | grep -q ' 503' && grep -qi 'retry-after' <<<\"\$1\" && grep -q '\"error\":\"unavailable\"' <<<\"\$1\"" _ "$down"
check "Server aus: Antwort in < 8 s (war bis 60 s)" test "$elapsed" -lt 8

# Rate-Limit des nginx: 429 als JSON mit Retry-After (limit_req lineal_auth 10 r/min, burst 10).
last=""
for _ in $(seq 1 25); do last="$(curl_in -D - -X POST "$BASE/auth/login")"; done
check "nginx-429 als JSON mit Retry-After" \
  bash -c "head -1 <<<\"\$1\" | grep -q ' 429' && grep -qi 'retry-after' <<<\"\$1\" && grep -q '\"error\":\"rate_limited\"' <<<\"\$1\"" _ "$last"

if (( fail )); then echo "nginx-check: FEHLER" >&2; exit 1; fi
echo "nginx-check: alles grün"
