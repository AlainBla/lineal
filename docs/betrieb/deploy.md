# Signierter Deploy

## Zweck

`scripts/deploy.sh` ist ein abgesicherter Produktions-Deploy. Statt `git pull` checkt es nur einen
**gültig signierten, geprüften** Commit aus und baut ihn. Der öffentliche Signierschlüssel liegt
**nur auf dem Host** (git-Config `gpg.ssh.allowedSignersFile`), nicht auf GitHub. Ein übernommenes
GitHub-Konto kann zwar auf `main` pushen, aber keinen Commit erzeugen, den dieser Deploy als gültig
signiert akzeptiert — er bricht dann ab, bevor etwas gebaut oder gestartet wird.

## Voraussetzungen

Einmalig auf dem Host (im Projektverzeichnis `/opt/<app>`):

```bash
git config gpg.format ssh
git config gpg.ssh.allowedSignersFile /opt/<app>/.allowed_signers
# .allowed_signers (eine Zeile je erlaubtem Signierer):
#   <principal> namespaces="git" <ssh-pubkey>
```

Die Datei `.allowed_signers` ist **host-lokal** und wird bewusst **nicht** eingecheckt — sie legt
fest, welche Schlüssel als vertrauenswürdig gelten. Die privaten Signierschlüssel liegen nur bei den
Entwickler:innen, die signierte Commits erzeugen.

## Variablen (mit Defaults)

| Variable / Argument | Default | Zweck |
|---|---|---|
| `<ref>` (Argument 1) | `origin/main` | Branch, Tag oder SHA, der deployt wird |
| `DEPLOY_SIGNER` | — | Erwarteter Principal; die Signatur muss von ihm stammen |
| `DEPLOY_ALLOW_NONFF` | — | `1` erlaubt einen Ziel-Commit, der **kein** Nachfolger des aktuell deployten ist (bewusster Rollback) |

Die Signaturprüfung läuft mit `gpg.minTrustLevel=fully`: ohne Eintrag in `.allowed_signers` ist der
Trust-Level nicht „fully“ → Abbruch.

## Beispielaufruf

```bash
cd /opt/<app>

# Standard: neuesten signierten main-Stand deployen
scripts/deploy.sh

# Bestimmte Ref, nur von einem erwarteten Signierer akzeptieren
DEPLOY_SIGNER="deploy@example.org" scripts/deploy.sh origin/main
```

Ablauf: `git fetch` → Signatur prüfen → (optional) Principal prüfen → Fast-Forward-Zwang →
`git checkout --detach <commit>` → `docker compose --profile app up -d --build`.

## Verifikation

```bash
# Welcher Commit läuft?
git rev-parse HEAD

# Dienste laufen/healthy?
docker compose --profile app ps

# Signatur eines Commits manuell prüfen (wie im Deploy)
git -c gpg.minTrustLevel=fully verify-commit HEAD
```

Ein nicht gültig signierter `<ref>` führt zu `ABBRUCH: … ist nicht gültig signiert. Nichts
deployt.` — der Stack bleibt dann unverändert.

## Rollback

`deploy.sh` erzwingt Vorwärts-Deploys (Rollback-Schutz). Um bewusst auf einen älteren, **gültig
signierten** Commit zurückzugehen:

```bash
DEPLOY_ALLOW_NONFF=1 scripts/deploy.sh <älterer-signierter-commit>
```

Datenbank-/Schema-Rückrollen gehört zu [Backup & Restore](backup-restore.md): Ein älterer Code-Stand
mit neuerem DB-Schema kann inkompatibel sein — im Zweifel das zum Code-Stand passende Backup
einspielen.
