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

- **Sync:** `/sync/bootstrap` fehlt (Clients laden per GraphQL); Bulk-Inbox-Aktionen, Issue-Sharing und `NotificationUpdated/Archived` erzeugen keine Sync-Aktionen; Gäste haben keine org-Gruppe (Caches für User/Labels nicht live); Tab-Leader-Wahl im Web fehlt.
- **Issues:** `moveSubIssues`, Bulk-Move, `issueBatchUpdate`, Label-Merge/-Delete, Erwähnung `ENG-123` → `related`-Relation, Duplicate-Subscriber-Übernahme; `issue.children/relations/history` und der Abo-Zustand als GraphQL-Felder für die Detailansicht.
- **Teams/Nesting:** Label-Merge, Cycle-Instanzen und Projektstatus-Neuauflösung beim Nesting; `update_team` stößt die Cycle-Generierung nicht sofort an (Job holt innerhalb 1 min nach).
- **Notifications:** Kommentare an Initiativen/Dokumenten/Meilensteinen/Berichten, Dokument- und View-Abos, 1-Minuten-Bündelung, `issueUnassignedFromYou`/`issueReopened`/`issueUnblocked` (Events ohne alte Werte).
- **Planung/Automationen:** Snooze-Wiedervorlage bei Aktivität, Auto-Archive-Benachrichtigung, Cycles/Projekte auto-archivieren, Kapazität, `cycleNumberBase`.
- **Dokumente/Templates:** Dokument-Kaskade bei Issue-/Projekt-/Team-Löschen nur teilweise, Snapshot-Bündelung, Dokument-Mentions, CRDT, Form-Templates, `useDefaultTemplate`.
- **Views/Suche:** Favoriten umsortieren, View-Abos, Eigentümerwechsel, Stemming (Volltext-Konfiguration `simple`).
- **Web:** Filter-Chips aus `filterData` zurückwandeln, Initiativen-Tabs, Roadmap-Timeline, Team-Settings (Templates/SLA/Hierarchie/Webhooks/Access/Recurring), Workspace-Settings (Members/Teams/Security/API/Limits), Account-Settings.
- **Webhooks/Auth:** `client_credentials`, App-User und feingranulare Scopes; Notification bei Webhook-Deaktivierung; SSRF-Bausteine (`UrlPolicy`) für alle ausgehenden HTTP-Pfade; Legacy-SSE-/stdio-MCP.
- **Import:** Linear-Feldnamen nur gegen Mock getestet; Views/Favoriten/Issue-Reaktionen/Update-Kommentare werden nicht importiert; Dry-Run ohne Downloads.
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
| `LINEAL_BACKEND` | `http://localhost:8080` | nur Web (Vite-Proxy/Preview): Adresse des Servers |
| `RUST_LOG` | `info` | Log-Filter |

### 3.2 Prozesse und Ports

- `lineal migrate`, `lineal seed [--demo]`, `lineal serve`, `lineal worker`, `lineal import`, `lineal schema-export`.
- `serve` und `worker` sind getrennte Prozesse und teilen sich die Datenbank; der Worker braucht keinen Port. Der Server muss mit `ConnectInfo` laufen (Client-IP für Rate-Limit); `lineal serve` tut das.
- Entwicklung: PostgreSQL im Container auf `127.0.0.1:5440`, Server `:8080`, Vite-Dev `:5173`. Belegte Ports über `LINEAL_BIND`/`LINEAL_BACKEND` umstellen. E2E (`scripts/e2e.sh`) nutzt die Datenbank `lineal_e2e` und freie Ports (ab 18741 bzw. 4173).
- Hinter einem Reverse-Proxy: `LINEAL_BASE_URL` auf die öffentliche URL, `LINEAL_TRUSTED_PROXIES` auf die Proxy-Adressen, WebSocket-Upgrade für `/sync` durchreichen.

### 3.3 Hintergrundjobs

Event-Konsumenten (Outbox): Notifications, Suchindex, Sync, Webhooks. Fehlversuche je (Konsument, Event) stehen in `outbox_delivery.attempts`/`last_error` und überleben Neustarts; nach `max_event_attempts` (10) gilt das Event als aufgegeben (Dead-Letter: zugestellt mit `last_error`, abfragbar über `OutboxRepo::list_dead_letters`). Periodische Jobs: `cycles` (60 s), `update_reminders` (1 h), `snooze` (60 s), `sync_purge` (1 h, Sync-Log 14 Tage), `notification_cap` (5 min), `due_reminders` (5 min), `email_digest` (60 s), `auto_close`, `auto_archive` (je 5 min), `purge` (5 min: Papierkorb nach Frist, abgelaufene Upload-Reservierungen, zugehörige Blobs), `sla` (60 s), `triage_rotation` (60 s), `import_runner` (10 s), Webhook-Zustellung, `team_purge` (1 h).

## 4. Sicherheitsmaßnahmen

### 4.1 Authentifizierung und CSRF

- Session-Cookie `lineal_session` (`HttpOnly`, `SameSite=Lax`, `Secure` bei https); API-Keys und OAuth-Tokens per `Authorization`. Tokens liegen nur als SHA-256-Hash in der Datenbank.
- CSRF für Cookie-Sessions: `POST /graphql` und `POST /auth/logout` verlangen einen erlaubten `Origin` (ersatzweise `Referer`) **und** den Header `X-Lineal-CSRF: 1`; `/auth/login`, `/oauth/authorize` (der session-gebundene Nonce ist das Token) und signierte Uploads prüfen nur den Origin. Anfragen mit `Authorization`-Header sowie `/mcp` und `/sync` (eigene Origin-Prüfung) sind ausgenommen.
- Einladungen: `POST /invite/accept` (JSON `token`, `email`, `name?`, `password`) nimmt eine Einladung ohne Session an, legt den User an bzw. verknüpft den Platzhalter und setzt das Session-Cookie; ungültige Tokens oder fremde E-Mails antworten einheitlich `404`; CSRF wie beim Login (nur Origin). Die Web-Seite `/invite/<token>` ist noch nicht gebaut.
- OAuth-Zustimmung nur per `POST` mit session-gebundenem Nonce, Zielhost und Warnhinweis; CSP `form-action 'self'`.
- Brute-Force-Schutz: fehlgeschlagene Authentifizierungen zählen je IP (20/min), beim Login zusätzlich je Paar (Account, IP) (10/15 min) und je Account gesamt (100/15 min); Zähler werden vor der Passwortprüfung atomar reserviert.

### 4.2 Sync-Verbindungen

Das Token einer WebSocket-Verbindung wird alle 60 s und nach jeder Zustellung von User-/Team-/Organisations-Änderungen neu geprüft; widerrufene oder abgelaufene Tokens und suspendierte User beenden die Verbindung mit Close-Code 4401. Sichtbarkeit wird bei jeder Zustellung neu berechnet.

### 4.3 Zugriff auf Daten

Jeder Lese- und Schreibpfad (GraphQL, MCP, REST, Jobs) läuft durch dieselbe Policy (`visible_team_ids`, `require_*`, Scope); Namensauflösung, Fehlermeldungen und Aggregate verraten nichts über unsichtbare Objekte. Rich Text wird beim Speichern bereinigt, URLs nur über die Allowlist akzeptiert. API-Keys mit `team_ids` sind auf diese Teams beschränkt (`RequestCtx.team_scope`, Schnitt in `visible_team_ids`; Teams außerhalb verhalten sich wie unsichtbare: `not_found`); eine leere Liste bedeutet „kein Team“, `NULL` keine Beschränkung. Workspace-weite Admin-Aktionen schränkt der Team-Scope nicht ein.

### 4.4 Rate-Limit und Client-IP

- Design: Stufe 1 begrenzt je Client-IP (IPv4 je Adresse, IPv6 je /64) **vor** dem Token-Lookup (Anfragen ohne Zugangsdaten niedrig, mit Zugangsdaten höher), Stufe 2 je User/Token nach dem Lookup (Requests und GraphQL-Komplexität). Bucket-Maps sind begrenzt (200 000 Schlüssel); bei voller Map teilen sich neue Schlüssel einen Überlauf-Bucket mit einem Zehntel des Limits, aktive Buckets werden nie verdrängt.
- `X-Forwarded-For` zählt nur, wenn die Peer-IP in `LINEAL_TRUSTED_PROXIES` liegt: alle Header werden zu einer Liste zusammengeführt, jeder Eintrag muss eine reine IP sein (sonst 400), von rechts werden vertrauenswürdige Proxys abgezogen. `Forwarded` (RFC 7239) wird nicht ausgewertet.
- **Einordnung:** Das In-Process-Rate-Limit ist Verteidigung in der Tiefe. Produktiv gehört ein Edge-Rate-Limit (Reverse-Proxy/WAF) davor. Das Restrisiko „geteilter Überlauf unter Massen-Spoofing“ ist bewusst akzeptiert.

### 4.5 Dateien und Löschen

Objekte liegen hinter HMAC-signierten, kurzlebigen URLs; `BlobStore::delete` entfernt Blobs beim endgültigen Löschen von Issues und Teams sowie bei abgelaufenen, nie finalisierten Upload-Reservierungen (nur wenn kein Anhang mehr auf das Objekt verweist).

## 5. Entscheidungen bei Widersprüchen zwischen Kapiteln

| Stelle | Entscheidung |
|---|---|
| [04](04-planung.md) A10 | Lead-Team-Wechsel vergibt neue Projektnummer, alte Kennung als Alias |
| [03](03-issues-workflow.md) §6.3/§7.2 vs. [07](07-mcp-server.md) | Kommentar-Löschregeln wie 03; Größenlimit 2 GiB exklusiv → `limit_exceeded`; `finalize`-Fehler `invalid_request` mit `reason` |
| [05](05-views-suche-inbox.md) §4.4 | Workspace-Views nur mit `ManageWorkspaceViews`, Gäste nur persönliche Views |
| [14](14-firmenweite-sicht.md) §4 | `includeSubTeams`-Default `true` für Übersicht, `false` für Team-Views |
| [06](06-dokumente-templates.md) §1.4/§2.4 | fünf Fremdschlüssel (Cycle-Dokument mit `teamId`); übersprungene Template-Referenzen als `warnings` |
| [01](01-domaenenmodell.md) §2.6, [07](07-mcp-server.md) §3.14 | `currentUserId` gespeichert, vom Job `triage_rotation` gepflegt |
| [09](09-api-events.md) | Sync-Pfad `GET /sync?since=`, `resetRequired` → Close 4001, kein `/sync/bootstrap` |
| [13](13-import-linear.md) | Scan asynchron über Job, Status `canceled`, Nummern über Import-Zähler |
| [01](01-domaenenmodell.md)/[11](11-implementierungsarchitektur.md) | `lineal-sync` nutzt `sqlx` für `LISTEN/NOTIFY` |
