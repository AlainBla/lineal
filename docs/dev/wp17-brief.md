# WP17 – Härtung, End-to-End-Tests, Doku-Abgleich (Briefing)

Letztes Arbeitspaket. Lies zuerst `docs/dev/agent-briefing.md` (alle Abschnitte inkl. Sicherheits-Checkliste), `docs/dev/schnittstellen.md` und `docs/dev/offene-punkte.md`. Branch `wp/17-hardening`. Du darfst in diesem Paket **alle Dateien** ändern, aber jede Änderung muss einem der Punkte unten dienen. Arbeite die Punkte **in der Reihenfolge des Abschnitts "Reihenfolge"** ab. **Nach jedem nummerierten Punkt committen**, und jeder Commit muss für sich grün sein (fmt, clippy, Tests) – so ist auch ein unvollständiges WP17 Punkt für Punkt mergebar. Was nicht fertig wird, steht im Abschlussbericht.

Werkzeuge: Die Sandbox blockiert `bash scripts/*.sh` – führe die Schritte von `check.sh` und `e2e.sh` einzeln aus.

## Reihenfolge

P1.1 → P1.2 → P1.3 → P1.4 → P2 (die ersten vier Flows) → P4 → P1.6 → P1.7 → P1.8 → P3 → restliche P2-Flows → P5.

## P1 – Sicherheit (Pflicht)

1. **CSRF für Cookie-Sessions:** Neue Konfiguration `LINEAL_ALLOWED_ORIGINS` (Komma-Liste, Default = Origin von `LINEAL_BASE_URL`; im Dev-Modus zusätzlich jede `http://localhost:*` und `http://127.0.0.1:*`). Für über das Session-Cookie authentifizierte Requests:
   - `/graphql` (POST) und `/auth/logout`: `Origin` (ersatzweise `Referer`) muss erlaubt sein **und** Header `X-Lineal-CSRF: 1` gesetzt sein (die Web-App setzt ihn in `lib/graphql.ts` und in den `fetch`-Aufrufen auf `/auth/*`).
   - `/auth/login`, `/oauth/authorize` (POST, HTML-Formular – Browser können dort keine eigenen Header setzen; der session-gebundene HMAC-Nonce ist bereits das CSRF-Token) und signierte `/uploads/*`: **nur** Origin-Prüfung.
   - Requests mit `Authorization`-Header (API-Key/OAuth) sowie `/mcp` und `/sync` (eigene Prüfung) sind ausgenommen.
   Tests: Cross-Origin-Mutation per Cookie → 403; gleiche Origin mit Header → ok; ohne Header → 403; API-Key ohne Header → ok; OAuth-Consent-Formular funktioniert weiter (bestehende Tests in `oauth/tests.rs` grün).
2. **Client-IP hinter Proxy:** `LINEAL_TRUSTED_PROXIES` (CIDR-Liste, Default leer). Nur wenn die Peer-IP darin liegt, wird `X-Forwarded-For` (rechtester nicht vertrauenswürdiger Eintrag) für Rate-Limit und Logs genutzt. Test mit gefälschtem Header von nicht vertrauenswürdigem Peer.
3. **WebSocket-Re-Validierung:** `lineal-sync` prüft das Token der Verbindung periodisch (z. B. alle 60 s) und bei jeder Zustellung nach Mitgliedschaftsänderung; widerrufene/abgelaufene Tokens oder suspendierte User → Close 4401. Test.
4. **`BlobStore::delete`** im Port + `LocalBlobStore`; Purge-Jobs (Issues, Teams, Upload-Reservierungen, abgelaufene Import-Downloads) löschen zugehörige Blobs. Test.
5. *(erledigt durch den Orchestrator: `hash_token` existiert nur noch in `lineal-app`.)*
6. **`api_key.team_ids`:** API-Keys mit Team-Beschränkung dürfen nur diese Teams sehen/ändern. `Principal` und `RequestCtx` um ein Feld `team_scope: Option<Vec<TeamId>>` (Default `None`) erweitern, damit bestehende Konstruktoren (WP02/03/04/16, `FakeStore`) weiter kompilieren; in `visible_team_ids` schneiden. Test.
7. **Dead-Letter persistent:** `outbox_delivery` um `attempts int` und `last_error text` erweitern (Migration im Bereich `1700`–`1799`); der Worker zählt dort statt im Speicher, nach Neustart gilt das Limit weiter. Test.
8. **Einladung ohne Session:** REST `POST /invite/accept` (Token + Name + Passwort → User anlegen/verknüpfen, Session setzen) gemäß `App::accept_invitation`; CSRF/Origin wie oben. Test.

## P2 – End-to-End-Tests (Pflicht, mindestens die ersten vier)

Playwright unter `web/e2e/` gegen einen echten Server (`lineal migrate`, `lineal seed --demo`, `lineal serve` auf freiem Port, Vite-Build bzw. `vite preview` mit Proxy). Eigene Datenbank `lineal_e2e` im Container (Port 5440), vorher neu anlegen. Skript `scripts/e2e.sh`. Flows aus `docs/08`:
- F3 Issue anlegen (Modal, Taste `c`, „Create more“), Kennung lückenlos.
- F5 Issue durch den Workflow bewegen (Board Drag & Drop oder Status-Picker `S`).
- F12 Filter setzen, als View speichern, als Favorit sehen.
- F13 Inbox: Erwähnung in Kommentar → zweiter User sieht Notification (zwei Browser-Kontexte), markiert gelesen.
- danach: F7 Teamwechsel (`Ctrl+Shift+M`), F17 MCP-Agent (HTTP-Client gegen `/mcp` mit API-Key: `list_issues`, `save_issue`), Sichtbarkeit privates Team (Alice sieht DES nicht).
Playwright-Browser über `npx playwright install chromium` (headless). **Auf dieser Maschine braucht Chromium `chromiumSandbox: false`** in `playwright.config.ts` (AppArmor-Userns-Einschränkung, sonst "No usable sandbox"). Der E2E-Server läuft mit `LINEAL_DEV=1` (sonst ist `LINEAL_SESSION_SECRET` Pflicht), auf einem freien Port (8080 und 5432 sind belegt) und mit `LINEAL_BASE_URL`/`LINEAL_ALLOWED_ORIGINS` passend zur Origin von `vite preview` (Standard 4173). Nicht in `scripts/check.sh` einhängen (dauert zu lange), sondern eigenes Skript `scripts/e2e.sh`; im Bericht Laufzeit nennen.

## P3 – Lieferkette (Pflicht, klein)

`deny.toml` + `cargo deny check advisories bans licenses` (falls `cargo-deny` nicht installiert ist: `cargo install cargo-deny --locked` in `~/.cargo`, im Bericht nennen) und `npm audit --audit-level=high` als optionale Schritte in `scripts/check.sh` (übersprungen mit Hinweis, wenn das Werkzeug fehlt). Gefundene hohe Schwachstellen beheben oder begründet ausnehmen.

## P4 – Doku-Abgleich (Pflicht)

Die Arbeitspakete haben bei Widersprüchen Entscheidungen getroffen (Liste in den Abschlussberichten, gesammelt in `docs/dev/offene-punkte.md`, u. a.: 04 A10 Lead-Team-Wechsel → neue Projektnummer + Alias; 03 vs. 07 Kommentar-Wurzel löschen / finalize-Fehler / Größenlimit; 05 §4.4 Workspace-Views; 14 §4 `includeSubTeams`-Default; 06 §1.4/§2.4; 01 §2.6 `currentUserId`; 07 §3.14 Rotation; 09 Sync-Pfad `/sync?since=` und `resetRequired`; 13 asynchroner Scan, Status `canceled`, Projektnummern; 01/11 Abweichungen Sync-Adapter kennt sqlx). Ergänze pro Widerspruch einen kurzen **Umsetzungshinweis** an der betreffenden Stelle des Fachkapitels (z. B. Blockzitat „Umsetzung: …“) – keinen Umbau der Kapitel; neues Kapitel `docs/15-umsetzungsstand.md`: was implementiert ist, bekannte Lücken (aus `offene-punkte.md`, bereinigt), Betrieb (Env-Variablen, Jobs, Ports), Sicherheitsmaßnahmen. README-Tabelle und `tools/build_index.py` um Kapitel 15 ergänzen, `python3 tools/build_index.py` ausführen.

## P5 – Kleinkram (nur wenn Zeit bleibt)

Bulk-Inbox-Aktionen erzeugen Sync-Events; Issue-Sharing-Event; `issue.children/relations/history` in GraphQL für die Detailansicht. Die Umstellung auf `query!`-Makros entfällt (bewusst gestrichen).

## Fertig

Alle Einzelschritte von `scripts/check.sh` grün, die E2E-Flows einzeln grün, alles committet (ein Commit pro Punkt). Abschlussbericht mit: erledigten Punkten je Priorität, Testanzahl, offenen Punkten, geänderten Dateien außerhalb der üblichen Bereiche, neuen Abhängigkeiten.
