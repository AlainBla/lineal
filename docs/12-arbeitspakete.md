# 12 – Arbeitspakete und Umsetzungsplan

> **Für ausführende Agenten:** Jedes Arbeitspaket (WP) wird von genau einem Agenten in einem eigenen Git-Worktree auf dem Branch `wp/<nr>-<kurzname>` umgesetzt. Lies zuerst [00](00-konventionen.md), [11](11-implementierungsarchitektur.md) und die im WP genannten Fachkapitel. Ändere **nur** Dateien aus deiner Eigentumsliste. Fertig ist ein WP erst, wenn `scripts/check.sh` grün ist und alles committet ist.

**Ziel:** Lineal (Linear-Nachbau) gemäß Kapitel 00–11 implementieren: Rust-Backend mit GraphQL-API und MCP-Server, React-Frontend.

**Architektur:** Ports & Adapters ([11](11-implementierungsarchitektur.md)): `lineal-domain` (rein) ← `lineal-app` (Use-Cases, Ports, Policy) ← Adapter (`lineal-db`, `lineal-api`, `lineal-mcp`, `lineal-sync`, `lineal-jobs`) ← `lineal-server`.

**Stack:** Rust stable, tokio, axum, sqlx/PostgreSQL, async-graphql, rmcp; TypeScript, React 19, Vite, Tailwind v4, shadcn/ui, TanStack, cmdk ([10 §11](10-architektur-umsetzung.md#11-technologie-stack-entscheidung)).

## Globale Vorgaben (gelten für jedes WP)

- Fehlercodes nur aus [00 §7](00-konventionen.md#7-fehlermodell); Enums exakt wie [00 §4](00-konventionen.md#4-feste-enums).
- Team-Key `^[A-Z][A-Z0-9]{0,6}$`; Kennung `KEY-n`; Zähler atomar pro Team.
- Jede SQL-Abfrage parametrisiert und mit `organization_id`; Laufzeit-Queries (`sqlx::query_as`), keine `query!`-Makros bis WP17.
- Jede Leseoperation läuft durch `policy::visible_team_ids`; private Teams → `not_found`.
- Jede Schreiboperation: eine Transaktion inkl. History und Outbox-Event.
- Listen: `{ <plural>: [...], hasNextPage, cursor? }`, `limit` Default 50, Max 250.
- MCP-Eingabeschemas exakt aus `docs/mcp-tools.snapshot.json`; Antwortformen nach [07](07-mcp-server.md).
- Rust: `#![forbid(unsafe_code)]`, kein `unwrap()` außerhalb von Tests, `clippy -D warnings`.
- Nur erfundene Beispieldaten; **niemals Linear-MCP-Tools aufrufen**.
- Paketmanager `npm`. PostgreSQL für Tests: `postgres://lineal:lineal@localhost:5440/lineal` (Container aus `docker-compose.yml`, gemeinsam genutzt; `#[sqlx::test]` legt pro Test eine eigene DB an). Port 5432 nicht verwenden.
- Ressourcen: `CARGO_BUILD_JOBS=4`, `CARGO_TARGET_DIR` im eigenen Worktree (Default).

## Review-Fokus (häufigste Fehlerquellen, je mit Test im zuständigen WP)

1. **Gleichzeitige Issue-Anlage im selben Team** → keine doppelten Nummern, keine Lücken. Test in WP06 (20 parallele Creates).
2. **Zugriff auf Objekte privater Teams über direkte ID/Kennung** → `not_found`, auch über MCP und Suche. Tests in WP02, WP06, WP11.
3. **Teamwechsel eines Issues** → neue Kennung, alte auflösbar, Team-Labels/Cycle/Projekt korrekt bereinigt. Test in WP06.
4. **Patch-Operationen mit mehrdeutigem Anker** → `conflict`, Text unverändert (atomar). Test in WP01 und WP06.
5. **Cycle-Grenzen über Sommerzeitwechsel in der Team-Zeitzone** → Mitternacht lokal bleibt korrekt. Test in WP01 und WP07.

---

## Wellen und Abhängigkeiten

```mermaid
flowchart LR
  subgraph W0["Welle 0: Fundament"]
    S1["S1 Scaffold-Code"]
    S2["S2 Datenbankschema"]
  end
  subgraph W1["Welle 1: Plattform"]
    WP01["WP01 Domain-Regeln"]
    WP02["WP02 Identitaet und Teams"]
    WP03["WP03 API-Plattform"]
    WP04["WP04 MCP-Plattform"]
    WP05["WP05 Web-Shell"]
  end
  subgraph W2["Welle 2: Kernfachlichkeit"]
    WP06["WP06 Issues-Kern"]
    WP07["WP07 Planung"]
    WP08["WP08 Inbox"]
    WP09["WP09 Kommentare und Anhaenge"]
  end
  subgraph W3["Welle 3: Erweiterung und UI"]
    WP10["WP10 Dokumente und Templates"]
    WP11["WP11 Views, Filter, Suche"]
    WP12["WP12 Triage und Automationen"]
    WP13["WP13 Web Issues"]
    WP14["WP14 Web Planung, Inbox, Settings"]
  end
  subgraph W4["Welle 4: Echtzeit, Integration, Import"]
    WP15["WP15 Sync-Engine"]
    WP16["WP16 Webhooks und OAuth"]
    WP18["WP18 Linear-Import"]
  end
  subgraph W5["Welle 5: Haertung"]
    WP17["WP17 Haertung und E2E"]
  end
  S1 --> W1
  S2 --> W1
  W1 --> W2
  W2 --> W3
  W3 --> W4
  W4 --> W5
```

| Welle | Pakete | Parallel | Voraussetzung |
|---|---|---|---|
| 0 | S1, S2 | 2 | – |
| 1 | WP01–WP05 | 5 | Welle 0 gemergt, `check.sh` grün |
| 2 | WP06–WP09 | 4 | Welle 1 gemergt |
| 3 | WP10–WP14 | 5 | Welle 2 gemergt |
| 4 | WP15, WP16, WP18 | 3 | Welle 3 gemergt |
| 5 | WP17 | 1 | Welle 4 gemergt |

Agenten-Budget: 20 Pakete (S1, S2, WP01–WP18) = 20. Es gibt keine Reserve-Agenten; Integrationsfehler behebt der Orchestrator selbst.

**Ablauf pro Welle (Orchestrator):** Agenten starten → Branches nacheinander in `main` mergen → nach jedem Merge `scripts/check.sh`, `web/schema.graphql` neu erzeugen und committen → Konflikte und Integrationsfehler beheben (Lockfiles: eine Seite nehmen, neu bauen) → `target/` der Worktrees löschen → nächste Welle.

## Migrationsbereiche

| Bereich | Eigentümer |
|---|---|
| `0001`–`0009` | S1 (Outbox, Jobs, Sync-Sequenz) |
| `0010`–`0099` | S2 (komplettes Schema aus [01](01-domaenenmodell.md)) |
| `0200`–`0299` | WP02 |
| `0600`–`0699` | WP06 |
| `0700`–`0799` | WP07 |
| `0800`–`0899` | WP08 |
| `0900`–`0999` | WP09 |
| `1000`–`1099` | WP10 |
| `1100`–`1199` | WP11 |
| `1200`–`1299` | WP12 |
| `1500`–`1599` | WP15 |
| `1600`–`1699` | WP16 |
| `1800`–`1899` | WP18 |

Zusätzliche Migrationen dürfen das Schema nur **ergänzen** (Indizes, Spalten, Tabellen), nie Objekte anderer Bereiche löschen.

## Gemeinsame Dateien (nur S1, danach nur der Orchestrator)

`Cargo.toml` (Workspace), alle `Cargo.toml` der Crates, `crates/*/src/lib.rs`, `crates/lineal-app/src/ports/mod.rs` und `ports/tx.rs`, `crates/lineal-db/src/store.rs`, `crates/lineal-api/src/graphql/mod.rs`, `crates/lineal-api/src/router.rs`, `crates/lineal-mcp/src/dispatch.rs`, `crates/lineal-jobs/src/registry.rs`, `crates/lineal-testkit/src/seed/mod.rs`, `web/package.json`, `web/vite.config.ts`, `web/tsconfig*.json`, `scripts/*`, `docker-compose.yml`.

**Generierte Dateien und Lockfiles** (`web/schema.graphql`, `Cargo.lock`, `web/package-lock.json`): Arbeitspakete dürfen sie ändern; bei Merge-Konflikten nimmt der Orchestrator eine Seite, baut neu (`cargo build`, `npm install`, `lineal schema-export`) und committet das Ergebnis. `scripts/check.sh` erzeugt `web/schema.graphql` neu, statt es zu vergleichen.

Braucht ein WP eine neue Crate-Abhängigkeit, darf es **ausnahmsweise** die `[dependencies]` seines Crates erweitern und meldet das im Abschlussbericht (der Orchestrator löst Konflikte beim Merge).

---

## Welle 0

### S1 – Scaffold-Code

**Ziel:** Kompilierender, getesteter Rumpf des gesamten Systems, in dem alle gemeinsam genutzten Dateien existieren.

**Spezifikation:** [11](11-implementierungsarchitektur.md) vollständig, [00](00-konventionen.md), [10 §11](10-architektur-umsetzung.md#11-technologie-stack-entscheidung).

**Liefert:**
- Workspace mit allen Crates aus [11 §1](11-implementierungsarchitektur.md#1-repository-layout), Abhängigkeitsrichtung wie [11 §2](11-implementierungsarchitektur.md#2-crate-abhängigkeiten), `rust-toolchain.toml`, `#![forbid(unsafe_code)]`.
- Zeit: `chrono` + `chrono-tz` in allen Crates (`DateTime<Utc>` gespeichert, `chrono_tz::Tz` für Team-Zeitzonen); sqlx-Feature `chrono`. **Kein** MCP-Crate (`rmcp` o. ä.) hinzufügen – das entscheidet WP04.
- **Jede** im Scaffold deklarierte Trait-Methode hat einen kompilierenden Stub (`Ok(None)`, `Ok(vec![])` oder `Err(AppError::internal("not implemented"))`); jede Datei, die ein späteres WP besitzt, existiert bereits als lauffähiger Stub (REST-Endpunkte → 501, `lineal-server/src/adapters.rs` mit System-Clock, UUID-v7-IdGen, Dummy-BlobStore und Dummy-Hasher).
- `lineal-domain`: `ids` (alle Newtypes), `enums` (alle Enums aus 00 §4 mit serde-Schreibweise exakt), `identifier::TeamKey` mit Validierung + Tests, `events::DomainEvent` (Varianten für alle Ereignisse aus [09 §6](09-api-events.md), Felder minimal: betroffene IDs), `error::DomainError`. Leeres Modul `rules/mod.rs`.
- `lineal-app`: `RequestCtx`, `Actor`, `Scopes`, `AppError`/`ErrorKind`, `App`-Struktur, alle Port-Traits (`Store`, `Tx` mit **allen** Accessoren aus [11 §3.2](11-implementierungsarchitektur.md#32-application-lineal-app), je Repo-Trait eine Datei `ports/repos/<bereich>.rs` – zunächst leer bis auf die unten genannten Methoden), `Clock`, `IdGen`, `BlobStore`, `SearchIndex`, `PasswordHasher`, `Mailer`. Module `policy/`, `resolve/`, `usecases/` mit je einer leeren Datei pro Bereich: `org, user, auth, team, state, label, issue, comment, attachment, cycle, project, initiative, view, notification, document, template, triage, webhook`.
  - Bereits implementierte Methoden: `OutboxRepo::push(event)`, `JobRepo` (enqueue, claim mit `SKIP LOCKED`, complete, fail), `AuthRepo::find_principal_by_token_hash(hash) -> Option<Principal>` (Signatur; Implementierung durch WP02).
- `lineal-db`: `PgStore`/`PgTx` (begin/commit, setzt `lastSyncId` über Sequenz `sync_seq`), je Repo eine Datei `repos/<bereich>.rs` mit `impl … for PgTx` (Stubs); Outbox und Job implementiert; Migration `0001_bootstrap.sql` nur für `outbox_event` (ohne FK, mit `organization_id` und `origin` `user|system|import`, siehe [13 §4.3](13-import-linear.md)), `outbox_delivery`, `job`, `sync_seq` (S2 übernimmt ab `0010`).
- `lineal-api`: axum-Router mit allen Pfaden aus [11 §3.4](11-implementierungsarchitektur.md#34-adapter) (nicht implementierte antworten `501`), GraphQL-Schema mit `MergedObject`-Roots und einem Teilobjekt pro Bereich in `graphql/<bereich>.rs` (je ein Platzhalterfeld; prüfe per Test, dass `Schema::build` damit durchläuft – falls versteckte Felder nicht reichen, sichtbares Feld `_<bereich>Ready: Boolean`, das das zuständige WP löscht), Context-Daten (`Arc<App>`, `RequestCtx`), `/healthz`.
- `lineal-mcp`: `dispatch.rs` mit Zuordnung aller 55 Tool-Namen aus `docs/mcp-tools.snapshot.json` zu Handler-Dateien `handlers/<gruppe>.rs` (Gruppen: `teams_users, issues, statuses, labels, comments, attachments, projects, milestones, cycles, initiatives, status_updates, views, inbox, triage, documents, templates, sharing`), jede Handler-Funktion liefert `not implemented`.
- `lineal-jobs`: Traits `EventConsumer`, `PeriodicJob`, `registry.rs` (Liste aller Konsumenten/Jobs, Einträge pro Bereich als leere Module `consumers/<name>.rs`, `periodic/<name>.rs`), Worker-Schleife.
- `lineal-server`: Subcommands `serve`, `worker`, `migrate`, `seed`, `schema-export`; Konfiguration aus Umgebung.
- `lineal-testkit`: `seed/mod.rs` ruft `seed::identity`, `seed::issues`, `seed::planning`, `seed::content` (leere Funktionen in eigenen Dateien) auf; Helper `ctx_for(user)`; `FakeStore`/`FakeTx` (alle Repos als Stubs, konfigurierbare `find_principal_by_token_hash`), damit WP03/WP04 Auth ohne WP02 testen können; README mit Testmuster `#[sqlx::test(migrations = "../lineal-db/migrations")]` für Crates außerhalb von `lineal-db`.
- `web/`: Vite + React 19 + TS strict + Tailwind v4 + shadcn/ui-Init + TanStack Router (dateibasiert, `routeTree.gen.ts` in `.gitignore`) + TanStack Query + ESLint/Prettier + Vitest; eine Route `/` „Lineal“; Proxy `/graphql`, `/auth`, `/sync`, `/mcp` → `:8080`.
- `docker-compose.yml` mit `name: lineal` ganz oben (alle Worktrees teilen denselben Container), postgres:16 auf **5440**, User/Passwort/DB `lineal`; `.env.example`; `.gitignore` erweitern; `scripts/check.sh` (exportiert `DATABASE_URL` mit Default `postgres://lineal:lineal@localhost:5440/lineal`, `docker compose up -d --wait`, fmt, clippy, test, `schema-export` neu erzeugen, `npm ci`/typecheck/lint/test); `scripts/dev.sh`; `README.md` (Entwickler-Schnellstart).

**DoD:** `scripts/check.sh` grün; `cargo run -p lineal-server -- serve` startet und `/healthz` liefert 200; Unit-Test: `dispatch.rs` kennt exakt die 55 Tool-Namen aus dem Snapshot plus die Lineal-Erweiterung `get_workspace_overview` ([14 §4](14-firmenweite-sicht.md)); Handler-Gruppe `workspace` dafür angelegt.

### S2 – Datenbankschema

**Ziel:** Das vollständige relationale Schema aus [01](01-domaenenmodell.md) als Migrationen `0001`–`0099`.

**Eigentum:** `crates/lineal-db/migrations/0010_*.sql` … `0099_*.sql`, `crates/lineal-db/tests/schema.rs`. **Nicht** anlegen: `outbox_event`, `outbox_delivery`, `job`, `sync_seq` (gehören S1).

**Inhalt:** Alle Entitäten aus 01 (inkl. `Draft`, `Invitation`, `IssueIdentifierAlias`, `TeamKey`, `DocumentContent`, Join-Tabellen, `InitiativeParent`) sowie die Import-Ergänzungen aus [13 §5](13-import-linear.md) (`import_run`, `import_mapping`, `import_issue`, `import_credential`, `user.status` mit Wert `imported`, `team.import_locked_until`, `issue_history.source`), Spaltennamen `snake_case` (Feld `issueCount` → `issue_count`), `organization_id` in jeder Tabelle mit zusammengesetzten FKs wo in 01 beschrieben, Unique-Constraints und Indizes aus 01 §7, Check-Constraints (Enums als `text` + `CHECK (... IN (...))`, genau-ein-Parent bei Dokumenten), `version int not null default 1`, Audit-Spalten. Volltext: `issue.search_vector tsvector` generiert aus Titel/Beschreibung + GIN-Index, `pg_trgm`-Index auf Kennungen. Reihenfolge der Dateien nach FK-Abhängigkeit (Org/User/Team → States/Labels → Issues → …).

**Tests:** `schema.rs` mit `#[sqlx::test]`: Migrationen laufen durch; Stichproben-Inserts (Org, Team, State, Issue) und Constraint-Verletzungen (zwei Issues gleiche Nummer im Team → Fehler; ungültiger `state_type` → Fehler; Dokument mit zwei Parents → Fehler).

**DoD:** Im eigenen Worktree gibt es noch kein Crate – Prüfung mit `psql` gegen eine temporäre Datenbank im Container auf 5440 (alle Dateien in Reihenfolge anwenden, Smoke-Inserts, Constraint-Verletzungen). `tests/schema.rs` wird geschrieben und vom Orchestrator nach dem Merge ausgeführt.

---

## Welle 1

### WP01 – Domain-Regeln

**Spezifikation:** [01 §6](01-domaenenmodell.md), [03](03-issues-workflow.md) §1–3, §11–12, [04](04-planung.md) §1, [05](05-views-suche-inbox.md) §1, [07](07-mcp-server.md) §2 (Patch).

**Eigentum:** `crates/lineal-domain/src/rules/**`, `crates/lineal-domain/src/identifier.rs` (Erweiterung um `IssueIdentifier`, `ProjectIdentifier`, `parse_identifier`).

**Liefert (reine Funktionen mit Tests):**
- `parse_identifier(&str) -> Option<ParsedIdentifier>`
- `rules::state::apply_transition(old: StateType, new: StateType, now, timestamps) -> IssueTimestamps` (startedAt/completedAt/canceledAt/triagedAt-Regeln aus 03 §1)
- `rules::estimate::{allowed_values(EstimationType, extended, allow_zero) -> Vec<f64>, convert(...)}`
- `rules::labels::check_group_constraints(selected, groups) -> Result<Vec<LabelId>, DomainError>` inkl. „addLabels ersetzt Label derselben singleSelect-Gruppe“
- `rules::progress::{issue_points, progress(&[IssueProgressInput]) -> f64}` (erledigt + 0,25 × begonnen)
- `rules::cycle_calendar::generate(settings, tz, from, count) -> Vec<CycleWindow>` (DST-sicher)
- `rules::sla::{breach_at(start, duration, SlaType, tz) -> OffsetDateTime, risk_levels(...)}`
- `rules::patch::apply(text: &str, ops: &[PatchOp]) -> Result<String, PatchError{op_index, reason}>` (atomar, Anker genau einmal)
- `rules::filter::{IssueFilter (AST, serde nach 05 §1 JSON Schema), validate(&IssueFilter)}`
- `rules::sort_order::between(a: Option<f64>, b: Option<f64>) -> f64`
- `rules::markdown::sanitize(&str) -> String` (Allowlist, z. B. Crate `ammonia`)

**DoD:** `cargo test -p lineal-domain` grün; Review-Fokus 4 und 5 als Tests enthalten.

### WP02 – Identität, Workspace, Teams, Workflow-Status

**Spezifikation:** [02](02-teams-rechte.md) komplett, [03 §1](03-issues-workflow.md) (Status-CRUD), [07](07-mcp-server.md) Gruppen `teams_users`, `statuses`, [09](09-api-events.md) (Auth, Workspace-/Einladungs-Mutationen).

**Eigentum:**
- `crates/lineal-app/src/usecases/{org,user,auth,team,state}.rs`, `ports/repos/{org,user,auth,team,state}.rs`, `policy/**`, `resolve/{team,user,state}.rs`
- `crates/lineal-db/src/repos/{org,user,auth,team,state}.rs`, Migrationen `0200`–`0299`
- `crates/lineal-api/src/graphql/{org,team}.rs`, `crates/lineal-api/src/rest/auth.rs`
- `crates/lineal-mcp/src/handlers/{teams_users,statuses}.rs`
- `crates/lineal-testkit/src/seed/identity.rs`

**Liefert:** Workspace anlegen (Default-Team), User, Passwort-Login mit Session-Cookie (`/auth/login|logout|me`), API-Keys (anlegen, Hash, widerrufen), `AuthRepo::find_principal_by_token_hash`, Teams (Key-Generierung/-Validierung, Default-Workflow beim Anlegen), Mitgliedschaften + Owner, Sichtbarkeit public/private/restricted, Sub-Teams (Nesting inkl. Vorschau, Vererbungsflags, Tiefenlimit), Workflow-Status CRUD (≥ 1 pro Kategorie, Duplicate system-verwaltet), `policy::visible_team_ids`, `policy::require` mit Matrix aus 02 §6, Referenzauflösung Team/User/State. Seed: acme, ENG, WEB (Sub-Team von ENG), DES (privat), Alice/Bob/Carol wie in 02.

**Tests:** Zähler-Spalte initial, Key-Regeln, Nesting-Zyklus verboten, privates Team → `not_found` für Nicht-Mitglied (Review-Fokus 2), Gast sieht nur seine Teams, Status-Löschen mit Migrationsziel, Login/Logout, MCP `list_teams`/`get_team`/`list_users`/`get_user`/`list_issue_statuses`/`get_issue_status`/`get_workspace` Contract.

### WP03 – API-Plattform

**Spezifikation:** [09](09-api-events.md) §1–3, [10 §11.4](10-architektur-umsetzung.md#114-sicherheitsvorgaben-für-die-umsetzung), [11 §3.4–3.5](11-implementierungsarchitektur.md#34-adapter).

**Eigentum:** `crates/lineal-api/src/{middleware/**,error.rs,context.rs,pagination.rs,rate_limit.rs,security_headers.rs,blob.rs}`, `crates/lineal-api/tests/platform.rs`, `crates/lineal-server/src/{config.rs,adapters.rs}` (Clock, IdGen, lokaler `BlobStore`, Argon2-`PasswordHasher`).

**Liefert:** Request-ID, Tracing, Auth-Extraktor (Session-Cookie oder `Authorization`-Header → `AuthRepo::find_principal_by_token_hash` → `RequestCtx`), Token-Bucket-Rate-Limit mit `X-RateLimit-*` und 429, Security-Header (CSP, HSTS, X-Content-Type-Options, Referrer-Policy), CORS für Dev, Abbildung `AppError` → HTTP/GraphQL-Fehler (`extensions.code`), Relay-Paginierungshelfer, lokaler Blob-Store mit signierten URLs (HMAC, Ablauf 60 s/5 min) und Endpunkten `/uploads/{token}`, `/files/{token}`, `schema-export` schreibt `web/schema.graphql`.

**Tests:** Fehlermodell-Snapshot je ErrorKind, Rate-Limit greift, abgelaufene Signatur → 403, CSP-Header vorhanden, unauthentifiziert → `unauthenticated`.

### WP04 – MCP-Plattform

**Spezifikation:** [07](07-mcp-server.md) §1–2, §4–6.

**Eigentum:** `crates/lineal-mcp/src/{server.rs,transport.rs,validate.rs,envelope.rs,fields.rs,error.rs,auth.rs,wellknown.rs}`, `crates/lineal-mcp/tests/**`.

**Liefert:** JSON-RPC 2.0 über Streamable HTTP (`/mcp`, rmcp oder schlanke eigene Umsetzung), `initialize`, `tools/list` (Snapshot), `tools/call` mit Schema-Validierung (Crate `jsonschema`) → Dispatch, `isError`-Abbildung nach Fehlermodell, unbekanntes Tool → `-32601`, Bearer-Auth (API-Key) → `RequestCtx`, Scopes read/write, Hilfen für Paginierungshülle, `fields`-Auswahl, Kürzung auf 500 Zeichen, `/.well-known/oauth-protected-resource` und `/.well-known/oauth-authorization-server` (Metadaten; Endpunkte liefert WP16).

**Tests:** Schema-Snapshot-Test aus 07 §6.1 (55 Tools, identische Schemas), ungültige Eingabe → `invalid_request` mit Pfad, fehlender Token → 401 mit `WWW-Authenticate`.

### WP05 – Web-Shell und Design-System

**Spezifikation:** [08](08-userflows.md) §1–3, [11 §4](11-implementierungsarchitektur.md#4-frontend-architektur).

**Eigentum:** `web/src/app/**`, `web/src/components/ui/**`, `web/src/lib/{shortcuts.ts,graphql.ts,theme.ts,cn.ts}`, `web/src/routes/{__root.tsx,index.tsx,login.tsx}`, `web/codegen.ts`, `web/src/**/*.test.tsx` in diesen Ordnern.

**Liefert:** Linear-artige Shell: Sidebar (Workspace-Menü, Inbox, My Issues, Projects, Initiatives, Views, Teams mit Unterpunkten und Sub-Teams eingerückt), Topbar, hell/dunkel, Design-Tokens, Command-Menü (`cmdk`, Cmd/Ctrl+K) mit Registry, Shortcut-Registry mit Kontexten und Sequenzen (`g i`, `g m`), Login-Seite gegen `/auth/login`, Auth-Guard, GraphQL-Client mit generierten Typen (`graphql-codegen` aus `web/schema.graphql`), leere Platzhalterseiten für die Feature-Routen. Optik: ruhig, dicht, Inter/Geist-Schrift, 1px-Linien, dezente Animationen.

**Tests:** Shortcut-Registry (Kontexte, Sequenzen), Command-Menü filtert, Theme-Umschaltung, Login-Formular-Validierung.

---

## Welle 2

### WP06 – Issues-Kern (inkl. Labels, Relationen, Teamwechsel)

**Spezifikation:** [03](03-issues-workflow.md) §2–5, §8–9, §12, [02 §7](02-teams-rechte.md) (Teamwechsel), [06 §2](06-dokumente-templates.md) nur Verweis, [07](07-mcp-server.md) Gruppen `issues`, `labels`, `sharing`.

**Eigentum:** `usecases/{issue,label}.rs`, `ports/repos/{issue,label}.rs`, `resolve/{issue,label}.rs`, `lineal-db/src/repos/{issue,label}.rs`, Migrationen `0600`–`0699`, `graphql/{issue,label}.rs`, `mcp/handlers/{issues,labels,sharing}.rs`, `testkit/src/seed/issues.rs`.

**Liefert:** create (atomarer Zähler, Default-State, Triage-State für Nicht-Mitglieder, sortOrder), update mit Tri-State und Patch, Zuordnung zu Cycle/Projekt/Meilenstein inkl. Validierung (Issue-Team am Projekt beteiligt, Cycle gehört zum Team, Meilenstein zum Projekt) per SQL gegen das S2-Schema, Statuswechsel mit Zeitstempeln, Priorität, Schätzung (Skala des Teams), Labels (Workspace/Team/vererbt, Gruppen, retire/restore), Sub-Issues (Zyklen verboten), Relationen inkl. Duplicate → Duplicate-State, History mit Coalescing, Teamwechsel (17 Schritte aus 02 §7), Papierkorb/Restore, Kennungsauflösung inkl. Alias, `list_issues` mit den MCP-Filterparametern (ohne `customView`/`query` – kommt in WP11), Outbox-Events.

**Tests:** Review-Fokus 1 (20 parallele Creates), 2 (Issue in privatem Team), 3 (Teamwechsel), 4 (Patch über `save_issue`); singleSelect-Gruppe; Duplicate; MCP-Contract für alle Issue-/Label-Tools.

### WP07 – Planung

**Spezifikation:** [04](04-planung.md) komplett, [07](07-mcp-server.md) Gruppen `projects`, `milestones`, `cycles`, `initiatives`, `status_updates`.

**Eigentum:** `usecases/{cycle,project,initiative}.rs`, `ports/repos/{cycle,project,initiative}.rs`, `resolve/{cycle,project,initiative}.rs`, `lineal-db/src/repos/{cycle,project,initiative}.rs`, Migrationen `0700`–`0799`, `graphql/{cycle,project,initiative}.rs`, `mcp/handlers/{projects,milestones,cycles,initiatives,status_updates}.rs`, `lineal-jobs/src/periodic/{cycles,update_reminders}.rs`, `testkit/src/seed/planning.rs`.

**Liefert:** Cycle-Einstellungen, Generierung (WP01-Kalender), Cycle-Wechsel-Job mit Rollover und Snapshots, Sub-Team-Vererbung; Projekte mit mehreren Teams, Lead-Team, `P-KEY-n`, Projektstatus, Meilensteine, Fortschritt; Initiativen als DAG (Zyklenprüfung, Tiefe), Projektzuordnung; Status-Updates mit Health und Thread; Fortschritt/Scope werden per SQL aus der Issue-Tabelle gelesen; die Zuordnung von Issues zu Cycles/Projekten/Meilensteinen gehört zu WP06.

**Tests:** Review-Fokus 5 (DST), Rollover, Projekt mit Team ohne Mitgliedschaft, Initiative-Zyklus → `invalid_request`, MCP-Contract.

### WP08 – Inbox, Notifications, Abonnements

**Spezifikation:** [05 §6](05-views-suche-inbox.md), [10 §4](10-architektur-umsetzung.md), [07](07-mcp-server.md) Gruppe `inbox`.

**Eigentum:** `usecases/notification.rs`, `ports/repos/notification.rs`, `lineal-db/src/repos/notification.rs`, Migrationen `0800`–`0899`, `graphql/notification.rs`, `mcp/handlers/inbox.rs`, `lineal-jobs/src/consumers/notifications.rs`, `lineal-jobs/src/periodic/{snooze,notification_cap,due_reminders}.rs`.

**Liefert:** Abo-Verwaltung (automatisch aus Events: Creator, Assignee, Mention, Kommentar; manuell subscribe/unsubscribe), Fan-out-Konsument mit Self-Suppression und Gruppierung, read/unread, snooze/unsnooze, archive, archive all, 2 000er-Limit, Einstellungen pro User, E-Mail-Digest über `Mailer`-Port (Logging-Implementierung reicht).

**Tests:** Mention erzeugt genau eine Notification beim Erwähnten, keine beim Autor; Snooze-Wiedervorlage; Limit.

### WP09 – Kommentare, Reaktionen, Anhänge

**Spezifikation:** [03 §6–7](03-issues-workflow.md), [07](07-mcp-server.md) Gruppen `comments`, `attachments`.

**Eigentum:** `usecases/{comment,attachment}.rs`, `ports/repos/{comment,attachment}.rs`, `lineal-db/src/repos/{comment,attachment}.rs`, Migrationen `0900`–`0999`, `graphql/comment.rs`, `mcp/handlers/{comments,attachments}.rs`.

**Liefert:** Threads, Replies, Inline-Kommentare mit `quotedText`, Bearbeiten/Löschen, Resolve, Reaktionen, Link-Anhänge (Dedupe pro URL), Upload-Flow prepare → PUT → finalize über `BlobStore`, signierte Downloads, Outbox-Events (für Mentions/Notifications). Kommentar-Parents: Issue, Projekt, Initiative, Dokument, Meilenstein, Status-Update (Sichtbarkeit über den Parent).

**Tests:** Kommentar auf Issue im privaten Team als Nicht-Mitglied → `not_found`; Root eines Inline-Kommentars nicht löschbar; Upload mit abgelaufenem Token scheitert.

---

## Welle 3

### WP10 – Dokumente und Templates

**Spezifikation:** [06](06-dokumente-templates.md), [07](07-mcp-server.md) Gruppen `documents`, `templates`.
**Eigentum:** `usecases/{document,template}.rs`, `ports/repos/{document,template}.rs`, `lineal-db/src/repos/{document,template}.rs`, Migrationen `1000`–`1099`, `graphql/{document,template}.rs`, `mcp/handlers/{documents,templates}.rs`, `testkit/src/seed/content.rs`.
**Liefert:** Dokumente mit genau einem Parent, Reparent, Snapshot-Historie, Patch; Templates (issue/project/document, Workspace/Team, Vererbung), `applyTemplate` inkl. Sub-Issues in einer Transaktion; Integration in `create_issue`/`create_project` über `template`-Parameter.
**Tests:** Dokument mit zwei Parents → `invalid_request`; Template mit 3 Sub-Issues → lückenlose Nummern; Merge-Regeln der Beschreibung.

### WP11 – Views, Filter, Suche

**Spezifikation:** [05 §1–5](05-views-suche-inbox.md), [14](14-firmenweite-sicht.md) (Backend), [10 §5](10-architektur-umsetzung.md), [07](07-mcp-server.md) Gruppe `views`, `list_issues` (`query`, `customView`).
**Eigentum:** `usecases/view.rs`, `ports/repos/view.rs`, `lineal-db/src/repos/view.rs`, `lineal-db/src/{filter_sql.rs,search.rs}`, Migrationen `1100`–`1199`, `graphql/{view,search,overview}.rs`, `mcp/handlers/{views,workspace}.rs`, `lineal-app/src/usecases/overview.rs`, `lineal-db/src/repos/overview.rs`, `lineal-jobs/src/consumers/search_index.rs`. Darf in `repos/issue.rs` und `handlers/issues.rs` ausschließlich die Listen-/Suchfunktion erweitern.
**Liefert:** Filter-AST → SQL (alle Comparators, to-many, relative Daten, `me`), immer AND mit Sichtbarkeitsfilter; Custom Views (personal/team/workspace), Favoriten; Volltextsuche (Issues inkl. `previousIdentifiers`, Projekte, Dokumente) mit Ranking; Anzeigeoptionen-Persistenz; firmenweite Sicht: `workspaceOverview`, `workload`, `activeCycles`, MCP `get_workspace_overview`, Rollups über Sub-Teams.
**Tests:** Filter-Beispiele aus 05 als Tabellentests; Suche findet kein Issue aus privatem Team (Review-Fokus 2); View + Zusatzfilter = AND; Abnahmekriterien 1–4 aus [14 §6](14-firmenweite-sicht.md).

### WP12 – Triage und Automationen

**Spezifikation:** [03 §9–11](03-issues-workflow.md), [10 §3](10-architektur-umsetzung.md), [07](07-mcp-server.md) Gruppe `triage`.
**Eigentum:** `usecases/triage.rs`, `ports/repos/triage.rs`, `lineal-db/src/repos/triage.rs`, Migrationen `1200`–`1299`, `graphql/triage.rs`, `mcp/handlers/triage.rs`, `lineal-jobs/src/periodic/{auto_close,auto_archive,purge,sla,triage_rotation}.rs`.
**Liefert:** Triage accept/decline/duplicate/snooze/move, Triage-Verantwortung und Rotation, Auto-Close mit Ausnahmen, Auto-Archive, Papierkorb-Purge (30 Tage), SLA-Regeln und Risikostufen, Parent/Child-Auto-Close-Automation.
**Tests:** Auto-Close-Ausnahmen (aktiver Cycle, unfertiges Projekt, künftiges Due-Date), Decline setzt canceled + Kommentar, Rotation wechselt Schicht.

### WP13 – Web: Issues

**Spezifikation:** [08](08-userflows.md) F3–F8, Screens Issue-Liste/Board/Detail/Create-Modal.
**Eigentum:** `web/src/features/issues/**`, `web/src/routes/{team,issue,my-issues}/**`.
**Liefert:** Liste und Board (Gruppierung nach Status, Drag & Drop mit `dnd-kit`), Issue-Detail mit Property-Sidebar, Activity, Sub-Issues, Relationen, Kommentaren (TipTap-Editor mit Mentions), Create-Modal (Template, „Create more“), Shortcuts S/A/P/L/E/C, Teamwechsel-Dialog, My Issues.
**Tests:** Komponententests für Modal, Property-Picker, Board-Spaltenlogik.

### WP14 – Web: Planung, Inbox, Views, Suche, Settings

**Spezifikation:** [08](08-userflows.md) F1, F2, F9–F16, [14](14-firmenweite-sicht.md) (UI), [13 §6–7](13-import-linear.md) (Import-Assistent in den Settings).
**Eigentum:** `web/src/features/{planning,inbox,views,search,docs,settings,overview,import}/**`, `web/src/routes/{projects,project,initiatives,initiative,cycles,inbox,views,view,search,settings,document,overview,issues,workload}/**`.
**Liefert:** Cycles-Ansicht, Projektliste/-seite (Overview, Issues, Updates, Milestones), Initiativen, Inbox (Liste + Detail, Aktionen, Shortcuts), Filter-Bar + View-Builder + Favoriten, Suche, Dokument-Editor, Team- und Workspace-Settings (Workflow-Editor, Labels, Cycles, Triage, Mitglieder, API-Keys); firmenweite Sicht (Overview-Dashboard, Workspace-Issues gruppiert nach Team, Roadmap-Timeline, Workload, Cycle-Übersicht); Import-Assistent (Token, Plan, Fortschritt, Bericht) gegen die in [13 §6](13-import-linear.md) definierten Operationen.
**Tests:** Filter-Bar erzeugt korrektes Filter-JSON; Inbox-Aktionen.

---

## Welle 4

### WP15 – Sync-Engine

**Spezifikation:** [09 §5](09-api-events.md), [11 §3.7 und §4](11-implementierungsarchitektur.md).
**Eigentum:** `crates/lineal-sync/**`, `lineal-jobs/src/consumers/sync.rs`, Migrationen `1500`–`1599`, `web/src/lib/store/**`; in `web/src/features/**` nur Austausch der Datenquelle (Hooks) für Issues, Projekte, Notifications.
**Liefert:** `sync_log`, Bootstrap-Endpunkt, WebSocket mit `since`, Filter nach sichtbaren Teams, Reconnect; Client-Store (MobX + IndexedDB), optimistische Mutationen mit Rollback.
**Tests:** User sieht keine SyncActions privater Teams; Reconnect liefert Lücke nach.

### WP16 – Webhooks, OAuth, Team-Lebenszyklus

**Spezifikation:** [09 §2, §4](09-api-events.md), [07 §1](07-mcp-server.md) (OAuth 2.1 + PKCE + DCR).
**Eigentum:** `usecases/{webhook,team_lifecycle,invitation}.rs`, `ports/repos/webhook.rs`, `lineal-db/src/repos/webhook.rs` (Team-Lebenszyklus-Methoden in neuer Datei `repos/team_lifecycle.rs`), Migrationen `1600`–`1699`, `lineal-api/src/oauth/**`, `lineal-jobs/src/consumers/webhooks.rs`, `graphql/` neue Datei `webhook.rs` (Orchestrator ergänzt Root).
**Liefert:** Webhook-CRUD, Zustellung mit HMAC-Signatur `Lineal-Signature`, Retries 1 min/1 h/6 h, Deaktivierung; OAuth `/oauth/register|authorize|token` mit PKCE, Refresh-Rotation, Scopes; MCP akzeptiert OAuth-Tokens. Aus WP02 übernommen: Einladungen (invite/accept/resend/revoke), Team-Key-Änderung mit Historie und Alias, Copy settings, Team retire/restore/delete (30 Tage).
**Tests:** Signatur prüfbar, Retry-Plan, PKCE-Fehlerfälle, MCP-Call mit OAuth-Token.

### WP18 – Import aus Linear

**Spezifikation:** [13](13-import-linear.md) komplett.
**Eigentum:** neues Crate `crates/lineal-import/**` (Orchestrator trägt es beim Merge in den Workspace ein; das WP darf das Workspace-`Cargo.toml` dafür ausnahmsweise ergänzen), `usecases/import.rs`, `ports/repos/import.rs`, `lineal-db/src/repos/import.rs`, Migrationen `1800`–`1899`, `graphql/import.rs` (Orchestrator ergänzt Root), `lineal-server/src/cli/import.rs`, `lineal-jobs/src/periodic/import_runner.rs`.
**Liefert:** `ImportSource`-Port mit Linear-GraphQL-Adapter (Paginierung, Rate-Limit-Steuerung über Header) und CSV-Adapter, Fake-Quelle aus JSON-Fixtures, Phasen 1–8 mit Checkpoints, `import_mapping`-Upserts, Nummernübernahme und Zähler-Setzen, Markdown-Umschreibung, Datei-Download in den Blob-Store, Dry-Run, Delta-Import mit Konflikterkennung, `origin = import` für alle Events, verschlüsselte Token-Ablage, GraphQL-Operationen und CLI.
**Tests:** Abnahmekriterien 1–6 aus [13 §9](13-import-linear.md) gegen die Fake-Quelle. **Kein Test darf gegen das echte Linear laufen.**

## Welle 5

### WP17 – Härtung und E2E

**Eigentum:** alle `lineal-db/src/repos/*.rs` (nur Umstellung auf `query!`/`query_as!`), `.sqlx/`, `deny.toml`, `web/e2e/**`, `scripts/check.sh` (Erweiterung).
**Liefert:** compile-time-geprüfte Queries (`cargo sqlx prepare`, `SQLX_OFFLINE=true` in CI), `cargo-deny`, `npm audit` im Check, Playwright-E2E für F1, F3, F5, F6, F7, F12, F13, F17, Security-Review-Checkliste aus [10 §11.4](10-architektur-umsetzung.md#114-sicherheitsvorgaben-für-die-umsetzung) abgehakt.

---

## Briefing-Vorlage für Agenten

```text
Du implementierst Arbeitspaket <WP> von Lineal (Linear-Nachbau) im aktuellen Git-Worktree.
1. Lies docs/00-konventionen.md, docs/11-implementierungsarchitektur.md, docs/12-arbeitspakete.md (Globale Vorgaben + dein WP) und die dort genannten Fachkapitel.
2. Lege Branch wp/<nr>-<name> an (falls nicht vorhanden). Ändere NUR Dateien aus deiner Eigentumsliste.
3. Arbeite testgetrieben: erst Test, dann Implementierung. Halte dich an die Interfaces aus Kapitel 11.
4. docker compose up -d (Postgres :5440) falls nicht läuft; DATABASE_URL=postgres://lineal:lineal@localhost:5440/lineal; CARGO_BUILD_JOBS=4.
5. Fertig = scripts/check.sh grün, alles committet (Conventional Commits, Abschluss-Zeile Co-Authored-By).
6. Rufe niemals Linear-MCP-Tools auf. Nur erfundene Daten.
7. Abschlussbericht (max. 15 Zeilen): Branch, letzter Commit-SHA, was fertig ist, was offen ist, geänderte Dateien außerhalb der Eigentumsliste (sollte leer sein) und neue Abhängigkeiten.
```
