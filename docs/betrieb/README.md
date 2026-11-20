# Betrieb (öffentliche Runbooks)

Generalisierte Betriebsanleitungen für eine selbst gehostete Lineal-Installation. Sie beschreiben
die mitgelieferten Skripte unter `scripts/` und die Konfigurationsvorlagen unter `deploy/`.

**Platzhalter statt echter Werte.** Diese Seiten nennen bewusst keine konkrete Infrastruktur.
Überall, wo ein echter Wert stünde, steht ein Platzhalter, den jede Installation durch ihre eigenen
Werte ersetzt:

| Platzhalter | Bedeutung |
|---|---|
| `example.org` | die eigene (Apex-)Domain |
| `lineal.example.org` | App-Origin (SPA, API) |
| `lineal-files.example.org` | cookielose Datei-Origin (`${LINEAL_FILES_HOST}`) |
| `/opt/<app>` | Projektverzeichnis auf dem Host (z. B. `/opt/lineal`) |
| `<admin@example.org>` | Kontakt-/Alarm-Postfach |

Konkrete Werte der jeweiligen Installation gehören **nicht** in dieses öffentliche Repository,
sondern in ein privates Infra-Repo bzw. in die `.env` auf dem Host (die nie eingecheckt wird,
siehe `.gitignore`/`.dockerignore`).

## Seiten

| Thema | Skript/Vorlage | Seite |
|---|---|---|
| Backup & Restore | `scripts/backup.sh`, `scripts/restore.sh` | [backup-restore.md](backup-restore.md) |
| Signierter Deploy | `scripts/deploy.sh`, `.allowed_signers` | [deploy.md](deploy.md) |
| nginx (Datei-Origin, Security-Header) | `deploy/nginx.conf`, `deploy/security-headers.conf`, `scripts/nginx-check.sh` | [nginx.md](nginx.md) |
| Platten-Alarm | `scripts/disk-alarm.sh` | [disk-alarm.md](disk-alarm.md) |
| SSH-Härtung | `deploy/sshd-hardening.conf` | [ssh-hardening.md](ssh-hardening.md) |
| TLS / CAA | — (generalisiert aus `docs/15` §3) | [tls-caa.md](tls-caa.md) |

## Kontext

Der Produktionsbetrieb läuft über den Docker-App-Stack (`docker compose --profile app up -d`, siehe
`docker-compose.yml` und `docs/15-umsetzungsstand.md` §3). Vor `web` terminiert ein Reverse-Proxy
auf dem Host (z. B. Caddy) TLS; die Skripte in `scripts/` laufen auf dem **Host**, nicht im
Container. Jede Seite nennt zuerst ihren Zweck, dann Voraussetzungen, Variablen (mit Defaults),
einen Beispielaufruf, die Verifikation und den Rollback.
