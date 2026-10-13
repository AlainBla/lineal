# Testlauf-Briefing (Testfenster, 3 Wellen à 1 Stunde)

Du bist Test-Agent für Lineal (Linear-Nachbau). Ziel: **Bugs finden und sauber dokumentieren –
nichts reparieren.** Fachliche Soll-Vorgaben: docs/01–09, 13, 14 (Linear-Verhalten), Umsetzungsstand
docs/15, bekannte Lücken docs/15 §2 und docs/dev/offene-punkte.md (bekannte Lücken NICHT als Bug
melden, außer das Verhalten ist schlimmer als dort beschrieben).

## Umgebung

- **Test-Stack:** http://localhost:8097 (nginx → Server, Worker läuft). Demo-Workspace `acme`,
  User (Passwort jeweils `lineal-dev`):
  - `alice@acme.example`: Owner, Mitglied ENG/WEB, **nicht** im privaten Team DES.
  - `bob@acme.example`: Mitglied ENG und DES.
  - `carol@acme.example`: Gast, nur WEB.
- **Anmeldung:** Login per `POST /auth/login` (JSON email/password, Header `Origin: http://localhost:8097`).
- **GraphQL:** `POST /graphql` mit Cookie, Header `Origin` und `X-Lineal-CSRF: 1`.
- **MCP:** `POST /mcp` mit API-Key. Den Key legst du per GraphQL `apiKeyCreate` aus einer Session an.
- **Sync / Collab:** Sync per WebSocket `/sync`, gemeinsames Bearbeiten per WebSocket `/collab/{document|issue|project|initiative}/{id}`.
- **Browser-Tests:**
  - Mit Playwright aus `web/node_modules`. Chromium ist installiert und läuft headless mit `chromiumSandbox: false`.
  - Skripte schreibst du NUR unter `var/testrun/scripts/<deine-kennung>/`.
  - Ausführen z. B. per `node` mit `import { chromium } from "<repo>/web/node_modules/playwright/index.mjs"` oder per `npx playwright test --config` mit eigener Config dort.
- **Direkter DB-Zugriff:** nur auf den Test-Stack, nur zum Nachsehen oder Vorbereiten:
  `docker exec -i lineal-test-app-db-1 psql -U lineal -d lineal`
- **Server-Logs:** `docker logs lineal-test-server-1 --since 10m`, Worker: `lineal-test-worker-1`.

## Verbote

- Den Betriebs-Stack (Port **8088**, Container `lineal-server-1`, `lineal-app-db-1` usw.) und die Entwicklungs-DB (Port 5440) **nie** anfassen.
- Keine Container stoppen oder neu starten. Kein `docker compose down`.
- Keine Dateien im Repo ändern außer unter `var/testrun/`. Nichts committen.
- Niemals `mcp__claude_ai_Linear__*` oder Playwright-MCP gegen linear.app. Keine echten Personendaten (nur acme-Demo).
- Keine Last- oder DoS-Angriffe, die den Stack lahmlegen (kurze Grenzwert-Proben sind ok). Stürzt der Stack ab, sofort melden und aufhören, ihn weiter zu belasten.
- Das Passwort von Alice/Bob/Carol **nicht** ändern und diese User **nicht** sperren. Für solche Tests eigene User per Einladung anlegen.

## Bug-Bericht

Datei: `var/testrun/<welle>-<bereich>.md` (deine Kennung steht im Auftrag). Je Fund ein Abschnitt:

```
### <Kennung>-<laufende Nr>: <kurzer Titel>
- Schwere: kritisch | hoch | mittel | niedrig   (Sicherheit/Datenverlust = kritisch/hoch)
- Bereich: <z. B. Issues/Board>
- Schritte: 1. … 2. …
- Erwartet: … (mit Verweis auf docs/NN §x, falls möglich)
- Tatsächlich: …
- Belege: HTTP-Status/Antwort (gekürzt), Log-Zeile, Screenshot-Pfad unter var/testrun/
- Reproduzierbar: ja/nein/selten
```

Schreibe Funde **sofort** in die Datei, nicht erst am Ende. Am Ende deines Zeitfensters schreibst du einen Abschnitt „Zusammenfassung“: Anzahl Funde nach Schwere, was getestet wurde, was nicht.

## Zeitfenster

Arbeite bis zur im Auftrag genannten Uhrzeit (prüfe mit `date`). Teste breit und tief: Normalfälle, Grenzfälle, Fehlbedienung, Rechte (Gast, Nicht-Mitglied, Admin), Nebenläufigkeit (zwei Browser-Kontexte), Tastaturbedienung und Live-Aktualisierung. Hör nicht früher auf: Ist dein Bereich erschöpft, nimm angrenzende Bereiche dazu. Ein Testlauf ohne Funde ist verdächtig.

## Nachtrag ab Welle 2

- **Logins sparsam:** Alle Agenten teilen sich eine Client-IP; das nginx-Limit für `/auth/login` (10/min) gilt für alle zusammen. Melde dich einmal je User an und verwende das Cookie (bzw. den Playwright-`storageState`) weiter.
- **Vorhandene Funde:** Lies vor dem Start alle Berichte unter `var/testrun/` und melde nichts doppelt. Ergänzungen zu einem vorhandenen Fund (neue Variante, schwerere Folge) als eigener Fund mit Verweis auf die bestehende Kennung.
- **Fremde Testdaten:** Testdaten anderer Agenten (Präfixe W1A…W1F) nicht verändern.
- **Zeitfenster:** Das Zeitfenster ist verbindlich. Wer vorher fertig ist, wechselt in einen angrenzenden Bereich, bis die Endzeit erreicht ist.
