# Lineal

Nachbau von Linear (Issue-Tracking) mit Rust-Backend (GraphQL, MCP, Sync) und React-Frontend.
Die Spezifikation liegt in [`docs/`](docs/README.md); Architektur in
[`docs/11-implementierungsarchitektur.md`](docs/11-implementierungsarchitektur.md), Arbeitspakete in
[`docs/12-arbeitspakete.md`](docs/12-arbeitspakete.md).

## Voraussetzungen

- Rust stable (Edition 2024, `rustfmt` und `clippy`; `rust-toolchain.toml` ist hinterlegt)
- Node 22+ mit npm
- Docker (PostgreSQL 16 auf `127.0.0.1:5440`)

## Schnellstart

```bash
cp .env.example .env
docker compose up -d --wait               # Projektname "lineal", Postgres :5440
cargo run -p lineal-server -- migrate     # Migrationen anwenden
cargo run -p lineal-server -- seed        # Beispiel-Workspace acme (im Scaffold noch leer)
cargo run -p lineal-server -- serve       # http://localhost:8080  (GET /healthz)
cargo run -p lineal-server -- worker      # Outbox-Konsumenten und periodische Jobs
cd web && npm install && npm run dev      # http://localhost:5173 (Proxy auf :8080)
```

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
