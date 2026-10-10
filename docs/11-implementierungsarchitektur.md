# 11 – Implementierungsarchitektur (Rust + TypeScript)

Dieses Kapitel übersetzt die sprachneutrale Spezifikation (Kapitel 00–10) in eine konkrete Code-Struktur für den gewählten Stack ([10 §11](10-architektur-umsetzung.md#11-technologie-stack-entscheidung)). Es ist die verbindliche Vorlage für das Scaffold und alle Arbeitspakete in [12-arbeitspakete.md](12-arbeitspakete.md).

Leitprinzipien:

1. **Ein Domain-Core, dünne Adapter.** Geschäftsregeln leben ausschließlich in `lineal-domain` (rein, ohne I/O) und `lineal-app` (Use-Cases, Rechte, Transaktionen). GraphQL, MCP, Sync und Jobs sind Adapter und rufen nur `lineal-app` auf.
2. **Abhängigkeiten zeigen nach innen.** Kein inneres Crate kennt ein äußeres.
3. **Gemeinsame Dateien entstehen einmal im Scaffold.** Alles, was mehrere Arbeitspakete anfassen würden (Workspace-`Cargo.toml`, Port-Traits, GraphQL-Root, MCP-Dispatch, Datenbankschema, Router), wird vorab mit Platzhaltern angelegt. Arbeitspakete füllen nur Dateien aus ihrer Eigentumsliste.

## 1. Repository-Layout

```text
lineal/
├── Cargo.toml                  # Workspace, gemeinsame Dependency-Versionen
├── rust-toolchain.toml         # stable
├── docker-compose.yml          # name: lineal; PostgreSQL :5440
├── .env.example
├── scripts/
│   ├── check.sh                # komplette Prüfung (CI und lokal)
│   └── dev.sh                  # DB starten, Migrationen, Server + Web
├── crates/
│   ├── lineal-domain/          # Werte, Enums, IDs, reine Regeln, Domain-Events
│   ├── lineal-app/             # Use-Cases, Ports (Traits), Policy, Referenzauflösung
│   ├── lineal-db/              # PostgreSQL-Implementierung der Ports + migrations/
│   ├── lineal-api/             # axum-Router, GraphQL (async-graphql), REST-Endpunkte, Auth-Middleware
│   ├── lineal-mcp/             # MCP-Server (Streamable HTTP), Tool-Handler
│   ├── lineal-sync/            # WebSocket-Delta-Stream, Bootstrap
│   ├── lineal-jobs/            # Job-Runner, Outbox-Konsumenten, periodische Jobs
│   ├── lineal-server/          # Binary `lineal`: Composition Root, Konfiguration, Subcommands serve/worker/migrate
│   └── lineal-testkit/         # Seed-Daten (acme, ENG, WEB, DES, Alice, Bob, Carol), Test-Helfer
├── web/                        # Vite + React + TypeScript
│   ├── src/
│   │   ├── app/                # Shell: Layout, Sidebar, Theme, Provider
│   │   ├── components/ui/      # shadcn/ui-Bausteine
│   │   ├── features/<feature>/ # pro Fachbereich: Komponenten, Hooks, GraphQL-Dokumente
│   │   ├── lib/                # GraphQL-Client, Shortcut-Registry, Store
│   │   ├── routes/             # TanStack Router (dateibasiert)
│   │   └── gql/                # generiert durch graphql-codegen (nicht einchecken)
│   └── schema.graphql          # exportiert aus dem Backend (eingecheckt)
└── docs/                       # diese Dokumentation
```

## 2. Crate-Abhängigkeiten

```mermaid
flowchart BT
  DOMAIN["lineal-domain<br/>rein, kein I/O"]
  APP["lineal-app<br/>Use-Cases, Ports, Policy"]
  DB["lineal-db<br/>PostgreSQL-Adapter"]
  API["lineal-api<br/>axum + GraphQL"]
  MCP["lineal-mcp<br/>MCP-Server"]
  SYNC["lineal-sync<br/>WebSocket"]
  JOBS["lineal-jobs<br/>Worker"]
  SERVER["lineal-server<br/>Binary, Verdrahtung"]
  TESTKIT["lineal-testkit<br/>nur dev-dependency"]
  APP --> DOMAIN
  DB --> APP
  API --> APP
  MCP --> APP
  SYNC --> APP
  JOBS --> APP
  SERVER --> DB
  SERVER --> API
  SERVER --> MCP
  SERVER --> SYNC
  SERVER --> JOBS
  TESTKIT --> DB
```

Regeln:
- `lineal-domain` hängt nur von `serde`, `uuid`, `chrono`, `chrono-tz`, `thiserror` und reinen Regel-Crates (z. B. `pulldown-cmark`, `ammonia`) ab – nie von I/O-Crates.
- `lineal-app` kennt keine Datenbank, kein HTTP. Es definiert die **Ports** als Traits.
- Adapter (`api`, `mcp`, `sync`, `jobs`) kennen `lineal-app`, aber nicht `lineal-db`. Sie erhalten zur Laufzeit ein `Arc<App>`.
- Nur `lineal-server` kennt alle Crates und verdrahtet sie (Composition Root).
- `lineal-testkit` ist ausschließlich `dev-dependency`.

## 3. Schichten und Kern-Typen

### 3.1 Domain (`lineal-domain`)

| Modul | Inhalt |
|---|---|
| `ids` | Newtypes pro Entität: `OrgId`, `UserId`, `TeamId`, `IssueId`, `StateId`, `LabelId`, `CycleId`, `ProjectId`, `MilestoneId`, `InitiativeId`, `CommentId`, `DocumentId`, `TemplateId`, `ViewId`, `NotificationId` … (je `struct XId(Uuid)`, `Copy`, `serde` transparent). Verwechslungen sind Kompilierfehler. |
| `enums` | `StateType`, `Priority`, `TeamVisibility`, `UserRole`, `EstimationType`, `RelationType`, `ProjectStatusType`, `InitiativeStatus`, `Health`, `LabelGroupType`, `SlaType`, `TemplateType` – exakt die Werte aus [00 §4](00-konventionen.md#4-feste-enums). |
| `identifier` | `TeamKey` (validiert `^[A-Z][A-Z0-9]{0,6}$`), `IssueIdentifier`, `ProjectIdentifier`, Parser `parse_identifier` ([01 §6](01-domaenenmodell.md)). |
| `rules::*` | Reine Funktionen: Statuswechsel-Zeitstempel, Schätzskalen, Label-Gruppenregel, Fortschrittsformel, Cycle-Kalender, SLA-Arbeitstage, Patch-Operationen, Filter-AST. |
| `events` | `DomainEvent` (Enum aller Ereignisse aus [09](09-api-events.md)), serialisierbar für die Outbox. |
| `error` | `DomainError` (Validierungsfehler mit Feld und Grund). |

### 3.2 Application (`lineal-app`)

```rust
/// Wer handelt in welchem Workspace. Wird von jedem Adapter aus Session, API-Key oder OAuth-Token gebaut.
pub struct RequestCtx {
    pub org_id: OrgId,
    pub actor: Actor,            // User { id, role } | App { id } | System
    pub request_id: String,
    pub scopes: Scopes,          // read / write / admin
}

/// Zentrale Fassade, die Adapter benutzen.
pub struct App {
    pub store: Arc<dyn Store>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub blobs: Arc<dyn BlobStore>,
    pub search: Arc<dyn SearchIndex>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub config: AppConfig,
}
```

Use-Cases sind Methoden auf `App`, verteilt auf eigene Dateien pro Fachbereich (`impl App` darf in mehreren Modulen stehen):

```rust
// crates/lineal-app/src/usecases/issues.rs
impl App {
    pub async fn create_issue(&self, ctx: &RequestCtx, input: CreateIssueInput) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;           // Transaktion
        let team = resolve::team(&mut *tx, ctx, &input.team).await?; // Referenzauflösung inkl. Sichtbarkeit
        policy::require(ctx, Action::CreateIssue, &team)?;           // Rechte
        let number = tx.teams().next_issue_number(team.id).await?;   // atomarer Zähler
        // ... Regeln aus lineal-domain anwenden, speichern, History schreiben
        tx.outbox().push(DomainEvent::IssueCreated { .. }).await?;   // Outbox im selben Commit
        tx.commit().await?;
        Ok(view)
    }
}
```

**Ports** (alle `#[async_trait]`, objektsicher):

```rust
pub trait Store: Send + Sync {
    async fn begin(&self, org: OrgId) -> AppResult<Box<dyn Tx>>;
}

/// Eine Datenbank-Transaktion mit Zugriff auf alle Repositories.
/// Die Accessor-Liste ist vollständig im Scaffold angelegt; Arbeitspakete ergänzen nur Methoden in ihren Repo-Traits.
pub trait Tx: Send {
    fn orgs(&mut self) -> &mut dyn OrgRepo;
    fn users(&mut self) -> &mut dyn UserRepo;
    fn auth(&mut self) -> &mut dyn AuthRepo;
    fn teams(&mut self) -> &mut dyn TeamRepo;
    fn states(&mut self) -> &mut dyn WorkflowStateRepo;
    fn labels(&mut self) -> &mut dyn LabelRepo;
    fn issues(&mut self) -> &mut dyn IssueRepo;
    fn comments(&mut self) -> &mut dyn CommentRepo;
    fn attachments(&mut self) -> &mut dyn AttachmentRepo;
    fn cycles(&mut self) -> &mut dyn CycleRepo;
    fn projects(&mut self) -> &mut dyn ProjectRepo;
    fn initiatives(&mut self) -> &mut dyn InitiativeRepo;
    fn views(&mut self) -> &mut dyn ViewRepo;
    fn notifications(&mut self) -> &mut dyn NotificationRepo;
    fn documents(&mut self) -> &mut dyn DocumentRepo;
    fn templates(&mut self) -> &mut dyn TemplateRepo;
    fn triage(&mut self) -> &mut dyn TriageRepo;
    fn webhooks(&mut self) -> &mut dyn WebhookRepo;
    fn outbox(&mut self) -> &mut dyn OutboxRepo;
    fn jobs(&mut self) -> &mut dyn JobRepo;
    async fn commit(self: Box<Self>) -> AppResult<()>;
}
```

Weitere Ports: `Clock`, `IdGen`, `BlobStore` (signierte PUT/GET-URLs), `SearchIndex`, `PasswordHasher`, `Mailer`.

**Policy und Sichtbarkeit** (`lineal-app/src/policy/`): `visible_team_ids(ctx)` liefert die für den Akteur sichtbaren Teams (gecacht pro Request); jede Listenabfrage bekommt diese Menge als Parameter. `require(ctx, action, resource)` setzt die Matrix aus [02 §6](02-teams-rechte.md) um. Für private Teams lautet der Fehler `not_found`.

**Referenzauflösung** (`lineal-app/src/resolve/`): eine Funktion pro Entitätstyp nach der Reihenfolge aus [00 §2](00-konventionen.md#2-identifikatoren); sie läuft immer durch den Sichtbarkeitsfilter.

**Fehler:** `AppError { kind: ErrorKind, message, details: serde_json::Value }` mit `ErrorKind` = `InvalidRequest | Unauthenticated | Forbidden | NotFound | AmbiguousReference | Conflict | LimitExceeded | RateLimited | Internal`. Die Adapter bilden das 1:1 auf das Fehlermodell aus [00 §7](00-konventionen.md#7-fehlermodell) ab.

### 3.3 Persistenz (`lineal-db`)

- `PgStore` implementiert `Store`; `PgTx` hält eine `sqlx::Transaction<'static, Postgres>` und implementiert **alle** Repo-Traits, je Trait in einer eigenen Datei `src/repos/<bereich>.rs`.
- Jede Abfrage ist parametrisiert und enthält `organization_id = $1`.
- Migrationen: `crates/lineal-db/migrations/<NNNN>_<name>.sql`. `0001` legt die Infrastruktur-Tabellen an (Outbox, Jobs, Sync-Sequenz), `0010`–`0099` das komplette Schema aus [01](01-domaenenmodell.md). Arbeitspakete dürfen nur Migrationen in ihrem zugewiesenen Nummernbereich hinzufügen (siehe [12](12-arbeitspakete.md)), z. B. für zusätzliche Indizes.
- Tests: `#[sqlx::test(migrations = "./migrations")]` – jede Testfunktion bekommt eine frische Datenbank; parallele Arbeitspakete teilen sich gefahrlos denselben PostgreSQL-Container.

### 3.4 Adapter

**GraphQL (`lineal-api`).** Die Root-Typen sind `MergedObject`s aus Teilobjekten pro Fachbereich; jedes Teilobjekt liegt in einer eigenen Datei und ist im Scaffold mit einem versteckten Platzhalterfeld angelegt:

```rust
#[derive(MergedObject, Default)]
pub struct QueryRoot(OrgQuery, TeamQuery, IssueQuery, LabelQuery, CommentQuery, CycleQuery,
                     ProjectQuery, InitiativeQuery, ViewQuery, SearchQuery, NotificationQuery,
                     DocumentQuery, TemplateQuery, TriageQuery);
#[derive(MergedObject, Default)]
pub struct MutationRoot(/* dieselben Bereiche als *Mutation */);
```

Der `RequestCtx` und `Arc<App>` liegen in den GraphQL-Context-Daten. Das Schema wird mit `lineal schema-export` nach `web/schema.graphql` exportiert; `scripts/check.sh` **erzeugt** die Datei neu (statt sie zu vergleichen), damit parallele Pakete nicht an ihr scheitern. Der Orchestrator committet sie nach jedem Merge.

**MCP (`lineal-mcp`).** `tools/list` liefert genau den Inhalt von `docs/mcp-tools.snapshot.json` (zur Build-Zeit eingebettet). `tools/call` verteilt nach Tool-Namen an eine Handler-Datei pro Gruppe aus [07](07-mcp-server.md) (`handlers/issues.rs`, `handlers/projects.rs` …). Nicht implementierte Tools antworten mit `isError: true` und `error: "internal"`, Meldung `not implemented`. Eingaben werden gegen das Snapshot-Schema validiert, bevor der Handler läuft.

**HTTP-Oberfläche:**

| Pfad | Zweck |
|---|---|
| `POST /graphql`, `GET /graphql` (nur dev: GraphiQL) | öffentliche API |
| `POST /mcp`, `GET /mcp` | MCP Streamable HTTP |
| `/.well-known/oauth-protected-resource`, `/.well-known/oauth-authorization-server` | MCP-Auth-Discovery |
| `/oauth/register`, `/oauth/authorize`, `/oauth/token` | OAuth-2.1-Authorization-Server |
| `POST /auth/login`, `POST /auth/logout`, `GET /auth/me` | Session-Login der Web-App |
| `PUT /uploads/{token}`, `GET /files/{token}` | signierte Upload-/Download-URLs (lokaler Blob-Store) |
| `GET /sync` (WebSocket) | Delta-Stream |
| `GET /healthz`, `GET /readyz` | Betrieb |

### 3.5 Request-Pipeline

```mermaid
sequenceDiagram
  participant C as Client
  participant MW as "axum Middleware"
  participant AD as "Adapter GraphQL oder MCP"
  participant APP as "App Use-Case"
  participant POL as Policy
  participant TX as "PgTx"
  participant OB as Outbox
  participant W as Worker
  C->>MW: HTTP Request
  MW->>MW: Request-ID, Rate-Limit, Auth zu RequestCtx
  MW->>AD: Request mit RequestCtx
  AD->>APP: Use-Case mit typisiertem Input
  APP->>TX: begin
  APP->>TX: Referenzen aufloesen mit Sichtbarkeitsfilter
  APP->>POL: require Aktion
  APP->>APP: Domain-Regeln anwenden
  APP->>TX: schreiben, History
  APP->>OB: Event in derselben Transaktion
  APP->>TX: commit, lastSyncId
  APP-->>AD: Ergebnis oder AppError
  AD-->>C: JSON, Fehler nach Fehlermodell
  W->>OB: Events abholen mit SKIP LOCKED
  W->>W: Notifications, Webhooks, Suchindex, Sync-Broadcast
```

### 3.6 Jobs und Outbox (`lineal-jobs`)

- `lineal worker` startet eine Schleife: `SELECT … FROM outbox_event WHERE processed_at IS NULL ORDER BY sync_id FOR UPDATE SKIP LOCKED LIMIT 100`.
- Konsumenten implementieren `trait EventConsumer { fn name(&self) -> &'static str; async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()>; }`. Zustellung pro Konsument wird in `outbox_delivery(consumer, event_id)` vermerkt, damit jeder Konsument idempotent genau einmal verarbeitet.
- Periodische Jobs (Cycle-Wechsel, Auto-Close, Auto-Archive, Purge, SLA, Snooze, Erinnerungen) implementieren `trait PeriodicJob { fn schedule(&self) -> Schedule; async fn run(&self, app: &App, now: OffsetDateTime) -> AppResult<()>; }` und laufen über die Tabelle `job` mit Sperre, damit mehrere Worker sicher sind.

### 3.7 Sync (`lineal-sync`)

Umsetzung von [09 §5](09-api-events.md): `GET /sync?since=<lastSyncId>` öffnet einen WebSocket; der Server sendet zuerst fehlende `SyncAction`s aus `sync_log`, danach live alle neuen, gefiltert nach den sichtbaren Teams des Users. Der Broadcast wird vom Outbox-Konsumenten `sync` gespeist (PostgreSQL `LISTEN/NOTIFY` als Wecksignal).

## 4. Frontend-Architektur

```mermaid
flowchart LR
  subgraph Browser
    UI["React-Komponenten<br/>shadcn/ui, Tailwind"]
    KEYS["Shortcut-Registry<br/>und Command-Menue"]
    STORE["Sync-Store<br/>MobX + IndexedDB"]
    GQL["GraphQL-Client<br/>generierte Typen"]
    WS["WebSocket-Client"]
  end
  API["/graphql"]
  SYNC["/sync"]
  UI --> STORE
  UI --> GQL
  KEYS --> UI
  GQL --> API
  STORE --> WS
  WS --> SYNC
  GQL --> STORE
```

- **Bis zur Sync-Welle** lesen und schreiben Features über TanStack Query + GraphQL. Der Sync-Store ersetzt danach das Lesen für Issues, Projekte und Notifications; Schreibzugriffe laufen weiterhin als GraphQL-Mutationen, optimistisch im Store angewendet.
- **Feature-Ordner** (`src/features/issues`, `…/planning`, `…/inbox`, `…/views`, `…/docs`, `…/settings`) gehören jeweils genau einem Arbeitspaket.
- **Routen** sind dateibasiert (`src/routes/…`); die generierte `routeTree.gen.ts` ist nicht eingecheckt, sondern wird von `npm run build` und `npm run typecheck` erzeugt. Dadurch gibt es keine zentrale Routendatei mit Merge-Konflikten.
- **Design-System:** Farben, Abstände, Radien und Schriften als CSS-Variablen in `src/app/theme.css`; hell/dunkel per `class="dark"`. Bausteine ausschließlich aus `components/ui`.
- **Tastatur:** `lib/shortcuts.ts` registriert Shortcuts mit Kontext (`global`, `list`, `issue`, `modal`) und Sequenzen (`g i`). Jede Aktion wird zusätzlich als Command im Menü registriert.

## 5. Entwicklungsumgebung

```bash
cp .env.example .env
docker compose up -d                      # Projektname "lineal": in allen Worktrees derselbe Container auf :5440
cargo run -p lineal-server -- migrate     # Migrationen
cargo run -p lineal-server -- seed        # Beispiel-Workspace acme
cargo run -p lineal-server -- serve       # http://localhost:8080
cargo run -p lineal-server -- worker      # Jobs
cd web && npm install && npm run dev      # http://localhost:5173 (Proxy auf :8080)
scripts/check.sh                          # alles prüfen
```

`.env.example`:

```text
DATABASE_URL=postgres://lineal:lineal@localhost:5440/lineal
LINEAL_BASE_URL=http://localhost:8080
LINEAL_SESSION_SECRET=change-me-32-bytes-minimum-xxxxxxxx
LINEAL_BLOB_DIR=./var/blobs
RUST_LOG=info,lineal=debug
```

## 6. Teststrategie pro Schicht

| Schicht | Werkzeug | Was |
|---|---|---|
| `lineal-domain` | `cargo test` | reine Regeln, Tabellentests, Property-Tests wo sinnvoll |
| `lineal-app` + `lineal-db` | `#[sqlx::test]` mit `lineal-testkit::seed` | Use-Cases gegen echte DB: die zehn Kernregeln aus [10 §8.2](10-architektur-umsetzung.md) |
| `lineal-api` | `#[sqlx::test]` + Request gegen den Router (`tower::ServiceExt::oneshot`) | GraphQL-Contract, Fehlermodell, Auth |
| `lineal-mcp` | wie API | `tools/list` = Snapshot, Contract je Tool, `isError`-Abbildung |
| `web` | Vitest + Testing Library | Komponenten, Shortcut-Registry, Store |
| E2E | Playwright | Flows F1–F17 aus [08](08-userflows.md) |

## 7. Coding-Konventionen

- Rust: `cargo fmt`, `clippy -D warnings`, kein `unwrap()` außerhalb von Tests, `#![forbid(unsafe_code)]` in allen Crates.
- Keine SQL-Strings mit Interpolation; nur `bind`.
- Jede öffentliche Use-Case-Methode hat mindestens einen `#[sqlx::test]`.
- TypeScript: `strict`, kein `any`, ESLint + Prettier.
- Commits: Conventional Commits (`feat(issues): …`), ein Arbeitspaket = ein Branch `wp/<nr>-<name>`.

## 8. Abweichungen aus dem Scaffold (S1/S2, verbindlich für alle WPs)

- **`lineal-sync` kennt `sqlx`** (Ausnahme von der Adapter-Regel in §2): `LISTEN/NOTIFY` auf dem Kanal `lineal_sync` braucht einen `PgListener`/`PgPool`; alles andere (Auth, Sichtbarkeit, `sync_log`) läuft über die Ports `Store`/`Tx`.


- `Tx` hat zusätzlich `org_id()`, `last_sync_id()`, `overview()` und `imports()`; `Store::begin_system()` für Worker ohne Benutzerkontext. `OutboxRepo` hat `claim_undelivered`, `mark_delivered`, `mark_processed`; `PeriodicJob` hat `name()`.
- `/mcp` und `/.well-known/*` liefert `lineal_mcp::router`, `/sync` liefert `lineal_sync::router`; `lineal_server::build_router` setzt sie mit dem API-Router zusammen.
- GraphQL-Platzhalter sind sichtbare Felder `_<bereich>Ready` / `_<bereich>MutationReady`; das zuständige WP entfernt seinen Platzhalter, sobald es echte Felder hat.
- Server-Adresse über `LINEAL_BIND` (Default `127.0.0.1:8080`), Vite-Proxy-Ziel über `LINEAL_BACKEND`.
- `lineal-testkit` ist optionale Laufzeit-Abhängigkeit von `lineal-server` (Feature `seed`) für das Subcommand `seed`.
- Schema-Eigenheiten: Tabelle `"user"` immer in Anführungszeichen; `issue.identifier` und `issue.state_type` pflegen Trigger; `updated_at`/`version` setzt der Core; ein Issue mit `project_id` braucht eine `project_team`-Zeile für sein Team.
