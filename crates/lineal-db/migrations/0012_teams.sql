-- 0012: Team, TeamKey-Historie, TeamMembership, TimeSchedule(+Entry), TriageResponsibility (docs/01 §2.3, §2.4, §2.6, §2.7).
-- Zirkulaere Referenzen Team -> WorkflowState / Template werden in 0013 per ALTER TABLE ergaenzt
-- (docs/01 §7.1: Team zuerst mit leerer Referenz anlegen, dann Status, dann Referenz setzen).

create table team (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) between 1 and 80),
  -- Team-Key: kein Bindestrich, nicht P/I (docs/00 §2, docs/01 §2.3).
  key text not null check (key ~ '^[A-Z][A-Z0-9]{0,6}$' and key not in ('P', 'I')),
  description text,
  icon text,
  color text check (color ~ '^#[0-9A-Fa-f]{6}$'),
  parent_id uuid,
  visibility text not null default 'public' check (visibility in ('public','private','restricted')),
  all_members_can_join boolean not null default true,
  timezone text not null default 'America/Los_Angeles',
  security_settings jsonb not null default '{}',
  retired_at timestamptz,
  trashed boolean not null default false,
  trashed_at timestamptz,
  -- Zaehler (zuletzt vergebene Nummer); nur atomar erhoehen: UPDATE team SET last_issue_number = last_issue_number + 1 RETURNING ...
  last_issue_number int not null default 0 check (last_issue_number >= 0),
  last_project_number int not null default 0 check (last_project_number >= 0),
  last_cycle_number int not null default 0 check (last_cycle_number >= 0),
  -- Annahme: in der DB nullable, weil Team und WorkflowState sich gegenseitig referenzieren
  -- (docs/01 §7.1); "Pflicht" erzwingt der Core. FK (inkl. gleiches Team) in 0013.
  default_issue_state_id uuid,
  triage_enabled boolean not null default false,
  triage_issue_state_id uuid,
  require_priority_to_leave_triage boolean not null default false,
  auto_close_period int check (auto_close_period in (1, 3, 6, 9, 12)),
  auto_close_state_id uuid,
  auto_archive_period int not null default 6 check (auto_archive_period in (1, 3, 6, 9, 12)),
  auto_close_parent_issues boolean not null default false,
  auto_close_child_issues boolean not null default false,
  set_issue_sort_order_on_state_change text not null default 'top'
    check (set_issue_sort_order_on_state_change in ('top','bottom','none')),
  group_issue_history boolean not null default true,
  inherit_workflow_statuses boolean not null default false,
  issue_estimation_type text not null default 'notUsed'
    check (issue_estimation_type in ('notUsed','exponential','fibonacci','linear','tShirt')),
  issue_estimation_extended boolean not null default false,
  issue_estimation_allow_zero boolean not null default false,
  default_issue_estimate int not null default 1 check (default_issue_estimate >= 0),
  inherit_issue_estimation boolean not null default false,
  cycles_enabled boolean not null default false,
  cycle_duration int not null default 2 check (cycle_duration between 1 and 8),
  cycle_cooldown_time int not null default 0 check (cycle_cooldown_time between 0 and 4),
  cycle_start_day int not null default 1 check (cycle_start_day between 0 and 6),
  upcoming_cycle_count int not null default 2 check (upcoming_cycle_count between 1 and 15),
  cycle_issue_auto_assign_started boolean not null default false,
  cycle_issue_auto_assign_completed boolean not null default false,
  cycle_lock_to_active boolean not null default false,
  default_template_for_members_id uuid,
  default_template_for_non_members_id uuid,
  default_project_template_id uuid,
  inherit_project_statuses boolean not null default true,
  -- docs/13 §4.3: periodische Jobs ueberspringen Teams mit laufendem Import.
  import_locked_until timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, parent_id) references team (organization_id, id),
  check (parent_id is distinct from id),
  -- Invariante 5: restricted nur mit Parent.
  check (visibility <> 'restricted' or parent_id is not null)
);
-- docs/01 §7.3: Key und Name je Workspace eindeutig, partiell ohne Papierkorb.
create unique index team_key_uq on team (organization_id, key) where not trashed;
create unique index team_name_uq on team (organization_id, lower(name)) where not trashed;
create index team_parent_idx on team (organization_id, parent_id);
create index team_trashed_idx on team (trashed_at) where trashed;

-- Historie frueherer Team-Keys (docs/01 §2.3 Invariante 1/2): ein frueherer Key darf nicht an ein anderes
-- Team vergeben werden. Annahme: team_key enthaelt NUR frueher genutzte Keys (der aktuelle steht in team.key);
-- der Core schreibt beim Key-Wechsel den alten Key hierher. Die Trigger pruefen die Gegenseite.
create table team_key (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  key text not null check (key ~ '^[A-Z][A-Z0-9]{0,6}$' and key not in ('P', 'I')),
  replaced_at timestamptz not null default now(),
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade
);
create unique index team_key_history_uq on team_key (organization_id, key);
create index team_key_team_idx on team_key (organization_id, team_id);

create function team_key_guard_team() returns trigger
language plpgsql as $$
begin
  if exists (select 1 from team_key h
             where h.organization_id = new.organization_id and h.key = new.key and h.team_id <> new.id) then
    raise exception 'team key % ist als frueherer Key eines anderen Teams reserviert', new.key
      using errcode = '23505';
  end if;
  return new;
end;
$$;
create trigger team_key_guard before insert or update of key on team
  for each row execute function team_key_guard_team();

create function team_key_guard_history() returns trigger
language plpgsql as $$
begin
  if exists (select 1 from team t
             where t.organization_id = new.organization_id and t.key = new.key
               and t.id <> new.team_id and not t.trashed) then
    raise exception 'team key % wird von einem anderen Team verwendet', new.key
      using errcode = '23505';
  end if;
  return new;
end;
$$;
create trigger team_key_history_guard before insert or update of key, team_id on team_key
  for each row execute function team_key_guard_history();

create table team_membership (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  user_id uuid not null,
  owner boolean not null default false,
  sort_order double precision not null default 0,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create unique index team_membership_uq on team_membership (team_id, user_id) where archived_at is null;
create index team_membership_user_idx on team_membership (organization_id, user_id);
create index team_membership_team_idx on team_membership (organization_id, team_id);

create table time_schedule (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null,
  external_id text,
  external_url text,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id)
);
create unique index time_schedule_external_id_uq on time_schedule (organization_id, external_id) where external_id is not null;

create table time_schedule_entry (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  schedule_id uuid not null,
  user_id uuid,
  user_email text,
  starts_at timestamptz not null,
  ends_at timestamptz not null,
  overridden_user_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, schedule_id) references time_schedule (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  foreign key (organization_id, overridden_user_id) references "user" (organization_id, id),
  check (ends_at > starts_at),
  -- Invariante 1: userId oder userEmail.
  check (user_id is not null or user_email is not null)
);
create index time_schedule_entry_idx on time_schedule_entry (organization_id, schedule_id, starts_at);

create table triage_responsibility (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  action text not null default 'notify' check (action in ('assign','notify')),
  manual_user_ids uuid[],
  assignment_index int not null default 0 check (assignment_index >= 0),
  time_schedule_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, time_schedule_id) references time_schedule (organization_id, id),
  -- Invariante 1: genau eine Quelle (manuelle Liste ODER Schichtplan).
  check ((coalesce(cardinality(manual_user_ids), 0) > 0) <> (time_schedule_id is not null))
);
create unique index triage_responsibility_team_uq on triage_responsibility (team_id);
