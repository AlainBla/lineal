# Platten-Alarm

## Zweck

`scripts/disk-alarm.sh` überwacht die Belegung der Dateisysteme, auf denen die App-Blobs und
Backups liegen, und schlägt Alarm, sobald ein Schwellwert überschritten ist. Gedacht als
**Host**-Cron (nicht im Container), da nur der Host die reale Plattenbelegung kennt. Ergänzt die
Byte-Quota je Workspace.

## Voraussetzungen

- Läuft auf dem Host (Standard-`df`, `awk`).
- Für Mail-Alarm optional das Kommando `mail` (sonst nur stderr + Exit-Code).

## Variablen (mit Defaults)

| Variable | Default | Zweck |
|---|---|---|
| `LINEAL_DISK_PATHS` | `/opt/lineal /var/lib/docker` | Leerzeichenliste der geprüften Pfade |
| `LINEAL_DISK_WARN` | `85` | Warnschwelle in Prozent |
| `LINEAL_DISK_CRIT` | `92` | Kritisch-Schwelle in Prozent |
| `LINEAL_DISK_MAILTO` | — | Optionale Mailadresse für den Alarm |

Exit-Code: `0` = ok, `1` = Warnung, `2` = kritisch (so greift auch systemd-/cron-Mailrouting).

## Beispielaufruf

```bash
# Einmalige Prüfung
LINEAL_DISK_PATHS="/opt/<app> /var/lib/docker" scripts/disk-alarm.sh

# Stündlicher Cron mit Mail-Alarm
# 0 * * * * LINEAL_DISK_MAILTO=<admin@example.org> /opt/<app>/scripts/disk-alarm.sh
```

## Verifikation

```bash
scripts/disk-alarm.sh; echo "Exit: $?"     # 0/1/2 je nach Belegung

# Alarmpfad testen, ohne zu warten: Schwelle künstlich niedrig setzen
LINEAL_DISK_WARN=1 LINEAL_DISK_CRIT=1 scripts/disk-alarm.sh; echo "Exit: $?"   # -> 2
```

Die Ausgabe listet je Pfad `belegt %` und `frei` sowie die Spitzenbelegung und den Hostnamen.

## Rollback

Das Skript ist rein lesend (kein Eingriff ins System). „Rollback“ = den Cron-Eintrag entfernen bzw.
die Schwellwerte über die Variablen anpassen.
