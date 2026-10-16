<p align="center">
  <img src="docs/assets/lineal-logo.svg" alt="Lineal" width="560">
</p>

<p align="center">
  <strong>Issue-Tracking für Teams – ein Nachbau von Linear</strong><br>
  Rust-Backend (GraphQL, MCP, Echtzeit-Sync, CRDT) · React-Frontend · selbst gehostet per Docker
</p>

---

## Was Lineal kann

- **Issues & Workflow:** Teams mit eigenen Status, Labels, Estimates, Triage, SLA, Sub-Issues,
  Relationen, wiederkehrenden Issues, Templates (auch Formulare), Mehrfachauswahl und Bulk-Aktionen,
  Papierkorb mit Wiederherstellen.
- **Planung:** Projekte mit Meilensteinen und Status-Updates, Initiativen, Cycles mit Kapazität,
  Roadmap-Zeitachse, firmenweite Übersicht über alle Teams.
- **Views, Suche, Inbox:** gespeicherte Filter-Views, Favoriten, Volltextsuche, Command-Menü,
  Inbox mit Priority-Tabs, Snooze und „Remind me“.
- **Dokumente:** Echtzeit-Mitbearbeitung (Yjs/CRDT) mit Inline-Kommentaren, Historie und Templates.
- **Zusammenarbeit:** Echtzeit-Sync per WebSocket, Benachrichtigungen (In-App und E-Mail-Digest),
  Freigabe einzelner Issues an Gäste.
- **Integration:** GraphQL-API, MCP-Server für KI-Agenten (55 Linear-kompatible Tools), OAuth 2.1,
  API-Keys mit Team-Beschränkung, Webhooks, Import aus Linear inkl. regelmäßigem Abgleich.
- **Betrieb & Sicherheit:** Rollen und Security-Settings, Audit-Log, Rate-Limits, CSRF/CSP,
  Backup/Restore-Skripte, Ersteinrichtung per CLI.

Die vollständige, sprachneutrale Spezifikation liegt in [`docs/`](docs/README.md)
(als HTML: [`docs/index.html`](docs/index.html)); den aktuellen Umsetzungsstand beschreibt
[`docs/15-umsetzungsstand.md`](docs/15-umsetzungsstand.md).

## Betrieb mit Docker

```bash
cp .env.example .env    # LINEAL_SESSION_SECRET und LINEAL_DB_PASSWORD setzen (openssl rand -hex 32)
docker compose --profile app up -d --build
docker compose --profile app run --rm migrate create-workspace \
  --name "Meine Firma" --owner-email du@example.com --owner-name "Dein Name"
# → http://localhost:8088
```

Details zu Umgebungsvariablen, TLS, Backups und Updates: [`docs/15-umsetzungsstand.md`](docs/15-umsetzungsstand.md) §3.

## Entwicklung

### Voraussetzungen

- Rust stable (Edition 2024, `rustfmt` und `clippy`; `rust-toolchain.toml` ist hinterlegt)
- Node 22+ mit npm
- Docker (PostgreSQL 16 auf `127.0.0.1:5440`)

### Schnellstart

```bash
cp .env.example .env
docker compose up -d --wait               # Projektname "lineal", Postgres :5440
cargo run -p lineal-server -- migrate     # Migrationen anwenden
cargo run -p lineal-server -- seed        # Beispiel-Workspace acme (nur mit LINEAL_DEV=1, Passwort lineal-dev)
cargo run -p lineal-server -- serve       # http://localhost:8080  (GET /healthz)
cargo run -p lineal-server -- worker      # Outbox-Konsumenten und periodische Jobs
cd web && npm install && npm run dev      # http://localhost:5173 (Proxy auf :8080)
```

Einen echten Workspace samt Owner legt `lineal create-workspace --name … --owner-email … --owner-name …`
an (Passwort aus `LINEAL_OWNER_PASSWORD` oder interaktiv); Betrieb und Docker: docs/15 §3.2b/§3.2c.

`scripts/dev.sh` startet DB, Migrationen, Server und Web-Dev-Server in einem Schritt
(`SEED=1 scripts/dev.sh` legt zusätzlich die Beispieldaten an). Belegt ein anderer Prozess `:8080`,
setze `LINEAL_BIND=127.0.0.1:18080` und für das Web `LINEAL_BACKEND=http://localhost:18080`.

`cargo test` und `#[sqlx::test]` brauchen eine laufende Datenbank; `DATABASE_URL` hat per
`.cargo/config.toml` den Default `postgres://lineal:lineal@localhost:5440/lineal`.

## Prüfen

```bash
scripts/check.sh
```

führt aus: `docker compose up -d --wait`, `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`, Schema-Export nach `web/schema.graphql` (wird neu erzeugt), `npm ci`, `typecheck`,
`lint`, `test`.

## Aufbau

| Pfad | Inhalt |
|---|---|
| `crates/lineal-domain` | IDs, Enums, Regeln, Domain-Events (rein, kein I/O) |
| `crates/lineal-app` | Use-Cases, Ports (`Store`/`Tx`, Repos, Dienste), Policy, Referenzauflösung |
| `crates/lineal-db` | PostgreSQL-Adapter (`PgStore`/`PgTx`), `migrations/` |
| `crates/lineal-api` | axum-Router, GraphQL (`MergedObject` pro Fachbereich), REST/OAuth-Stubs |
| `crates/lineal-mcp` | MCP-Tool-Dispatch (55 kompatible Tools + `get_workspace_overview`), Handler pro Gruppe |
| `crates/lineal-sync` | WebSocket-Delta-Stream |
| `crates/lineal-jobs` | Outbox-Konsumenten, periodische Jobs, Worker-Schleife |
| `crates/lineal-server` | Binary `lineal`: `serve`, `worker`, `migrate`, `seed`, `schema-export` |
| `crates/lineal-testkit` | Seed-Daten, `FakeStore`, Test-Helfer (siehe dessen README) |
| `web/` | Vite, React 19, TypeScript strict, Tailwind v4, shadcn/ui, TanStack Router/Query |

Abhängigkeiten zeigen nach innen; nur `lineal-server` kennt alle Crates. Geschäftsregeln leben
ausschließlich in `lineal-domain` und `lineal-app`.

## Konventionen

- Conventional Commits; ein Arbeitspaket = ein Branch `wp/<nr>-<name>`.
- `#![forbid(unsafe_code)]`, kein `unwrap()` außerhalb von Tests, keine SQL-String-Interpolation.
- Fehlercodes und Enums exakt nach [`docs/00-konventionen.md`](docs/00-konventionen.md).
- Neue Migrationen nur im eigenen Nummernbereich (siehe `docs/12-arbeitspakete.md`).
- npm statt pnpm; das generierte `web/src/routeTree.gen.ts` und `web/src/gql/` sind nicht eingecheckt.

## Lizenz

[MIT](LICENSE)
