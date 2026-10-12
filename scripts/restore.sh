#!/usr/bin/env bash
# Wiederherstellung eines Backups aus scripts/backup.sh in den Docker-App-Stack.
#
# Nutzung:  scripts/restore.sh [--yes] <backup-verzeichnis>
#   --yes   ohne Rückfrage (Automatisierung); sonst muss "restore" eingegeben werden.
#
# Ablauf:
#   1. manifest.txt lesen, sha256 und Größen von database.dump und blobs.tar.gz prüfen
#   2. Warnung, wenn der Migrationsstand des Backups neuer ist als die Migrationen dieses
#      Checkouts (die Images bauen aus demselben Stand; sonst bricht migrate ab)
#   3. Bestätigung; danach: server, worker, web stoppen
#   4. DB per pg_restore --clean --if-exists --no-owner (über app-db, Passwort nicht nötig)
#   5. Inhalt des Volumes "<Projekt>_blobs" durch das Archiv ersetzen
#   6. migrate ausführen (vor dem Start der Anwendung, damit das Schema zum Image passt)
#   7. server, worker, web wieder starten
#
# Umgebung: COMPOSE_PROJECT_NAME (Default lineal), wie bei scripts/backup.sh.
# Exit-Code ungleich 0 bei jedem Fehler. Wird vor Schritt 3 abgebrochen, bleibt der Stack
# unverändert.
set -euo pipefail
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() { echo "FEHLER: $*" >&2; exit 1; }
info() { echo "==> $*"; }
warn() { echo "WARNUNG: $*" >&2; }

load_from_dotenv() {
  local key="$1" line value
  [[ -z "${!key+x}" && -f "$ROOT/.env" ]] || return 0
  line="$(grep -E "^${key}=" "$ROOT/.env" | tail -n 1 || true)"
  [[ -n "$line" ]] || return 0
  value="${line#*=}"
  value="${value%\"}"; value="${value#\"}"
  value="${value%\'}"; value="${value#\'}"
  export "$key=$value"
}
load_from_dotenv COMPOSE_PROJECT_NAME
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-lineal}"
BLOB_VOLUME="${COMPOSE_PROJECT_NAME}_blobs"

ASSUME_YES=0
BACKUP_ARG=""
for arg in "$@"; do
  case "$arg" in
    --yes) ASSUME_YES=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    -*) die "unbekannte Option: $arg" ;;
    *)
      [[ -z "$BACKUP_ARG" ]] || die "nur ein Backup-Verzeichnis angeben"
      BACKUP_ARG="$arg"
      ;;
  esac
done
[[ -n "$BACKUP_ARG" ]] || die "Aufruf: scripts/restore.sh [--yes] <backup-verzeichnis>"
[[ -d "$BACKUP_ARG" ]] || die "kein Verzeichnis: $BACKUP_ARG"
BACKUP_DIR="$(cd "$BACKUP_ARG" && pwd)"

MANIFEST="$BACKUP_DIR/manifest.txt"
DUMP="$BACKUP_DIR/database.dump"
BLOBS="$BACKUP_DIR/blobs.tar.gz"
[[ -f "$MANIFEST" ]] || die "manifest.txt fehlt in $BACKUP_DIR"
[[ -f "$DUMP" ]] || die "database.dump fehlt in $BACKUP_DIR"
[[ -f "$BLOBS" ]] || die "blobs.tar.gz fehlt in $BACKUP_DIR"

command -v docker >/dev/null || die "docker nicht gefunden"
docker compose version >/dev/null 2>&1 || die "'docker compose' (Compose v2) nicht verfügbar"
command -v sha256sum >/dev/null || command -v shasum >/dev/null || die "sha256sum oder shasum fehlt"

dc() { docker compose --profile app "$@"; }

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mget() {
  local key="$1" value
  value="$(grep -E "^${key}=" "$MANIFEST" | tail -n 1 | cut -d= -f2- || true)"
  [[ -n "$value" ]] || die "Eintrag '$key' fehlt in manifest.txt"
  printf '%s' "$value"
}

# 1. Integrität
info "Prüfe Backup $BACKUP_DIR"
for pair in "database:$DUMP" "blobs:$BLOBS"; do
  kind="${pair%%:*}"
  file="${pair#*:}"
  expected_sha="$(mget "${kind}_sha256")"
  expected_bytes="$(mget "${kind}_bytes")"
  actual_sha="$(sha256_of "$file")"
  actual_bytes="$(wc -c < "$file" | tr -d ' ')"
  [[ "$actual_bytes" == "$expected_bytes" ]] \
    || die "Größe von $(basename "$file") weicht ab (manifest $expected_bytes, Datei $actual_bytes)"
  [[ "$actual_sha" == "$expected_sha" ]] \
    || die "sha256 von $(basename "$file") stimmt nicht mit manifest.txt überein – Backup beschädigt oder verändert"
done
BACKUP_VERSION="$(mget migration_version)"
[[ "$BACKUP_VERSION" =~ ^[0-9]+$ ]] || die "migration_version in manifest.txt ungültig"
BLOB_VOLUME_BACKUP="$(mget blobs_volume)"
echo "    Zeitpunkt (UTC): $(mget created_utc), Git: $(mget git_commit), Migrationsstand: $BACKUP_VERSION"

# 2. Migrationsstand gegen den Checkout (Quelle der Images)
IMAGE_VERSION=0
for f in "$ROOT"/crates/lineal-db/migrations/*.sql; do
  v="$(basename "$f" | cut -d_ -f1)"
  [[ "$v" =~ ^[0-9]+$ ]] || continue
  if (( 10#$v > IMAGE_VERSION )); then IMAGE_VERSION=$((10#$v)); fi
done
if (( BACKUP_VERSION > IMAGE_VERSION )); then
  warn "Das Backup hat den Migrationsstand $BACKUP_VERSION, die Migrationen dieses Checkouts reichen nur bis $IMAGE_VERSION."
  warn "Das Backup stammt aus einer neueren Version. Die Images bzw. den Checkout zuerst auf diesen Stand bringen,"
  warn "sonst bricht 'migrate' nach dem Restore ab und die Anwendung startet nicht."
fi

# 3. Bestätigung
echo
echo "Ziel: Projekt '$COMPOSE_PROJECT_NAME' – Datenbank 'lineal' und Volume '$BLOB_VOLUME' werden ÜBERSCHRIEBEN."
echo "Backup-Volume laut Manifest: $BLOB_VOLUME_BACKUP"
if (( ASSUME_YES == 0 )); then
  read -r -p "Zum Fortfahren 'restore' eingeben: " answer || answer=""
  [[ "$answer" == "restore" ]] || die "abgebrochen (keine Änderung vorgenommen)"
fi

docker volume inspect "$BLOB_VOLUME" >/dev/null 2>&1 \
  || die "Volume '$BLOB_VOLUME' nicht gefunden (Projekt '$COMPOSE_PROJECT_NAME')"

info "app-db starten, server/worker/web stoppen"
dc up -d --wait app-db
dc stop server worker web >/dev/null 2>&1 || true

info "Datenbank wiederherstellen (pg_restore --clean --if-exists --no-owner)"
dc exec -T app-db pg_restore -U lineal -d lineal --clean --if-exists --no-owner < "$DUMP" \
  || die "pg_restore ist fehlgeschlagen. server/worker/web bleiben gestoppt; Ursache beheben und erneut ausführen."

info "Blob-Volume $BLOB_VOLUME ersetzen"
docker run --rm \
  -v "$BLOB_VOLUME:/data" \
  -v "$BACKUP_DIR:/backup:ro" \
  alpine sh -c 'find /data -mindepth 1 -maxdepth 1 -exec rm -rf {} + && tar xzf /backup/blobs.tar.gz --numeric-owner -C /data' \
  || die "Blob-Volume konnte nicht wiederhergestellt werden. server/worker/web bleiben gestoppt."

info "Migrationen ausführen"
dc run --rm migrate \
  || die "migrate ist fehlgeschlagen. server/worker/web bleiben gestoppt (siehe Warnung oben, falls der Stand neuer ist)."

info "server, worker, web starten"
dc up -d --wait server worker web

echo
echo "Restore abgeschlossen. Backup: $BACKUP_DIR"
