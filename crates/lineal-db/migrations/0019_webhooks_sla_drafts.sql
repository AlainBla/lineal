-- 0019: Webhook, SlaRule, Draft (docs/01 §2.10, §3.11, §3.12).
-- Triage-Verantwortung/Schichtplaene stehen in 0012, die Triage-Felder am Team/Issue in 0012/0015.

create table webhook (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  label text,
  -- Annahme: HTTPS-Pflicht prueft der Core (in Tests/Dev sind http-Ziele noetig); die DB verlangt nur http(s).
  url text not null check (url ~* '^https?://'),
  -- Annahme: Secret wird verschluesselt gespeichert (Core), hier nur der Chiffretext.
  secret text not null,
  enabled boolean not null default true,
  team_id uuid,
  all_public_teams boolean not null default false,
  resource_types text[] not null check (cardinality(resource_types) >= 1 and resource_types <@ array[
    'Issue','Comment','Attachment','IssueLabel','Reaction','Project','ProjectUpdate','Cycle','Document',
    'Initiative','InitiativeUpdate','User','IssueSLA']),
  creator_id uuid not null,
  failure_count int not null default 0 check (failure_count >= 0),
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  -- Invariante 1: genau eines von team_id / all_public_teams.
  check ((team_id is not null) <> all_public_teams)
);
create index webhook_org_enabled_idx on webhook (organization_id) where enabled and archived_at is null;
create index webhook_team_idx on webhook (organization_id, team_id) where team_id is not null;

create table sla_rule (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid,
  position double precision not null default 0,
  filter jsonb not null,
  action text not null default 'set' check (action in ('set','remove')),
  duration text,
  sla_type text not null default 'all' check (sla_type in ('all','onlyBusinessDays')),
  enabled boolean not null default true,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  -- duration (ISO-8601) ist bei action = set Pflicht.
  check (action <> 'set' or duration is not null)
);
create index sla_rule_order_idx on sla_rule (organization_id, team_id, position);

create table draft (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  team_id uuid,
  title text check (char_length(title) <= 255),
  description text,
  template_id uuid,
  properties jsonb not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id) on delete set null (team_id),
  foreign key (organization_id, template_id) references template (organization_id, id) on delete set null (template_id),
  -- Invariante 2: title oder description nicht leer.
  check (coalesce(title, '') <> '' or coalesce(description, '') <> '')
);
create index draft_user_idx on draft (organization_id, user_id, updated_at);
