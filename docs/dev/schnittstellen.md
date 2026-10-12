Schema-Hinweise aus S2 (für alle WP-Briefings):
- Tabelle heißt "user" -> in SQL IMMER "user" in Anführungszeichen.
- Zähler: team.last_issue_number, last_project_number, last_cycle_number; organization.last_initiative_number.
- PostgreSQL >= 15 (NULLS NOT DISTINCT).
- issue.identifier und issue.state_type werden per Trigger gepflegt (nicht selbst setzen). Key-Wechsel aktualisiert identifier automatisch.
- Issue mit project_id verlangt project_team-Zeile für sein Team (FK) -> vor Teamwechsel/Team-Entfernen Projektzuordnung leeren.
- team_key enthält nur frühere Keys; beim Key-Wechsel alten Key eintragen.
- team.default_issue_state_id nullable (Zirkelreferenz).
- project.lead_team_id ∈ project_team, Zyklen/Tiefe: Core prüft.
- user_session, user.password_hash vorhanden; oauth_token.family_id/rotated_at; import_mapping.imported_at.
- issue_history.source: user, automation, api, mcp, import, system. user.status: active, imported.
- issue_relation related/similar/blocks eindeutig je ungeordnetem Paar (least/greatest).
- document_content.owner_id polymorph ohne FK.
- Volltext-Konfig 'simple'; description max 100000 Zeichen.
- Kein Trigger für updated_at/version -> Core setzt updated_at und version = version+1.

MCP-Hilfen aus WP04 (crates/lineal-mcp/src):
- auth::hash_token(&str) = SHA-256 hex lowercase über komplettes Token inkl. Präfix.
- envelope: parse_limit(args, default, max) (DEFAULT_LIMIT 50, MAX_LIMIT 250; list_projects max 50), query_fingerprint(args), encode_cursor/decode_cursor, paginate(rows, limit, cursor_of) (limit+1 laden), page(key, items, next_cursor), success(id), Refs team_ref/user_ref/member_ref/project_ref/initiative_ref/status_ref/label_ref/milestone_ref/priority(u8)/opt_text. Eigener Content: {"_mcpContent":[...]} (envelope::CONTENT_KEY) z.B. für extract_images.
- fields: parse_fields(args), select_fields(&Map,&requested,list_fields), truncate_description(text,"get_issue"), truncate_content(text,"get_document").
- error: Handler geben Err(AppError) zurück; Transport baut Fehlermodell.
- validate: validate_args(tool,args) läuft VOR Handler; get_workspace_overview-Schema in validate.rs (WP11 darf anpassen).
- auth::required_scope(tool): list_*/get_*/extract_images/search_documentation=read, sonst write.
- serde_json Feature preserve_order workspace-weit aktiv.

Web-Hilfen aus WP05 (web/src):
- app/use-sidebar-data.ts useSidebarData()/SidebarData (Mock, durch GraphQL ersetzen, Form bleibt).
- app/commands.ts useActions(Action[]) registriert Shortcut+Command; lib/shortcuts.ts useShortcutContext("list"|"issue"|"modal"), useShortcuts.
- app/ui-state.ts createIssueOpen (Aktion issue.create / Taste c), detailsOpen (rechte Sidebar).
- app/nav.ts NavLink, PlaceholderPage, useSession(), workspacePaths()/teamPaths() (URL-Schema 08 §1.3); Feature-Routen unter /{ws}/... ersetzen Platzhalter automatisch.
- lib/graphql.ts gqlRequest, GraphQLRequestError; codegen nach src/gql/graphql.ts (gitignored, typecheck ruft codegen).
- app/auth.ts normalizeSession erwartet /auth/me: {user:{id,email,displayName|name}, organization|workspace:{name,urlKey}}.
- Workspace-Issues Shortcut: g ⇧W (g w = Upcoming cycle).

API-Hilfen aus WP03 (crates/lineal-api):
- Resolver: lineal_api::context::request_ctx(ctx) holt RequestCtx; Fehler: GqlResultExt::gql(ctx) aus lineal_api::error.
- REST geschützt: Extraktor Authenticated(RequestCtx); Login/Logout: lineal_api::middleware::auth::{session_cookie, clear_session_cookie, hash_token}.
- Listen: pagination::{page_request, finish_page, Page, PageInfo, PaginationOrderBy}; fetch_limit() Zeilen laden, dann finish_page; Page::envelope für REST/MCP.
- Blobs: app.blobs.signed_put_url / signed_get_url mit blob::PUT_TTL / GET_TTL.
- Tests ohne WP02: FakeStore::with_principal(hash_token("lin_api_..."), principal).
- Lese-Scope darf keine Mutations (forbidden).

Domain-Hilfen aus WP01 (crates/lineal-domain/src/rules):
- TeamKey::parse lehnt P und I ab.
- state::apply_transition/check_transition (duplicate nur via MarkDuplicate), apply_project_transition.
- estimate::allowed_values/convert; labels::check_group_constraints(sel, groups, added: Option<&[LabelId]>), add_labels (singleSelect ersetzt Geschwister).
- progress::issue_points/progress/stats; cycle_calendar::generate(...)->Result (from = lokales NaiveDate), generate_following, local_midnight_to_utc.
- sla::breach_at(start, amount, SlaType, tz, WorkWeek) (All: ms, OnlyBusinessDays: Tage), risk_levels, status.
- patch::apply; Fehler: NoOps/TooManyOps/EmptyArgument -> invalid_request; AnchorNotFound/AnchorAmbiguous -> conflict details{opIndex,op,anchor,matches}; LimitExceeded -> limit_exceeded (422). MAX_TEXT_CHARS 500000.
- filter::parse_issue_filter(json) (validiert), normalize, canonical_json; IsoDuration, DateTimeOrDuration::value(); FilterError.path = JSON-Pointer.
- sort_order::between/needs_renumber/on_state_change; markdown::sanitize/render_html/is_safe_url (nur http/https/mailto/relativ).

Issue-Hilfen aus WP06:
- App::create_issue_in / update_issue_in (in fremder Tx, für Templates/Import); App::list_issues(ListIssuesInput) Einstieg für WP11.
- usecases::issue::history::record (Coalescing); fields::change_state = zentrale Statuswechsel-Logik (Triage, Automationen).
- HistoryEntry.changes {feld:{from,to}} camelCase; Flags created, descriptionUpdated, trashed, restored.
- delete_issue setzt trashed, trashed_at, archived_at.
- IssueRepo hat minimale Lesezugriffe find_cycles/get_cycle/find_projects/get_project/find_milestones.
WP09: App::list_issue_attachments, App::link_attachment, tx.comments().parent_scope(ParentRef), App::resolve_assets/asset_prefix. FakeBlobs ohne head(): MemBlobs-Muster in lineal-db/tests/comments.rs.
WP08: issueSubscribe/issueUnsubscribe GraphQL belegt; NotificationRepo; fan_out_event.
WP32 (Sync, Notifications):
- Sync-Gruppen: `team:<id>`, `org` (nicht Gäste), `orgpub` (alle: Organization, Workspace-Labels), `tmem:<team>` (Personen; Gäste für ihre Mitglieds-Teams), `user:<id>` (persönlich, Freigaben). Neue Modelle in `lineal-db/src/repos/outbox.rs::model_sql` immer mit der richtigen Gruppe versehen; `$2` ist ein Array von IDs (`= ANY($2)`), `OutboxRepo::sync_bootstrap_page(model, after, limit)` pagt ein Modell (Tabelle/Filter in `bootstrap_source`).
- `GET /sync/bootstrap` (lineal-sync, `modelNames`, `limit`, `cursor`): NDJSON + `_metadata_=`; Web: `web/src/lib/store/bootstrap.ts::fetchBootstrap`, `SyncClient`-Option `bootstrap`.
- `DomainEvent::IssueUpdated.previous: Option<IssuePrevious>` (`assignee_id`, `state_type`) mit `IssuePrevious::of(&changed, old_assignee, old_state_type.as_str())`; neue Emitter, die Assignee oder Status ändern, sollen es füllen.
- View-Abos: `usecases::notification::views::{ViewSubscriptionSource, ViewIssueEvent}`; `App::fan_out_event_with(ev, Some(&source))`, `NotificationsWithViewsConsumer { views: Arc<dyn ViewSubscriptionSource> }` (lineal-jobs) statt `NotificationsConsumer` in `registry::consumers()`. Typen: `issueAddedToView`, `issueStatusChanged`, `issueStatusChangedAll`.
- Notifications: `NotificationRepo::{dedup_seen, find_bundle_target, merge_notification}` (Bündelung), `document_facts`/`milestone_facts`/`initiative_update_facts`, `subscribe_document`/`unsubscribe_document`/`document_subscribers`; `CommentFacts` trägt alle Elternfelder. `Notification.bundled_types`, GraphQL `bundledTypes`, `projectMilestoneId`, `projectUpdateId`, `initiativeUpdateId`; Mutationen `documentSubscribe`/`documentUnsubscribe`, Query `documentSubscribed`.
- Web: `features/inbox/notification-types.ts` (`typeLabel`, `bundleSummary`, `targetKind`).
