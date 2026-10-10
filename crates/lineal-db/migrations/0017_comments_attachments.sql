-- 0017: Comment, Reaction, Attachment, IssueSubscription (docs/01 §3.6-3.8, §3.10).

create table comment (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  body text not null check (char_length(body) between 1 and 100000),
  body_data jsonb,
  user_id uuid,
  issue_id uuid,
  project_id uuid,
  project_milestone_id uuid,
  project_update_id uuid,
  initiative_id uuid,
  initiative_update_id uuid,
  document_id uuid,
  parent_id uuid,
  quoted_text text,
  edited_at timestamptz,
  resolved_at timestamptz,
  resolving_comment_id uuid,
  resolving_user_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, project_milestone_id) references project_milestone (organization_id, id) on delete cascade,
  foreign key (organization_id, project_update_id) references project_update (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_update_id) references initiative_update (organization_id, id) on delete cascade,
  foreign key (organization_id, document_id) references document (organization_id, id) on delete cascade,
  foreign key (organization_id, parent_id) references comment (organization_id, id) on delete cascade,
  foreign key (organization_id, resolving_comment_id) references comment (organization_id, id)
    on delete set null (resolving_comment_id),
  foreign key (organization_id, resolving_user_id) references "user" (organization_id, id),
  -- Invariante 1: genau ein Elternobjekt.
  check (num_nonnulls(issue_id, project_id, project_milestone_id, project_update_id,
                      initiative_id, initiative_update_id, document_id) = 1),
  -- Invariante 3: quoted_text/resolved_*/resolving_comment_id nur bei Top-Level-Kommentaren.
  check (parent_id is null or (quoted_text is null and resolved_at is null and resolving_comment_id is null
                               and resolving_user_id is null)),
  check (parent_id is distinct from id)
);
-- Annahme: "Antwort hat dasselbe Elternobjekt wie ihr Parent", "nur eine Thread-Ebene" und
-- "resolving_comment ist Kind dieses Threads" prueft der Core (Invarianten 2-4).
create index comment_issue_idx on comment (organization_id, issue_id, created_at) where issue_id is not null;
create index comment_project_idx on comment (organization_id, project_id, created_at) where project_id is not null;
create index comment_milestone_idx on comment (organization_id, project_milestone_id, created_at) where project_milestone_id is not null;
create index comment_project_update_idx on comment (organization_id, project_update_id, created_at) where project_update_id is not null;
create index comment_initiative_idx on comment (organization_id, initiative_id, created_at) where initiative_id is not null;
create index comment_initiative_update_idx on comment (organization_id, initiative_update_id, created_at) where initiative_update_id is not null;
create index comment_document_idx on comment (organization_id, document_id, created_at) where document_id is not null;
create index comment_parent_idx on comment (organization_id, parent_id) where parent_id is not null;
create index comment_user_idx on comment (organization_id, user_id);

-- Nachgeholte Issue-FK auf den Ursprungs-Kommentar.
alter table issue
  add constraint issue_source_comment_fk foreign key (organization_id, source_comment_id)
    references comment (organization_id, id) on delete set null (source_comment_id);

create table reaction (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  emoji text not null check (char_length(emoji) >= 1),
  user_id uuid not null,
  issue_id uuid,
  comment_id uuid,
  project_update_id uuid,
  initiative_update_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, comment_id) references comment (organization_id, id) on delete cascade,
  foreign key (organization_id, project_update_id) references project_update (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_update_id) references initiative_update (organization_id, id) on delete cascade,
  -- Invariante 1: genau ein Ziel.
  check (num_nonnulls(issue_id, comment_id, project_update_id, initiative_update_id) = 1)
);
-- Invariante 2: eindeutig je (Ziel, user, emoji) - NULL-sicher via NULLS NOT DISTINCT.
create unique index reaction_uq on reaction (user_id, emoji, issue_id, comment_id, project_update_id, initiative_update_id)
  nulls not distinct;
create index reaction_comment_idx on reaction (organization_id, comment_id) where comment_id is not null;
create index reaction_issue_idx on reaction (organization_id, issue_id) where issue_id is not null;

create table attachment (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  url text not null,
  title text not null,
  subtitle text,
  source_type text,
  content_type text,
  -- Annahme: bigint, weil das Limit "< 2 GB" knapp ueber dem int4-Maximum liegen kann.
  size bigint check (size >= 0),
  storage_key text,
  metadata jsonb not null default '{}',
  group_by_source boolean not null default false,
  creator_id uuid,
  original_issue_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (issue_id, url),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, original_issue_id) references issue (organization_id, id) on delete set null (original_issue_id)
);
create index attachment_issue_idx on attachment (organization_id, issue_id);

create table issue_subscription (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  user_id uuid not null,
  thread_comment_id uuid,
  active boolean not null default true,
  reason text not null check (reason in ('creator','assignee','mention','comment','manual','addedByOther')),
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  foreign key (organization_id, thread_comment_id) references comment (organization_id, id) on delete cascade
);
create unique index issue_subscription_uq on issue_subscription (issue_id, user_id, thread_comment_id) nulls not distinct;
create index issue_subscription_user_idx on issue_subscription (organization_id, user_id);
