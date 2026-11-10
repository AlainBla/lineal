#!/usr/bin/env bash
# Wiederherstellung eines Backups aus scripts/backup.sh in den Docker-App-Stack.
#
# Nutzung:  scripts/restore.sh [--yes] [-f <compose-datei>]... [--allow-newer] <backup-verzeichnis>
#   --yes           ohne Rückfrage (Automatisierung); sonst muss "restore" eingegeben werden.
#   -f, --file F    Compose-Datei (wiederholbar), z. B. das Overlay des Zweitstacks; entspricht
#                   COMPOSE_FILE. Ohne -f gelten COMPOSE_FILE aus der Umgebung bzw. aus .env.
#   --allow-newer   Backup mit neuerem Migrationsstand als dieser Checkout trotzdem einspielen
#                   (Standard: Abbruch vor jeder Änderung).
#
# Zweitstack (z. B. Test): dieselben Dateien und denselben Projektnamen wie beim Start verwenden,
# sonst würde Compose die Container mit falscher Konfiguration (Ports, Netze) neu anlegen:
#   COMPOSE_PROJECT_NAME=lineal-test scripts/restore.sh --yes \
#     -f docker-compose.yml -f deploy/docker-compose.test.yml backups/lineal-<ts>
#
# Ablauf:
#   1. manifest.txt lesen, sha256 und Größen von database.dump und blobs.tar.gz prüfen
#   2. Warnung, wenn der Migrationsstand des Backups neuer ist als die Migrationen dieses
#      Checkouts (die Images bauen aus demselben Stand; sonst bricht migrate ab)
#   3. Bestätigung, Vorabprüfung (DB-Verbindung der Anwendung, Passwort passt), Sicherung des
#      Ist-Zustands (pg_dump nach <Backup-Verzeichnis>/../pre-restore-<Zeitstempel>.dump);
#      danach: server, worker, web stoppen (Container werden nie neu angelegt)
#   4. DB per pg_restore --clean --if-exists --no-owner --single-transaction --exit-on-error
#      (atomar: bei einem Fehler bleibt der alte Datenstand)
#   5. Inhalt des Volumes "<Projekt>_blobs" durch das Archiv ersetzen
#   6. migrate ausführen (vor dem Start der Anwendung, damit das Schema zum Image passt)
#   7. server, worker, web wieder starten
#
# Umgebung: COMPOSE_PROJECT_NAME (Default lineal) und COMPOSE_FILE, wie bei scripts/backup.sh;
# beide werden ggf. aus .env übernommen.
#
# Verschlüsselte Backups (S-05/LIN-33): erkennt der Restore an der .age-Endung bzw. encrypted=1 im
# Manifest. Dann muss LINEAL_BACKUP_AGE_IDENTITY auf die age-Identity (privater Schlüssel) zeigen –
# NUR aus der Umgebung des Operators, nie aus .env (die .env ist evtl. gerade das, was
# wiederhergestellt wird, und der Schlüssel soll nicht auf dem Host liegen). Entschlüsselt wird im
# Stream, es landet kein Klartext-Backup auf der Platte.
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
load_from_dotenv COMPOSE_FILE
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-lineal}"
BLOB_VOLUME="${COMPOSE_PROJECT_NAME}_blobs"

ASSUME_YES=0
ALLOW_NEWER=0
BACKUP_ARG=""
FILES=()
while (( $# > 0 )); do
  arg="$1"; shift
  case "$arg" in
    --yes) ASSUME_YES=1 ;;
    --allow-newer) ALLOW_NEWER=1 ;;
    -f|--file)
      (( $# > 0 )) || die "$arg braucht eine Datei"
      FILES+=("$1"); shift
      ;;
    -h|--help) sed -n '2,38p' "$0"; exit 0 ;;
    -*) die "unbekannte Option: $arg" ;;
    *)
      [[ -z "$BACKUP_ARG" ]] || die "nur ein Backup-Verzeichnis angeben"
      BACKUP_ARG="$arg"
      ;;
  esac
done
if (( ${#FILES[@]} > 0 )); then
  COMPOSE_FILE="$(IFS=:; echo "${FILES[*]}")"
  export COMPOSE_FILE
fi
[[ -n "$BACKUP_ARG" ]] || die "Aufruf: scripts/restore.sh [--yes] <backup-verzeichnis>"
[[ -d "$BACKUP_ARG" ]] || die "kein Verzeichnis: $BACKUP_ARG"
BACKUP_DIR="$(cd "$BACKUP_ARG" && pwd)"

MANIFEST="$BACKUP_DIR/manifest.txt"
[[ -f "$MANIFEST" ]] || die "manifest.txt fehlt in $BACKUP_DIR"
# DUMP/BLOBS werden unten aus dem Manifest aufgelöst (database_file/blobs_file), sobald mget
# verfügbar ist – ein Pfad für Klartext- wie für verschlüsselte (.age) Backups.

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

# Dateinamen aus dem Manifest (Klartext: database.dump/blobs.tar.gz; verschlüsselt: *.age).
DB_FILE="$(mget database_file)"
BLOB_FILE="$(mget blobs_file)"
DUMP="$BACKUP_DIR/$DB_FILE"
BLOBS="$BACKUP_DIR/$BLOB_FILE"
[[ -f "$DUMP" ]] || die "$DB_FILE fehlt in $BACKUP_DIR"
[[ -f "$BLOBS" ]] || die "$BLOB_FILE fehlt in $BACKUP_DIR"

# Verschlüsselt? encrypted=1 im Manifest oder .age-Endung. Dann age + Identity nötig.
ENCRYPTED=0
if [[ "$(grep -E '^encrypted=' "$MANIFEST" | tail -n1 | cut -d= -f2-)" == "1" || "$DB_FILE" == *.age ]]; then
  ENCRYPTED=1
fi
AGE_ID="${LINEAL_BACKUP_AGE_IDENTITY:-}"
if (( ENCRYPTED )); then
  command -v age >/dev/null || die "age nicht gefunden (z. B. 'apt install age'), für das verschlüsselte Backup nötig"
  [[ -n "$AGE_ID" ]] || die "LINEAL_BACKUP_AGE_IDENTITY (Pfad zur age-Identity) ist nicht gesetzt – zum Entschlüsseln nötig"
  [[ -f "$AGE_ID" ]] || die "LINEAL_BACKUP_AGE_IDENTITY zeigt auf keine Datei: $AGE_ID"
fi

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
if grep -qE '^env_file=' "$MANIFEST"; then
  env_file="$(grep -E '^env_file=' "$MANIFEST" | tail -n1 | cut -d= -f2-)"
  echo "    Hinweis: Backup enthält $env_file (verschlüsselte .env). Dieser Restore fasst die .env NICHT an."
  echo "             Bei Bedarf separat: age -d -i <identity> \"$BACKUP_DIR/$env_file\" > .env"
fi

# 2. Migrationsstand gegen den Checkout (Quelle der Images)
IMAGE_VERSION=0
for f in "$ROOT"/crates/lineal-db/migrations/*.sql; do
  v="$(basename "$f" | cut -d_ -f1)"
  [[ "$v" =~ ^[0-9]+$ ]] || continue
  if (( 10#$v > IMAGE_VERSION )); then IMAGE_VERSION=$((10#$v)); fi
done
if (( BACKUP_VERSION > IMAGE_VERSION )); then
  warn "Das Backup hat den Migrationsstand $BACKUP_VERSION, die Migrationen dieses Checkouts reichen nur bis $IMAGE_VERSION."
  warn "Das Backup stammt aus einer neueren Version des Codes. Images bzw. Checkout zuerst auf diesen Stand bringen;"
  warn "mit älterem Code kann die Anwendung nach dem Restore fehlerhaft laufen."
  (( ALLOW_NEWER == 1 )) || die "Abbruch (keine Änderung vorgenommen). Mit --allow-newer trotzdem einspielen."
fi

# 3. Bestätigung
echo
echo "Ziel: Projekt '$COMPOSE_PROJECT_NAME' – Datenbank 'lineal' und Volume '$BLOB_VOLUME' werden ÜBERSCHRIEBEN."
echo "Compose-Dateien: ${COMPOSE_FILE:-<Standard: docker-compose.yml>}"
echo "Backup-Volume laut Manifest: $BLOB_VOLUME_BACKUP"
if (( ASSUME_YES == 0 )); then
  printf "Zum Fortfahren 'restore' eingeben: " >&2
  read -r answer || answer=""
  [[ "$answer" == "restore" ]] || die "abgebrochen (keine Änderung vorgenommen)"
fi

docker volume inspect "$BLOB_VOLUME" >/dev/null 2>&1 \
  || die "Volume '$BLOB_VOLUME' nicht gefunden (Projekt '$COMPOSE_PROJECT_NAME')"

# Container nie neu anlegen (kein Recreate von app-db durch abweichende Compose-Konfiguration).
info "app-db starten"
dc up -d --no-recreate --wait app-db \
  || die "app-db ließ sich nicht starten (keine Änderung vorgenommen). Stimmen -f/COMPOSE_FILE und COMPOSE_PROJECT_NAME mit dem laufenden Stack überein?"

# Vorabprüfung VOR dem Überschreiben: Verbindet sich die Anwendung mit ihren Zugangsdaten
# (DATABASE_URL, Passwort) mit der Datenbank? Sonst scheiterte migrate erst nach dem Restore.
info "Vorabprüfung: Datenbankverbindung der Anwendung (migrate)"
dc run --rm --no-deps migrate >/dev/null \
  || die "migrate kann sich vor dem Restore nicht mit der Datenbank verbinden (Passwort/Konfiguration?). Nichts wurde verändert."

# Ist-Zustand sichern, damit ein fehlgeschlagener oder falscher Restore umkehrbar ist.
PRE_DUMP="$(dirname "$BACKUP_DIR")/pre-restore-$(date -u +%Y%m%dT%H%M%SZ).dump"
info "Sichere den Ist-Zustand nach $PRE_DUMP"
( umask 077; dc exec -T app-db pg_dump -U lineal -Fc lineal > "$PRE_DUMP" ) \
  || { rm -f "$PRE_DUMP"; die "Sicherung des Ist-Zustands fehlgeschlagen (keine Änderung vorgenommen)."; }

info "server, worker, web stoppen"
dc stop server worker web >/dev/null 2>&1 || true

info "Datenbank wiederherstellen (pg_restore --clean --if-exists --no-owner --single-transaction --exit-on-error)"
# Verschlüsselt: age entschlüsselt im Stream in pg_restore. `set -o pipefail` lässt einen falschen
# Schlüssel (age-Fehler) den Lauf abbrechen; --single-transaction hält die DB dabei unverändert.
if (( ENCRYPTED )); then
  age -d -i "$AGE_ID" < "$DUMP" \
    | dc exec -T app-db pg_restore -U lineal -d lineal --clean --if-exists --no-owner \
        --single-transaction --exit-on-error \
    || die "pg_restore ist fehlgeschlagen (atomar: die Datenbank blieb unverändert). Falscher Schlüssel? server/worker/web bleiben gestoppt. Ist-Zustand: $PRE_DUMP"
else
  dc exec -T app-db pg_restore -U lineal -d lineal --clean --if-exists --no-owner \
      --single-transaction --exit-on-error < "$DUMP" \
    || die "pg_restore ist fehlgeschlagen (atomar: die Datenbank blieb unverändert). server/worker/web bleiben gestoppt; Ursache beheben und erneut ausführen. Ist-Zustand: $PRE_DUMP"
fi

info "Blob-Volume $BLOB_VOLUME ersetzen"
# Wichtig: der DB-Restore oben lief bereits erfolgreich und beweist bei verschlüsselten Backups den
# Schlüssel. Das `rm` leert das Volume VOR dem ersten tar-Byte – deshalb erst hier, nach bewiesenem
# Schlüssel und geprüfter Chiffrat-Prüfsumme, damit ein falscher Schlüssel nicht das Volume leert.
if (( ENCRYPTED )); then
  age -d -i "$AGE_ID" < "$BLOBS" \
    | docker run -i --rm -v "$BLOB_VOLUME:/data" \
        alpine sh -c 'find /data -mindepth 1 -maxdepth 1 -exec rm -rf {} + && tar xzf - --numeric-owner -C /data' \
    || die "Blob-Volume konnte nicht wiederhergestellt werden. server/worker/web bleiben gestoppt."
else
  docker run --rm \
    -v "$BLOB_VOLUME:/data" \
    -v "$BACKUP_DIR:/backup:ro" \
    alpine sh -c 'find /data -mindepth 1 -maxdepth 1 -exec rm -rf {} + && tar xzf /backup/blobs.tar.gz --numeric-owner -C /data' \
    || die "Blob-Volume konnte nicht wiederhergestellt werden. server/worker/web bleiben gestoppt."
fi

info "Migrationen ausführen"
dc run --rm migrate \
  || die "migrate ist fehlgeschlagen. server/worker/web bleiben gestoppt (siehe Warnung oben, falls der Stand neuer ist)."

info "server, worker, web starten"
# worker hat keinen Healthcheck (--wait würde scheitern); server und web werden geprüft.
dc up -d --no-recreate worker
dc up -d --no-recreate --wait server web

echo
echo "Restore abgeschlossen. Backup: $BACKUP_DIR"
