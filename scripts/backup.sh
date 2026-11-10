#!/usr/bin/env bash
# Backup des Docker-App-Stacks (docker compose --profile app, docs/15 §3.2b).
#
# Erzeugt ${LINEAL_BACKUP_DIR:-./backups}/lineal-<UTC-Zeitstempel>/ (Rechte 700) mit
#   database.dump   pg_dump -Fc der App-Datenbank (Dienst app-db, Ausgabe auf stdout)
#   blobs.tar.gz    Inhalt des Volumes "blobs" (Dateien unter /data/blobs), über einen
#                   kurzlebigen alpine-Container gelesen
#   manifest.txt    Zeitpunkt, Git-Commit, Migrationsstand, Größen und sha256 beider Dateien
#
# Das Backup entsteht zuerst im Verzeichnis lineal-<ts>.partial und wird erst nach Erfolg
# umbenannt; bei einem Fehler wird das .partial-Verzeichnis entfernt.
#
# Umgebung (in der Shell, in der Crontab oder in .env des Projekts; .env wird nur für diese
# drei Variablen gelesen, nicht gesourct):
#   LINEAL_BACKUP_DIR   Zielverzeichnis, relativ zum Projektverzeichnis (Default ./backups)
#   LINEAL_BACKUP_KEEP  Anzahl aufbewahrter Backups, Default 14; ältere lineal-*-Verzeichnisse
#                       werden nach erfolgreichem Lauf gelöscht
#   COMPOSE_PROJECT_NAME  Compose-Projekt (Default lineal). Das Volume heißt
#                       "<Projekt>_blobs". Für einen zweiten Stack (Test) anders setzen.
#   COMPOSE_FILE        Compose-Dateien (":"-getrennt), z. B. docker-compose.yml:deploy/docker-compose.test.yml
#                       für den Zweitstack; sonst legt Compose Dienste mit falscher Konfiguration an.
#
# Verschlüsselung (S-05/LIN-33) – Backup wird mit `age` verschlüsselt, der private Schlüssel liegt
# NICHT auf dem Host:
#   LINEAL_BACKUP_AGE_RECIPIENTS_FILE  Datei mit age-Empfängern (eine je Zeile: `age1...` oder ein
#                       SSH-Public-Key). Gesetzt ⇒ database.dump.age, blobs.tar.gz.age und env.age
#                       (die .env wird mitgesichert). Ungesetzt ⇒ ABBRUCH (verschlüsseltes Backup
#                       ist Pflicht), außer LINEAL_BACKUP_ALLOW_PLAINTEXT=1 erlaubt bewusst Klartext.
#   LINEAL_BACKUP_ALLOW_PLAINTEXT  "1" erlaubt ein unverschlüsseltes Backup (altes Verhalten).
#   LINEAL_BACKUP_OFFSITE_CMD  Optionaler Befehl, der nach Erfolg mit dem Backup-Verzeichnis als
#                       $1 (und in $LINEAL_BACKUP_PATH) läuft, z. B. ein rclone-/restic-Push. Scheitert
#                       er, bleibt das lokale Backup erhalten und der Lauf endet mit Fehler.
#
# Keine Passwörter auf der Kommandozeile oder in Dateien: pg_dump läuft im Container über den
# lokalen Socket.
#
# Nutzung (Cron, täglich 03:00):  0 3 * * * cd /pfad && scripts/backup.sh
# Exit-Code ungleich 0 bei jedem Fehler.
set -euo pipefail
export LC_ALL=C
shopt -s nullglob

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() { echo "FEHLER: $*" >&2; exit 1; }
info() { echo "==> $*"; }

# Nur die Backup-Variablen aus .env übernehmen, falls sie nicht in der Umgebung gesetzt sind.
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
load_from_dotenv LINEAL_BACKUP_DIR
load_from_dotenv LINEAL_BACKUP_KEEP
load_from_dotenv COMPOSE_PROJECT_NAME
load_from_dotenv COMPOSE_FILE
load_from_dotenv LINEAL_BACKUP_AGE_RECIPIENTS_FILE
load_from_dotenv LINEAL_BACKUP_ALLOW_PLAINTEXT
load_from_dotenv LINEAL_BACKUP_OFFSITE_CMD

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-lineal}"
KEEP="${LINEAL_BACKUP_KEEP:-14}"
[[ "$KEEP" =~ ^[1-9][0-9]*$ ]] || die "LINEAL_BACKUP_KEEP muss eine Zahl >= 1 sein (ist: '$KEEP')"
BLOB_VOLUME="${COMPOSE_PROJECT_NAME}_blobs"

command -v docker >/dev/null || die "docker nicht gefunden"
docker compose version >/dev/null 2>&1 || die "'docker compose' (Compose v2) nicht verfügbar"
command -v sha256sum >/dev/null || command -v shasum >/dev/null || die "sha256sum oder shasum fehlt"

# Verschlüsselung (S-05/LIN-33): fail-closed. Ohne Empfänger-Datei nur mit ausdrücklichem Opt-out.
ENCRYPT=1
RECIP_FILE="${LINEAL_BACKUP_AGE_RECIPIENTS_FILE:-}"
if [[ -n "$RECIP_FILE" ]]; then
  [[ -f "$RECIP_FILE" ]] || die "LINEAL_BACKUP_AGE_RECIPIENTS_FILE zeigt auf keine Datei: $RECIP_FILE"
  [[ -s "$RECIP_FILE" ]] || die "Empfänger-Datei ist leer: $RECIP_FILE"
  command -v age >/dev/null || die "age nicht gefunden (z. B. 'apt install age'), wird für die Verschlüsselung gebraucht"
elif [[ "${LINEAL_BACKUP_ALLOW_PLAINTEXT:-}" == "1" ]]; then
  ENCRYPT=0
  echo "WARNUNG: Unverschlüsseltes Backup (LINEAL_BACKUP_ALLOW_PLAINTEXT=1). S-05/LIN-33." >&2
else
  die "Kein LINEAL_BACKUP_AGE_RECIPIENTS_FILE gesetzt. Verschlüsseltes Backup ist Pflicht (S-05).
     Empfänger-Datei anlegen und setzen, oder bewusst Klartext mit LINEAL_BACKUP_ALLOW_PLAINTEXT=1."
fi

dc() { docker compose --profile app "$@"; }

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

[[ -n "$(dc ps --status running -q app-db)" ]] \
  || die "Dienst app-db läuft nicht (Projekt '$COMPOSE_PROJECT_NAME'). Stack mit 'docker compose --profile app up -d' starten."
docker volume inspect "$BLOB_VOLUME" >/dev/null 2>&1 \
  || die "Volume '$BLOB_VOLUME' nicht gefunden (Projekt '$COMPOSE_PROJECT_NAME')"

BACKUP_ROOT_RAW="${LINEAL_BACKUP_DIR:-./backups}"
mkdir -p "$BACKUP_ROOT_RAW" || die "Zielverzeichnis '$BACKUP_ROOT_RAW' nicht anlegbar"
BACKUP_ROOT="$(cd "$BACKUP_ROOT_RAW" && pwd)"

TS="$(date -u +%Y%m%dT%H%M%SZ)"
NAME="lineal-$TS"
FINAL="$BACKUP_ROOT/$NAME"
PARTIAL="$BACKUP_ROOT/$NAME.partial"
[[ ! -e "$FINAL" && ! -e "$PARTIAL" ]] || die "Backup '$NAME' existiert bereits"

cleanup() {
  local rc=$?
  if [[ $rc -ne 0 && -d "$PARTIAL" ]]; then
    rm -rf -- "$PARTIAL"
    echo "Backup abgebrochen, unvollständiges Verzeichnis entfernt." >&2
  fi
  exit "$rc"
}
trap cleanup EXIT

umask 077
mkdir -m 700 "$PARTIAL"
info "Backup $NAME nach $BACKUP_ROOT"

# Bei Verschlüsselung wird gestreamt (pg_dump|age, tar|age), damit NIE Klartext auf der Platte
# landet. `set -o pipefail` sorgt dafür, dass ein Fehler von pg_dump/tar/age den Lauf abbricht.
ENVFILE=""
if (( ENCRYPT )); then
  DBFILE="database.dump.age"
  BLOBFILE="blobs.tar.gz.age"
  info "Datenbank (pg_dump -Fc | age)"
  dc exec -T app-db pg_dump -U lineal -d lineal -Fc | age -R "$RECIP_FILE" > "$PARTIAL/$DBFILE"
  info "Blob-Volume $BLOB_VOLUME (tar | age)"
  docker run --rm -v "$BLOB_VOLUME:/data:ro" \
    alpine tar czf - --numeric-owner -C /data . | age -R "$RECIP_FILE" > "$PARTIAL/$BLOBFILE"
  if [[ -f "$ROOT/.env" ]]; then
    info ".env (age) mitsichern"
    age -R "$RECIP_FILE" < "$ROOT/.env" > "$PARTIAL/env.age"
    ENVFILE="env.age"
  fi
else
  DBFILE="database.dump"
  BLOBFILE="blobs.tar.gz"
  info "Datenbank (pg_dump -Fc)"
  dc exec -T app-db pg_dump -U lineal -d lineal -Fc > "$PARTIAL/$DBFILE"
  info "Blob-Volume $BLOB_VOLUME (tar)"
  docker run --rm \
    -v "$BLOB_VOLUME:/data:ro" \
    -v "$PARTIAL:/backup" \
    alpine sh -c 'tar czf /backup/blobs.tar.gz --numeric-owner -C /data . && chown "$0" /backup/blobs.tar.gz' \
    "$(id -u):$(id -g)"
fi

info "Migrationsstand"
MIGRATION_VERSION="$(dc exec -T app-db psql -U lineal -d lineal -At -c \
  "SELECT CASE WHEN to_regclass('_sqlx_migrations') IS NULL THEN '0' ELSE (SELECT COALESCE(MAX(version), 0)::text FROM _sqlx_migrations) END")"
[[ "$MIGRATION_VERSION" =~ ^[0-9]+$ ]] || die "Migrationsstand nicht lesbar: '$MIGRATION_VERSION'"

GIT_COMMIT="unbekannt"
if git -C "$ROOT" rev-parse --verify -q HEAD >/dev/null 2>&1; then
  GIT_COMMIT="$(git -C "$ROOT" rev-parse --short HEAD)"
  if [[ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ]]; then
    GIT_COMMIT="$GIT_COMMIT (mit ungeprüften Änderungen)"
  fi
fi

{
  echo "# Lineal-Backup, erzeugt von scripts/backup.sh"
  echo "created_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git_commit=$GIT_COMMIT"
  echo "compose_project=$COMPOSE_PROJECT_NAME"
  echo "migration_version=$MIGRATION_VERSION"
  echo "encrypted=$ENCRYPT"
  # database_sha256/blobs_sha256 sind die Prüfsummen der GESPEICHERTEN Datei (bei Verschlüsselung
  # also des Chiffrats) – restore.sh prüft sie vor dem Entschlüsseln.
  echo "database_file=$DBFILE"
  echo "database_bytes=$(wc -c < "$PARTIAL/$DBFILE" | tr -d ' ')"
  echo "database_sha256=$(sha256_of "$PARTIAL/$DBFILE")"
  echo "blobs_volume=$BLOB_VOLUME"
  echo "blobs_file=$BLOBFILE"
  echo "blobs_bytes=$(wc -c < "$PARTIAL/$BLOBFILE" | tr -d ' ')"
  echo "blobs_sha256=$(sha256_of "$PARTIAL/$BLOBFILE")"
  if [[ -n "$ENVFILE" ]]; then
    echo "env_file=$ENVFILE"
    echo "env_sha256=$(sha256_of "$PARTIAL/$ENVFILE")"
  fi
} > "$PARTIAL/manifest.txt"
chmod 600 "$PARTIAL"/*

mv "$PARTIAL" "$FINAL"
trap - EXIT
info "Fertig: $FINAL"
cat "$FINAL/manifest.txt"

# Off-Site (S-05/LIN-33): nach dem erfolgreichen Backup, VOR dem Prune. Scheitert der Push, bleibt
# das lokale Backup erhalten und der Lauf endet mit Fehler (kein stiller Verlust der Off-Site-Kopie).
if [[ -n "${LINEAL_BACKUP_OFFSITE_CMD:-}" ]]; then
  info "Off-Site-Ablage"
  if ! LINEAL_BACKUP_PATH="$FINAL" bash -c "$LINEAL_BACKUP_OFFSITE_CMD" offsite "$FINAL"; then
    die "Off-Site-Ablage fehlgeschlagen. Lokales Backup behalten: $FINAL"
  fi
fi

# Aufbewahrung: nur lineal-*-Verzeichnisse (ohne .partial), die ältesten zuerst entfernen.
ALL=()
for d in "$BACKUP_ROOT"/lineal-*; do
  if [[ -d "$d" && ! -L "$d" && "$d" != *.partial ]]; then
    ALL+=("$d")
  fi
done
COUNT=${#ALL[@]}
if (( COUNT > KEEP )); then
  for ((i = 0; i < COUNT - KEEP; i++)); do
    info "Aufbewahrung: entferne $(basename "${ALL[i]}")"
    rm -rf -- "${ALL[i]}"
  done
fi
