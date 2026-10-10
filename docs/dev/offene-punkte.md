- 07: Abweichungstabelle list_templates paginiert? (aus 06)
- 06/01/03/10: gemeinsamer DocumentContent für Rich-Text konsistent?
- 03 vs 10: addLabels in singleSelect-Gruppe ersetzt vorhandenes Label
- 04 vs 10: Initiative-Zyklus -> invalid_request
- 03 vs 10: Duplicate-State umbenennen/löschen -> invalid_request
- 03 vs 10: Auto-Close/Archive Defaults
- 05 vs 10: Digest-Verzögerung
- 07 vs 04: list_cycles {cycles:[...]}, flexible team ref, isActive vs isCurrent; milestone progress 0..1
- 01/07 vs 04: Projekt summary/content (A23)
- 09: Default-Sortierung updatedAt ok (00 sagt updatedAt)
- GLOBAL: Sub-Team-Key ENG-WEB -> WEB (Name "Web", Parent ENG). Key-Regel ^[A-Z][A-Z0-9]{0,6}$ ohne Bindestrich; 02 Abschnitt ~Z.107-118 + Annahme 1 umschreiben. Erst nach 01/07 fertig.
- KONFLIKT progress: 04 sagt 0..1, 07 sagt 0..100 -> vereinheitlichen auf 0..1 (wie GraphQL-Schema Float), 07 anpassen
- 07 isActive vs isCurrent prüfen gegen 04
- tools.json aus scratchpad/mcp07 als docs/mcp-tools.json Fixture übernehmen
- 05->01: reason `comment`, Felder stateEnteredAt, isPriority
- 05->10: issueDue-Job, Abo-Tabellennamen angleichen
- 05->09: highlights, completedIssues, notificationArchiveAll, issueUnsubscribe fehlen
- 08->09: API-Operationen Workspace-Anlage, Einladungen, previewTeamNesting/nestTeam, Unsubscribe (U7-U9)
- 08->01: Entitäten Draft und Invitation (U6,U7)
- 04 §6.2: Alice legt Projekt mit DES an, ist laut 02 nicht DES-Mitglied (U12)
- 02: Alice einmal owner, einmal admin (U13) -> vereinheitlichen
- 08: Alice muss in F2 WEB verlassen damit F6 funktioniert (U14) – prüfen
INTEGRATION-TODO (Orchestrator):
- /mcp und /sync laufen nicht durch lineal-api Middleware (Security-Header, Rate-Limit, Request-ID) -> in build_router auf Gesamt-Router anwenden.
- CSRF-Schutz für Cookie-Sessions fehlt (Header-Name festlegen, z.B. X-Lineal-CSRF / Double-Submit) -> WP16/WP17.
- X-Forwarded-For nicht ausgewertet (hinter Proxy konfigurierbar machen) -> WP17.
- Theme-Flash: Inline-Script index.html -> WP14 oder ich.
FÜR WP16/WP17 (Leftovers):
- hash_token in 3 Crates (app usecases::auth, api middleware::auth, mcp auth) -> Adapter sollen app-Version nutzen (WP17).
- api_key.team_ids im Principal-Lookup ignoriert -> WP16.
- Nesting: Label-Merge, Cycle-Instanzen, Projektstatus-Neuauflösung fehlen; Kinder, die selbst erben, nicht nachgezogen -> WP17 oder WP07-Nachtrag.
- GraphQL createWorkspace fehlt -> WP16.
- CSRF Cookie-Sessions, X-Forwarded-For -> WP16/17.
- Legacy-SSE-Transport/stdio MCP, Origin-Prüfung -> WP17 optional.
WP08-Leftovers:
- email_digest eigener periodic Job + Registry-Eintrag (Orchestrator) – läuft derzeit in due_reminders.
- issueUnassignedFromYou/issueReopened (IssueUpdated ohne alte Werte) -> evtl. `previous` im Event; issueUnblocked.
- Kommentare an Initiativen/Dokumenten/Meilensteinen/Berichten -> Notifications; Dokument-Abos; IssueTriaged-Notifications (WP12); View-/User-Abos; 1-Min-Bündelung.
- NotificationUpdated/Archived Events fehlen -> Sync (WP15) braucht sie.
- Placeholder _notificationReady bleibt wegen Test in graphql/mod.rs -> Orchestrator: Test generisch machen.
WP06-Leftovers:
- save_issue template (WP10), links (App::link_attachment aus WP09 einhängen), releases ignoriert.
- Parent/Child-Automationen §3.4, SLA-Risikostufen -> WP12.
- moveSubIssues, Bulk-Move, issueBatchUpdate, issueRelationDelete, Label-Merge/-Delete -> WP13 braucht evtl. Batch; sonst WP17.
- ENG-123-Erwähnung -> related-Relation; Duplicate: Subscriber wandern -> WP17.
- GraphQL Payload lastSyncId, Relay-Connections -> WP15 (Sync) prüfen.
- Share/Unshare ohne Event; issueSharingEnabled Default false.
- Issue-Seed leer -> für Web-UI-Demo Seed nötig (WP13 eigenes seed? Orchestrator).
- get_issue.attachments: App::list_issue_attachments (WP09) einhängen.
WP07-Leftovers:
- update_team ruft Cycle-Generierung nicht (Job holt nach 1 min) -> WP16 Team-Lebenszyklus: App::configure_team_cycles; Nesting: CycleRepo::set_team_inherits + engine::sync_inherited.
- WP06 soll usecases::cycle::auto_cycle_for_state nutzen (Auto-Cycle bei Statuswechsel) -> Integration Orchestrator/WP12.
- template/customView in save_project/list_projects -> WP10/WP11.
- Doku-Konflikte: Lead-Team-Wechsel neue P-Nummer (04 A10 anpassen); list_projects initiative rekursiv vs. 07.
- cycleNumberBase, Kapazität, cycle_remove_moves_to_backlog fehlen.
- GraphQL-Platzhalter _xReady: Test in graphql/mod.rs generisch machen -> Orchestrator.
WP13-Lücken (Backend-GraphQL fehlt):
- users-Query (Assignee-Picker, Mentions) – größte UX-Lücke.
- issue.children (teamübergreifend), issue.relations, issue.history lesen.
- Abo-Zustand eines Issues (issueSubscription für Viewer).
- issueBatchUpdate, Drafts, Template-Auswahl im Create-Modal (nach WP10).
- issues-Query mit Filter-JSON (WP11) -> Client-Filter ersetzen.
- Web: Cache-Normalisierung Key vs UUID für Sync (WP15).
WP12-Leftovers:
- Snooze-Wiedervorlage bei Aktivität; Auto-Archive-Notification; Cycles/Projekte auto-archivieren.
- SLA-Regel-CRUD + team.sla_enabled/sla_work_week Setter (Team-Settings) -> WP14/WP16.
- BlobStore::delete fehlt -> Purge löscht Blobs nicht (WP17).
- Mentions in Triage-Kommentaren; IssueTriaged-Notification.
WP10-Leftovers:
- Dokument-Kaskade (trash_documents_of/restore_documents_of) in delete_issue, set_project_trashed, Team-Löschen einhängen -> Orchestrator/WP16.
- Snapshot-Bündelung, Dokument-Mentions/Abos, documentMoved, CRDT; useDefaultTemplate API; Form-Templates; get_issue.attachments/documents einhängen.
WP11-Leftovers: ancestors.some nur id; Favoriten-Umsortieren; View-Abos; View-Archiv bei Team-Löschung; Eigentümerwechsel; Suche 'simple' statt Stemming; Triage-Ausblendung flache MCP-Params.
Erledigt durch Orchestrator: GraphQL `users(query, team, first)` -> [UserRef] ohne E-Mail (für WP13-Assignee-Picker/Mentions; WP14 bzw. Nachtrag WP13 einhängen).
WP14-Leftovers: Filter-Chips aus filterData zurückwandeln; Filter-Bar an Team-Seiten; Such-Command global (Shell); Initiativen-Tabs; Roadmap-Timeline; Board in Views; Update-Diff; Initiative-Update-Composer; Team-Settings Templates/SLA/Hierarchie/Webhooks/Access/Recurring; Workspace-Settings Members/Teams/Security/API/Limits; Account-Settings; Projekt-Issues nach Milestone; users ohne Rolle/Owner.
WP14 Query-Keys: siehe Abschlussbericht (inbox, overview, planning, views, search, docs, settings, users/known-users) – Hooks in features/<bereich>/queries.ts.
WP16-Leftovers: api_key.team_ids im Principal; REST /invite/accept ohne Session verdrahten; openIssuesOnRetirement cancel/moveToTeam; Notification bei Webhook-Deaktivierung; client_credentials/App-User/feingranulare Scopes; updatedFrom bei Nicht-Issues; BlobStore::delete; CSRF; X-Forwarded-For; OAuth-Consent als signierter GET (POST-Consent wäre sauberer); SSRF-Bausteine usecases::webhook::{UrlPolicy, parse_target_url, resolve_target} für alle ausgehenden HTTP-Pfade nutzen.
ERLEDIGT (Orchestrator): consent-phishing OAuth – Zustimmung nur per POST mit session-gebundenem Nonce, Ziel-Host + Warnhinweis angezeigt, CSP form-action self.
WP18-Leftovers: Linear-Feldnamen nur gegen Mock getestet; Views/Favorites/Issue-Reaktionen/Update-Kommentare nicht importiert; project.last_update_id/health; Projekt-Templates leer; Platzhalter-User-Verknüpfung bei Einladung prüfen (WP16 hat accept_invitation verknüpft); Webhook-Consumer origin=import (WP16 überspringt – ok); Dry-Run ohne Downloads.
WP15-Leftovers: /sync/bootstrap; Bulk-Inbox-Aktionen ohne Events; Issue-Sharing nicht gesynct (kein Event); Gäste ohne org-Gruppe (Caches users/Labels nicht live); sync_purge als PeriodicJob registrieren; Token während WS-Verbindung nicht neu geprüft (Revocation wirkt erst bei Reconnect) -> periodische Re-Validierung; Auth doppelt aufgelöst; Leader-Wahl Tabs.
WP17 erledigt: CSRF (Origin + X-Lineal-CSRF), X-Forwarded-For hinter Proxy, WS-Re-Validierung (Close 4401), BlobStore::delete + Purge, api_key.team_ids, persistenter Dead-Letter, POST /invite/accept, deny.toml/npm audit, E2E (F3, F5, F7, F12, F13, F17, privates Team), Bulk-Inbox-/Sharing-Events, Issue.children/relations/history.
WP17-Leftovers:
- Rate-Limit/Login (Bereich abgeschlossen, nur noch notiert): In-Process-Limit ist Verteidigung in der Tiefe, Edge-Rate-Limit (Proxy/WAF) davor einplanen; geteilter Überlauf-Bucket unter Massen-Spoofing akzeptiert; Fehlversuchs-Zähler der Token-Prüfung zählt nach dem Lookup statt reserviert; Zähler/Buckets nur im Prozessspeicher (mehrere Instanzen = je Instanz); Login-Sperre je Account gesamt (100/15 min) kann einen Account bei verteiltem Angriff verlangsamen; `ip_credentialed`-Limit (10 000/h) und Fenster sind Konstanten, nicht konfigurierbar.
- Team-Scope von API-Keys: Workspace-Admin-Aktionen (Rollen, Einstellungen) schränkt `team_scope` nicht ein; OAuth-Tokens haben keinen Team-Scope; API-Keys lassen sich weder per GraphQL noch REST anlegen (nur App::create_api_key) – für E2E per SQL.
- Sync: geteilte Issues erreichen den Empfänger nicht live (Sync-Gruppe `user:<id>` für Shares fehlt); `/sync/bootstrap`.
- Einladungen: Web-Seite `/invite/<token>` fehlt (REST-Endpunkt vorhanden).
- Issue.children/relations/history: N+1 in Listen (nur für Detailansicht gedacht); Web-Detailansicht nutzt die Felder noch nicht.
- E2E: nur die Flows F3, F5, F7, F12, F13, F17 und Sichtbarkeit privater Teams; übrige Flows aus docs/08 ungetestet; Kein E2E-Lauf in CI (scripts/e2e.sh manuell).
- Lieferkette: braces (ReDoS) in der @graphql-codegen-Kette ohne gepatchte Version (Dev-Werkzeug, siehe docs/15 §4.6).
