# HTML-Anhänge im Tab öffnen (eigene Datei-Origin) — Design

**Status:** Entwurf, zur Freigabe. Nichts gebaut, bis freigegeben.
**Anlass:** KOL-478 (embeddete 8-Folien-HTML-Präsentation als Anhang). Heute löst ein `text/html`-Anhang nur einen Download aus.
**Audit-Bezug:** Fund N4 (sec-web) — „stärkste Mitigation = eigene Datei-Origin". Eigentümer wählte **Weg B**.

## Intent / Warum

Interne Nutzer und Bots laden HTML-Dateien (v. a. embeddete Präsentationen) hoch; **ein anderer interner Nutzer** schaut sie an. Sie sollen beim Öffnen **im neuen Tab laufen** (inkl. Folien-JS), ohne dass hochgeladenes HTML als gespeichertes XSS den **Session-Kontext des Betrachters** angreifen kann.

Trust-Modell: Uploader ≠ Betrachter, **beide intern** (kein externes Teilen in dieser Stufe). Die Härtung zielt auf „böswilliger/kompromittierter interner Uploader greift Kollegen an".

## Kernidee

HTML wird **nur auf einer eigenen, cookielosen Origin** (`lineal-files.kollegeplus.de`) **inline + script-fähig** ausgeliefert. Die App-Origin (`lineal.kollegeplus.de`) bleibt **unverändert**: HTML → `attachment` + `sandbox` (kein Script). Weil `GET /files/{token}` **nur per HMAC-Token** autorisiert (kein Session-Cookie, `blob.rs:525`) und der Host **nicht** in den Claims steht, verifiziert dasselbe Token auf beiden Origins — es ist kein Umbau der Autorisierung nötig.

Warum das sicher ist: Das Session-Cookie ist **host-only** (`middleware/auth.rs:127`, kein `Domain=`) → eine Subdomain sieht es nie. Zusätzlich `sandbox allow-scripts` (ohne `allow-same-origin`) → **opake Origin**: Script läuft, kann aber weder Cookie noch App-API noch Storage erreichen. Doppelte Absicherung (eigene Origin **und** Sandbox).

## Goals

- `text/html`-Anhang öffnet per „Im Tab öffnen" auf der Datei-Origin und rendert inkl. JS (Folien-Navigation) + externer Google-Fonts.
- „Herunterladen" bleibt verfügbar (App-Origin, `attachment`).
- MCP/Bots erhalten denselben Ansichts-Link (Backend liefert `viewUrl`).
- Feature ist per Config schaltbar; **ungesetzt = exakt heutiges Verhalten**.

## Non-Goals / Anti-Goals

- **Kein** externes/öffentliches Teilen von HTML (spätere Stufe; dann wäre eigene Registrable-Domain Pflicht).
- **Kein** Inline-Scripting auf der App-Origin — bricht die Isolation und ist ausdrücklich verboten.
- **Keine** eigene Registrable-Domain in dieser Stufe (Subdomain genügt, solange kein Cookie mit `Domain=.kollegeplus.de` existiert — heute keins; siehe Invariante unten).
- SVG/XML bleiben vorerst `attachment` auf beiden Origins (nur `text/html` wird inline geschaltet). Erweiterung später.

## Invariante (muss gelten, sonst bricht B)

> Es darf **nie** ein Cookie mit `Domain=.kollegeplus.de` gesetzt werden. Alle heutigen Cookies sind host-only — bleibt so. (Als Kommentar an `session_cookie` verankern.)

## Architektur

### Zwei Origins, ein Prozess

- **App-Origin** `lineal.kollegeplus.de`: SPA, API, `/files/` → `attachment`/`sandbox` wie heute.
- **Datei-Origin** `lineal-files.kollegeplus.de`: **nur** `/files/*`. Alles andere → 404 (nginx **und** App-Guard). Cookielos by construction.

### DNS + TLS (Vorbereiten+Anleitung — Nutzer führt aus)

- DNS: `lineal-files.kollegeplus.de` als CNAME (oder A) auf denselben Host wie `lineal`. CAA wird von `kollegeplus.de` geerbt (LIN-52) — kein neuer Record, aber Issuer-Liste muss Caddys ZeroSSL-Fallback decken (bereits in docs/15 §3.4 vermerkt).
- Caddy: zweiter Site-Block `lineal-files.kollegeplus.de` → Reverse-Proxy auf dasselbe nginx :8080, **Host erhalten**. Auto-LE-Cert.
- HSTS `includeSubDomains` ist bereits aktiv → die Subdomain MUSS gültiges HTTPS liefern (Caddy erledigt das).

### nginx (deploy/nginx.conf)

Neuer, **separater** `server`-Block **vor** dem Catch-all:

```
server {
    listen 8080;
    server_name lineal-files.kollegeplus.de;   # exakt; Catch-all _ bleibt für die App
    server_tokens off;
    access_log /var/log/nginx/access.log lineal_masked;   # Token-Pfade maskiert (S-12)
    set $lineal_upstream http://server:8080;
    resolver 127.0.0.11 valid=5s ipv6=off;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;

    location ^~ /files/ { limit_req zone=lineal_api burst=200 nodelay; proxy_pass $lineal_upstream; }
    location / { return 404; }    # keine SPA, kein /login, kein Cookie auf dieser Origin
}
```

(Die `limit_req_zone`/`map`/`log_format` auf http-Ebene gelten für beide Blöcke.)

### App-Guard (Defense-in-Depth, nicht nur nginx)

Kleine Middleware in lineal-api/-server: wenn `Host` == konfigurierte Datei-Origin **und** Pfad nicht unter `/files/` → **404**, bevor Router/SPA/Login greifen. So hält die Isolation auch bei Fehlkonfiguration des Proxys. Nur aktiv, wenn `LINEAL_FILES_BASE_URL` gesetzt.

### Config

- Neu: `LINEAL_FILES_BASE_URL` (z. B. `https://lineal-files.kollegeplus.de`). Ungesetzt → Feature aus, heutiges Verhalten.
- Validierung analog `base_url` (https, trailing-slash-normalisiert); sollte sich von `base_url` unterscheiden (Warnung/Fehler, wenn gleich).

### blob.rs — `download`-Handler

Host aus dem `Host`-Header lesen. **Host-Match: exakt, case-insensitive, Port abgeschnitten, fail-closed.** Nur wenn `Host` == Datei-Origin **und** `essence(content_type) == "text/html"`:

- `Content-Disposition: inline`
- CSP (ersetzt die heutige `default-src 'none'; sandbox`):
  ```
  default-src 'none'; sandbox allow-scripts;
  script-src 'unsafe-inline'; style-src 'unsafe-inline' https://fonts.googleapis.com;
  font-src https://fonts.gstatic.com; img-src data:;
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'
  ```
  (kein `connect-src` → `default-src 'none'` verbietet fetch/XHR/WebSocket: das Deck kann nicht nach Hause telefonieren. `img-src data:` ohne `'self'` — „alles embedded". `sandbox allow-scripts` ohne `allow-same-origin` → opake Origin.)

Sonst **unverändert** (heutiges `inline_safe`/`attachment` + `default-src 'none'; sandbox`). `nosniff` kommt weiter global. `Referrer-Policy: strict-origin-when-cross-origin` (global) sendet an Fonts nur die Origin, **nicht** den Token-Pfad — ausreichend; optional explizit `no-referrer` auf Blob-Antworten.

**Verifizieren bei Umsetzung:** kol478.html (scratchpad) navigiert tatsächlich unter `sandbox allow-scripts` (keydown/click-Nav, kein localStorage/fetch).

### Anhangs-Auflösung — `viewUrl`

Resolver, der Anhänge liefert, bekommt ein Feld `viewUrl`: für `text/html` die Datei-Origin-URL (gleiches Token, Host getauscht), sonst `null`. Gründe: (a) `download`-Attribut wird cross-origin ignoriert → „Herunterladen" MUSS die App-Origin-URL nutzen, zwei URLs sind ohnehin nötig; (b) MCP/Bots konsumieren dasselbe `url`-Feld und sollen den Ansichts-Link auch bekommen. Host-Swap passiert im Backend, nicht im Frontend.

### Frontend (web/)

Anhangsliste: bei `text/html` Primäraktion „Im Tab öffnen" → `viewUrl` via `target="_blank" rel="noopener noreferrer"`. Sekundär „Herunterladen" → App-Origin-`url` (`attachment`). Andere Typen unverändert.

## Review Focus (red-first-Tests, je Task verankert)

1. **App-Origin + `text/html`** → weiterhin `attachment` + `default-src 'none'; sandbox` (kein Script). *Darf sich nie ändern.*
2. **Datei-Origin + `text/html`** → `inline` + Präsentations-CSP mit `sandbox allow-scripts`.
3. **Datei-Origin + Nicht-HTML** (png/pdf/…) → unverändert wie App-Origin.
4. **Host-Spoofing:** `evil.lineal-files.kollegeplus.de`, `lineal-files.kollegeplus.de.evil.com`, Groß/Kleinschreibung, mit Port → **kein** Inline-Match (fail-closed).
5. **`LINEAL_FILES_BASE_URL` ungesetzt** → Verhalten exakt wie heute (Feature aus).
6. **App-Guard:** Host == Datei-Origin + Pfad `/login`,`/`,`/graphql` → 404 (SPA/Login nie auf der Datei-Origin).

## Entschieden (Eigentümer, 2026-11-13)

- Subdomain-Name: **`lineal-files.kollegeplus.de`**.
- **CNAME** auf `lineal`.
- Freigabe zum Bau erteilt.

## Aufwand / Risiko

Projektgröß: Config + blob.rs (Host-Branch + CSP) + App-Guard-Middleware + Resolver-`viewUrl` + Frontend + nginx-Block + Caddy/DNS-Handoff + Tests. Risiko gering, weil ungesetzte Config = No-op und App-Origin unangetastet bleibt. Sensible Schritte (DNS/Caddy) laufen als Anleitung über den Eigentümer.
