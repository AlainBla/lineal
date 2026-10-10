-- 0014: Cycle, ProjectStatus, Project (+Team/Member/Milestone/Label), Initiative (+Parent/Project/Label) (docs/01 §4).
-- Rueckreferenzen auf spaeter angelegte Tabellen (Project.last_update_id, Project.converted_from_issue_id,
-- Initiative.last_update_id) werden in 0015/0016 per ALTER TABLE ergaenzt.

create table cycle (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  number int not null check (number > 0),
  name text,
  description text,
  starts_at timestamptz not null,
  ends_at timestamptz not null,
  completed_at timestamptz,
  inherited_from_id uuid,
  auto_archived_at timestamptz,
  uncompleted_issue_ids_upon_close uuid[],
  scope_history double precision[] not null default '{}',
  completed_scope_history double precision[] not null default '{}',
  in_progress_scope_history double precision[] not null default '{}',
  issue_count_history double precision[] not null default '{}',
  completed_issue_count_history double precision[] not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  -- Ziel fuer Issue.cycle_id / Document.cycle_id: erzwingt "Cycle gehoert zum Team" (Issue-Invariante 2).
  unique (organization_id, team_id, id),
  unique (team_id, number),
  foreign key (organization_id, team_id) references team (organization_id, id),
  foreign key (organization_id, inherited_from_id) references cycle (organization_id, id),
  check (ends_at > starts_at),
  -- Cycle-Invariante 1: keine Ueberlappung je Team (Exclusion-Constraint, docs/01 §7.3; [starts_at, ends_at)).
  exclude using gist (team_id with =, tstzrange(starts_at, ends_at, '[)') with &&)
);
create index cycle_team_starts_idx on cycle (organization_id, team_id, starts_at);

create table project_status (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) >= 1),
  type text not null check (type in ('backlog','planned','started','paused','completed','canceled')),
  color text not null check (color ~ '^#[0-9A-Fa-f]{6}$'),
  description text,
  position double precision not null default 0,
  -- Annahme: Default true; docs/01 verlangt false fuer planned/started -> der Core setzt den Wert je Kategorie explizit.
  indefinite boolean not null default true,
  team_id uuid,
  inherited_from_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id),
  foreign key (organization_id, inherited_from_id) references project_status (organization_id, id)
);
create trigger project_status_type_immutable before update of type on project_status
  for each row execute function lineal_forbid_type_change();
create index project_status_scope_idx on project_status (organization_id, team_id, type, position);
create unique index project_status_name_uq on project_status (organization_id, team_id, lower(name))
  nulls not distinct where archived_at is null;

create table project (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) between 1 and 255),
  lead_team_id uuid not null,
  number int not null check (number > 0),
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12)
    check (slug_id ~ '^[0-9a-f]{12}$'),
  description text check (char_length(description) <= 255),
  content text,
  color text not null default '#5E6AD2' check (color ~ '^#[0-9A-Fa-f]{6}$'),
  icon text,
  status_id uuid not null,
  priority int not null default 0 check (priority between 0 and 4),
  -- Annahme: sort_order/priority_sort_order werden vom Core vergeben ("ans Ende"); DB-Default 0.
  priority_sort_order double precision not null default 0,
  sort_order double precision not null default 0,
  lead_id uuid,
  creator_id uuid,
  start_date date,
  start_date_resolution text check (start_date_resolution in ('month','quarter','halfYear','year')),
  target_date date,
  target_date_resolution text check (target_date_resolution in ('month','quarter','halfYear','year')),
  started_at timestamptz,
  completed_at timestamptz,
  canceled_at timestamptz,
  health text check (health in ('onTrack','atRisk','offTrack')),
  health_updated_at timestamptz,
  last_update_id uuid,
  update_reminder_frequency_in_weeks int check (update_reminder_frequency_in_weeks between 0 and 4),
  update_reminders_day text check (update_reminders_day in ('Monday','Tuesday','Wednesday','Thursday','Friday','Saturday','Sunday')),
  update_reminders_hour int check (update_reminders_hour between 0 and 23),
  update_reminders_paused_until_at timestamptz,
  converted_from_issue_id uuid,
  last_applied_template_id uuid,
  auto_archived_at timestamptz,
  trashed boolean not null default false,
  trashed_at timestamptz,
  scope_history double precision[] not null default '{}',
  completed_scope_history double precision[] not null default '{}',
  in_progress_scope_history double precision[] not null default '{}',
  issue_count_history double precision[] not null default '{}',
  completed_issue_count_history double precision[] not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (lead_team_id, number),
  unique (organization_id, slug_id),
  foreign key (organization_id, lead_team_id) references team (organization_id, id),
  foreign key (organization_id, status_id) references project_status (organization_id, id),
  foreign key (organization_id, lead_id) references "user" (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, last_applied_template_id) references template (organization_id, id),
  check (start_date is null or target_date is null or start_date <= target_date)
);
-- Annahme: "lead_team_id muss in project_team enthalten sein" (Invariante 1) wird vom Core erzwungen
-- (docs/01 §7.1 erlaubt das explizit), um zirkulaere Deferred-FKs zwischen project und project_team zu vermeiden.
create index project_lead_team_idx on project (organization_id, lead_team_id);
create index project_status_idx on project (organization_id, status_id);
create index project_updated_idx on project (organization_id, updated_at desc, id) where archived_at is null and not trashed;
create index project_trashed_idx on project (trashed_at) where trashed;

create table project_team (
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  team_id uuid not null,
  sort_order double precision not null default 0,
  created_at timestamptz not null default now(),
  primary key (project_id, team_id),
  -- Ziel fuer Issue (project_id, team_id): erzwingt "Team des Issues ist am Projekt beteiligt" (Issue-Invariante 3).
  unique (organization_id, project_id, team_id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id)
);
create index project_team_team_idx on project_team (organization_id, team_id);

create table project_member (
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  user_id uuid not null,
  created_at timestamptz not null default now(),
  primary key (project_id, user_id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create index project_member_user_idx on project_member (organization_id, user_id);

create table project_milestone (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  name text not null check (char_length(name) >= 1),
  description text,
  target_date date,
  sort_order double precision not null default 0,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  -- Ziel fuer Issue (project_id, milestone_id): Meilenstein gehoert zum Projekt des Issues (Issue-Invariante 4).
  unique (organization_id, project_id, id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade
);
create index project_milestone_sort_idx on project_milestone (organization_id, project_id, sort_order);
create unique index project_milestone_name_uq on project_milestone (project_id, lower(name)) where archived_at is null;

create table project_label (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) >= 1),
  color text not null check (color ~ '^#[0-9A-Fa-f]{6}$'),
  description text,
  team_id uuid,
  parent_id uuid,
  is_group boolean not null default false,
  inherited_from_id uuid,
  creator_id uuid,
  retired_at timestamptz,
  retired_by_id uuid,
  last_applied_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id),
  foreign key (organization_id, parent_id) references project_label (organization_id, id),
  foreign key (organization_id, inherited_from_id) references project_label (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, retired_by_id) references "user" (organization_id, id),
  check (not is_group or parent_id is null),
  check (parent_id is distinct from id)
);
create unique index project_label_name_uq on project_label (organization_id, team_id, parent_id, lower(name))
  nulls not distinct where archived_at is null;
create index project_label_team_idx on project_label (organization_id, team_id);

create table project_to_label (
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  label_id uuid not null,
  -- Denormalisierte Gruppe (parent_id des Labels); Unique erzwingt "hoechstens ein Label je Gruppe".
  group_id uuid,
  created_at timestamptz not null default now(),
  primary key (project_id, label_id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, label_id) references project_label (organization_id, id) on delete cascade,
  foreign key (organization_id, group_id) references project_label (organization_id, id) on delete cascade
);
create index project_to_label_label_idx on project_to_label (organization_id, label_id);
create unique index project_to_label_group_uq on project_to_label (project_id, group_id) where group_id is not null;

create table initiative (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  number int not null check (number > 0),
  name text not null check (char_length(name) >= 1),
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12)
    check (slug_id ~ '^[0-9a-f]{12}$'),
  description text check (char_length(description) <= 255),
  content text,
  color text check (color ~ '^#[0-9A-Fa-f]{6}$'),
  icon text,
  status text not null default 'Planned' check (status in ('Proposed','Planned','Active','Completed','Canceled')),
  owner_id uuid,
  creator_id uuid,
  lead_team_id uuid,
  priority int not null default 0 check (priority between 0 and 4),
  priority_sort_order double precision not null default 0,
  sort_order double precision not null default 0,
  target_date date,
  target_date_resolution text check (target_date_resolution in ('month','quarter','halfYear','year')),
  started_at timestamptz,
  completed_at timestamptz,
  health text check (health in ('onTrack','atRisk','offTrack')),
  health_updated_at timestamptz,
  last_update_id uuid,
  update_reminder_frequency_in_weeks int check (update_reminder_frequency_in_weeks between 0 and 4),
  update_reminders_day text check (update_reminders_day in ('Monday','Tuesday','Wednesday','Thursday','Friday','Saturday','Sunday')),
  update_reminders_hour int check (update_reminders_hour between 0 and 23),
  include_sub_initiative_projects boolean not null default true,
  trashed boolean not null default false,
  trashed_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, number),
  unique (organization_id, slug_id),
  foreign key (organization_id, owner_id) references "user" (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, lead_team_id) references team (organization_id, id)
);
create index initiative_status_idx on initiative (organization_id, status);
create unique index initiative_name_uq on initiative (organization_id, lower(name)) where archived_at is null and not trashed;
create index initiative_trashed_idx on initiative (trashed_at) where trashed;

-- DAG der Initiative-Hierarchie (mehrere Eltern moeglich). Zyklus-/Tiefenpruefung im Core (docs/01 §7.8).
create table initiative_parent (
  organization_id uuid not null references organization (id) on delete cascade,
  parent_id uuid not null,
  child_id uuid not null,
  sort_order double precision not null default 0,
  created_at timestamptz not null default now(),
  primary key (parent_id, child_id),
  foreign key (organization_id, parent_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, child_id) references initiative (organization_id, id) on delete cascade,
  check (parent_id <> child_id)
);
create index initiative_parent_child_idx on initiative_parent (organization_id, child_id);

create table initiative_to_project (
  organization_id uuid not null references organization (id) on delete cascade,
  initiative_id uuid not null,
  project_id uuid not null,
  sort_order double precision not null default 0,
  created_at timestamptz not null default now(),
  primary key (initiative_id, project_id),
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade
);
create index initiative_to_project_project_idx on initiative_to_project (organization_id, project_id);

-- Workspace-weite Labels fuer Initiativen (kein Team-Scope).
create table initiative_label (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) >= 1),
  color text not null check (color ~ '^#[0-9A-Fa-f]{6}$'),
  description text,
  parent_id uuid,
  is_group boolean not null default false,
  inherited_from_id uuid,
  creator_id uuid,
  retired_at timestamptz,
  retired_by_id uuid,
  last_applied_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, parent_id) references initiative_label (organization_id, id),
  foreign key (organization_id, inherited_from_id) references initiative_label (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, retired_by_id) references "user" (organization_id, id),
  check (not is_group or parent_id is null),
  check (parent_id is distinct from id)
);
create unique index initiative_label_name_uq on initiative_label (organization_id, parent_id, lower(name))
  nulls not distinct where archived_at is null;

create table initiative_to_label (
  organization_id uuid not null references organization (id) on delete cascade,
  initiative_id uuid not null,
  label_id uuid not null,
  group_id uuid,
  created_at timestamptz not null default now(),
  primary key (initiative_id, label_id),
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, label_id) references initiative_label (organization_id, id) on delete cascade,
  foreign key (organization_id, group_id) references initiative_label (organization_id, id) on delete cascade
);
create index initiative_to_label_label_idx on initiative_to_label (organization_id, label_id);
create unique index initiative_to_label_group_uq on initiative_to_label (initiative_id, group_id) where group_id is not null;

create table project_relation (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  related_project_id uuid not null,
  type text not null default 'blocks' check (type in ('blocks')),
  project_milestone_id uuid,
  related_project_milestone_id uuid,
  user_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (project_id, related_project_id, type),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, related_project_id) references project (organization_id, id) on delete cascade,
  -- Anker-Meilensteine muessen zum jeweiligen Projekt gehoeren.
  foreign key (organization_id, project_id, project_milestone_id)
    references project_milestone (organization_id, project_id, id),
  foreign key (organization_id, related_project_id, related_project_milestone_id)
    references project_milestone (organization_id, project_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  check (project_id <> related_project_id)
);
create index project_relation_related_idx on project_relation (organization_id, related_project_id);
