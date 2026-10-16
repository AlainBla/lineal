# Plan WP29–WP34: fehlende Oberfläche und Funktionen

Stand 2026-10-12. Quelle der Lücken: docs/15 §2 und docs/dev/offene-punkte.md. Fachliche
Vorgaben stehen in den Kapiteln 01–09 und 14 (Linear-Verhalten); bei Widersprüchen gilt das
Fachkapitel, Abweichungen als „Umsetzung: …“-Blockzitat dort vermerken.

## Gemeinsame Regeln (zusätzlich zu docs/dev/agent-briefing.md)

- Eigener Worktree, Branch `wp/<nr>-<name>` vom aktuellen main. Je Punkt ein grüner Commit.
- Migrationen nur im eigenen Nummernbereich (siehe Tabelle).
- `web/schema.graphql` nach Backend-Änderungen neu exportieren
  (`cargo run -q -p lineal-server -- schema-export --out web/schema.graphql`). Konflikte darin löst
  der Orchestrator durch Neu-Export – nicht von Hand mergen.
- Platte ist knapp: `CARGO_INCREMENTAL=0`; nach dem Abschluss `cargo clean` im eigenen Worktree.
- `web/node_modules` im Worktree per `npm ci` anlegen – NIEMALS als Symlink auf das Hauptrepo und
  nie `node_modules` im Hauptrepo verändern.
- E2E nur gegen eine eigene Datenbank (`LINEAL_E2E_DB=lineal_e2e_wp<nr>`), danach löschen.
- Docker-Container (`lineal-*`) nicht anfassen, `.env` im Hauptrepo nicht lesen.
- Niemals `mcp__claude_ai_Linear__*` oder Playwright gegen linear.app; keine echten Kundendaten.
- Prüfung: cargo fmt --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test
  (mindestens betroffene Crates, am Ende --workspace), Schema-Export, in web: npm run typecheck,
  npm run lint, npm test. rustfmt/clippy müssen installiert sein
  (`rustup component add rustfmt clippy` bzw. Distributionspakete).
- Doku: docs/15 §2 (erledigte Lücken streichen, Neues beschreiben), docs/dev/offene-punkte.md,
  `python3 tools/build_index.py`.
- Nicht pushen, nicht mergen. Abschlussbericht deutsch, knapp: Commits, Tests, Entscheidungen,
  geänderte gemeinsame Dateien, offene Punkte.

## Pakete und Eigentum

| WP | Thema | Eigene Bereiche (Web) | Migrationen |
|---|---|---|---|
| 29 | Account & Workspace-Admin-Rest | `routes/$ws/settings/account*`, `features/settings/account-*`; Security-Seite (Audit-Aufbewahrung); Teams-Seite (Wiederherstellen, Parent ändern) | 2600–2699 |
| 30 | Team-Settings-Rest | `features/settings/team-extra-sections.tsx` (SLA, Webhooks, Recurring), Template-Editor-Erweiterung | 2700–2799 |
| 31 | Planung & Views-UI | `features/planning`, `features/views` (Roadmap, Initiativen-Tabs, Filter-Chips aus filterData, Favoriten sortieren, View-Abos, Eigentümerwechsel) | 2800–2899 |
| 32 | Sync & Notifications | `crates/lineal-sync`, `lineal-jobs` Notification-Konsumenten, `features/inbox` | 2900–2999 |
| 33 | Issue-Funktionen | `features/issues` (Mehrfachauswahl/Bulk), Use-Cases Issue/Label | 3000–3099 |
| 34 | Planungs-Automationen, Dokumente, Import-Rest, Suche | `lineal-jobs` periodische Jobs, `features/docs`, `lineal-import` | 3100–3199 |

Gemeinsame Dateien (nur minimal ändern, Konflikte erwartet): `features/settings/nav.ts`,
`settings-pages.tsx` (Abschnittsliste), `crates/lineal-api/src/graphql/mod.rs`,
`crates/lineal-jobs/src/registry.rs`, `crates/lineal-domain` Events, docs/15, offene-punkte.md.
