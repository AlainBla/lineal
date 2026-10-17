<p align="center">
  <img src="docs/assets/lineal-logo.svg" alt="Lineal" width="560">
</p>

<p align="center">
  <strong>Issue tracking for teams – a Linear clone</strong><br>
  Rust backend (GraphQL, MCP, real-time sync, CRDT) · React frontend · self-hosted with Docker
</p>

<p align="center">
  <img src="docs/assets/screenshot-board.png" alt="Lineal board view with issues across Backlog, Todo, In Progress and Done" width="900">
</p>

<p align="center">
  <img src="docs/assets/screenshot-list.png" alt="Lineal list view with issues grouped by status, showing priority, labels, due dates and assignees" width="900">
</p>

---

## Features

- **Issues & workflow:** teams with their own statuses, labels, estimates, triage, SLAs, sub-issues,
  relations, recurring issues, templates (including forms), multi-select and bulk actions,
  trash with restore.
- **Planning:** projects with milestones and status updates, initiatives, cycles with capacity,
  roadmap timeline, company-wide overview across all teams.
- **Views, search, inbox:** saved filter views, favorites, full-text search, command menu,
  inbox with priority tabs, snooze and "Remind me".
- **Documents:** real-time co-editing (Yjs/CRDT) with inline comments, history and templates.
- **Collaboration:** real-time sync over WebSocket, notifications (in-app and email digest),
  sharing individual issues with guests.
- **Integrations:** GraphQL API, MCP server for AI agents (55 Linear-compatible tools), OAuth 2.1,
  team-scoped API keys, webhooks, import from Linear including scheduled sync.
- **Operations & security:** roles and security settings, audit log, rate limits, CSRF/CSP,
  backup/restore scripts, first-time setup via CLI.

The full, implementation-neutral specification lives in [`docs/`](docs/README.md)
(as HTML: [`docs/index.html`](docs/index.html)); the current implementation status is described in
[`docs/15-umsetzungsstand.md`](docs/15-umsetzungsstand.md). The documentation is written in German.

## Running with Docker

```bash
cp .env.example .env    # set LINEAL_SESSION_SECRET and LINEAL_DB_PASSWORD (openssl rand -hex 32)
docker compose --profile app up -d --build
docker compose --profile app run --rm migrate create-workspace \
  --name "My Company" --owner-email you@example.com --owner-name "Your Name"
# → http://localhost:8088
```

Environment variables, TLS, backups and updates: [`docs/15-umsetzungsstand.md`](docs/15-umsetzungsstand.md) §3.

## Development

### Requirements

- Rust stable (edition 2024, `rustfmt` and `clippy`; a `rust-toolchain.toml` is included)
- Node 22+ with npm
- Docker (PostgreSQL 16 on `127.0.0.1:5440`)

### Quick start

```bash
cp .env.example .env
docker compose up -d --wait               # project name "lineal", Postgres :5440
cargo run -p lineal-server -- migrate     # apply migrations
cargo run -p lineal-server -- seed        # sample workspace acme (LINEAL_DEV=1 only, password lineal-dev)
cargo run -p lineal-server -- serve       # http://localhost:8080  (GET /healthz)
cargo run -p lineal-server -- worker      # outbox consumers and periodic jobs
cd web && npm install && npm run dev      # http://localhost:5173 (proxies to :8080)
```

`lineal create-workspace --name … --owner-email … --owner-name …` creates a real workspace with its
owner (password from `LINEAL_OWNER_PASSWORD` or prompted); operations and Docker: docs/15 §3.2b/§3.2c.

`scripts/dev.sh` starts the database, migrations, server and web dev server in one step
(`SEED=1 scripts/dev.sh` also loads the sample data). If another process uses `:8080`,
set `LINEAL_BIND=127.0.0.1:18080` and, for the web app, `LINEAL_BACKEND=http://localhost:18080`.

`cargo test` and `#[sqlx::test]` need a running database; `.cargo/config.toml` sets the default
`DATABASE_URL` to `postgres://lineal:lineal@localhost:5440/lineal`.

## Checks

```bash
scripts/check.sh
```

runs `docker compose up -d --wait`, `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`, `cargo deny`, the schema export to `web/schema.graphql` (regenerated), `npm ci`,
`typecheck`, `lint` and `test`. End-to-end tests: `scripts/e2e.sh`.

## Structure

| Path | Contents |
|---|---|
| `crates/lineal-domain` | IDs, enums, rules, domain events (pure, no I/O) |
| `crates/lineal-app` | use cases, ports (`Store`/`Tx`, repositories, services), policy, reference resolution |
| `crates/lineal-db` | PostgreSQL adapter (`PgStore`/`PgTx`), `migrations/` |
| `crates/lineal-api` | axum router, GraphQL (`MergedObject` per domain area), REST/OAuth |
| `crates/lineal-mcp` | MCP tool dispatch (55 compatible tools + `get_workspace_overview`), handlers per group |
| `crates/lineal-sync` | WebSocket delta stream and document collaboration |
| `crates/lineal-jobs` | outbox consumers, periodic jobs, worker loop |
| `crates/lineal-import` | import and scheduled sync from Linear |
| `crates/lineal-server` | binary `lineal`: `serve`, `worker`, `migrate`, `seed`, `schema-export` |
| `crates/lineal-testkit` | seed data, `FakeStore`, test helpers (see its README) |
| `web/` | Vite, React 19, TypeScript strict, Tailwind v4, shadcn/ui, TanStack Router/Query |

Dependencies point inwards; only `lineal-server` knows all crates. Business rules live
exclusively in `lineal-domain` and `lineal-app`.

## Conventions

- Conventional Commits; one work package = one branch `wp/<nr>-<name>`.
- `#![forbid(unsafe_code)]`, no `unwrap()` outside tests, no SQL string interpolation.
- Error codes and enums exactly as in [`docs/00-konventionen.md`](docs/00-konventionen.md).
- New migrations only within the assigned number range (see `docs/12-arbeitspakete.md`).
- npm instead of pnpm; the generated `web/src/routeTree.gen.ts` and `web/src/gql/` are not checked in.

## License

[MIT](LICENSE)
