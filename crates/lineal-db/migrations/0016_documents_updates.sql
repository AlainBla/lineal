-- 0016: Document (+Subscription), DocumentContent(+History), ProjectUpdate, InitiativeUpdate, EntityExternalLink
-- (docs/01 §5.7, §5.8, §5.10, §4.8, §4.13, §4.15).

create table document (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  title text not null default '' check (char_length(title) <= 255),
  content text,
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12)
    check (slug_id ~ '^[0-9a-f]{12}$'),
  icon text,
  color text check (color ~ '^#[0-9A-Fa-f]{6}$'),
  project_id uuid,
  issue_id uuid,
  initiative_id uuid,
  cycle_id uuid,
  team_id uuid,
  creator_id uuid,
  updated_by_id uuid,
  sort_order double precision not null default 0,
  hidden_at timestamptz,
  last_applied_template_id uuid,
  trashed boolean not null default false,
  trashed_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, slug_id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id),
  -- Invariante 1: Cycle-Dokument -> team_id = cycle.team_id (Zugriffsspalte).
  foreign key (organization_id, team_id, cycle_id) references cycle (organization_id, team_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, updated_by_id) references "user" (organization_id, id),
  foreign key (organization_id, last_applied_template_id) references template (organization_id, id),
  -- Invariante 1: genau ein Elternobjekt aus {Project, Issue, Initiative, Cycle, Team}.
  check (
    -- Project-/Issue-/Initiative-Dokument: genau eines davon, kein Cycle, team_id = null
    (num_nonnulls(project_id, issue_id, initiative_id) = 1 and cycle_id is null and team_id is null)
    -- Cycle-Dokument: team_id ist Pflicht (= Team des Cycles)
    or (cycle_id is not null and project_id is null and issue_id is null and initiative_id is null and team_id is not null)
    -- Team-Dokument
    or (project_id is null and issue_id is null and initiative_id is null and cycle_id is null and team_id is not null)
  )
);
create index document_project_idx on document (organization_id, project_id) where project_id is not null;
create index document_issue_idx on document (organization_id, issue_id) where issue_id is not null;
create index document_initiative_idx on document (organization_id, initiative_id) where initiative_id is not null;
create index document_cycle_idx on document (organization_id, cycle_id) where cycle_id is not null;
create index document_team_idx on document (organization_id, team_id) where team_id is not null;
create index document_trashed_idx on document (trashed_at) where trashed;

create table document_subscription (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  document_id uuid not null,
  user_id uuid not null,
  active boolean not null default true,
  reason text not null check (reason in ('creator','mention','manual','addedByOther')),
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (document_id, user_id),
  foreign key (organization_id, document_id) references document (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create index document_subscription_user_idx on document_subscription (organization_id, user_id);

-- Gemeinsamer Traeger langer Rich-Text-Inhalte. Annahme: owner_id ist polymorph OHNE Fremdschluessel
-- (docs/01 §5.10 / §7.7: genau ein Paar (owner_type, owner_id)); das Loeschen beim Besitzer erledigt der Core.
-- version dient als Basis fuer Patch-Konflikte und optimistische Sperre des Inhalts.
create table document_content (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  owner_type text not null check (owner_type in ('document','issue','project','initiative','projectMilestone')),
  owner_id uuid not null,
  markdown text not null default '',
  content_state text,
  restored_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, owner_type, owner_id)
);

-- Snapshots (append-only).
create table document_content_history (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  document_content_id uuid not null,
  markdown text not null default '',
  title text,
  actor_ids uuid[] not null default '{}',
  content_snapshot_at timestamptz not null default now(),
  kind text not null check (kind in ('created','edited','restored')),
  restored_from_id uuid,
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, document_content_id) references document_content (organization_id, id) on delete cascade,
  foreign key (organization_id, restored_from_id) references document_content_history (organization_id, id)
    on delete set null (restored_from_id)
);
create trigger document_content_history_append_only before update on document_content_history
  for each row execute function lineal_forbid_update();
create index document_content_history_idx on document_content_history (organization_id, document_content_id, content_snapshot_at desc);

create table project_update (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  user_id uuid not null,
  body text not null,
  health text not null default 'onTrack' check (health in ('onTrack','atRisk','offTrack')),
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12),
  edited_at timestamptz,
  progress_snapshot jsonb not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, slug_id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create index project_update_project_idx on project_update (organization_id, project_id, created_at);

create table initiative_update (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  initiative_id uuid not null,
  user_id uuid not null,
  body text not null,
  health text not null default 'onTrack' check (health in ('onTrack','atRisk','offTrack')),
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12),
  edited_at timestamptz,
  progress_snapshot jsonb not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, slug_id),
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create index initiative_update_initiative_idx on initiative_update (organization_id, initiative_id, created_at);

alter table project
  add constraint project_last_update_fk foreign key (organization_id, last_update_id)
    references project_update (organization_id, id) on delete set null (last_update_id);
alter table initiative
  add constraint initiative_last_update_fk foreign key (organization_id, last_update_id)
    references initiative_update (organization_id, id) on delete set null (last_update_id);

create table entity_external_link (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  url text not null,
  label text not null,
  project_id uuid,
  initiative_id uuid,
  sort_order double precision not null default 0,
  creator_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  -- Invariante 1: genau ein Elternobjekt.
  check (num_nonnulls(project_id, initiative_id) = 1)
);
create unique index entity_external_link_project_uq on entity_external_link (project_id, url) where project_id is not null;
create unique index entity_external_link_initiative_uq on entity_external_link (initiative_id, url) where initiative_id is not null;
