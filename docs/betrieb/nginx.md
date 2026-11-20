# nginx: Datei-Origin & Security-Header

## Zweck

`deploy/nginx.conf` ist die nginx-Konfiguration des `web`-Containers: Auslieferung der SPA,
Reverse-Proxy auf den `server`, Rate-Limits, Body-Limits, Dotfile-Sperren und – als zweiter
`server`-Block – die **cookielose Datei-Origin** für die Inline-Ansicht von HTML-Anhängen
(`${LINEAL_FILES_HOST}`, nur `/files/*`, sonst 404; siehe `docs/15` §3.4b). `deploy/security-headers.conf`
liefert die Sicherheits-Header für Antworten, die nginx **selbst** erzeugt (SPA-HTML, `/assets`,
Favicon, Dotfile-404). `scripts/nginx-check.sh` validiert beides ohne App-Stack und ist als
CI-Job „nginx-Konfiguration“ eingebunden.

## Datei-Host als Vorlage (`${LINEAL_FILES_HOST}`)

In der öffentlichen Quelle steht **kein** konkreter Datei-Host. Der `server_name` des Datei-Blocks
ist ein Platzhalter `${LINEAL_FILES_HOST}`, der zur **Build-Zeit** gerendert wird:

- Der Docker-Build (`Dockerfile`, Ziel `web`) ersetzt den Platzhalter per `envsubst` (nur diese eine
  Variable – nginx-Variablen wie `$host`, `$remote_addr`, `$lineal_upstream` bleiben unangetastet).
- Der Wert kommt aus dem Build-Arg `LINEAL_FILES_HOST`; ist er leer, aus dem **Host-Teil** von
  `LINEAL_FILES_BASE_URL` (ohne Schema/Port/Pfad); ist auch das leer, aus dem Platzhalter
  `lineal-files.example.org`.
- `docker-compose.yml` reicht beide Werte als `build.args` aus der Umgebung/`​.env` durch. Wer in der
  `.env` `LINEAL_FILES_BASE_URL` setzt (wie für die Datei-Origin ohnehin nötig), muss **nichts
  zusätzlich** tun — der Datei-Host wird daraus abgeleitet und kann nicht abweichen.

Gerendert wird bewusst zur Build-Zeit, **nicht** über das Laufzeit-`templates/`-Verfahren von nginx,
weil der `web`-Container `read_only` läuft und `conf.d` zur Laufzeit nicht beschreibbar ist.

## Voraussetzungen

- Docker (für `scripts/nginx-check.sh` und den Build).
- Für die Datei-Origin: DNS-Eintrag + Reverse-Proxy-Site für `lineal-files.example.org` und
  `LINEAL_FILES_BASE_URL` gesetzt (siehe [tls-caa.md](tls-caa.md) und `docs/15` §3.4b).

## Variablen (mit Defaults)

| Variable | Default | Wirkung |
|---|---|---|
| `LINEAL_FILES_HOST` (Build-Arg / Env) | — | Datei-Host; überschreibt die Ableitung aus der Base-URL |
| `LINEAL_FILES_BASE_URL` (Env) | — | Datei-Origin der App; liefert ersatzweise den Datei-Host |
| gerenderter Platzhalter | `lineal-files.example.org` | Fallback, wenn beide leer (Feature ist dann ohnehin aus) |
| `scripts/nginx-check.sh` Argument 1 | `nginxinc/nginx-unprivileged:1.28-alpine` | Test-Image |

## Beispielaufruf

```bash
# CI-/lokaler Check (baut nichts, nutzt ein kurzlebiges Docker-Netz):
scripts/nginx-check.sh

# Gezielt mit eigenem Datei-Host prüfen:
LINEAL_FILES_HOST=lineal-files.example.org scripts/nginx-check.sh

# Produktions-Build (Datei-Host aus der Base-URL abgeleitet):
LINEAL_FILES_BASE_URL=https://lineal-files.example.org \
  docker compose --profile app build web
```

## Verifikation

```bash
# Rendert der Build den richtigen server_name? (im gebauten Image)
docker run --rm --entrypoint sh lineal-web:latest -c \
  'grep server_name /etc/nginx/conf.d/default.conf'

# Vollständiger Verhaltenstest (Syntax + Header + Routing der Datei-Origin):
scripts/nginx-check.sh      # endet mit "nginx-check: alles grün"
```

`nginx-check.sh` rendert die Vorlage mit einem Beispiel-Host, bevor `nginx -t` läuft, und testet
u. a., dass die Datei-Origin alles außer `/files/*` mit 404 beantwortet und jeder Security-Header
genau einmal gesetzt ist.

## Rollback

- `deploy/nginx.conf`/`security-headers.conf` sind in das `web`-Image eingebaut. Zurückrollen =
  das vorherige `lineal-web`-Image erneut deployen bzw. den vorherigen Commit bauen (siehe
  [deploy.md](deploy.md)). Der CI-Job „nginx-Konfiguration“ ist das Gate: Eine Änderung, die ihn
  rot macht, wird nicht gemerged.
- Ein falsch gerenderter Datei-Host wird korrigiert, indem `LINEAL_FILES_HOST` bzw.
  `LINEAL_FILES_BASE_URL` angepasst und `web` neu gebaut wird.
