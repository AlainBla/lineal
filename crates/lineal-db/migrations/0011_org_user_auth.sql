-- 0011: Organization, User, Sessions, ApiKey, OAuth, Invitation (docs/01 §2.1, §2.2, §2.8, §2.9, §2.11).

create table organization (
  id uuid primary key default gen_random_uuid(),
  name text not null check (char_length(name) between 1 and 100),
  url_key text not null check (url_key ~ '^[a-z0-9-]{3,32}$'),
  -- Annahme: Eindeutigkeit/Reservierung von previous_url_keys (global) prueft der Core;
  -- der GIN-Index unterstuetzt nur die Suche nach einem Key.
  previous_url_keys text[] not null default '{}' check (cardinality(previous_url_keys) <= 3),
  logo_url text,
  fiscal_year_start_month int not null default 0 check (fiscal_year_start_month between 0 and 11),
  initiatives_enabled boolean not null default true,
  -- Zaehler fuer Initiative.number (I-4); Annahme: wie die Team-Zaehler als last_*_number benannt.
  last_initiative_number int not null default 0 check (last_initiative_number >= 0),
  created_issue_count int not null default 0,
  project_update_reminder_frequency_in_weeks int default 1
    check (project_update_reminder_frequency_in_weeks between 1 and 4),
  project_update_reminders_day text not null default 'Friday'
    check (project_update_reminders_day in ('Monday','Tuesday','Wednesday','Thursday','Friday','Saturday','Sunday')),
  project_update_reminders_hour int not null default 14 check (project_update_reminders_hour between 0 and 23),
  initiative_update_reminder_frequency_in_weeks int
    check (initiative_update_reminder_frequency_in_weeks between 1 and 4),
  initiative_update_reminders_day text not null default 'Friday'
    check (initiative_update_reminders_day in ('Monday','Tuesday','Wednesday','Thursday','Friday','Saturday','Sunday')),
  initiative_update_reminders_hour int not null default 14 check (initiative_update_reminders_hour between 0 and 23),
  allowed_file_upload_content_types text[],
  security_settings jsonb not null default '{}',
  limits jsonb not null default '{
    "maxTeams": null,
    "maxSubTeamDepth": 5,
    "maxInitiativeDepth": 5,
    "privateTeamsEnabled": true,
    "maxOpenIssuesPerTeam": 60000,
    "maxLabelsPerGroup": 250,
    "maxUnarchivedNotificationsPerUser": 2000,
    "trashRetentionDays": 30
  }'::jsonb,
  deletion_requested_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz
);
create unique index organization_url_key_uq on organization (url_key);
create index organization_previous_url_keys_idx on organization using gin (previous_url_keys);

create table "user" (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null check (char_length(name) between 1 and 255),
  display_name text not null
    check (display_name ~ '^[a-z0-9._-]{1,40}$' and display_name not in ('me', 'none')),
  email text not null check (position('@' in email) > 1),
  role text not null default 'user' check (role in ('owner','admin','user','guest','app')),
  active boolean not null default true,
  -- docs/13 §5: zusaetzlicher Status 'imported' (Platzhalter-User ohne Login).
  -- Annahme: 'active' = normaler User; Suspendierung bleibt ueber die Spalte active (docs/01).
  status text not null default 'active' check (status in ('active', 'imported')),
  disable_reason text,
  avatar_url text,
  -- Annahme: "aus id abgeleitet" -> DB-Default ist ein zufaelliger Hex-Wert, der Core darf ueberschreiben.
  avatar_background_color text not null default ('#' || substr(md5(gen_random_uuid()::text), 1, 6))
    check (avatar_background_color ~ '^#[0-9A-Fa-f]{6}$'),
  timezone text,
  title text,
  description text,
  status_emoji text,
  status_label text,
  status_until_at timestamptz,
  last_seen timestamptz,
  created_issue_count int not null default 0,
  default_home_view text,
  email_format text not null default 'digest' check (email_format in ('digest','immediate')),
  notification_priority_categories text[] not null default array['assignments','mentions','reminders','triage'],
  due_date_reminders boolean not null default false,
  notification_settings jsonb not null default '{}',
  -- Annahme (docs/11: Passwort-Login mit Session-Cookie): Argon2-Hash am User; null = kein Passwort-Login.
  password_hash text,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id)
);
create unique index user_email_uq on "user" (organization_id, lower(email));
create unique index user_display_name_uq on "user" (organization_id, lower(display_name));
create index user_role_idx on "user" (organization_id, role);

-- Annahme: Session-Tabelle fuer den Web-Login (docs/09 §Auth, docs/11 /auth/login); Token nur als SHA-256-Hash.
create table user_session (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  token_hash text not null,
  csrf_token_hash text,
  user_agent text,
  ip_address text,
  expires_at timestamptz not null,
  last_used_at timestamptz,
  revoked_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade
);
create unique index user_session_token_hash_uq on user_session (token_hash);
create index user_session_user_idx on user_session (organization_id, user_id);

create table api_key (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  label text not null,
  key_prefix text not null,
  key_hash text not null,
  scopes text[] not null default array['read','write'] check ('read' = any (scopes)),
  team_ids uuid[],
  expires_at timestamptz,
  last_used_at timestamptz,
  revoked_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade
);
create unique index api_key_key_hash_uq on api_key (key_hash);
create index api_key_user_idx on api_key (organization_id, user_id);

create table oauth_application (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  name text not null,
  client_id text not null default replace(gen_random_uuid()::text, '-', ''),
  client_secret_hash text not null,
  redirect_uris text[] not null check (cardinality(redirect_uris) >= 1),
  distribution text not null default 'private' check (distribution in ('private','public')),
  grant_types text[] not null default array['authorization_code']
    check ('authorization_code' = any (grant_types)
           and grant_types <@ array['authorization_code','client_credentials']),
  developer text not null,
  developer_url text not null,
  image_url text,
  app_user_id uuid,
  webhook_url text,
  webhook_resource_types text[] not null default '{}',
  creator_id uuid not null,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, app_user_id) references "user" (organization_id, id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id)
);
create unique index oauth_application_client_id_uq on oauth_application (client_id);

create table oauth_token (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  application_id uuid not null,
  user_id uuid not null,
  actor text not null default 'user' check (actor in ('user','app')),
  scopes text[] not null default array['read'],
  access_token_hash text not null,
  access_expires_at timestamptz not null default (now() + interval '24 hours'),
  refresh_token_hash text,
  previous_refresh_token_hash text,
  -- docs/09 §Speicherung: familyId und rotatedAt fuer die Refresh-Rotation.
  family_id uuid not null default gen_random_uuid(),
  rotated_at timestamptz,
  revoked_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, application_id) references oauth_application (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id)
);
create unique index oauth_token_access_hash_uq on oauth_token (access_token_hash);
create unique index oauth_token_refresh_hash_uq on oauth_token (refresh_token_hash) where refresh_token_hash is not null;
create index oauth_token_family_idx on oauth_token (family_id);
create index oauth_token_app_idx on oauth_token (organization_id, application_id);

create table invitation (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  email text not null,
  role text not null default 'user' check (role in ('admin','user','guest')),
  -- Annahme: Werte-Liste statt Join-Tabelle (docs/01 §2.11).
  team_ids uuid[] not null default '{}',
  inviter_id uuid not null,
  token_hash text not null,
  status text not null default 'pending' check (status in ('pending','accepted','revoked','expired')),
  expires_at timestamptz not null default (now() + interval '7 days'),
  last_sent_at timestamptz not null default now(),
  accepted_at timestamptz,
  accepted_user_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, inviter_id) references "user" (organization_id, id),
  foreign key (organization_id, accepted_user_id) references "user" (organization_id, id),
  check (role <> 'guest' or cardinality(team_ids) >= 1)
);
create unique index invitation_token_hash_uq on invitation (token_hash);
create index invitation_email_idx on invitation (organization_id, lower(email));
create unique index invitation_pending_email_uq on invitation (organization_id, lower(email)) where status = 'pending';
