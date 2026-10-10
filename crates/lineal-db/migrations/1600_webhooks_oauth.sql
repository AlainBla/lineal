-- 1600 (WP16): Webhook-Zustellprotokoll, OAuth-Autorisierungscodes, Dynamic Client Registration.
-- Nur Ergaenzungen (docs/12 Migrationsbereiche).

alter table webhook
  add column oauth_application_id uuid,
  add column disabled_reason text,
  add column last_success_at timestamptz,
  -- Vorheriges Secret (verschluesselt) fuer den Header Lineal-Signature-Previous (24 h nach Rotation).
  add column previous_secret text,
  add column secret_rotated_at timestamptz,
  add foreign key (organization_id, oauth_application_id)
    references oauth_application (organization_id, id) on delete cascade;

-- Zustellprotokoll (docs/09 §4.5). Eindeutig je (Webhook, Event): macht den at-least-once-Konsumenten idempotent.
create table webhook_delivery (
  id uuid primary key,
  organization_id uuid not null references organization (id) on delete cascade,
  webhook_id uuid not null,
  event_id uuid not null,
  resource_type text not null,
  action text not null,
  attempt int not null default 0 check (attempt >= 0),
  status text not null default 'pending' check (status in ('pending','succeeded','given_up')),
  response_code int,
  duration_ms int,
  error text,
  -- Payload ohne webhookTimestamp (wird bei jedem Versuch neu gesetzt).
  payload jsonb not null,
  next_attempt_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (webhook_id, event_id),
  foreign key (organization_id, webhook_id) references webhook (organization_id, id) on delete cascade
);
create index webhook_delivery_due_idx on webhook_delivery (next_attempt_at) where status = 'pending';
create index webhook_delivery_webhook_idx on webhook_delivery (organization_id, webhook_id, created_at desc);

-- Dynamic Client Registration (RFC 7591): global, ohne Workspace. Pro Workspace wird beim ersten
-- Consent ein Spiegel in oauth_application angelegt (oauth_application.dcr_client_id).
create table oauth_client_registration (
  client_id text primary key,
  client_name text not null check (char_length(client_name) between 1 and 200),
  redirect_uris text[] not null check (cardinality(redirect_uris) between 1 and 10),
  grant_types text[] not null default array['authorization_code','refresh_token'],
  token_endpoint_auth_method text not null default 'none' check (token_endpoint_auth_method = 'none'),
  created_at timestamptz not null default now()
);

alter table oauth_application add column dcr_client_id text references oauth_client_registration (client_id);
create unique index oauth_application_dcr_uq on oauth_application (organization_id, dcr_client_id)
  where dcr_client_id is not null;

-- Einmalige, kurzlebige Autorisierungscodes (nur SHA-256-Hash). family_id verbindet alle daraus
-- entstehenden Tokens (Wiederverwendung widerruft die Familie).
create table oauth_authorization_code (
  code_hash text primary key,
  organization_id uuid not null references organization (id) on delete cascade,
  application_id uuid not null,
  user_id uuid not null,
  redirect_uri text not null,
  scopes text[] not null,
  code_challenge text not null,
  code_challenge_method text not null check (code_challenge_method = 'S256'),
  actor text not null default 'user' check (actor in ('user','app')),
  family_id uuid not null,
  expires_at timestamptz not null,
  used_at timestamptz,
  created_at timestamptz not null default now(),
  foreign key (organization_id, application_id) references oauth_application (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade
);
create index oauth_authorization_code_exp_idx on oauth_authorization_code (expires_at);

create index oauth_token_prev_refresh_idx on oauth_token (previous_refresh_token_hash)
  where previous_refresh_token_hash is not null;
