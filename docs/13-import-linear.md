# 13 – Import aus Linear (Migration)

Lineal kann einen bestehenden Linear-Workspace vollständig übernehmen, damit Teams ohne Datenverlust umziehen können. Dieses Kapitel spezifiziert Quelle, Mapping, Ablauf, Sicherheit und Wiederholbarkeit des Imports.

## 1. Ziele und Grundsätze

1. **Verlustarm:** Alle Kernobjekte aus dem Scope (README) werden übernommen, inklusive Erstellzeitpunkten, Autoren, Kommentaren und Anhängen.
2. **Kennungen bleiben gleich:** `ENG-123` in Linear ist nach dem Import `ENG-123` in Lineal. Links in Beschreibungen und Kommentaren funktionieren weiter.
3. **Idempotent und fortsetzbar:** Ein abgebrochener Import läuft beim nächsten Start dort weiter, wo er aufgehört hat. Ein erneuter Lauf („Delta-Import“) übernimmt nur Änderungen seit dem letzten Lauf.
4. **Ruhig:** Während des Imports entstehen keine Benachrichtigungen, keine Webhooks, keine Automationen (Auto-Close, SLA, Triage-Regeln).
5. **Nur lesend auf Linear-Seite:** Der Import ruft ausschließlich lesende Linear-API-Operationen auf.
6. **Sicher:** Der Linear-API-Schlüssel wird nie im Klartext gespeichert und nie geloggt.

## 2. Quelle

| Quelle | Umfang | Einsatz |
|---|---|---|
| **Linear GraphQL API** (`https://api.linear.app/graphql`) mit Personal API Key oder OAuth-Token (Scope `read`) | vollständig: alle Entitäten, Beziehungen, Historie, Anhänge | **Standardweg** |
| Linear CSV-Export | nur Issues mit Basisfeldern | Notlösung ohne API-Zugang; wird als eigenes Format unterstützt, mit Warnung über fehlende Daten |

Der Import ist als **Adapter mit eigenem Port** gebaut (`ImportSource`), damit später weitere Quellen (Jira, GitHub Issues) dazukommen können.

```text
trait ImportSource {
  async fn workspace(&self) -> SourceWorkspace
  async fn page(&self, entity: SourceEntity, cursor: Option<Cursor>, updated_after: Option<DateTime>) -> SourcePage
  async fn download(&self, url: Url) -> Bytes
}
```

**Rate-Limits der Quelle:** Linear erlaubt mit API-Key etwa 2 500 Requests pro Stunde und begrenzt die Abfragekomplexität (siehe [09](09-api-events.md)). Der Importer liest die Header `X-RateLimit-Requests-Remaining` und `X-RateLimit-Complexity-Remaining`, drosselt vorausschauend und wartet bei `RATELIMITED` bis zum Reset. Seitengröße 50–100, Felder pro Abfrage minimal, verschachtelte Listen (Labels, Kommentare) in eigenen Abfragen.

## 3. Mapping

Jedes übernommene Objekt wird in der Tabelle `import_mapping` vermerkt:

| Feld | Typ | Bedeutung |
|---|---|---|
| `importRunId` | UUID | Lauf |
| `source` | Enum `linear`, `linearCsv` | Quelle |
| `sourceType` | String | z. B. `Issue`, `Team`, `Comment` |
| `sourceId` | String | ID in der Quelle |
| `targetId` | UUID | ID in Lineal |
| `sourceUpdatedAt` | DateTime | Stand beim Import (für Delta) |
| `checksum` | String | Hash der übernommenen Felder |

Unique: `(organizationId, source, sourceType, sourceId)`.

### 3.1 Entitäten

| Linear | Lineal | Besonderheiten |
|---|---|---|
| Organization | Workspace | Import läuft in einen **bestehenden** Lineal-Workspace; Name/URL-Key werden nicht überschrieben |
| User | User | Zuordnung per E-Mail (case-insensitiv). Unbekannte User → **Platzhalter-User** (`status = imported`, kein Login, keine Lizenz), die bei späterer Einladung mit derselben E-Mail automatisch verknüpft werden. Rollen: Linear-Admin → `admin`, Guest → `guest`, sonst `user`. |
| Team (inkl. `parent`) | Team | `key` identisch; Kollision mit bestehendem Lineal-Team → Abbruch vor dem Start mit Liste (oder Ziel-Key per Mapping-Datei). Sub-Team-Hierarchie, Sichtbarkeit, Zeitzone, Cycle-/Estimate-/Triage-Einstellungen, Auto-Close/Archive übernehmen. |
| TeamMembership | TeamMembership | inkl. `owner` |
| WorkflowState | WorkflowState | gleicher Name, `type`, Farbe, Position. Der Default-Workflow eines neu angelegten Teams wird **ersetzt**, nicht ergänzt. Duplicate-State wird zugeordnet, nicht doppelt angelegt. |
| IssueLabel (Workspace/Team, Gruppen) | Label | Gruppen mit `groupType`; retired Labels bleiben retired |
| Cycle | Cycle | Nummer, Zeitraum, abgeschlossene Cycles mit eingefrorenen Scope-Historien |
| Project, ProjectStatus, ProjectMilestone, ProjectUpdate, ProjectLabel | dto. | Lead-Team, Teams, Members, `P-KEY-n` aus Linear übernommen |
| Initiative (+ Sub-Initiativen), InitiativeUpdate | dto. | DAG-Kanten aus `parentInitiatives` |
| Issue | Issue | **`number` aus Linear übernommen** (siehe 3.2); Felder: Titel, Beschreibung, State, Priority, Estimate, Assignee, Creator, Labels, Cycle, Project, Milestone, Parent, Due Date, SLA-Felder, sortOrder, Zeitstempel (`createdAt`, `startedAt`, `completedAt`, `canceledAt`, `archivedAt`), `previousIdentifiers` |
| IssueRelation | IssueRelation | `blocks`, `related`, `duplicate`, `similar` |
| IssueHistory | IssueHistory | als Einträge mit `source = import`; nur Statuswechsel, Assignee, Priorität, Team, Projekt, Cycle (reicht für Cycle-/Projekt-Graphen) |
| Comment (Threads, Replies, Reaktionen, Inline-Kommentare) | Comment, Reaction | Autor, `createdAt`, `editedAt`; Inline-Anker über `quotedText` |
| Attachment, eingebettete Uploads | Attachment, Blob | Uploads aus `uploads.linear.app` herunterladen und im Lineal-Blob-Store ablegen; URLs in Markdown umschreiben. Externe Links bleiben Links. |
| Document | Document | Parent (Project/Initiative/Team/Issue/Cycle) |
| Template | Template | `templateData` übersetzt; nicht abbildbare Felder werden verworfen und protokolliert |
| CustomView, Favorite | CustomView, Favorite | Filter werden in das Lineal-Filtermodell übersetzt (gleiche Struktur, siehe [05](05-views-suche-inbox.md)); unbekannte Filterfelder → View wird mit Warnung ohne dieses Kriterium angelegt |
| IssueSubscription | IssueSubscription | ja |
| Notifications | – | **nicht** übernommen (flüchtig) |
| Webhooks, API-Keys, OAuth-Apps, Integrationen | – | nicht übernommen (Sicherheit; müssen neu eingerichtet werden) |

### 3.2 Issue-Nummern und Zähler

- Importierte Issues behalten ihre Linear-Nummer. Lücken (in Linear gelöschte Issues) bleiben Lücken. **Ausnahme zur Regel „lückenlos“** aus [00 §2](00-konventionen.md#2-identifikatoren): gilt nur für importierte Issues.
- Nach dem Import: `team.lastIssueNumber = max(number)` des Teams. Neue Issues zählen nahtlos weiter.
- Linear-`previousIdentifiers` (nach früheren Teamwechseln) werden als Aliase übernommen; alte Links bleiben auflösbar.
- Importiert man in ein Team, das in Lineal bereits Issues hat, ist das ein Konflikt (Nummernüberschneidung) → Import für dieses Team nur mit explizitem Ziel-Key eines neuen Teams.

### 3.3 Inhalte umschreiben

Linear-Markdown enthält eigene Elemente. Der Importer übersetzt:

| Linear | Lineal |
|---|---|
| `<issue id="…" href="…">ENG-12</issue>` | `ENG-12` (wird von Lineal verlinkt) |
| Links `https://linear.app/<ws>/issue/ENG-12/...` | `https://<lineal>/<ws>/issue/ENG-12` |
| Links auf Projekte/Dokumente/Views | Lineal-URLs über `import_mapping` |
| `@user`-Erwähnungen | `@displayName` des zugeordneten Lineal-Users |
| `<linear-image>{json}</linear-image>`, Upload-URLs | heruntergeladene Datei, Lineal-Blob-URL |
| Tag-artige Folgen (`<token>`, `<uuid>`, `<script>` …) | als wörtlicher Text erhalten (`\<token>`), im Bericht gelistet; nichts wird gelöscht |

Anschließend läuft die normale Markdown-Bereinigung (Allowlist, [10 §11.4](10-architektur-umsetzung.md#114-sicherheitsvorgaben-für-die-umsetzung)). Sie **löscht kein Markup mehr**: Rohes HTML wird nie ausgeführt, aber jede Tag-artige Folge wird mit `\` maskiert und bleibt als Text sichtbar (Linear-Texte enthalten Platzhalter wie `<token>` als normalen Text). Dasselbe gilt für alle Schreibwege (API, MCP, Editor), weil sie dieselbe Funktion `markdown::sanitize` nutzen; einfacher Text (Titel, Namen, Kurzbeschreibungen) behält Tags ohne Maskierung (`sanitize_plain`) und wird vom Web immer als Text ausgegeben. Code-Spans und Code-Blöcke bleiben unverändert, gefährliche Links (`javascript:`, `data:`) verlieren weiterhin ihr Ziel. Der Bericht meldet je Tag-Name einmal „kept as literal text“ (aktive Elemente wie `script`, `iframe` als Warnung „neutralised“).

## 4. Ablauf

```mermaid
flowchart TD
  START["Admin startet Import"] --> CONNECT["Linear-API-Key eingeben<br/>Verbindung pruefen"]
  CONNECT --> SCAN["Scan: Teams, User, Mengen zaehlen"]
  SCAN --> PLAN["Import-Plan anzeigen<br/>Teams waehlen, User-Zuordnung, Key-Konflikte"]
  PLAN --> DRY{"Probelauf?"}
  DRY -->|ja| DRYRUN["Dry-Run: Mapping und Validierung<br/>ohne Schreiben, Bericht"]
  DRYRUN --> PLAN
  DRY -->|nein| RUN["Import-Lauf in Phasen"]
  RUN --> VERIFY["Verifikation: Zaehlungen und Stichproben"]
  VERIFY --> REPORT["Bericht: uebernommen, uebersprungen, Warnungen"]
  REPORT --> DELTA{"Delta-Import spaeter?"}
  DELTA -->|ja| RUN
  DELTA -->|nein| DONE["Fertig: Zaehler gesetzt, Automationen wieder aktiv"]
```

### 4.1 Phasen (Reihenfolge nach Abhängigkeiten)

| Phase | Inhalt |
|---|---|
| 1 | User, Teams (Eltern vor Kindern), Mitgliedschaften |
| 2 | Workflow-States, Labels (Gruppen vor Labels), Projektstatus, Templates |
| 3 | Cycles, Initiativen (Topologische Sortierung des DAG), Projekte, Meilensteine |
| 4 | Issues ohne Parent-Bezug (nach `number` aufsteigend pro Team), dann Parent-Bezüge und Relationen |
| 5 | Kommentare, Reaktionen, History, Abos |
| 6 | Anhänge und eingebettete Dateien (parallel, begrenzt auf 4 gleichzeitige Downloads) |
| 7 | Dokumente, Status-Updates, Custom Views, Favoriten |
| 8 | Abschluss: Zähler setzen, Suchindex füllen, Fortschritt/Scope neu berechnen, Automationen freigeben |

Jede Phase verarbeitet Seiten von 50–100 Objekten pro Transaktion und speichert den Cursor in `import_run.checkpoint`. Bei Neustart wird ab dem Checkpoint fortgesetzt; bereits gemappte Objekte werden über `import_mapping` erkannt (Upsert statt Doppel-Anlage).

### 4.2 Delta-Import

Läuft mit `updatedAfter = letzter erfolgreicher Laufbeginn`. Geänderte Objekte werden aktualisiert, wenn sich die `checksum` unterscheidet und das Objekt **in Lineal seit dem Import nicht geändert** wurde (`updatedAt <= importedAt`). Sonst Konflikt → Lineal-Stand bleibt, Eintrag im Bericht. In Linear gelöschte oder archivierte Objekte werden im **Abgleich** (§12) archiviert, nie hart gelöscht; ein manueller Delta-Lauf meldet sie nur. Typischer Einsatz: Testimport → paralleler Betrieb → finaler Delta-Import am Umstellungstag.

### 4.3 Stille während des Imports

- Domain-Events des Imports tragen `origin = import`. Die Konsumenten `notifications` und `webhooks` überspringen sie; `search_index` und `sync` verarbeiten sie.
- Periodische Jobs (Auto-Close, Auto-Archive, SLA, Cycle-Wechsel) überspringen Teams mit laufendem Import (`team.importLockedUntil`).
- Importierte Issues landen **nicht** in Triage, egal wer sie erstellt hat; der State kommt aus Linear.

## 5. Datenmodell-Ergänzungen

| Entität | Felder |
|---|---|
| `ImportRun` | `id`, `organizationId`, `source`, `status` (`scanning`, `planned`, `running`, `paused`, `failed`, `completed`), `options` (JSON: Teams, User-Zuordnung, Key-Mapping, dryRun), `checkpoint` (JSON: Phase, Entität, Cursor), `stats` (JSON: Zähler pro Entität), `startedBy`, `startedAt`, `finishedAt`, `error` |
| `ImportMapping` | siehe Abschnitt 3 |
| `ImportIssue` (Bericht) | `importRunId`, `severity` (`info`, `warning`, `error`), `sourceType`, `sourceId`, `message` |
| `ImportCredential` | `importRunId`, `encryptedToken` (AES-GCM mit Server-Schlüssel), `expiresAt` – wird nach Abschluss gelöscht |
| `User.status` | zusätzlicher Wert `imported` (Platzhalter) |
| `Team.importLockedUntil` | DateTime? |
| `IssueHistory.source` | `user`, `system`, `import` |

## 6. Schnittstellen

**GraphQL (nur Admins):**

| Operation | Zweck |
|---|---|
| `importScan(input: { source, token })` → `ImportRun` mit Mengen und Konflikten | Verbindung prüfen, Plan vorbereiten |
| `importStart(input: { runId, teams, userMapping, keyMapping, dryRun })` | Lauf starten (asynchron als Job) **Umsetzung:** `importScan` legt den Lauf mit Status `scanning` an und kehrt sofort zurück; der Job `import_runner` (10 s) führt den Scan asynchron aus. Zusätzlicher Status `canceled` (Abbruch durch Admin, Migration 1800). Projekt- und Initiativnummern vergibt der Import über die Team-/Workspace-Zähler (`next_import_number`) statt die Linear-Nummern zu übernehmen; die Zuordnung steht in `import_mapping`. |
| `importPause(runId)`, `importResume(runId)`, `importCancel(runId)` | Steuerung |
| `importRun(id)`, `importRuns` | Status, Fortschritt, Bericht |

**CLI** (für große Workspaces, ohne Browser-Timeouts):

```bash
lineal import linear --org acme --token-env LINEAR_API_KEY --teams ENG,DES --dry-run
lineal import linear --org acme --token-env LINEAR_API_KEY --resume <runId>
lineal import linear-csv --org acme --file export.csv --team ENG
```

**Betrieb (Paket 7):** Der Runner führt Importe in einer eigenen Task aus (der Worker-Takt mit Outbox-Konsumenten und periodischen Jobs bleibt frei, je Prozess höchstens ein Lauf). Ein Heartbeat verlängert Lease und Team-Sperren auch während langer Rate-Limit-Wartezeiten; Pause/Abbruch werden währenddessen alle 2 s geprüft. Die Team-Nummernzähler werden nach jeder Issue-Seite nachgeführt, `importCancel` repariert sie; von der CLI gestartete Läufe ohne Lebenszeichen (30 min) markiert der Runner als `failed` (fortsetzbar). Ein zweiter echter Lauf für ein überlappendes Team wird abgelehnt, solange ein anderer `running` ist. Parent-Zyklen aus der Quelle werden verworfen (Bericht-Warnung), ebenso meldet der CSV-Import jede stille Datenänderung als Warnung.

**Konfiguration:** `LINEAL_IMPORT_LINEAR_ENDPOINT` überschreibt den Linear-GraphQL-Endpunkt (z. B. lokaler Mock für Tests). Nur mit `LINEAL_DEV=1` oder `LINEAL_IMPORT_ALLOW_CUSTOM_ENDPOINT=1` erlaubt; `http(s)` ohne Zugangsdaten in der URL, wird protokolliert. Der API-Key geht an diesen Endpunkt; Downloads bleiben auf `uploads.linear.app` beschränkt. Aufbewahrung (Job `purge`): erledigte Jobs 1 Tag, fehlgeschlagene 14 Tage, verarbeitete Outbox-Events 7 Tage (mit Dead-Letter 30 Tage).

**MCP:** kein Tool – der Import ist eine Admin-Aktion und bewusst nicht für Agenten freigegeben.

## 7. User Flow

```mermaid
sequenceDiagram
  actor A as "Admin Alice"
  participant UI as "Lineal Settings Import"
  participant API as "Lineal API"
  participant W as "Import-Worker"
  participant L as "Linear API"
  A->>UI: Linear-API-Key eingeben
  UI->>API: importScan
  API->>L: viewer, teams, users, Mengen
  L-->>API: Daten
  API-->>UI: Plan mit Konflikten
  A->>UI: Teams und User-Zuordnung bestaetigen
  UI->>API: importStart
  API->>W: Job einreihen
  loop Phasen 1 bis 8
    W->>L: Seite lesen mit Rate-Limit
    W->>W: mappen, Markdown umschreiben
    W->>API: in Transaktion schreiben, Checkpoint
  end
  W-->>UI: Fortschritt ueber Sync
  W-->>A: Bericht in der Inbox
```

## 8. Sicherheit

- Token nur verschlüsselt (`ImportCredential`), nie in Logs, Fehlermeldungen oder Berichten; Löschung nach Abschluss oder nach 7 Tagen.
- Nur Workspace-Admins/Owner dürfen importieren.
- Private Linear-Teams werden nur importiert, wenn der Token sie sehen kann; ihre Sichtbarkeit wird übernommen (nie offener).
- Heruntergeladene Dateien: Größenlimit, Content-Type-Prüfung, kein Ausführen; gespeichert wie normale Uploads.
- Eingehende Inhalte gelten als nicht vertrauenswürdig: Markdown-Bereinigung, Längenlimits, Ablehnung ungültiger Enums mit Bericht statt Abbruch.

## 9. Abnahmekriterien

1. Ein Testworkspace (2 Teams, 1 Sub-Team, 1 privates Team, 200 Issues mit Sub-Issues, Relationen, Kommentaren, Anhängen, 3 Projekte, 2 Initiativen, 2 Cycles) wird vollständig übernommen; Zählungen pro Entität stimmen.
2. `ENG-123` aus Linear ist in Lineal unter `ENG-123` erreichbar; ein neues Issue erhält `max + 1`.
3. Abbruch nach Phase 4 und Wiederaufnahme ergibt keine Duplikate.
4. Während des Imports entsteht keine einzige Notification und kein Webhook-Aufruf.
5. Ein zweiter Lauf ohne Änderungen in Linear ändert nichts (alle Checksummen gleich).
6. Der Token erscheint in keiner Log-Zeile (Test durchsucht Logs).

Für automatisierte Tests gibt es eine **Fake-Quelle** (`ImportSource` aus JSON-Fixtures mit erfundenen Daten), damit Tests nie gegen das echte Linear laufen.

## 10. Umsetzungshinweise Teilimport (WP28)

### 10.1 Abgleich mit dem Linear-Schema

Alle Abfragen des Adapters werden im Test `crates/lineal-import/tests/linear_schema.rs` gegen das öffentliche Schema (`tests/data/linear-schema.graphql`, aus `github.com/linear/linear`, MIT-Lizenz, Lizenztext daneben) validiert: Feldnamen, Argumente, Typen, Verbindungsform, Filter. Dieselbe Prüfung schätzt die Komplexität (Feld 0,1 Punkte, Objekt 1 Punkt, Verbindung × `first`) und verlangt höchstens 5 000 Punkte je Abfrage (Linear lehnt ab 10 000 ab). Folgen: kleinere Seiten je Entität (`max_first`: Issues 25, Projekte 15, History 10, Abos 20, sonst 100) und niedrigere `first`-Werte in verschachtelten Verbindungen (History 30 je Issue). Gefundene Abweichungen: `Team.autoCloseState` gibt es nicht (nur `autoCloseStateId`), `Team.private` ist veraltet (Ersatz `visibility`: `public`/`private`/`restricted`, `restricted` wird vorsichtig als privat übernommen), `IssueLabel.groupType` wurde gelesen, aber nicht abgefragt. Neu abgefragt: `Issue.url/identifier/delegate`, Relationen mit `identifier url`. Ein Test der Antwortform gegen die echte API bleibt dem ersten echten Lauf vorbehalten.

### 10.2 Teilimport

`importScan` liefert je Team `projects: [{id, name, status, issueCount, teamIds}]` (ein Projekt über mehrere Teams erscheint bei jedem). `importStart` nimmt zusätzlich `projects: [linearProjectId]` und `targetTeams: {linearTeamKey: linealTeamId}`; die CLI `--projects "Name,ID"` und `--target-team KOL=KP`.

- **Projektfilter:** übernommen werden nur Issues der gewählten Projekte, deren Team importiert wird. **Sub-Issues anderer Projekte (oder ohne Projekt) bleiben draußen**; ein Sub-Issue, dessen Elternteil fehlt, verliert den Eltern-Bezug (Bericht). Projekte, Meilensteine, Status-Updates, Dokumente folgen über das Mapping; Cycles werden vollständig übernommen. Personen und (bei Zielteam) Labels werden erst beim ersten Bezug angelegt bzw. zugeordnet. Initiativen werden nicht über den Projektfilter gezogen (Verknüpfungen entfallen, wenn die Initiative nicht importiert wurde). Ohne Filter (alle Projekte angehakt) kommen auch Issues ohne Projekt mit.
- **Relationen nach außen:** ist nur ein Ende importiert, entsteht stattdessen ein Anhang `Linear: blocks KOL-33` mit Link in den Quell-Workspace (und eine Warnung im Bericht).
- **Abfragen** werden auf die gewählten Teams/Projekte eingeschränkt (`team.id in`, `project.id in`, Kommentare über `issue.team`); Anhänge, Relationen und Dokumente lassen sich nicht so einschränken und werden gelesen und über das Mapping gefiltert.

### 10.3 Ziel = bestehendes Lineal-Team

Das Linear-Team wird nur auf das Lineal-Team gemappt; dessen Einstellungen, Standard-/Triage-Status und Mitglieder bleiben unverändert. Status werden per (Typ, Name ohne Beachtung der Schreibweise) zugeordnet, fehlende angelegt (Name, Typ, Farbe, Position übernommen; `Duplicate` wird zum Typ `duplicate`). Labels: Team-Label → Label des Zielteams, sonst Workspace-Label gleichen Namens, sonst neu als Team-Label; Linear-**Workspace**-Labels → gleichnamiges Workspace-Label, sonst neu als Workspace-Label (zurückgezogene bleiben zurückgezogen). Issue-Nummern und Cycle-Nummern werden aus den Zählern des Zielteams neu vergeben; die Linear-Kennung bleibt als Alias in `issue_identifier_alias` auflösbar (nicht, wenn Linear-Key und Key des Zielteams übereinstimmen) und immer als Anhang `Linear: KOL-12` mit Link. Delta-Läufe ändern weder Nummern noch zugeordnete Status/Labels.

Rechte: Beim Start prüft `importStart` Sichtbarkeit (sonst `not_found`) sowie ViewTeam, CreateIssue und UpdateTeamSettings auf dem Zielteam; **bei jeder Ausführung** (Start, Fortsetzen, Delta, Probelauf) prüft `import_verify_targets` erneut: Team im Workspace, nicht stillgelegt/gelöscht, eigene Status, Starter (`options.startedBy`) aktiv, mit Import-Recht und obigen Rechten; sonst bricht der Lauf vor jedem Schreiben ab. Importierte Personen werden nur auf aktive Benutzer gematcht; Platzhalter sind nie Admin.

### 10.4 Dry-Run-Bericht

`stats.plan` enthält `teamsMapped`, `statesMapped/statesNew`, `labelsMapped/labelsNew`, `usersMatched/usersPlaceholder`, `aliasesCreated`, `relationsOutside`; dazu die Zähler je Entität und die Warnungen im Bericht. Die Web-Seite und `lineal import --dry-run` zeigen ihn an.

## 11. Umsetzungshinweise Import-Rest (WP34)

- **Custom Views:** Der Linear-Filter (`filterData`, `projectFilterData`, `initiativeFilterData`) wird in das Lineal-Filtermodell übersetzt: Quell-IDs (User, Team, Status, Label, Projekt, Cycle, Initiative, Meilenstein) werden auf Lineal-IDs umgeschrieben, unbekannte Referenzen mit Warnung weggelassen. Danach entfernt der Übersetzer Stelle für Stelle, was der Lineal-Filterparser ablehnt (JSON-Pointer aus dem Parser-Fehler), jeweils mit Warnung im Bericht (der Filter wird dadurch breiter), und normalisiert wie beim normalen Anlegen. Unterstützt sind Issue-, Projekt- und Initiative-Views; andere `modelName` (Customer, Cycle, Document) werden mit Hinweis übersprungen. Eine nicht geteilte View mit unbekanntem Eigentümer wird übersprungen, eine geteilte fällt auf den Starter zurück; Views eines nicht importierten Teams entfallen, im Teilimport auch Workspace-Views.
- **Favoriten:** Ordner zuerst, dann Issue, Projekt, Cycle, View, Dokument, Initiative, Label, Projektlabel, Team, User und vordefinierte Ansichten. Unbekannte Ziele/Typen werden mit Hinweis übersprungen; fehlt der Ordner, gilt „ohne Ordner“ mit Warnung (nur eine Ebene).
- **Issue-Reaktionen** werden wie Kommentar-Reaktionen importiert (unbekannte Nutzer entfallen, Dubletten verhindert der Unique-Index). **Kommentare an Projekt-Updates** (inkl. Antworten und Reaktionen) kommen im neuen Schritt `UpdateComments` nach `InitiativeUpdates`; Kommentare an Initiative-Updates bleiben mit Hinweis übersprungen.
- **Dry-Run:** zählt Downloads, ohne sie zu laden: `ImportSource::probe()` (Linear-Adapter per HEAD mit denselben URL-Prüfungen wie `fetch`, Fake über die Dateilänge). `stats.plan` enthält `downloadFiles`, `downloadBytes`, `downloadSizeUnknown` (kein `Content-Length`), `downloadRejected`; gezählt werden Anhänge und eingebettete Uploads in Issue-Beschreibungen und Kommentaren, je URL einmal.
- Alle Abfragen bleiben im Schema-Test (`linear_schema.rs`, ≤ 5 000 Punkte). Die Antwortformen des echten API sind weiterhin nicht geprüft.

## 12. Regelmäßiger Abgleich (Sync)

Ein Owner/Admin richtet in `/{ws}/settings/import` (Abschnitt „Sync“) für einen **abgeschlossenen** Linear-Import einen Abgleich ein: Intervall (aus / stündlich / alle 6 h / täglich zu einer Uhrzeit in UTC), Teams (und Projekte) wie beim Import, Linear-API-Schlüssel. „Sync now“, Pausieren/Fortsetzen und Löschen stehen daneben; Status, nächster Lauf und letztes Ergebnis erscheinen im Abschnitt, der Bericht jedes Laufs im Verlauf der Import-Seite (Läufe tragen `options.syncId`).

**Schlüssel.** AES-GCM wie die Import-Tokens (`encrypt_token`, Schlüssel per HKDF aus `LINEAL_SESSION_SECRET`), aber in der eigenen Tabelle `import_sync_credential` (gebunden an die Sync-ID, **keine** 7-Tage-Löschung) und je Lauf neu für die Lauf-ID verschlüsselt. Er erscheint nie in API-Antworten (`hasKey`), Berichten, Audit-Einträgen oder Logs; `GraphQL secret`. Löschen des Syncs löscht den Schlüssel.

**Ablauf.** Der Job `import_runner` (10 s) legt für fällige Einrichtungen einen Delta-Lauf an (`delta`, `updatedAfter` = das spätere von letztem erfolgreichen Sync-Lauf und letztem abgeschlossenen Import, minus 5 min Überlappung) und führt ihn wie jeden Lauf aus. Start nur, wenn kein Lauf der Einrichtung aktiv ist und **kein anderer Lauf für ein überlappendes Team läuft** (Sperre B7.08, `ensure_no_overlapping_run`; sonst wird der Termin übersprungen). Der Lauf gehört dem Einrichtenden: ist er nicht mehr Admin/Owner, startet nichts und er wird benachrichtigt.

**Konfliktregel (Linear ist die Quelle).** Ein Objekt wird nur überschrieben, wenn sich sein Stand in Linear geändert hat (`checksum`) **und** es in Lineal seit dem letzten Abgleich nicht lokal geändert wurde (`updated_at <= imported_at` der Zeile im Mapping). Sonst bleibt der Lineal-Stand, der Bericht nennt den Konflikt („changed in Lineal since the import“). Die Regel gilt je **Objekt** (Issue, Kommentar, Dokument …), nicht je Feld: eine lokale Änderung irgendeines Feldes schützt das ganze Objekt (konservativer als feldweise; entschieden, weil die Importzeile keine Feld-Checksummen führt). Neue Objekte in Linear werden angelegt; Zuordnung läuft über `import_mapping`.

**Gelöschtes/Archiviertes.** In Linear archivierte Objekte kommen mit `archivedAt` und werden entsprechend aktualisiert. In Linear **hart gelöschte** (oder aus dem Umfang verschobene) Objekte liefert die API bei `updatedAfter` nicht mehr; deshalb liest der Schritt `Removed` bei manuellen Läufen und mindestens alle 24 h die IDs von Issues, Projekten, Initiativen und Dokumenten im **ganzen** Quell-Workspace (ohne Team-, Projekt- oder Delta-Filter) und **archiviert** (`archived_at`, nie `DELETE`) Objekte mit Mapping dieser Quelle (im eigenen Workspace), die Linear überhaupt nicht mehr auflistet – nie wegen eines Filters oder einer Verschiebung in ein anderes Projekt/Team –, sofern sie lokal unverändert sind (sonst Konflikt im Bericht). Issues werden nur für Teams archiviert, die zur Konfiguration des Laufs gehören; wer ein Team oder Projekt aus dem Abgleich nimmt, archiviert damit nichts, der Sync stoppt nur für sie. Alle Listen werden zuerst vollständig gelesen; bei Fehler, Teilfehler der Quelle oder Seitenlimit wird in diesem Lauf nichts archiviert. Die Rechte des Einrichtenden (aktiv, Owner/Admin) werden beim Anlegen **und** bei jeder Ausführung des Laufs erneut geprüft. (Dies löst den früheren Satz „nur gemeldet“ ab: entschieden, weil ein Abgleich sonst Geisterobjekte sammelt.)

**Inhalte neu laden.** Option für einen Lauf: liest alle Objekte (ohne `updatedAfter`) und schreibt Beschreibungen, Kommentare, Dokumente und Updates **erneut**, deren Quelltext Tag-artige Folgen enthält (`<token>`; nur dort kann der frühere Importer etwas gelöscht haben), auch bei gleicher Checksum – aber nur, wenn das Objekt lokal unverändert ist (gleiche Konfliktregel). Es werden ausschließlich Zeilen über das Mapping dieser Quelle im eigenen Workspace aktualisiert; die Zeilenänderung ist auf `organization_id` des Laufs begrenzt, ein manipuliertes Mapping auf ein fremdes Objekt läuft ins Leere (Fehlereintrag).

**Label-Aufräumen.** Option „Merge duplicate labels“ (oder GraphQL `importMergeLinearLabels`): Importierte Labels `Name (Linear)` werden mit dem vorhandenen Merge (`merge_issue_label`) in das gleichnamige Label derselben Ebene (Team/Workspace, Gruppe) überführt, das Mapping zeigt danach auf das Ziel. Nie automatisch.

**Fehler und Audit.** Ein fehlgeschlagener Lauf setzt `last_status = failed` und benachrichtigt den Einrichtenden (Notification `importSyncFailed`, Kategorie `system`). Audit-Einträge: `import.sync.created/updated/paused/resumed/deleted`, `import.sync.run_started`, `import.labels_merged`, `import.placeholder_merged` (ohne Schlüssel).

**API.** GraphQL (nur Owner/Admin, Scope `admin`): `importSync`, `importSyncSave`, `importSyncPause`, `importSyncRun(reloadContent, mergeLinearLabels)`, `importSyncDelete`, `importMergeLinearLabels`.

## 13. Korrekturen nach dem ersten echten Lauf

1. **Platzhalter-Text:** siehe §3.3 (nichts wird mehr gelöscht).
2. **Dateien:** Der Worker schreibt direkt über den Port `BlobStore::put_bytes` (kein HTTP-`PUT` an `LINEAL_BASE_URL`, im Worker-Container oft nicht erreichbar). Der `LocalBlobStore` prüft Größenlimit (50 MB) und Magic-Bytes wie beim Upload. Downloads kommen weiter nur von `uploads.linear.app` (SSRF-Regeln der Quelle). Voraussetzung im Docker-Stack: das Volume `blobs` ist in API und Worker eingehängt (ist es). Sichtbarkeit einer Datei = Sichtbarkeit des Besitzer-Issues (`resolveAssets` prüft das Issue).
3. **Labels:** Ein gleichnamiges Label (ohne Beachtung der Schreibweise) auf derselben Ebene (Team/Workspace, gleiche Gruppe) wird zugeordnet statt dupliziert (`import_mapping`); nur wenn die Ebene nicht passt, bleibt der Ausweichname `(Linear)`.
4. **Dokumente ohne eindeutiges Elternobjekt:** nie an den Workspace oder ein fremdes Team. Ein Dokument ohne Projekt/Issue/Initiative/Cycle in Linear, aber mit eigenem importiertem Team, wird ein Team-Dokument dieses Teams (gleiche Sichtbarkeit wie in Linear); hing es in Linear an einem nicht importierten Objekt oder ist sein Team nicht importiert, wird es mit Warnung übersprungen.
5. **Elternkommentare:** Ursache war die Reihenfolge (Linear liefert Antworten vor dem Elternkommentar, auch über Seitengrenzen). Antworten werden vorgemerkt und nach dem letzten Kommentar nachverknüpft; nur wirklich fehlende Eltern (gelöscht, anderes Team/Projekt) bleiben Kommentare oberster Ebene, der Bericht nennt die Ursache.

## 14. Platzhalter einem bestehenden User zuordnen

Hat ein Mensch in Linear eine andere E-Mail-Adresse als in Lineal, entsteht ein Platzhalter (`status = imported`). Owner/Admin ordnen ihn in `/{ws}/settings/members` (Filter „Imported placeholders“, „Merge with…“) oder per GraphQL `importPlaceholderMerge(placeholderId, targetUserId)` einem aktiven User desselben Workspace zu (nie aktiv → aktiv, nie Platzhalter → Platzhalter). In **einer** Transaktion: alle Fremdschlüssel auf den User (aus dem Katalog ermittelt: Zuweisungen, Ersteller, Abos, Kommentar- und Reaktionsautoren, Notifications, History, Projekt-Lead/-Mitglieder, Initiativ-Owner, Team-Mitgliedschaften als Vereinigung mit ODER auf `owner`, …) wandern zum Ziel; gleichwertige Zeilen beim Ziel (Abo, Mitgliedschaft, Reaktion) entfallen; `@handle` in Beschreibungen, Kommentaren, Dokumenten und Updates wird zu `@handle-des-Ziels`; `import_mapping` (User) zeigt auf das Ziel, damit spätere Delta-/Sync-Läufe es nutzen; der Platzhalter wird gelöscht (ein übersehener Verweis lässt das Löschen und damit alles scheitern). Audit `import.placeholder_merged`, keine Benachrichtigungen. Beim Start eines Imports lässt sich im Plan („People“) je Person ohne E-Mail-Treffer ein vorhandener User wählen (`userMapping`).
