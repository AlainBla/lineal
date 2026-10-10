-- 0018: CustomView, ViewPreferences, Favorite, Notification, NotificationSubscription,
-- NotificationChannelPreference (docs/01 §5.1-5.6).

create table custom_view (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) between 1 and 80),
  description text,
  icon text,
  color text check (color ~ '^#[0-9A-Fa-f]{6}$'),
  slug_id text not null default substr(replace(gen_random_uuid()::text, '-', ''), 1, 12)
    check (slug_id ~ '^[0-9a-f]{12}$'),
  model_name text not null check (model_name in ('Issue','Project','Initiative')),
  filter_data jsonb,
  project_filter_data jsonb,
  initiative_filter_data jsonb,
  owner_id uuid not null,
  creator_id uuid not null,
  updated_by_id uuid,
  shared boolean not null default false,
  team_id uuid,
  project_id uuid,
  initiative_id uuid,
  sort_order double precision not null default 0,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, slug_id),
  foreign key (organization_id, owner_id) references "user" (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id),
  foreign key (organization_id, updated_by_id) references "user" (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  -- Invariante 1: genau ein Filter, passend zu model_name.
  check ((model_name = 'Issue' and filter_data is not null and project_filter_data is null and initiative_filter_data is null)
      or (model_name = 'Project' and project_filter_data is not null and filter_data is null and initiative_filter_data is null)
      or (model_name = 'Initiative' and initiative_filter_data is not null and filter_data is null and project_filter_data is null)),
  -- Invariante 2: hoechstens ein Scope.
  check (num_nonnulls(team_id, project_id, initiative_id) <= 1)
);
create index custom_view_owner_idx on custom_view (organization_id, owner_id);
create index custom_view_team_idx on custom_view (organization_id, team_id) where team_id is not null;
create index custom_view_project_idx on custom_view (organization_id, project_id) where project_id is not null;
create index custom_view_initiative_idx on custom_view (organization_id, initiative_id) where initiative_id is not null;

create table view_preferences (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  type text not null default 'user' check (type in ('organization','user')),
  user_id uuid,
  custom_view_id uuid,
  view_type text,
  -- Annahme: view_target_id ist polymorph (Team, Projekt ...) ohne FK.
  view_target_id uuid,
  preferences jsonb not null default '{}',
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, custom_view_id) references custom_view (organization_id, id) on delete cascade,
  check ((type = 'user') = (user_id is not null)),
  -- Invariante 1: entweder gespeicherte View oder vordefinierte View.
  check ((custom_view_id is not null) <> (view_type is not null)),
  check (custom_view_id is null or view_target_id is null)
);
-- Invariante 2: eindeutig je (type, user, View-Ziel).
create unique index view_preferences_uq on view_preferences
  (organization_id, type, user_id, custom_view_id, view_type, view_target_id) nulls not distinct;

create table favorite (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  owner_id uuid not null,
  type text not null check (type in ('folder','issue','project','cycle','customView','predefinedView','document',
                                      'initiative','label','projectLabel','team','user')),
  parent_id uuid,
  folder_name text,
  sort_order double precision not null default 0,
  issue_id uuid,
  project_id uuid,
  cycle_id uuid,
  custom_view_id uuid,
  document_id uuid,
  initiative_id uuid,
  label_id uuid,
  project_label_id uuid,
  team_id uuid,
  user_id uuid,
  predefined_view_type text,
  predefined_view_team_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, owner_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, parent_id) references favorite (organization_id, id) on delete cascade,
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, cycle_id) references cycle (organization_id, id) on delete cascade,
  foreign key (organization_id, custom_view_id) references custom_view (organization_id, id) on delete cascade,
  foreign key (organization_id, document_id) references document (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, label_id) references issue_label (organization_id, id) on delete cascade,
  foreign key (organization_id, project_label_id) references project_label (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, predefined_view_team_id) references team (organization_id, id) on delete cascade,
  -- Invariante 1: genau das zu type passende Zielfeld (bei folder: keines, dafuer folder_name).
  check (case type
    when 'folder' then folder_name is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 0
    when 'issue' then issue_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'project' then project_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'cycle' then cycle_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'customView' then custom_view_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'document' then document_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'initiative' then initiative_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'label' then label_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'projectLabel' then project_label_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'team' then team_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'user' then user_id is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 1
    when 'predefinedView' then predefined_view_type is not null
      and num_nonnulls(issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id, label_id,
                       project_label_id, team_id, user_id) = 0
    end),
  check (parent_id is distinct from id)
);
-- Annahme: Ordner nur eine Ebene tief (parent.type = folder) prueft der Core.
create index favorite_owner_idx on favorite (organization_id, owner_id, parent_id, sort_order);
-- Invariante 3: je Besitzer und Ziel hoechstens ein Favorit (Ordner ausgenommen).
create unique index favorite_target_uq on favorite
  (organization_id, owner_id, type, issue_id, project_id, cycle_id, custom_view_id, document_id, initiative_id,
   label_id, project_label_id, team_id, user_id, predefined_view_type, predefined_view_team_id)
  nulls not distinct where type <> 'folder';

create table notification (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  type text not null check (char_length(type) >= 1),
  category text not null check (category in ('assignments','mentions','commentsAndReplies','reactions','statusChanges',
    'subscriptions','triage','reminders','documentChanges','postsAndUpdates','system')),
  target_type text not null check (target_type in ('issue','project','initiative','document','system')),
  actor_id uuid,
  issue_id uuid,
  team_id uuid,
  project_id uuid,
  project_update_id uuid,
  project_milestone_id uuid,
  initiative_id uuid,
  initiative_update_id uuid,
  document_id uuid,
  comment_id uuid,
  parent_comment_id uuid,
  reaction_emoji text,
  read_at timestamptz,
  snoozed_until_at timestamptz,
  unsnoozed_at timestamptz,
  emailed_at timestamptz,
  is_priority boolean not null default false,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, actor_id) references "user" (organization_id, id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, project_update_id) references project_update (organization_id, id) on delete cascade,
  foreign key (organization_id, project_milestone_id) references project_milestone (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_update_id) references initiative_update (organization_id, id) on delete cascade,
  foreign key (organization_id, document_id) references document (organization_id, id) on delete cascade,
  foreign key (organization_id, comment_id) references comment (organization_id, id) on delete cascade,
  foreign key (organization_id, parent_comment_id) references comment (organization_id, id) on delete cascade,
  -- Invariante 1: Hauptfeld passend zu target_type.
  check (case target_type
    when 'issue' then issue_id is not null
    when 'project' then project_id is not null
    when 'initiative' then initiative_id is not null
    when 'document' then document_id is not null
    else true end),
  -- Invariante 2: eigene Aktionen erzeugen keine Benachrichtigung.
  check (actor_id is distinct from user_id)
);
create index notification_inbox_idx on notification (organization_id, user_id, archived_at, created_at desc);
create index notification_unread_idx on notification (organization_id, user_id) where read_at is null;
create index notification_snooze_idx on notification (snoozed_until_at) where snoozed_until_at is not null;
create index notification_issue_idx on notification (organization_id, issue_id) where issue_id is not null;

create table notification_subscription (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  subscriber_id uuid not null,
  target_type text not null check (target_type in ('team','project','initiative','cycle','customView','label','user')),
  team_id uuid,
  project_id uuid,
  initiative_id uuid,
  cycle_id uuid,
  custom_view_id uuid,
  label_id uuid,
  user_id uuid,
  context_view_type text check (context_view_type in ('activeCycle','activeIssues','backlog','triage','upcomingCycle')),
  user_context_view_type text check (user_context_view_type in ('assigned')),
  notification_subscription_types text[] not null default '{}',
  include_sub_initiative_updates boolean not null default false,
  active boolean not null default true,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, subscriber_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, cycle_id) references cycle (organization_id, id) on delete cascade,
  foreign key (organization_id, custom_view_id) references custom_view (organization_id, id) on delete cascade,
  foreign key (organization_id, label_id) references issue_label (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  -- Invariante 1: genau das zu target_type passende Zielfeld.
  check (num_nonnulls(team_id, project_id, initiative_id, cycle_id, custom_view_id, label_id, user_id) = 1
    and case target_type
      when 'team' then team_id is not null
      when 'project' then project_id is not null
      when 'initiative' then initiative_id is not null
      when 'cycle' then cycle_id is not null
      when 'customView' then custom_view_id is not null
      when 'label' then label_id is not null
      when 'user' then user_id is not null
    end),
  check (context_view_type is null or target_type = 'team'),
  check (user_context_view_type is null or target_type = 'user')
);
create unique index notification_subscription_uq on notification_subscription
  (organization_id, subscriber_id, target_type, team_id, project_id, initiative_id, cycle_id, custom_view_id,
   label_id, user_id, context_view_type, user_context_view_type) nulls not distinct;
create index notification_subscription_subscriber_idx on notification_subscription (organization_id, subscriber_id);

create table notification_channel_preference (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  category text not null check (category in ('assignments','mentions','commentsAndReplies','reactions','statusChanges',
    'subscriptions','triage','reminders','documentChanges','postsAndUpdates','system')),
  channel text not null check (channel in ('desktop','mobile','email','slack')),
  enabled boolean not null default true,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (user_id, category, channel),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade
);
