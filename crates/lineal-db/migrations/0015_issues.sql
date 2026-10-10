-- 0015: Issue, IssueIdentifierAlias, IssueToLabel, IssueRelation, IssueHistory (docs/01 §3.1-3.5, §3.9, §7).
-- Issue.source_comment_id und die FK Project.converted_from_issue_id folgen in 0017 bzw. unten.

create table issue (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  number int not null check (number > 0),
  -- Annahme: denormalisierte Kennung "KEY-n" (docs/01 §7.9: Kennung wird mitindiziert). Wird per Trigger aus
  -- team.key und number gepflegt (bei INSERT/Teamwechsel sowie bei Aenderung von team.key); der Core muss sie
  -- nicht setzen. NICHT eindeutig, weil Keys geloeschter (Papierkorb-)Teams wieder vergeben werden duerfen.
  identifier text not null,
  title text not null check (char_length(title) between 1 and 255),
  description text check (char_length(description) <= 500000),
  state_id uuid not null,
  -- Denormalisierte Kategorie des Status (docs/01 §7.4); per Trigger aus workflow_state.type gepflegt.
  state_type text not null
    check (state_type in ('triage','backlog','unstarted','started','completed','canceled','duplicate')),
  state_entered_at timestamptz not null default now(),
  priority int not null default 0 check (priority between 0 and 4),
  estimate int check (estimate >= 0),
  assignee_id uuid,
  delegate_id uuid,
  creator_id uuid,
  parent_id uuid,
  project_id uuid,
  project_milestone_id uuid,
  cycle_id uuid,
  due_date date,
  last_applied_template_id uuid,
  source_comment_id uuid,
  -- Annahme: sort_order-Felder werden vom Core vergeben (Listenanfang); DB-Default 0.
  sort_order double precision not null default 0,
  priority_sort_order double precision not null default 0,
  sub_issue_sort_order double precision,
  started_at timestamptz,
  completed_at timestamptz,
  canceled_at timestamptz,
  started_triage_at timestamptz,
  triaged_at timestamptz,
  added_to_team_at timestamptz not null default now(),
  added_to_cycle_at timestamptz,
  added_to_project_at timestamptz,
  auto_closed_at timestamptz,
  auto_archived_at timestamptz,
  snoozed_until_at timestamptz,
  snoozed_by_id uuid,
  sla_type text check (sla_type in ('all','onlyBusinessDays')),
  sla_started_at timestamptz,
  sla_medium_risk_at timestamptz,
  sla_high_risk_at timestamptz,
  sla_breaches_at timestamptz,
  trashed boolean not null default false,
  trashed_at timestamptz,
  -- Volltext (Annahme: Konfiguration 'simple', da Inhalte gemischt Deutsch/Englisch sind; Beschreibung auf
  -- 100 000 Zeichen begrenzt, weil tsvector maximal 1 MB gross sein darf).
  search_vector tsvector generated always as (
    setweight(to_tsvector('simple', coalesce(title, '')), 'A') ||
    setweight(to_tsvector('simple', left(coalesce(description, ''), 100000)), 'B')
  ) stored,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (team_id, number),
  foreign key (organization_id, team_id) references team (organization_id, id),
  -- Invariante 1: Status gehoert zum Team des Issues.
  foreign key (organization_id, team_id, state_id) references workflow_state (organization_id, team_id, id),
  foreign key (organization_id, assignee_id) references "user" (organization_id, id),
  foreign key (organization_id, delegate_id) references "user" (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, snoozed_by_id) references "user" (organization_id, id),
  foreign key (organization_id, parent_id) references issue (organization_id, id) on delete set null (parent_id),
  -- Projekt: bei endgueltigem Loeschen verlieren Issues projectId (und per Milestone-FK den Meilenstein).
  foreign key (organization_id, project_id) references project (organization_id, id) on delete set null (project_id),
  -- Invariante 3: Team des Issues ist am Projekt beteiligt (nur geprueft, wenn project_id gesetzt).
  -- Annahme: Beim Entfernen eines Teams aus einem Projekt raeumt der Core die Issues vorher auf (docs/01 §4.2 Inv. 4).
  foreign key (organization_id, project_id, team_id) references project_team (organization_id, project_id, team_id),
  -- Invariante 4: Meilenstein gehoert zum Projekt des Issues. "Milestone setzt Projekt voraus" prueft der Core.
  foreign key (organization_id, project_id, project_milestone_id)
    references project_milestone (organization_id, project_id, id) on delete set null (project_milestone_id),
  -- Invariante 2: Cycle gehoert zum Team des Issues.
  foreign key (organization_id, team_id, cycle_id)
    references cycle (organization_id, team_id, id) on delete set null (cycle_id),
  foreign key (organization_id, last_applied_template_id) references template (organization_id, id),
  check (parent_id is distinct from id)
);

-- Kennung und state_type per Trigger (Annahmen siehe oben).
create function issue_set_identifier() returns trigger
language plpgsql as $$
begin
  select t.key || '-' || new.number into new.identifier from team t where t.id = new.team_id;
  return new;
end;
$$;
create trigger issue_identifier_trg before insert or update of team_id, number on issue
  for each row execute function issue_set_identifier();

create function issue_set_state_type() returns trigger
language plpgsql as $$
begin
  select s.type into new.state_type from workflow_state s where s.id = new.state_id;
  return new;
end;
$$;
create trigger issue_state_type_trg before insert or update of state_id on issue
  for each row execute function issue_set_state_type();

-- Key-Wechsel eines Teams: alle Issues erhalten neue Kennungen (die alten legt der Core als Alias an).
create function team_key_changed_update_issues() returns trigger
language plpgsql as $$
begin
  update issue set identifier = new.key || '-' || number where team_id = new.id;
  return new;
end;
$$;
create trigger team_key_changed_issues after update of key on team
  for each row when (old.key is distinct from new.key)
  execute function team_key_changed_update_issues();

-- Indizes (docs/01 §7.4); "aktiv" = nicht archiviert und nicht im Papierkorb.
create index issue_team_state_idx on issue (organization_id, team_id, state_id) where archived_at is null and not trashed;
create index issue_team_sort_idx on issue (organization_id, team_id, sort_order) where archived_at is null and not trashed;
create index issue_assignee_idx on issue (organization_id, assignee_id, updated_at desc)
  where assignee_id is not null and archived_at is null and not trashed;
create index issue_creator_idx on issue (organization_id, creator_id, created_at desc);
create index issue_cycle_idx on issue (organization_id, cycle_id) where cycle_id is not null and archived_at is null and not trashed;
create index issue_project_idx on issue (organization_id, project_id) where project_id is not null and archived_at is null and not trashed;
create index issue_milestone_idx on issue (organization_id, project_milestone_id)
  where project_milestone_id is not null and archived_at is null and not trashed;
create index issue_parent_idx on issue (organization_id, parent_id, sub_issue_sort_order) where parent_id is not null;
create index issue_updated_idx on issue (organization_id, updated_at desc, id);
create index issue_created_idx on issue (organization_id, created_at desc, id);
create index issue_sla_idx on issue (organization_id, sla_breaches_at)
  where sla_breaches_at is not null and completed_at is null and canceled_at is null;
create index issue_closed_idx on issue (organization_id, team_id, updated_at)
  where state_type in ('completed','canceled','duplicate');
create index issue_completed_at_idx on issue (organization_id, completed_at) where completed_at is not null;
create index issue_trashed_idx on issue (trashed_at) where trashed;
create index issue_snooze_idx on issue (snoozed_until_at) where snoozed_until_at is not null;
create index issue_identifier_idx on issue (organization_id, identifier);
create index issue_identifier_trgm_idx on issue using gin (identifier gin_trgm_ops);
create index issue_search_idx on issue using gin (search_vector);

-- Rueckreferenz von Project auf das Ursprungs-Issue.
alter table project
  add constraint project_converted_from_issue_fk foreign key (organization_id, converted_from_issue_id)
    references issue (organization_id, id) on delete set null (converted_from_issue_id);

-- Physische Tabelle hinter Issue.previousIdentifiers und Project-Altkennungen (docs/01 §3.2).
create table issue_identifier_alias (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  identifier text not null check (identifier = upper(identifier)),
  entity_type text not null default 'issue' check (entity_type in ('issue','project')),
  issue_id uuid,
  project_id uuid,
  team_key text not null,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  -- Invariante 1 (Teil): Kennung workspaceweit hoechstens einmal als Alias.
  unique (organization_id, identifier),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  check ((entity_type = 'issue' and issue_id is not null and project_id is null)
      or (entity_type = 'project' and project_id is not null and issue_id is null))
);
create index issue_identifier_alias_issue_idx on issue_identifier_alias (organization_id, issue_id) where issue_id is not null;
create index issue_identifier_alias_project_idx on issue_identifier_alias (organization_id, project_id) where project_id is not null;
create index issue_identifier_alias_team_key_idx on issue_identifier_alias (organization_id, team_key);
create index issue_identifier_alias_trgm_idx on issue_identifier_alias using gin (identifier gin_trgm_ops);

create table issue_to_label (
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  label_id uuid not null,
  -- Denormalisiert: parent_id des Labels, wenn die Gruppe singleSelect ist (sonst null).
  single_select_group_id uuid,
  created_at timestamptz not null default now(),
  primary key (issue_id, label_id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, label_id) references issue_label (organization_id, id) on delete cascade,
  foreign key (organization_id, single_select_group_id) references issue_label (organization_id, id) on delete cascade
);
create index issue_to_label_label_idx on issue_to_label (organization_id, label_id);
create unique index issue_to_label_group_uq on issue_to_label (issue_id, single_select_group_id)
  where single_select_group_id is not null;

create table issue_relation (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  related_issue_id uuid not null,
  type text not null check (type in ('blocks','related','duplicate','similar')),
  creator_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (issue_id, related_issue_id, type),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, related_issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  check (issue_id <> related_issue_id)
);
create index issue_relation_issue_idx on issue_relation (organization_id, issue_id);
create index issue_relation_related_idx on issue_relation (organization_id, related_issue_id);
-- Invariante 2: related/similar sind symmetrisch (ungeordnetes Paar, ohne feste UUID-Reihenfolge erzwungen);
-- Invariante 3: blocks nicht gleichzeitig in beide Richtungen.
create unique index issue_relation_pair_uq on issue_relation
  (organization_id, least(issue_id, related_issue_id), greatest(issue_id, related_issue_id), type)
  where type in ('related', 'similar', 'blocks');
-- Invariante 4: hoechstens eine ausgehende Duplikat-Relation je Issue.
create unique index issue_relation_duplicate_uq on issue_relation (issue_id) where type = 'duplicate';

-- Append-only (Invariante 1). Annahme: kein version/updated_at/archived_at, da unveraenderlich.
-- source: Vereinigung aus docs/01 (user, automation, api, mcp, import) und docs/13 §5 (user, system, import).
create table issue_history (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  actor_id uuid,
  source text not null check (source in ('user','automation','api','mcp','import','system')),
  changes jsonb not null default '{}',
  added_label_ids uuid[],
  removed_label_ids uuid[],
  relation_changes jsonb,
  attachment_id uuid,
  flags text[] not null default '{}',
  notified_user_ids uuid[],
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, actor_id) references "user" (organization_id, id),
  -- Invariante 2: mindestens eine Aenderung.
  check (changes <> '{}'::jsonb or added_label_ids is not null or removed_label_ids is not null
         or relation_changes is not null or attachment_id is not null or cardinality(flags) > 0)
);
create trigger issue_history_append_only before update on issue_history
  for each row execute function lineal_forbid_update();
create index issue_history_issue_idx on issue_history (organization_id, issue_id, created_at);
