-- 0013: WorkflowState, IssueLabel, Template + nachgeholte Team-FKs (docs/01 §2.5, §3.3, §5.9).

create table workflow_state (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  name text not null check (char_length(name) between 1 and 80),
  type text not null check (type in ('triage','backlog','unstarted','started','completed','canceled','duplicate')),
  color text not null check (color ~ '^#[0-9A-Fa-f]{6}$'),
  description text,
  position double precision not null default 0,
  inherited_from_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  -- Ziel fuer Issue.state_id/Team.*_state_id: erzwingt "Status gehoert zum Team" (Issue-Invariante 1).
  unique (organization_id, team_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, inherited_from_id) references workflow_state (organization_id, id)
);
create trigger workflow_state_type_immutable before update of type on workflow_state
  for each row execute function lineal_forbid_type_change();
create index workflow_state_team_idx on workflow_state (organization_id, team_id, type, position);
create unique index workflow_state_name_uq on workflow_state (team_id, lower(name)) where archived_at is null;
-- docs/01 §7.3: genau ein duplicate-Status je Team, hoechstens ein aktiver triage-Status.
create unique index workflow_state_duplicate_uq on workflow_state (team_id) where type = 'duplicate';
create unique index workflow_state_triage_uq on workflow_state (team_id) where type = 'triage' and archived_at is null;

-- Nachgeholte Team-FKs: (organization_id, id, state_id) -> workflow_state(organization_id, team_id, id)
-- erzwingt, dass der referenzierte Status zum Team selbst gehoert (Team-Invariante 7).
-- MATCH SIMPLE: ist die Referenzspalte null, wird nicht geprueft. Die Kategorie (backlog/unstarted, triage, canceled)
-- prueft der Core.
alter table team
  add constraint team_default_issue_state_fk foreign key (organization_id, id, default_issue_state_id)
    references workflow_state (organization_id, team_id, id),
  add constraint team_triage_issue_state_fk foreign key (organization_id, id, triage_issue_state_id)
    references workflow_state (organization_id, team_id, id),
  add constraint team_auto_close_state_fk foreign key (organization_id, id, auto_close_state_id)
    references workflow_state (organization_id, team_id, id);

create table issue_label (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) between 1 and 80
    and lower(name) not in ('assignee','cycle','effort','estimate','hours','priority','project','state','status')),
  color text not null check (color ~ '^#[0-9A-Fa-f]{6}$'),
  description text,
  team_id uuid,
  parent_id uuid,
  is_group boolean not null default false,
  group_type text check (group_type in ('singleSelect','multiSelect')),
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
  foreign key (organization_id, parent_id) references issue_label (organization_id, id),
  foreign key (organization_id, inherited_from_id) references issue_label (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, retired_by_id) references "user" (organization_id, id),
  -- Invariante 2/5: Gruppen haben keinen Parent; groupType nur bei Gruppen (null = singleSelect).
  check (not is_group or parent_id is null),
  check (group_type is null or is_group),
  check (parent_id is distinct from id)
);
-- Annahme: "Child hat denselben Scope wie die Gruppe", "Parent ist Gruppe" und die Namenskollision
-- Team-Label vs. Workspace-Label prueft der Core (docs/01 §3.3 Invariante 1-3).
create unique index issue_label_name_uq on issue_label (organization_id, team_id, parent_id, lower(name))
  nulls not distinct where archived_at is null;
create index issue_label_team_idx on issue_label (organization_id, team_id);
create index issue_label_parent_idx on issue_label (organization_id, parent_id);

create table template (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  type text not null check (type in ('issue','project','document')),
  name text not null check (char_length(name) >= 1),
  description text,
  icon text,
  color text,
  team_id uuid,
  template_data jsonb not null default '{}',
  sort_order double precision not null default 0,
  inherited_from_id uuid,
  creator_id uuid,
  last_updated_by_id uuid,
  last_applied_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id),
  foreign key (organization_id, inherited_from_id) references template (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, last_updated_by_id) references "user" (organization_id, id)
);
create index template_scope_idx on template (organization_id, team_id, type);
create unique index template_name_uq on template (organization_id, team_id, type, lower(name))
  nulls not distinct where archived_at is null;

alter table team
  add constraint team_default_template_members_fk foreign key (organization_id, default_template_for_members_id)
    references template (organization_id, id),
  add constraint team_default_template_non_members_fk foreign key (organization_id, default_template_for_non_members_id)
    references template (organization_id, id),
  add constraint team_default_project_template_fk foreign key (organization_id, default_project_template_id)
    references template (organization_id, id);
