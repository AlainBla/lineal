# Plan: Settings-Vervollständigung und Import eines einzelnen Teams/Projekts

Stand 2026-10-12. Ziel: Ein Owner kann in Lineal Teams anlegen und vollständig konfigurieren
(wie in Linear unter Settings) und ein Linear-Team bzw. einzelne Linear-Projekte in ein neues
oder bestehendes Lineal-Team importieren (konkreter Anlass: Team „KollegePlus“).

## Analyse

Grundlage: docs/02 §1–§2 (Workspace-/Team-Settings, aus der ursprünglichen Linear-Analyse),
docs/06 (Templates), docs/13 (Import), Abgleich mit dem Code (GraphQL-Mutationen, Web-Settings).

| Bereich (Linear-Settings) | Backend (GraphQL) | Web-UI | Lücke |
|---|---|---|---|
| Team anlegen (Name, Key, Parent, Sichtbarkeit, „Copy settings from“) | `teamCreate` | fehlt | Dialog + Teams-Übersicht |
| Teams-Übersicht (Workspace-Settings „Teams“) | `teams` | fehlt | Liste mit Mitgliederzahl, Sichtbarkeit, Anlegen |
| Team General (Name, Key, Icon/Farbe, Zeitzone, Beschreibung) | `teamUpdate` | Name/Key/Zeitzone | Icon/Farbe/Beschreibung, Key-Wechsel mit Hinweis |
| Team Members, Sichtbarkeit, Join-Regeln | vorhanden | Members | Sichtbarkeit/`allMembersCanJoin` |
| Workflow-Status | vorhanden | vorhanden | – |
| Issue-Labels (Team) | vorhanden | vorhanden | – |
| Issue-Labels (Workspace) | vorhanden | fehlt | Workspace-Seite „Labels“ |
| Templates (Team/Workspace, Default-Template) | `templateCreate/Update/Delete`, `teamDefaultTemplateSet` | fehlt | Abschnitt/Seite „Templates“ |
| Cycles, Triage | vorhanden | vorhanden | – |
| Estimates | `teamUpdate` | fehlt | Abschnitt „Estimates“ |
| Auto-close/-archive, Parent/Sub-Automationen | `teamUpdate` | fehlt | Abschnitt „Automations“ |
| Team retire/restore/delete | vorhanden | fehlt | „Danger zone“ in General |
| Import: ganzes Team in neues Team | vorhanden | vorhanden | gegen echtes Linear-Schema ungeprüft |
| Import: nur ausgewählte Projekte | fehlt | fehlt | Filter |
| Import: in bestehendes Lineal-Team | fehlt | fehlt | Ziel-Team-Mapping |

## Arbeitspakete (parallel, Sonnet, Worktrees)

- **WP26 Teams anlegen und verwalten** (Web, wenig Backend): Workspace-Settings „Teams“ (Liste,
  „New team“-Dialog mit Name, Key-Vorschlag, Parent, Sichtbarkeit, Copy settings from), Eintrag
  „+ New team“ in der Sidebar, Team-General erweitert (Icon/Farbe/Beschreibung, Key-Wechsel mit
  Bestätigung, Sichtbarkeit/Join, Danger zone retire/delete). Dateien: features/settings/teams-*,
  nav.ts, Sidebar, team-sections.tsx **nur GeneralSection und MembersSection**.
- **WP27 Team-/Workspace-Konfiguration** (Web): neue Abschnitte Estimates, Automations,
  Templates (Team) in eigener Datei features/settings/team-config-sections.tsx und deren
  Einhängen in die Abschnittsliste; Workspace-Seiten „Labels“ und „Templates“. Fasst
  GeneralSection/MembersSection nicht an.
- **WP28 Import für einzelne Teams/Projekte** (Backend + Web): Abgleich der Importer-Abfragen
  mit dem offiziellen Linear-GraphQL-Schema (öffentlich, z. B. `packages/sdk/src/schema.graphql`
  im Repo linear/linear), Projekt-Filter (nur Issues gewählter Projekte samt benötigter Labels,
  Status, Cycles, Personen, Kommentare, Anhänge), Ziel = bestehendes Lineal-Team (Status per
  Name/Typ zuordnen oder anlegen, Labels per Name, neue Issue-Nummern, Linear-Kennung als
  auflösbarer Alias und als Link), Dry-Run-Bericht mit Zuordnungen, UI-Schritte dafür, Tests
  gegen den Mock. Kein Zugriff auf echtes Linear.

Danach: Merge, `check.sh`, E2E, Docker-Update mit Backup vorher; Import „KollegePlus“ führt der
Owner selbst aus (Dry run zuerst).
