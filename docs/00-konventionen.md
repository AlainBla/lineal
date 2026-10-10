# 00 – Konventionen (verbindlich für alle Dokumente)

Dieses Dokument legt die gemeinsamen Regeln fest, auf die sich alle anderen Kapitel beziehen. Es ist bewusst sprachneutral: Typen werden als abstrakte Typen (`String`, `Int`, `Float`, `Boolean`, `DateTime`, `Date`, `UUID`, `JSON`, `Enum`) oder als JSON Schema angegeben, nie als Typen einer konkreten Programmiersprache.

Arbeitstitel des Nachbaus in allen Beispielen: **„Lineal“**. Beispiel-Workspace `acme`, Teams `ENG` (Engineering), `WEB` (Web, Sub-Team von `ENG`), `DES` (Design), Personen `Alice`, `Bob`, `Carol`. Alle Beispieldaten sind erfunden.

## 1. Sprache und Begriffe

- Fließtext Deutsch, Fachbegriffe, Feldnamen, Enum-Werte und Tool-Namen Englisch und exakt wie hier angegeben.
- Feldnamen in `camelCase`, Enum-Werte exakt wie im Original-Schema (`started`, `onTrack`, `singleSelect`), Entitätsnamen in `PascalCase`.

### Glossar DE ↔ EN

| Deutsch | Englisch (Code/UI) | Kurzbedeutung |
|---|---|---|
| Arbeitsbereich | Workspace (Schema: `Organization`) | Mandant, oberster Container |
| Team | Team | Besitzt Issues, Workflow, Cycles; hat Key `ENG` |
| Sub-Team | Sub-team (`Team.parent`) | Team mit Parent-Team, erbt Teile der Konfiguration |
| Mitgliedschaft | TeamMembership | User ↔ Team, optional `owner` |
| Aufgabe / Ticket | Issue | Kernobjekt, gehört genau einem Team |
| Kennung | Identifier | `ENG-123` = Team-Key + Team-Zähler |
| Status | WorkflowState | Team-eigener Status mit Kategorie (`type`) |
| Statuskategorie | State type | `triage`, `backlog`, `unstarted`, `started`, `completed`, `canceled`, `duplicate` |
| Eingangskorb | Triage | Prüfbereich für eingehende Issues eines Teams |
| Iteration / Sprint | Cycle | Zeitbox pro Team |
| Projekt | Project | Zeitlich begrenztes Vorhaben, mehrere Teams möglich |
| Meilenstein | ProjectMilestone | Etappe innerhalb eines Projekts |
| Initiative | Initiative | Bündelt Projekte, hierarchisch |
| Statusbericht | ProjectUpdate / InitiativeUpdate | Bericht mit `health` |
| Ansicht | CustomView | Gespeicherter Filter + Anzeigeoptionen |
| Posteingang | Inbox / Notification | Benachrichtigungen pro User |
| Vorlage | Template | Vorbelegung für Issue/Projekt/Dokument |
| Dokument | Document | Markdown-Dokument an Projekt/Issue/Initiative/Cycle/Team |
| Verknüpfung | IssueRelation | `blocks`, `related`, `duplicate`, `similar` |
| Archivieren | archive (`archivedAt`) | Ausblenden, wiederherstellbar |
| Papierkorb | trash (`trashed`) | Gelöscht, 30 Tage wiederherstellbar |

## 2. Identifikatoren

Jede Entität hat eine **UUID** (`id`, v4, intern und in der API) und optional eine **lesbare Kennung**:

| Entität | Lesbare Kennung | Regel |
|---|---|---|
| Issue | `ENG-123` | `Team.key` + `-` + `Issue.number`. `number` ist ein **pro Team fortlaufender Zähler** (beginnt bei 1, lückenlos vergeben, nie wiederverwendet). |
| Issue (nach Teamwechsel) | neuer `KEY-n` | Alte Kennungen werden in `previousIdentifiers: [String]` gespeichert und bleiben auflösbar (Redirect, Suche). |
| Project | `P-ENG-7` | Präfix `P-` + Key des **Lead-Teams** + `-` + Projektzähler dieses Teams. Zusätzlich `slugId` (12 Hex-Zeichen) für URLs. |
| Initiative | `I-4` | Präfix `I-` + workspaceweiter Zähler. |
| Document | `slugId` (12 Hex) | URL: `/<workspace>/document/<titel-slug>-<slugId>`. |
| alle anderen | nur UUID | Labels, States, Cycles, Comments, Notifications, Views, Templates, Milestones. |

**Zähler-Vergabe ist atomar pro Team.** Die Vergabe von `Issue.number` muss in derselben Transaktion wie das Anlegen des Issues erfolgen (z. B. `UPDATE team SET lastIssueNumber = lastIssueNumber + 1 RETURNING lastIssueNumber` oder gleichwertige Sperre/Sequenz pro Team). Gleichzeitige Creates in einem Team dürfen nie dieselbe Nummer erhalten.

**Referenzauflösung („flexible Referenz“).** Überall wo die API/MCP eine Referenz annimmt, wird in dieser Reihenfolge aufgelöst:
1. UUID → direkte Suche.
2. Kennung (`ENG-123`, `P-ENG-7`, `I-4`, auch `previousIdentifiers`).
3. Slug / `slugId`.
4. Schlüssel (`Team.key`, case-insensitiv).
5. Exakter Name (case-insensitiv). Bei Mehrdeutigkeit → Fehler `ambiguous_reference` mit Kandidatenliste.
6. Für User zusätzlich E-Mail und das Literal `"me"` (= aufrufender User).
7. Für Status (`state`) zusätzlich der **Kategoriename** (`started` → erster Status dieser Kategorie im Team, nach `position`).

## 3. Datums- und Zahlenformate

| Art | Format | Beispiel |
|---|---|---|
| Zeitpunkt (`DateTime`) | ISO-8601 UTC mit Millisekunden | `2026-10-10T07:15:00.000Z` |
| Kalenderdatum (`Date`, „TimelessDate“) | `YYYY-MM-DD`, ohne Zeitzone | `2026-11-30` |
| Relativer Filterzeitpunkt (`DateTimeOrDuration`) | ISO-8601-Dauer, negativ = Vergangenheit | `-P1D`, `-P2W` |
| Datumsauflösung (Projekt/Initiative) | Enum `month`, `quarter`, `halfYear`, `year` | `targetDate=2026-12-31` + `quarter` = „Q4 2026“ |
| Sortierposition (`sortOrder`, `position`) | `Float` | Einfügen zwischen a und b: `(a+b)/2`; gelegentlich neu durchnummerieren |
| Farben | Hex `#RRGGBB` | `#5E6AD2` |

Cycle-Grenzen werden in der **Team-Zeitzone** (`Team.timezone`, Default `America/Los_Angeles`) auf Mitternacht berechnet und als UTC-Zeitpunkt gespeichert.

## 4. Feste Enums

### Priorität (`priority: Int`)

| Wert | Name | Sortierung in Listen |
|---|---|---|
| 0 | No priority | zuletzt |
| 1 | Urgent | zuerst |
| 2 | High | |
| 3 | Medium | |
| 4 | Low | |

Ausgabe in API/MCP immer als Objekt `{ "value": 2, "name": "High" }`.

### Statuskategorie (`WorkflowState.type`)

Feste Reihenfolge: `triage` → `backlog` → `unstarted` → `started` → `completed` → `canceled` → `duplicate`. „Offen“ = `triage|backlog|unstarted|started`; „geschlossen“ = `completed|canceled|duplicate`.

### Weitere

| Enum | Werte |
|---|---|
| Team-Sichtbarkeit `TeamVisibility` | `public`, `private`, `restricted` |
| Rolle `UserRoleType` | `owner`, `admin`, `user`, `guest`, `app` |
| Schätzungstyp `issueEstimationType` | `notUsed`, `exponential`, `fibonacci`, `linear`, `tShirt` |
| Relationstyp `IssueRelationType` | `blocks`, `related`, `duplicate`, `similar` |
| Projektstatus-Kategorie `ProjectStatusType` | `backlog`, `planned`, `started`, `paused`, `completed`, `canceled` |
| Initiative-Status | `Proposed`, `Planned`, `Active`, `Completed`, `Canceled` |
| Health `health` | `onTrack`, `atRisk`, `offTrack` |
| Label-Gruppentyp `groupType` | `singleSelect`, `multiSelect` |
| SLA-Zähltyp `slaType` | `all`, `onlyBusinessDays` |
| View-Typ | `Issue`, `Project`, `Initiative` |
| Template-Typ | `issue`, `project`, `document` |

## 5. Listen und Paginierung

Einheitliche Antwortform aller Listen (API und MCP):

```json
{
  "issues": [ { "id": "…" } ],
  "hasNextPage": true,
  "cursor": "c2b1…"
}
```

- Schlüssel ist der Plural des Entitätsnamens (`issues`, `teams`, `projects`, `notifications` …).
- `cursor` ist **opak** (Implementierung: base64 von `{sortKey, id}`) und **fehlt**, wenn `hasNextPage=false`.
- `limit`: Default 50, Maximum 250 (Projekte: Maximum 50).
- `orderBy`: `createdAt | updatedAt`, Default `updatedAt`, absteigend.
- Archivierte Objekte nur mit `includeArchived=true`.
- Es gibt **kein** `totalCount` (bewusst – teuer bei großen Teams).
- Filter `createdAt`/`updatedAt` bedeuten „nach diesem Zeitpunkt“ und akzeptieren `DateTime` oder `DateTimeOrDuration`.

## 6. Schreibsemantik

- **Upsert-Muster** (`save_*`): mit `id` = Update, ohne `id` = Create.
- **Tri-State bei Updates**: Feld weggelassen = unverändert; `null` = entfernen/leeren; Wert = setzen.
- **Mengenfelder** gibt es in drei Varianten: `x` (ersetzt die ganze Menge), `addX`/`removeX` (inkrementell), teils `setX`. Ersetzen und inkrementell sind nicht kombinierbar → Fehler `invalid_request`.
- **Beziehungsfelder append-only** (`blocks`, `blockedBy`, `relatedTo`, `links`): fügen nur hinzu; Entfernen nur über `remove*`.
- **Patch für Rich-Text** (`description`, `content`): Liste von 1–50 Operationen, atomar; jeder Anker muss **genau einmal** im aktuellen Text vorkommen. Operationen: `replace`, `insert_before`, `insert_after`, `prepend`, `append`, `replace_range`. Details in [07-mcp-server.md](07-mcp-server.md).
- **Markdown** ist das Austauschformat für alle Rich-Text-Felder. Erwähnungen: `@displayName` (User), `ENG-123` (Issue, wird zu Link). Intern darf ein strukturiertes Format (z. B. ProseMirror-JSON) gespeichert werden; nach außen wird Markdown geliefert.

## 7. Fehlermodell

Alle Fehler haben dieselbe Form:

```json
{
  "error": "not_found",
  "message": "Could not find referenced Issue.",
  "status": 404,
  "requestId": "req_01H…",
  "details": { "reference": "ENG-999999" }
}
```

| `error` | HTTP | Wann |
|---|---|---|
| `invalid_request` | 400 | Schemaverletzung, unvereinbare Parameter (z. B. `labels` + `addLabels`) |
| `unauthenticated` | 401 | Kein/ungültiger Token |
| `forbidden` | 403 | Rolle/Teamzugehörigkeit reicht nicht (private Teams: stattdessen `not_found`, um Existenz nicht zu verraten) |
| `not_found` | 404 | Referenz nicht auflösbar |
| `ambiguous_reference` | 409 | Name passt auf mehrere Objekte |
| `conflict` | 409 | Patch-Anker nicht genau einmal gefunden, Versionskonflikt |
| `limit_exceeded` | 422 | Fachliches Limit (z. B. 60 000 offene Issues pro Team) |
| `rate_limited` | 429 | Rate-Limit überschritten, `Retry-After`-Header |
| `internal` | 500 | Unerwartet |

**Bewusste Abweichung vom Original:** Linear liefert bei unbekanntem Filterwert teils stillschweigend eine leere Liste (z. B. Status-Liste für unbekanntes Team). Lineal liefert bei unauflösbaren **Pflicht**-Referenzen immer `not_found`; bei optionalen **Filter**-Referenzen ebenfalls `not_found` (fail loud).

## 8. Audit-Felder (alle Entitäten)

| Feld | Typ | Bedeutung |
|---|---|---|
| `id` | UUID | Primärschlüssel |
| `createdAt` | DateTime | Anlage |
| `updatedAt` | DateTime | letzte inhaltliche Änderung |
| `archivedAt` | DateTime? | gesetzt = archiviert |
| `version` | Int | Optimistic Locking; Update mit veralteter Version → `conflict` |

Zusätzlich trägt jede schreibende Transaktion eine workspaceweit monotone `lastSyncId` (Grundlage für Echtzeit-Sync, siehe [09](09-api-events.md)).

Löschen ist in der Regel **soft** (`trashed`/`archivedAt`); endgültiges Löschen erfolgt durch einen Hintergrundjob nach Ablauf der Frist (30 Tage).

## 9. Diagramme

Alle Diagramme sind **Mermaid** (`flowchart`, `sequenceDiagram`, `stateDiagram-v2`, `erDiagram`), damit sie in Git-Plattformen und in `index.html` gerendert werden.
