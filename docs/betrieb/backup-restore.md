# Backup & Restore

## Zweck

`scripts/backup.sh` sichert den Docker-App-Stack (`docker compose --profile app`): einen
konsistenten PostgreSQL-Dump der App-Datenbank und den Inhalt des Blob-Volumes (hochgeladene
Dateien). `scripts/restore.sh` spielt ein solches Backup wieder ein. Backups werden standardmäßig
mit [`age`](https://github.com/FiloSottile/age) verschlüsselt; der private Schlüssel liegt **nicht**
auf dem Host.

## Voraussetzungen

- Laufender App-Stack (`docker compose --profile app up -d`) im Projektverzeichnis `/opt/<app>`.
- `docker compose` (Compose v2), `pg_dump`/`pg_restore` laufen im Container (kein Host-Postgres nötig).
- Für verschlüsselte Backups: `age` auf dem Host und eine Empfängerdatei (Public Keys).
- Beide Skripte laufen auf dem **Host**, aus dem Projektverzeichnis.

## Variablen (mit Defaults)

### Backup (`scripts/backup.sh`)

| Variable | Default | Zweck |
|---|---|---|
| `LINEAL_BACKUP_DIR` | `./backups` | Zielverzeichnis (relativ zum Projektverzeichnis) |
| `LINEAL_BACKUP_KEEP` | `14` | Anzahl aufbewahrter Backups; ältere werden nach Erfolg gelöscht |
| `LINEAL_BACKUP_AGE_RECIPIENTS_FILE` | — | Datei mit age-Empfängern (eine je Zeile). Gesetzt ⇒ verschlüsseltes Backup (`*.age`, inkl. `env.age`). **Ungesetzt ⇒ Abbruch**, außer … |
| `LINEAL_BACKUP_ALLOW_PLAINTEXT` | — | `1` erlaubt bewusst ein unverschlüsseltes Backup |
| `LINEAL_BACKUP_OFFSITE_CMD` | — | Optionaler Befehl nach Erfolg (Backup-Pfad als `$1` / `$LINEAL_BACKUP_PATH`), z. B. ein `rclone`-/`restic`-Push |
| `COMPOSE_PROJECT_NAME` | `lineal` | Compose-Projekt; das Volume heißt `<Projekt>_blobs` |
| `COMPOSE_FILE` | — | Compose-Dateien (`:`-getrennt), z. B. für einen Zweitstack |

Nur diese Variablen werden ggf. aus der `.env` übernommen (die `.env` wird nicht gesourct).

### Restore (`scripts/restore.sh`)

| Variable / Flag | Default | Zweck |
|---|---|---|
| `--yes` | Rückfrage | Ohne Rückfrage (Automatisierung); sonst muss `restore` eingegeben werden |
| `-f, --file <datei>` | — | Compose-Datei (wiederholbar), entspricht `COMPOSE_FILE` |
| `--allow-newer` | Abbruch | Backup mit neuerem Migrationsstand als dieser Checkout trotzdem einspielen |
| `LINEAL_BACKUP_AGE_IDENTITY` | — | Pfad zur age-Identity (privater Schlüssel) für verschlüsselte Backups. **Nur aus der Umgebung des Operators**, nie aus `.env` |
| `COMPOSE_PROJECT_NAME`, `COMPOSE_FILE` | wie oben | Müssen zum gesicherten Stack passen |

## Beispielaufruf

```bash
cd /opt/<app>

# Einmaliges, verschlüsseltes Backup
LINEAL_BACKUP_AGE_RECIPIENTS_FILE=/opt/<app>/backup-recipients.txt scripts/backup.sh

# Als täglicher Cron (03:00). Variablen in der Crontab oder in der .env setzen.
# 0 3 * * * cd /opt/<app> && LINEAL_BACKUP_AGE_RECIPIENTS_FILE=... scripts/backup.sh >> /var/log/lineal-backup.log 2>&1

# Restore eines Backups (Automatisierung)
LINEAL_BACKUP_AGE_IDENTITY=/root/age-identity.txt scripts/restore.sh --yes backups/lineal-<ts>
```

Ein Backup landet in `${LINEAL_BACKUP_DIR}/lineal-<UTC-Zeitstempel>/` (Rechte `700`) und enthält
`database.dump`, `blobs.tar.gz` (bzw. deren `.age`-Varianten + `env.age`) und `manifest.txt`
(Zeitpunkt, Git-Commit, Migrationsstand, Größen, sha256). Es entsteht zuerst als `…​.partial` und
wird erst nach Erfolg umbenannt.

## Verifikation

```bash
# Backup: Verzeichnis, Dateien, Manifest
ls -l backups/lineal-<ts>/
cat backups/lineal-<ts>/manifest.txt        # sha256/Größen stimmen (prüft restore.sh erneut)

# Restore-Übung (empfohlen, regelmäßig): in einen separaten Zweitstack einspielen, nicht in Prod.
# Eigener Projektname + Overlay, damit Compose keine Prod-Container anfasst:
COMPOSE_PROJECT_NAME=lineal-test LINEAL_BACKUP_AGE_IDENTITY=/root/age-identity.txt \
  scripts/restore.sh --yes -f docker-compose.yml -f deploy/docker-compose.test.yml backups/lineal-<ts>
# Danach: App des Zweitstacks aufrufen, Stichproben (Issues, Anhänge) prüfen.
```

`restore.sh` prüft vor jeder Änderung die sha256-Summen und den Migrationsstand und bricht bei
Abweichung ab.

## Rollback

- **Backup** verändert den laufenden Stack nicht — ein Fehlschlag entfernt nur das `…​.partial`.
- **Restore** sichert vor dem Einspielen den Ist-Zustand als
  `<Backup-Verzeichnis>/../pre-restore-<Zeitstempel>.dump`. Wird der Restore **vor** Schritt 3
  abgebrochen (Manifest-/Verbindungsprüfung), bleibt der Stack unverändert. Ist nach einem Restore
  etwas falsch, spielt man den `pre-restore-*.dump` über denselben Ablauf (bzw. `pg_restore`)
  zurück. Die DB-Wiederherstellung selbst läuft in einer einzigen Transaktion
  (`--single-transaction --exit-on-error`): bei einem Fehler bleibt der alte Datenstand erhalten.
