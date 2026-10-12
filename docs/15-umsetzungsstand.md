# 15 – Umsetzungsstand

Dieses Kapitel beschreibt, was in der Implementierung (Rust-Workspace + `web/`) tatsächlich vorhanden ist, welche Lücken bekannt sind, wie der Server betrieben wird und welche Sicherheitsmaßnahmen gelten. Die Fachkapitel 00–14 bleiben die Spezifikation; wo die Implementierung bewusst abweicht, steht an der Stelle ein Blockzitat „Umsetzung“. Dieses Kapitel ersetzt `docs/dev/offene-punkte.md` nicht (das ist die Arbeitsliste der Pakete), fasst den bereinigten Stand aber zusammen.

## 1. Was implementiert ist

| Bereich | Stand |
|---|---|
| Domäne und Regeln (`lineal-domain`) | IDs, Enums, Status-/Estimate-/Label-/Cycle-/SLA-Regeln, Filter, Markdown-Sanitizer, Domain-Events |
| Use-Cases (`lineal-app`) | Workspace, Teams (Sub-Teams, Vererbung, Lebenszyklus), Issues (Workflow, Relationen, Sub-Issues, Historie, Teamwechsel), Kommentare, Reaktionen, Anhänge (Upload-Flow), Labels, Cycles, Projekte, Meilensteine, Initiativen, Status-Updates, Views, Favoriten, Suche, Inbox/Notifications, Dokumente, Templates, Triage, Webhooks, Einladungen, Import, firmenweite Übersicht |
| Persistenz (`lineal-db`) | PostgreSQL ≥ 15, Migrationen `0001`–`1800`, `PgStore`/`PgTx` mit allen Repos |
| API (`lineal-api`) | GraphQL, REST (`/auth/*`, `/oauth/*`, Blob-Endpunkte), Fehlermodell, Rate-Limit, Security-Header, CSRF-Schutz |
| MCP (`lineal-mcp`) | 55 kompatible Tools plus `get_workspace_overview`, Streamable HTTP, OAuth |
| Sync (`lineal-sync`) | WebSocket-Delta-Strom `GET /sync?since=`, Gruppenfilter, Token-Re-Validierung |
| Jobs (`lineal-jobs`) | Outbox-Konsumenten und periodische Jobs (siehe 3.3) |
| Import (`lineal-import`) | Linear-API-Import (Scan, Lauf, Delta), CLI |
| Web (`web/`) | Shell, Team-Issues (Liste/Board), Issue-Detail, Create-Modal, Views/Favoriten, Inbox, Suche, Planung, Dokumente, Einstellungen, Import |
| Tests | Rust-Unit-/Integrationstests mit echter Datenbank (`#[sqlx::test]`), Vitest, Playwright-E2E (`scripts/e2e.sh`) |

## 2. Bekannte Lücken

Bereinigt gegenüber `docs/dev/offene-punkte.md` (erledigte Punkte entfernt):

- **Sync:** Tab-Leader-Wahl im Web fehlt; der Web-Client hält weiter TanStack-Query-Caches (kein normalisierter Objektgraph), `/sync/bootstrap` befüllt daher beim Erststart nur das Personenverzeichnis (siehe §4.2); `/sync` und `/sync/bootstrap` liegen nicht hinter dem In-Process-Rate-Limit der API.
- **Issues:** `moveSubIssues`, Bulk-Move, `issueBatchUpdate`, Label-Merge/-Delete, Erwähnung `ENG-123` → `related`-Relation, Duplicate-Subscriber-Übernahme; der Abo-Zustand eines Issues als GraphQL-Feld. (`Issue.children`, `relations` und `history` gibt es für die Detailansicht; sie laden beim Zugriff nach und gehören nicht in Listenabfragen. Die Web-Detailansicht lädt sie mit einer eigenen Query `IssueExtras` (Query-Key `["issue-detail", id]`, Invalidierung bei eigenen Mutationen und Sync-Events zu Issue/IssueRelation): Sub-Issues teamübergreifend aus `children`, Relationen aus `relations`, Aktivitätsverlauf aus `history` chronologisch gemischt mit den Kommentaren, IDs über die Caches in Namen aufgelöst, geschwärzte Ziele als „ein privates Element“.)
- **Teams/Nesting:** Label-Merge, Cycle-Instanzen und Projektstatus-Neuauflösung beim Nesting; `update_team` stößt die Cycle-Generierung nicht sofort an (Job holt innerhalb 1 min nach).
- **Notifications:** View-Abos sind nur als Schnittstelle (`ViewSubscriptionSource`) angebunden, die Quelle liefert WP31; keine Notifications für `documentMention`/`documentContentChange`/`documentMoved`/`documentDeleted`/`documentRestored`/`documentSubscribed` (die Dokument-Events tragen keine Details), `projectDescriptionContentChange`/`initiativeDescriptionContentChange`, `initiativeUpdateCreated` und `*ThreadResolved`; `issueUnblocked` entsteht nur, wenn der letzte Blocker geschlossen wird (beim Entfernen einer `blocks`-Relation fehlt dem Event die Relationsart, die Zeile ist schon gelöscht).
- **Planung/Automationen:** Snooze-Wiedervorlage bei Aktivität, Auto-Archive-Benachrichtigung, Cycles/Projekte auto-archivieren, Kapazität, `cycleNumberBase`.
- **Dokumente/Templates:** Dokument-Kaskade bei Issue-/Projekt-/Team-Löschen nur teilweise, Snapshot-Bündelung, Dokument-Mentions, CRDT, Form-Templates, `useDefaultTemplate`.
- **Views/Suche:** Favoriten umsortieren, View-Abos, Eigentümerwechsel, Stemming (Volltext-Konfiguration `simple`).
- **Web:** Filter-Chips aus `filterData` zurückwandeln, Initiativen-Tabs, Roadmap-Timeline, Team-Settings (SLA/Hierarchie/Webhooks/Recurring; Estimates, Automations und Templates sind vorhanden), Workspace-Settings (Limits; Teams, Members, Security, API, Labels und Templates sind vorhanden), Account-Settings.
- **Webhooks/Auth:** `client_credentials`, App-User und feingranulare Scopes; Notification bei Webhook-Deaktivierung; SSRF-Bausteine (`UrlPolicy`) für alle ausgehenden HTTP-Pfade; Legacy-SSE-/stdio-MCP.
- **Import:** Abfragen sind gegen das öffentliche Linear-Schema validiert (WP28), die Antworten des echten API aber noch nie gelesen; Teilimport (Projektfilter, Zielteam) in docs/13 §10; Views/Favoriten/Issue-Reaktionen/Update-Kommentare werden nicht importiert; Dry-Run ohne Downloads; Sub-Issues anderer Projekte entfallen beim Projektfilter.
- **Rate-Limit:** In-Process-Verteidigung in der Tiefe (siehe 4.4); weitere Feinheiten (u. a. Fehlversuchs-Zähler der Token-Prüfung ohne Reservierung, geteilter Überlauf-Bucket) sind als offene Punkte notiert.

## 3. Betrieb

### 3.1 Umgebungsvariablen

| Variable | Default | Bedeutung |
|---|---|---|
| `DATABASE_URL` | `postgres://lineal:lineal@localhost:5440/lineal` | PostgreSQL ≥ 15 |
| `LINEAL_BASE_URL` | `http://localhost:8080` | öffentliche URL (Links, Cookie-`Secure` bei `https://`, Standard-Origin für CSRF) |
| `LINEAL_SESSION_SECRET` | – (Pflicht, ≥ 32 Byte) | signiert Blob-URLs, Consent-Nonces, verschlüsselt Import-Tokens; ohne `LINEAL_DEV=1` Pflicht, der Beispielwert wird abgelehnt |
| `LINEAL_DEV` | aus | Entwicklungsmodus: GraphiQL, zufälliges Secret, CORS/Origin für `localhost`/`127.0.0.1` |
| `LINEAL_BIND` | `127.0.0.1:8080` | Adresse des HTTP-Servers |
| `LINEAL_BLOB_DIR` | `./var/blobs` | Verzeichnis des lokalen Objektspeichers |
| `LINEAL_DB_MAX_CONNECTIONS` | `10` | Größe des Verbindungspools |
| `LINEAL_ALLOWED_ORIGINS` | Origin von `LINEAL_BASE_URL` | Komma-Liste erlaubter Origins für Cookie-Sessions (CSRF) |
| `LINEAL_TRUSTED_PROXIES` | leer | Komma-Liste von CIDR-Bereichen; nur von dort wird `X-Forwarded-For` ausgewertet |
| `LINEAL_WORKSPACE_CREATION` | `all` mit `LINEAL_DEV`, sonst `disabled` | wer per GraphQL `createWorkspace` weitere Workspaces anlegen darf: `disabled`, `admins`, `all`; ungültiger Wert verhindert den Start. Das CLI `create-workspace` ist ausgenommen |
| `LINEAL_INSTANCE_ADMINS` | leer | nur für `admins`: Komma-Liste von User-UUIDs (`lineal create-workspace` gibt die ID des Owners aus). Erlaubt ist nur ein aktiver User mit Rolle Owner/Admin und dieser ID. Bewusst keine E-Mail und kein `url_key`: E-Mails legt jeder Workspace-Admin beliebig an, `url_key`s lassen sich umbenennen und neu belegen. Ungültige UUIDs verhindern den Start |
| `LINEAL_OWNER_PASSWORD` | – | nur für `lineal create-workspace`: Passwort des Owners (sonst interaktive Abfrage ohne Echo) |
| `LINEAL_DB_PASSWORD` | – | nur Docker-Profil `app`: Passwort der Datenbank `app-db` (Pflicht; URL-sichere Zeichen, z. B. `openssl rand -hex 24`) |
| `LINEAL_HTTP_PORT` | `8088` | nur Docker: Host-Port des nginx |
| `LINEAL_BACKUP_DIR` | `./backups` | nur Backup-Skripte: Zielverzeichnis, relativ zum Projektverzeichnis |
| `LINEAL_BACKUP_KEEP` | `14` | nur Backup-Skripte: Anzahl aufbewahrter `lineal-*`-Backups |
| `LINEAL_BACKEND` | `http://localhost:8080` | nur Web (Vite-Proxy/Preview): Adresse des Servers |
| `RUST_LOG` | `info` | Log-Filter |

### 3.2 Prozesse und Ports

- `lineal migrate`, `lineal create-workspace`, `lineal seed [--demo]`, `lineal serve`, `lineal worker`, `lineal import`, `lineal schema-export`.
- `serve` und `worker` sind getrennte Prozesse und teilen sich die Datenbank; der Worker braucht keinen Port. Der Server muss mit `ConnectInfo` laufen (Client-IP für Rate-Limit); `lineal serve` tut das.
- Entwicklung: PostgreSQL im Container auf `127.0.0.1:5440`, Server `:8080`, Vite-Dev `:5173`. Belegte Ports über `LINEAL_BIND`/`LINEAL_BACKEND` umstellen. E2E (`scripts/e2e.sh`) nutzt die Datenbank `lineal_e2e` und freie Ports (ab 18741 bzw. 4173).
- Hinter einem Reverse-Proxy: `LINEAL_BASE_URL` auf die öffentliche URL, `LINEAL_TRUSTED_PROXIES` auf die Proxy-Adressen, WebSocket-Upgrade für `/sync` durchreichen.

### 3.2c Ersteinrichtung

Eine frische Instanz hat keinen Workspace; `lineal serve` loggt dann ein WARN mit dem Befehl. Ersten Workspace samt Owner anlegen (nach `lineal migrate`):

```bash
LINEAL_OWNER_PASSWORD='…' lineal create-workspace --name "Acme" --url-key acme \
  --owner-email alice@acme.example --owner-name "Alice"
```

Das Passwort wird nie als Argument übergeben: aus `LINEAL_OWNER_PASSWORD` oder interaktiv ohne Echo mit Wiederholung (mindestens 8 Zeichen). Ausgabe: Login-URL (`LINEAL_BASE_URL/login`) und die User-ID des Owners (für `LINEAL_INSTANCE_ADMINS`). Belegter `--url-key` ist ein Fehler. Das CLI ist Betreiberzugang und von `LINEAL_WORKSPACE_CREATION` ausgenommen. Weitere Workspaces über die Oberfläche/GraphQL nur, wenn der Betreiber es erlaubt (Default außerhalb des Dev-Modus: `disabled`, Antwort `forbidden` „Workspace creation is disabled on this instance.“).

`lineal seed` (auch `--demo`) legt Benutzer mit dem bekannten Passwort `lineal-dev` an und läuft deshalb nur mit `LINEAL_DEV=1` oder `--allow-insecure-demo-users`.

### 3.2a End-to-End-Tests

`scripts/e2e.sh` (nicht Teil von `check.sh`; Laufzeit ca. 1–2 Minuten) legt die Datenbank `lineal_e2e` im Container neu an, migriert, seedet (`lineal seed --demo`), legt zwei API-Keys für Bob an (frei und auf ENG beschränkt), baut das Web und startet über Playwright `lineal serve` (mit `LINEAL_DEV=1`), `lineal worker` und `vite preview` (Proxy auf den Server) auf freien Ports; Chromium läuft headless mit `chromiumSandbox: false`. Flows (`web/e2e/*.e2e.ts`): F3 Issue anlegen (Taste `c`, „Create more“, lückenlose Kennung), F5 Status-Picker `S` durch den Workflow, F12 Filter → View → Favorit, F13 Erwähnung → Inbox eines zweiten Users, F16 Einladung annehmen (Alice lädt ein, zweiter Browser-Kontext nimmt über `/invite/<token>` an), F7 Teamwechsel (`Strg+Umschalt+M`, alte Kennung bleibt auflösbar), F17 MCP-Agent per HTTP (`list_issues`, `save_issue`, team-beschränkter Key), F19 API-Key in der Oberfläche anlegen, mit `list_issues` über `/mcp` benutzen und widerrufen, Sichtbarkeit des privaten Teams DES. Einzelne Flows: `scripts/e2e.sh -g F3`.

### 3.2b Docker

`Dockerfile` (mehrstufig) baut zwei Images: `--target server` (Binary `lineal` auf `debian:bookworm-slim`, User `lineal` UID 10001, Daten unter `/data`, Default `serve` auf `0.0.0.0:8080`, Healthcheck `/healthz`) und `--target web` (`nginx-unprivileged` mit dem Vite-Build; leitet `/graphql`, `/auth/*`, `/oauth/*`, `/mcp`, `/sync` (WebSocket), `/uploads/*`, `/files/*`, `/invite/accept`, `/.well-known/*` an den Server weiter, alles andere auf `index.html`). nginx setzt `X-Forwarded-For` neu (nicht angehängt) und ein Edge-Rate-Limit je IP (Login/Einladung/OAuth-Token 10/min, übrige API 50/s mit Burst) vor das In-Process-Limit.

`docker-compose.yml` hat das Profil `app`; ohne Profil startet weiter nur PostgreSQL (Entwicklung, `check.sh`).

```bash
export LINEAL_SESSION_SECRET="$(openssl rand -hex 32)"   # Pflicht; z. B. in .env ablegen
export LINEAL_DB_PASSWORD="$(openssl rand -hex 24)"      # Pflicht: Passwort der App-Datenbank
docker compose --profile app up -d --build               # migrate (einmalig), server, worker, web
# Ersteinrichtung (statt seed): Workspace und Owner anlegen
LINEAL_OWNER_PASSWORD='…' docker compose --profile app run --rm -e LINEAL_OWNER_PASSWORD migrate \
  create-workspace --name "Acme" --owner-email alice@acme.example --owner-name "Alice"
# Web-App: http://localhost:8088 (LINEAL_HTTP_PORT), Login mit der Owner-E-Mail
```

Der App-Stack hat eine eigene Datenbank `app-db` (postgres:16, Volume `appdata`, kein veröffentlichter Port) und teilt sich nichts mit der Entwicklungs-DB `postgres` (Port 5440). Ohne `LINEAL_DB_PASSWORD` startet `app-db` nicht (klare Meldung im Log); `docker compose up` ohne Profil braucht keine Variablen. `LINEAL_WORKSPACE_CREATION` (Default `disabled`) und `LINEAL_INSTANCE_ADMINS` werden an Server und Worker durchgereicht; Vorlage in `.env.example`.

Dienste: `app-db`, `migrate` (Einmal-Job, Server und Worker warten auf Erfolg), `server` (Netze `default` + `edge`), `worker`, `web` (nur Netz `edge`, Port `127.0.0.1:${LINEAL_HTTP_PORT:-8088}`). `LINEAL_TRUSTED_PROXIES` ist nur die feste Adresse des nginx-Containers `172.31.88.10/32` (nicht das Subnetz: Das Gateway `.1` ist der Host). Blobs liegen im Volume `blobs` (Server und Worker teilen es). Für den Betrieb im Internet: TLS vor `web` terminieren, `LINEAL_BASE_URL=https://…` setzen (Cookie `Secure`, CSRF-Origin) und in `deploy/nginx.conf` `set_real_ip_from <TLS-Proxy>` + `real_ip_header X-Forwarded-For` ergänzen, sonst sieht der Server die Adresse des TLS-Proxys als Client-IP. Die Seed-User haben ein bekanntes Passwort; `seed` läuft deshalb nur mit `LINEAL_DEV=1` oder `--allow-insecure-demo-users` (im Container: `-e LINEAL_DEV=1`), nur für Demo-Installationen.

### 3.2d Backup und Restore

`scripts/backup.sh` sichert den App-Stack (Profil `app`) in `${LINEAL_BACKUP_DIR:-./backups}/lineal-<UTC-Zeitstempel>/` (Rechte 700):

- `database.dump`: `pg_dump -Fc` der Datenbank `lineal` aus `app-db` (über `docker compose exec`, Ausgabe auf stdout; keine Passwörter auf der Kommandozeile oder in Dateien).
- `blobs.tar.gz`: Inhalt des Volumes `blobs` (Dateien unter `/data/blobs`), gelesen über einen kurzlebigen `alpine`-Container. Der Volume-Name ist `<Projekt>_blobs` (Projekt `lineal`: `lineal_blobs`).
- `manifest.txt`: Zeitpunkt, Quellstand (Commit), Migrationsstand (`max(version)` aus `_sqlx_migrations`), Größen und sha256 beider Dateien.

Das Backup entsteht zuerst in `lineal-<ts>.partial` und wird erst nach Erfolg umbenannt; bei Fehlern bleibt kein halbfertiges Verzeichnis zurück. Nach erfolgreichem Lauf löscht die Aufbewahrung die ältesten `lineal-*`-Verzeichnisse über `LINEAL_BACKUP_KEEP` (Default 14). Der Stack muss laufen (`app-db`); die Anwendung selbst muss nicht gestoppt werden.

```bash
scripts/backup.sh                              # Backup anlegen (Exit-Code ≠ 0 bei Fehlern)
scripts/restore.sh backups/lineal-<ts>         # Restore: Eingabe "restore" verlangt
scripts/restore.sh --yes backups/lineal-<ts>   # ohne Rückfrage (Automatisierung)
```

Cron-Beispiel (täglich 03:00 Ortszeit des Hosts; `LINEAL_BACKUP_*` aus `.env` oder der Crontab):

```cron
0 3 * * * cd /pfad && scripts/backup.sh >> /var/log/lineal-backup.log 2>&1
```

`scripts/restore.sh` prüft zuerst sha256 und Größen aus dem Manifest; bei Abweichung bricht es ab, ohne etwas zu ändern. Ist der Migrationsstand des Backups neuer als die Migrationen des Checkouts (aus dem die Images gebaut werden), warnt es. Danach: `app-db` starten, `server`, `worker`, `web` stoppen, `pg_restore --clean --if-exists --no-owner` über `app-db`, Blob-Volume leeren und aus dem Archiv füllen, `migrate` ausführen, dann `server`, `worker`, `web` starten. Schlägt ein Schritt fehl, bleiben die Dienste gestoppt. `COMPOSE_PROJECT_NAME` (Default `lineal`) bestimmt, welcher Stack und welches Volume gemeint sind.

**Betreiberaufgaben (nicht im Skript):** Backups enthalten Datenbank und Dateien unverschlüsselt und liegen standardmäßig auf demselben Host wie der Stack. Eine Off-Site-Kopie (anderer Host, Objektspeicher) und eine Verschlüsselung vor dem Ablegen (z. B. `age -r <Empfänger>` oder `gpg --encrypt`) gehören zum Betrieb. Außerdem: Restore-Probe regelmäßig in einem eigenen Compose-Projekt durchführen (`COMPOSE_PROJECT_NAME=lineal-restore-test`, eigener Port und Subnetz; den Produktivstack nicht anfassen).

### 3.3 Hintergrundjobs

Event-Konsumenten (Outbox): Notifications, Suchindex, Sync, Webhooks. Fehlversuche je (Konsument, Event) stehen in `outbox_delivery.attempts`/`last_error` und überleben Neustarts; nach `max_event_attempts` (10) gilt das Event als aufgegeben (Dead-Letter: zugestellt mit `last_error`, abfragbar über `OutboxRepo::list_dead_letters`). Periodische Jobs: `cycles` (60 s), `update_reminders` (1 h), `audit_retention` (1 h, Audit-Log je Workspace nach `auditLogRetentionDays`), `snooze` (60 s), `sync_purge` (1 h, Sync-Log 14 Tage), `notification_cap` (5 min), `due_reminders` (5 min), `email_digest` (60 s), `auto_close`, `auto_archive` (je 5 min), `purge` (5 min: Papierkorb nach Frist, abgelaufene Upload-Reservierungen, zugehörige Blobs), `sla` (60 s), `triage_rotation` (60 s), `import_runner` (10 s), Webhook-Zustellung, `team_purge` (1 h).

## 4. Sicherheitsmaßnahmen

### 4.1 Authentifizierung und CSRF

- Session-Cookie `lineal_session` (`HttpOnly`, `SameSite=Lax`, `Secure` bei https); API-Keys und OAuth-Tokens per `Authorization`. Tokens liegen nur als SHA-256-Hash in der Datenbank. Die Token-Art bestimmt die Tabelle des Treffers (`Principal.kind`), nicht der Transportweg: Cookie nur für Sessions, `Authorization` nie für Sessions (sonst 401, zählt als Fehlversuch; auch `/auth/me`).

**API-Keys in der Oberfläche (WP19).** Einstellungsseite `/{ws}/settings/api` (persönlich; Eintrag „API“ in der Settings-Navigation): Liste der eigenen Keys (Label, Präfix, Scopes, Teams, zuletzt benutzt, Ablauf, Status), Dialog „New API key“ (Label ≤ 100 Zeichen, Read immer, optional Write, optionale Team-Auswahl – keine Auswahl = alle Teams, optionales Ablaufdatum), danach einmalige Token-Anzeige mit Kopieren-Button und Warnhinweis (der Klartext lebt nur im Dialogzustand, nicht im Query-Cache), Widerruf mit Bestätigung. GraphQL: `apiKeys` (nur eigene Keys, nie Hash/Klartext), `apiKeyCreate(input: {label, scopes: [READ|WRITE], teamIds?, expiresAt?})` mit `token` nur in dieser Antwort, `apiKeyRevoke(id)` (fremde IDs `not_found`, auch für Admins). Regeln: Anlegen nur mit echter Web-Session (`RequestCtx.via_session`; API-Key/OAuth → `forbidden`), ein team-beschränkter Aufrufer erzeugt nur Keys innerhalb seines Team-Scopes, `teamIds` nur sichtbare Teams (sonst `not_found`), höchstens 25 aktive Keys je User (`limit_exceeded`), kein `admin`-Scope über die UI.
- CSRF für Cookie-Sessions: `POST /graphql` und `POST /auth/logout` verlangen einen erlaubten `Origin` (ersatzweise `Referer`) **und** den Header `X-Lineal-CSRF: 1`; `/auth/login`, `/oauth/authorize` (der session-gebundene Nonce ist das Token) und signierte Uploads prüfen nur den Origin. Anfragen mit `Authorization`-Header sowie `/mcp` und `/sync` (eigene Origin-Prüfung) sind ausgenommen.
- Einladungen: `POST /invite/accept` (JSON `token`, `email`, `name?`, `password`) nimmt eine Einladung ohne Session an, legt den User an bzw. verknüpft den Platzhalter und setzt das Session-Cookie; ungültige Tokens oder fremde E-Mails antworten einheitlich `404`; CSRF wie beim Login (nur Origin). Die Web-Seite `/invite/<token>` (öffentlich, außerhalb von `$ws` und ohne App-Shell; Root-Guard lässt `/invite/…` ohne Session durch) fragt E-Mail (Nachweis, serverseitig geprüft), Name, Passwort (mind. 8 Zeichen) und Wiederholung ab, sendet `POST /invite/accept` mit `X-Lineal-CSRF: 1` und leitet nach `/<ws>/inbox`. Abgelaufene, widerrufene, angenommene und ungültige Links sowie fremde E-Mails zeigen dieselbe neutrale Meldung (der Server unterscheidet sie bewusst nicht). Ist bereits eine Session vorhanden, bietet die Seite „Mit diesem Konto annehmen“ (GraphQL `acceptInvitation`) oder „neues Konto anlegen“. Es gibt keinen Vorschau-Endpunkt: Die E-Mail muss ohnehin eingegeben werden, und ein Vorschau-Endpunkt würde Token-Gültigkeit und Workspace-Namen an Unangemeldete verraten. Die Seite setzt `referrer: no-referrer`; nginx liefert `/invite/*` mit `Cache-Control: no-store` und `Referrer-Policy: no-referrer`; der Vite-Proxy leitet nur `/invite/accept` an den Server.
- Mitglieder und Einladungen (Web): `/{ws}/settings/members` (Settings-Navigation „Members“) zeigt die sichtbaren Workspace-Mitglieder (Name; die GraphQL-Sicht `UserRef` liefert weder E-Mail noch Rolle, die eigene Rolle kommt aus `/auth/me`) und die offenen Einladungen (E-Mail, Rolle, Teams, Ablauf, Status; „Resend“, „Revoke“) sowie den Dialog „Invite people“ (mehrere Adressen, Rolle, Teams; Gäste brauchen ein Team; Admin-Rolle nur für Owner/Admin). Gäste sehen nur einen Hinweis; für andere Rollen entscheidet der Server (`invitationsRole`, Antwort `forbidden` → Hinweis statt Liste); Serverfehler (z. B. Adresse bereits Mitglied) erscheinen im Dialog. E2E: F16 (`web/e2e/invite.e2e.ts`; setzt den Token per SQL auf einen bekannten Hash, da der Dev-Mailer nichts versendet und nur der Hash gespeichert wird).
- Workspace-Administration (WP22): Settings-Seite `/{ws}/settings/security` (nur Owner/Admin sichtbar): Mindestrolle je Recht in Klartext-Fragen („Who can invite members?“), Domain-Liste für Einladungen, Speichern mit Bestätigungsdialog (zeigt die Änderungen) und Serverfehlern; Admins sehen `adminManagementRole` gesperrt. Members-Seite: Admins/Owner bekommen eine Tabelle (E-Mail, Rolle per Select, Status, Sperren mit Bestätigung/Entsperren), andere Rollen die bisherige Namensliste; der Invite-Dialog nennt die erlaubten Domains. GraphQL und Regeln: docs/02 §6.3. Sperren wirkt sofort: Token-Auflösung (`user.active`), Login, OAuth-Code/Refresh prüfen den Status; `revoke_all_credentials_of_user` widerruft Sessions, Keys, OAuth-Tokens, offene Codes und offene Einladungen des Users dauerhaft. Offene WebSocket-Verbindungen enden mit 4401, sobald die `UserUpdated`-Aktion zugestellt wird (sonst nach spätestens 60 s). E2E: `web/e2e/f22-workspace-admin.e2e.ts`; `scripts/e2e.sh` nimmt `LINEAL_E2E_DB=<name>` für eine eigene Datenbank.
- OAuth-Zustimmung nur per `POST` mit session-gebundenem Nonce, Zielhost und Warnhinweis; CSP `form-action 'self'`.
- Brute-Force-Schutz: fehlgeschlagene Authentifizierungen zählen je IP (20/min), beim Login zusätzlich je Paar (Account, IP) (10/15 min) und je Account gesamt (100/15 min); Zähler werden vor der Passwortprüfung atomar reserviert.

### 4.2 Sync-Verbindungen

Das Token einer WebSocket-Verbindung wird alle 60 s und nach jeder Zustellung von User-/Team-/Organisations-Änderungen neu geprüft; widerrufene oder abgelaufene Tokens und suspendierte User beenden die Verbindung mit Close-Code 4401. Sichtbarkeit wird bei jeder Zustellung neu berechnet.

**Gruppen (WP32).** Zusätzlich zu `team:<id>`, `org` und `user:<id>` gibt es zwei Gruppen, die Gästen (kein `org`) Live-Caches ohne fremde Daten erlauben: `orgpub` (Organisation und Workspace-Labels; alle User) und `tmem:<teamId>` (Personen: `User`-Aktionen tragen `org`, `user:<ownId>` und `tmem:<t>` je Mitgliedschaft; Gäste erhalten `tmem:<t>` nur für ihre Mitglieds-Teams, sehen also nur Personen ihrer Teams wie in GraphQL). Der Zustellfilter schneidet die Gruppen der Aktion mit denen des Empfängers und gibt in `syncGroups` nur die Schnittmenge zurück, fremde Team-IDs erscheinen nie. Freigaben (`issue_share`): Issue, Kommentare, Anhänge und Reaktionen des geteilten Issues tragen zusätzlich `user:<id>` des Empfängers; das Teilen löst ein `U` aus (der Empfänger kennt das Issue noch nicht und lädt es nach), der Entzug ein `D` nur an `user:<id>`. Mitgliedschaftsänderungen schreiben zusätzlich ein `User`-`U` (bei Entzug geht ein `D` an `tmem:<team>`).

**Bootstrap (WP32).** `GET /sync/bootstrap?modelNames=A,B&limit=N&cursor=…` (Auth wie `/sync`, Lese-Scope) liefert den Bestand in Paketen als `application/x-ndjson`: je Zeile ein Objekt mit `__class` und `syncGroups`, letzte Zeile `_metadata_={"lastSyncId","subscribedSyncGroups","databaseVersion":1,"nextCursor","count"}`. Default-Modelle: Organization, Team, User, WorkflowState, IssueLabel, Cycle, Project, ProjectMilestone, Initiative, CustomView, Issue (ohne Papierkorb und Archiv); auf Anfrage Template und Document. `limit` 1–2000 (Default 1000). `lastSyncId` ist der Stand **vor** dem ersten Lesen und steckt im Cursor (bleibt über alle Pakete gleich): danach `GET /sync?since=<lastSyncId>` öffnen und nachspielen. Rechte: derselbe Gruppenfilter wie im Strom, mit dem Policy-Snapshot der Anfrage (Sicherheitstests: ein Gast erhält weder fremde Teams noch deren Issues, Dokumente oder Personen). nginx: `location = /sync/bootstrap` (ohne Puffer). Web: `SyncClient` ruft beim Erststart (ohne gespeicherte `lastSyncId`) `fetchBootstrap(["User"])`, befüllt das Personenverzeichnis (`["users", …]`, spart die Abfrage `users`) und verbindet mit `?since=<lastSyncId>`; schlägt das fehl, gilt der bisherige Ablauf (Strom live, alles neu laden). Alle übrigen Caches werden nach dem Bootstrap invalidiert, weil Abfragen vor dem Bootstrap Änderungen verpasst haben können.

### 4.3 Zugriff auf Daten

Jeder Lese- und Schreibpfad (GraphQL, MCP, REST, Jobs) läuft durch dieselbe Policy (`visible_team_ids`, `require_*`, Scope); Namensauflösung, Fehlermeldungen und Aggregate verraten nichts über unsichtbare Objekte. Rich Text wird beim Speichern bereinigt, URLs nur über die Allowlist akzeptiert. API-Keys mit `team_ids` sind auf diese Teams beschränkt (`RequestCtx.team_scope`, Schnitt in `visible_team_ids`; Teams außerhalb verhalten sich wie unsichtbare: `not_found`); eine leere Liste bedeutet „kein Team“, `NULL` keine Beschränkung. Workspace-weite Admin-Aktionen schränkt der Team-Scope nicht ein. Die Issue-Historie (`App::issue_history`, auch für GraphQL `Issue.history`) entfernt Relationsänderungen zu unsichtbaren Issues und setzt Verweise (`teamId`, `parentId`, `projectId`, `cycleId`, `projectMilestoneId`) auf unsichtbare Ziele auf `null`. Die Nachladefelder `Issue.children/relations/history` sind pro Request auf 10 Aufrufe begrenzt (sonst `invalid_request`, „only available in the detail view“) und in der Komplexität hoch gewichtet (`history` Faktor 100, `children` als Liste mit 50).

### 4.4 Rate-Limit und Client-IP

- Design: Stufe 1 begrenzt je Client-IP (IPv4 je Adresse, IPv6 je /64) **vor** dem Token-Lookup (Anfragen ohne Zugangsdaten niedrig, mit Zugangsdaten höher), Stufe 2 je User/Token nach dem Lookup (Requests und GraphQL-Komplexität). Bucket-Maps sind begrenzt (200 000 Schlüssel); bei voller Map teilen sich neue Schlüssel einen Überlauf-Bucket mit einem Zehntel des Limits, aktive Buckets werden nie verdrängt.
- `X-Forwarded-For` zählt nur, wenn die Peer-IP in `LINEAL_TRUSTED_PROXIES` liegt: alle Header werden zu einer Liste zusammengeführt, jeder Eintrag muss eine reine IP sein (sonst 400), von rechts werden vertrauenswürdige Proxys abgezogen. `Forwarded` (RFC 7239) wird nicht ausgewertet.
- **Einordnung:** Das In-Process-Rate-Limit ist Verteidigung in der Tiefe. Produktiv gehört ein Edge-Rate-Limit (Reverse-Proxy/WAF) davor. Das Restrisiko „geteilter Überlauf unter Massen-Spoofing“ ist bewusst akzeptiert.

### 4.5 Dateien und Löschen

Objekte liegen hinter HMAC-signierten, kurzlebigen URLs; `BlobStore::delete` entfernt Blobs beim endgültigen Löschen von Issues und Teams sowie bei abgelaufenen, nie finalisierten Upload-Reservierungen (nur wenn kein Anhang mehr auf das Objekt verweist).

### 4.6 Lieferkette

`scripts/check.sh` führt (wenn installiert) `cargo deny check advisories bans licenses` aus (`deny.toml`: nur permissive Lizenzen, keine unbekannten Registries) und `npm audit --omit=dev --audit-level=high` für die Laufzeit-Abhängigkeiten des Web-Clients (derzeit 0 Funde). **Begründete Ausnahme:** `npm audit` meldet für Dev-Werkzeuge hohe Funde in der Kette `@graphql-codegen/cli` → `graphql-tools` → `globby`/`micromatch` → `braces` (Stack-Erschöpfung bei tief verschachtelten Mustern, GHSA-vfj7-8cjw-p6xm). Es gibt keine gepatchte `braces`-Version; die Pakete laufen nur beim Build über eigene Konfigurations- und Schemadateien, nie mit fremden Eingaben und nicht im ausgelieferten Bundle. Erneut prüfen, wenn `@graphql-codegen/cli` oder `braces` aktualisiert werden.

### 4.7 Audit-Log (WP25)

- Tabelle `audit_log` (Migration 2500): je Eintrag Zeitpunkt, Akteur (`actor_user_id`, `actor_kind` user/app/system), Zugangsart (`credential_kind` session/api_key/oauth/cli), Aktion, Ziel, Details (JSON), IP und Request-ID. Geschrieben wird in **derselben** Transaktion wie die Änderung (`App::record_audit`), nicht über die Outbox.
- Protokolliert: Login erfolgreich und fehlgeschlagen (Fehlversuch nur, wenn das Konto existiert; Details nur `reason`), Logout, Security-Settings (vorher/nachher je Einstellung), Rollenwechsel, Sperren/Entsperren, Einladung erstellt/erneut gesendet/widerrufen/angenommen, API-Key erstellt/widerrufen, OAuth-App autorisiert/widerrufen, Workspace angelegt (inkl. CLI `create-workspace`), Import gestartet/abgeschlossen, Team angelegt/archiviert/gelöscht, Webhook angelegt/gelöscht.
- Keine Geheimnisse: nie Tokens, Passwörter, API-Key-Klartext (nur `key_prefix`), Webhook-Secrets, Pfad und Query einer Webhook-URL (nur der Host), keine Issue-Inhalte. E-Mail-Adressen stehen nur in Einladungs-Einträgen (Admin-Sicht); die GraphQL-Antwort liefert Akteurs-Namen, keine E-Mails.
- Append-only: Trigger verbietet `UPDATE`, `DELETE` und `TRUNCATE`. Löschen ist nur über die Funktion `audit_log_purge_expired(now)` möglich, die den Sitzungsschalter `lineal.audit_purge` transaktionslokal setzt; der Job `audit_retention` ruft sie auf.
- Aufbewahrung: `securitySettings.auditLogRetentionDays` (Default 365, Minimum 30, Maximum 3650); nur über GraphQL `workspaceSecuritySettingsUpdate` änderbar, mit denselben Rechten wie die übrigen Security-Settings (`UpdateWorkspaceSettings`).
- Lesen: GraphQL `auditLog(first, after, action, actorId, since, until)`, nur Owner/Admin mit Scope `read` und `admin`; sonst `forbidden`. Cursor-Pagination, höchstens 100 je Seite. Mandantentrennung wie bei den übrigen Tabellen: jede Abfrage filtert auf `organization_id` der Transaktion (kein Postgres-RLS).
- Client-IP und Request-ID kommen aus `RequestCtx` (`client_ip`, `request_id`), `credential_kind` aus dem Principal (Adapter). Login und Logout über REST übergeben beide; Einladungsannahme und Konto-Login ohne Netzwerkkontext schreiben die IP leer.
- Web: `/{ws}/settings/audit` (Settings-Navigation „Audit log“, nur Owner/Admin).

Entscheidung: Die Leseberechtigung nutzt die Prüfung „Owner/Admin mit Scope `read` und `admin`“ direkt und nicht `Action::UpdateWorkspaceSettings`, weil diese Aktion zusätzlich einen Schreib-Scope verlangt. Ein Nur-Lese-Admin-Key darf das Log lesen, aber keine Einstellungen ändern.

**Team- und Workspace-Konfiguration in den Settings (WP27).** Team-Settings um drei Abschnitte erweitert (`web/src/features/settings/team-config-sections.tsx`, `templates-manager.tsx`): „Estimates“ (Skala notUsed/exponential/fibonacci/linear/tShirt mit Beispielwerten, extended, allowZero, Default-Estimate; bei `inheritIssueEstimation` schreibgeschützt mit Hinweis), „Automations“ (Auto-close 1/3/6/9/12 Monate oder aus mit Zielstatus der Kategorie canceled, Auto-archive, Parent-/Sub-Issue-Automationen) und „Templates“ (Issue/Projekt/Dokument je Team: anlegen, bearbeiten, löschen mit Bestätigung; vorbelegbar Titel/Beschreibung als Markdown-Textfeld, Priorität, Status, Assignee/„Creator“, Estimate, Labels bei Issues; Felder, die der Editor nicht kennt, z. B. Sub-Issues, bleiben erhalten; geerbte und Workspace-Templates schreibgeschützt mit Herkunft; Default-Templates für Mitglieder, Nicht-Mitglieder und Projekte). Workspace-Settings: „Labels“ (`/{ws}/settings/labels`, gemeinsame Komponente `label-manager.tsx` mit der Team-Label-Verwaltung: Gruppen, Umbenennen, Farbe, Retire/Restore) und „Templates“ (`/{ws}/settings/templates`, Status nur als Kategorie). GraphQL: `Team` und `TeamUpdateInput` um `issueEstimationExtended/AllowZero`, `defaultIssueEstimate`, `inheritIssueEstimation` (nur lesend), `autoClosePeriod` (Eingabe `0` = aus), `autoCloseStateId`, `autoArchivePeriod`, `autoCloseParentIssues`, `autoCloseChildIssues` erweitert; neue Query `teamDefaultTemplates(teamId)`.

### 4.8 Teams anlegen und verwalten (WP26)

- Web: Settings-Seite `/{ws}/settings/teams` (Eintrag „Teams“ in der Settings-Navigation): alle sichtbaren Teams im Baum (Name, Key, Icon/Farbe, Parent, Sichtbarkeit, Mitgliederzahl, Status aktiv/retired) mit Links zu den Team-Settings und Button „New team“ (ausgeblendet für Gäste; sonst entscheidet der Server über `teamCreationRole`, Fehler erscheinen im Dialog).
- Dialog „New team“ (Sidebar „+“ neben „Your teams“, Command-Menü „Create team“, Teams-Seite): Name, Key (Vorschlag aus dem Namen nach docs/02 §2.2, live gegen `^[A-Z][A-Z0-9]{0,6}$` und reservierte Keys geprüft, Hinweis bei Kollision mit einem sichtbaren Team, Konflikt mit historischen/unsichtbaren Keys meldet der Server), Icon, Farbe, Parent-Team, Sichtbarkeit (nach Parent eingeschränkt, docs/02 §5.2; unter privatem Parent Default `restricted`), „Copy settings from team“. Nach Erfolg wird die Session (`/auth/me`) neu geladen, die Sidebar aktualisiert sich ohne Reload und die App springt auf die Team-Übersicht.
- Team-General: Beschreibung, Icon, Farbe; Key-Wechsel mit Bestätigungsdialog (alle Kennungen wechseln, alte bleiben auflösbar) und Weiterleitung auf die URL mit neuem Key; Abschnitt „Access“ (unter „Members“): Sichtbarkeit und `allMembersCanJoin`; „Danger zone“: stilllegen (bei aktiven Sub-Teams Pflichtwahl `retire`/`unnest`), reaktivieren, löschen (Bestätigung durch Eintippen des Team-Keys, bei Sub-Teams Wahl `delete`/`unnest`). Rechte entscheidet der Server, Fehler erscheinen am Dialog.
- GraphQL: `Team.memberCount` (Feldresolver über `list_team_members`; `null`, wenn der Aufrufer die Mitglieder nicht sehen darf), `Team.allMembersCanJoin`, `allMembersCanJoin` in `TeamCreateInput`/`TeamUpdateInput`. Test: `crates/lineal-api/tests/team_settings_gql.rs`.
- Tests: Vitest (`team-key`, `new-team-dialog`, `teams-page`, `team-danger-zone`), E2E F26 (`web/e2e/f26-team-create.e2e.ts`: Alice legt „KollegePlus“/KP an, sieht es in der Sidebar, Issue KP-1).

### 4.9 Notifications (WP32)

- **Neue Typen:** Kommentare an Initiativen (`initiativeNewComment`/`initiativeCommentMention`), Initiative-Berichten (`initiativeUpdate*`), Meilensteinen (`projectMilestone*`, gruppiert unter dem Projekt, `projectMilestoneId` bleibt am Eintrag), Projekt-Berichten (`projectUpdateNewComment`/`projectUpdateCommentMention`, Empfänger: Autor des Berichts und Thread-Teilnehmer) und Dokumenten (`documentNewComment`/`documentCommentMention`, Empfänger: Dokument-Abonnenten). Meilenstein-Kommentare gehen an Projekt-Abonnenten mit Typ `projectNewComment`, Initiativ-Kommentare an Abonnenten mit Typ `initiativeNewComment`. Initiativen und Workspace-Dokumente (ohne Teambezug) sehen Gäste nie (`workspace_only` im Fan-out); Dokumente mit Teambezug folgen der Sichtbarkeit des Elternobjekts. Übergänge: `issueUnassignedFromYou` (bisheriger Assignee), `issueReopened` (Subscriber, geschlossen → offen), `issueUnblocked` (Assignee/Subscriber des blockierten Issues, wenn der letzte offene Blocker geschlossen wird). Dafür trägt `DomainEvent::IssueUpdated` das Feld `previous: Option<IssuePrevious>` (`assignee_id`, `state_type`, nur gültig, wenn `assigneeId`/`stateId` in `changed` stehen; gefüllt bei `update_issue` und Triage-Wechseln, sonst `None`: dann entstehen diese drei Typen nicht).
- **Dokument-Abos:** Tabelle `document_subscription` (Grund `comment` neu, Migration 2900); Ersteller, Kommentierende und Erwähnte werden Abonnenten (nur, wenn sie das Dokument sehen); GraphQL `documentSubscribe(id)`, `documentUnsubscribe(id)` und `documentSubscribed(id)` (nur eigene Abos, unsichtbare Dokumente: `not_found`). Die Oberfläche (Glocke am Dokument) gehört zu `features/docs`.
- **View-Abos:** Schnittstelle `usecases::notification::views::ViewSubscriptionSource` (siehe Moduldoku, `docs/dev/schnittstellen.md`); `App::fan_out_event_with` und `NotificationsWithViewsConsumer` reichen die Quelle durch. Ohne Quelle entstehen keine View-Benachrichtigungen.
- **Bündelung (docs/05 §6.6):** Ereignisse desselben Akteurs am selben Objekt (Gruppenschlüssel) innerhalb von 60 s gehen in die jüngste Notification des Empfängers ein: spezifischster Typ bleibt in `type` (bei Gleichstand der bestehende), `bundledTypes` (GraphQL, Sync) listet alle Ereignistypen (höchstens 20), die Notification wird wieder ungelesen, ein Snooze der Gruppe endet, der Sync meldet `U`. Bezüge (Kommentar, Reaktion) wechseln nur mit dem Haupttyp. Ohne menschlichen Akteur (Jobs, Automationen, Erinnerungen) wird nie gebündelt. Idempotenz: `notification_merge` (Migration 2901) vermerkt den `dedup_key` zusammengeführter Ereignisse.
- **Web-Inbox:** Typ-Badge je Eintrag (`features/inbox/notification-types.ts`, unbekannte Typen werden lesbar gemacht), Zähler-Badge und Änderungsliste bei gebündelten Einträgen, Bezugsobjekt-Art im Detail für Nicht-Issues.

## 5. Entscheidungen bei Widersprüchen zwischen Kapiteln

| Stelle | Entscheidung |
|---|---|
| [04](04-planung.md) A10 | Lead-Team-Wechsel vergibt neue Projektnummer, alte Kennung als Alias |
| [03](03-issues-workflow.md) §6.3/§7.2 vs. [07](07-mcp-server.md) | Kommentar-Löschregeln wie 03; Größenlimit 2 GiB exklusiv → `limit_exceeded`; `finalize`-Fehler `invalid_request` mit `reason` |
| [05](05-views-suche-inbox.md) §4.4 | Workspace-Views nur mit `ManageWorkspaceViews`, Gäste nur persönliche Views |
| [14](14-firmenweite-sicht.md) §4 | `includeSubTeams`-Default `true` für Übersicht, `false` für Team-Views |
| [06](06-dokumente-templates.md) §1.4/§2.4 | fünf Fremdschlüssel (Cycle-Dokument mit `teamId`); übersprungene Template-Referenzen als `warnings` |
| [01](01-domaenenmodell.md) §2.6, [07](07-mcp-server.md) §3.14 | `currentUserId` gespeichert, vom Job `triage_rotation` gepflegt |
| [09](09-api-events.md) | Sync-Pfad `GET /sync?since=`, `resetRequired` → Close 4001; `/sync/bootstrap` als zeilenweises JSON mit Cursor (WP32); Gruppen `orgpub`/`tmem:<team>` für Gäste statt `org` |
| [13](13-import-linear.md) | Scan asynchron über Job, Status `canceled`, Nummern über Import-Zähler |
| [01](01-domaenenmodell.md)/[11](11-implementierungsarchitektur.md) | `lineal-sync` nutzt `sqlx` für `LISTEN/NOTIFY` |
