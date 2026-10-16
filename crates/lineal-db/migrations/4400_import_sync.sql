-- 4400: Regelmäßiger Abgleich Linear -> Lineal (docs/13 §12).
-- Je Workspace höchstens eine Einrichtung. Der Linear-API-Schlüssel liegt AES-GCM-verschlüsselt
-- in `import_sync_credential` (AAD = Sync-ID); er wird nie im Klartext gespeichert, ausgegeben
-- oder geloggt. Jeder Lauf ist ein normaler `import_run` mit `options.syncId`.

create table import_sync (
  id uuid primary key,
  organization_id uuid not null references organization (id) on delete cascade,
  source text not null default 'linear' check (source = 'linear'),
  -- off = nur manuell; hourly; every6h; daily (zu `daily_at_minute`, Minuten seit 00:00 UTC)
  interval text not null default 'off' check (interval in ('off', 'hourly', 'every6h', 'daily')),
  daily_at_minute int not null default 180 check (daily_at_minute between 0 and 1439),
  paused boolean not null default false,
  teams text[] not null default '{}',
  projects text[] not null default '{}',
  target_teams jsonb not null default '{}',
  user_mapping jsonb not null default '{}',
  key_mapping jsonb not null default '{}',
  created_by uuid,
  last_run_id uuid,
  last_run_at timestamptz,
  last_success_at timestamptz,
  last_status text,
  last_error text,
  last_removal_check_at timestamptz,
  next_run_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id),
  unique (organization_id, id)
);
create index import_sync_due_idx on import_sync (next_run_at) where next_run_at is not null and not paused;

create table import_sync_credential (
  sync_id uuid primary key references import_sync (id) on delete cascade,
  organization_id uuid not null references organization (id) on delete cascade,
  encrypted_token bytea not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
