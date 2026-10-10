# WP17 – Härtung, End-to-End-Tests, Doku-Abgleich (Briefing)

Letztes Arbeitspaket. Lies zuerst `docs/dev/agent-briefing.md` (alle Abschnitte inkl. Sicherheits-Checkliste), `docs/dev/schnittstellen.md` und `docs/dev/offene-punkte.md`. Branch `wp/17-hardening`. Du darfst in diesem Paket **alle Dateien** ändern, aber jede Änderung muss einem der Punkte unten dienen. Arbeite die Prioritäten **in Reihenfolge** ab; was nicht fertig wird, steht im Abschlussbericht.

## P1 – Sicherheit (Pflicht)

1. **CSRF für Cookie-Sessions:** Schreibende Requests (`POST/PUT/PATCH/DELETE` und GraphQL-Mutations), die über das Session-Cookie authentifiziert sind, nur akzeptieren, wenn `Origin` (bzw. ersatzweise `Referer`) zur `LINEAL_BASE_URL` oder im Dev-Modus zu `http://localhost:5173` passt **und** der Header `X-Lineal-CSRF: 1` gesetzt ist (die Web-App setzt ihn in `lib/graphql.ts` und allen `fetch`-Aufrufen auf `/auth/*`). Requests mit `Authorization`-Header (API-Key/OAuth) sind ausgenommen. Gilt für `/graphql`, `/auth/logout`, `/uploads/*`, `/oauth/authorize` (POST) – Login selbst nur mit Origin-Prüfung. Tests: Cross-Origin-Mutation per Cookie → 403; gleiche Origin mit Header → ok; API-Key ohne Header → ok.
2. **Client-IP hinter Proxy:** `LINEAL_TRUSTED_PROXIES` (CIDR-Liste, Default leer). Nur wenn die Peer-IP darin liegt, wird `X-Forwarded-For` (rechtester nicht vertrauenswürdiger Eintrag) für Rate-Limit und Logs genutzt. Test mit gefälschtem Header von nicht vertrauenswürdigem Peer.
3. **WebSocket-Re-Validierung:** `lineal-sync` prüft das Token der Verbindung periodisch (z. B. alle 60 s) und bei jeder Zustellung nach Mitgliedschaftsänderung; widerrufene/abgelaufene Tokens oder suspendierte User → Close 4401. Test.
4. **`BlobStore::delete`** im Port + `LocalBlobStore`; Purge-Jobs (Issues, Teams, Upload-Reservierungen, abgelaufene Import-Downloads) löschen zugehörige Blobs. Test.
5. **`hash_token` vereinheitlichen:** `lineal-api` und `lineal-mcp` nutzen die Funktion aus `lineal-app` (keine eigenen Kopien). Test, dass alle drei denselben Hash erzeugen, entfällt danach.
6. **`api_key.team_ids`:** API-Keys mit Team-Beschränkung dürfen nur diese Teams sehen/ändern (Principal/RequestCtx um optionale Team-Einschränkung erweitern, in `visible_team_ids` schneiden). Test.
7. **Dead-Letter persistent:** Fehlversuche je Konsument/Event in `outbox_delivery` (oder eigener Tabelle) speichern statt im Speicher; nach Neustart gilt das Limit weiter. Test.
8. **Einladung ohne Session:** REST `POST /invite/accept` (Token + Name + Passwort → User anlegen/verknüpfen, Session setzen) gemäß `App::accept_invitation`; CSRF/Origin wie oben. Test.

## P2 – End-to-End-Tests (Pflicht, mindestens die ersten vier)

Playwright unter `web/e2e/` gegen einen echten Server (`lineal migrate`, `lineal seed --demo`, `lineal serve` auf freiem Port, Vite-Build bzw. `vite preview` mit Proxy). Eigene Datenbank `lineal_e2e` im Container (Port 5440), vorher neu anlegen. Skript `scripts/e2e.sh`. Flows aus `docs/08`:
- F3 Issue anlegen (Modal, Taste `c`, „Create more“), Kennung lückenlos.
- F5 Issue durch den Workflow bewegen (Board Drag & Drop oder Status-Picker `S`).
- F12 Filter setzen, als View speichern, als Favorit sehen.
- F13 Inbox: Erwähnung in Kommentar → zweiter User sieht Notification (zwei Browser-Kontexte), markiert gelesen.
- danach: F7 Teamwechsel (`Ctrl+Shift+M`), F17 MCP-Agent (HTTP-Client gegen `/mcp` mit API-Key: `list_issues`, `save_issue`), Sichtbarkeit privates Team (Alice sieht DES nicht).
Playwright-Browser über `npx playwright install chromium` (headless). Nicht in `scripts/check.sh` einhängen (dauert zu lange), sondern eigenes Skript; im Bericht Laufzeit nennen.

## P3 – Lieferkette (Pflicht, klein)

`deny.toml` + `cargo deny check advisories bans licenses` (falls `cargo-deny` nicht installiert ist: `cargo install cargo-deny --locked` in `~/.cargo`, im Bericht nennen) und `npm audit --audit-level=high` als optionale Schritte in `scripts/check.sh` (übersprungen mit Hinweis, wenn das Werkzeug fehlt). Gefundene hohe Schwachstellen beheben oder begründet ausnehmen.

## P4 – Doku-Abgleich (Pflicht)

Die Arbeitspakete haben bei Widersprüchen Entscheidungen getroffen (Liste in den Abschlussberichten, gesammelt in `docs/dev/offene-punkte.md`, u. a.: 04 A10 Lead-Team-Wechsel → neue Projektnummer + Alias; 03 vs. 07 Kommentar-Wurzel löschen / finalize-Fehler / Größenlimit; 05 §4.4 Workspace-Views; 14 §4 `includeSubTeams`-Default; 06 §1.4/§2.4; 01 §2.6 `currentUserId`; 07 §3.14 Rotation; 09 Sync-Pfad `/sync?since=` und `resetRequired`; 13 asynchroner Scan, Status `canceled`, Projektnummern; 01/11 Abweichungen Sync-Adapter kennt sqlx). Passe die Fachkapitel so an, dass sie den **implementierten** Stand beschreiben; neues Kapitel `docs/15-umsetzungsstand.md`: was implementiert ist, bekannte Lücken (aus `offene-punkte.md`, bereinigt), Betrieb (Env-Variablen, Jobs, Ports), Sicherheitsmaßnahmen. README-Tabelle und `tools/build_index.py` um Kapitel 15 ergänzen, `python3 tools/build_index.py` ausführen.

## P5 – Kleinkram (nur wenn Zeit bleibt)

E-Mail-Digest als eigener Job `periodic/email_digest.rs` (statt in `due_reminders`); Bulk-Inbox-Aktionen erzeugen Sync-Events; Issue-Sharing-Event; `issue.children/relations/history` in GraphQL für die Detailansicht. Die Umstellung auf `query!`-Makros entfällt (bewusst gestrichen).

## Fertig

`scripts/check.sh` (bzw. alle Einzelschritte) grün, `scripts/e2e.sh` grün, alles committet. Abschlussbericht mit: erledigten Punkten je Priorität, Testanzahl, offenen Punkten, geänderten Dateien außerhalb der üblichen Bereiche, neuen Abhängigkeiten.
