-- 0020: Import-Tabellen (docs/13 §3, §5). user.status = 'imported' (0011), team.import_locked_until (0012) und
-- issue_history.source in (user, system, import) (0015) sind bereits in den jeweiligen Tabellen enthalten.

create table import_run (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  source text not null check (source in ('linear','linearCsv')),
  status text not null default 'scanning'
    check (status in ('scanning','planned','running','paused','failed','completed')),
  options jsonb not null default '{}',
  checkpoint jsonb not null default '{}',
  stats jsonb not null default '{}',
  started_by uuid,
  started_at timestamptz,
  finished_at timestamptz,
  error text,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  foreign key (organization_id, started_by) references "user" (organization_id, id)
);
create index import_run_org_idx on import_run (organization_id, created_at desc);

create table import_mapping (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  import_run_id uuid not null,
  source text not null check (source in ('linear','linearCsv')),
  source_type text not null,
  source_id text not null,
  -- Annahme: target_id ist polymorph (je source_type eine andere Lineal-Tabelle) und hat keinen FK.
  target_id uuid not null,
  source_updated_at timestamptz,
  checksum text,
  -- Annahme: imported_at fuer die Delta-Regel "updatedAt <= importedAt" (docs/13 §4.2).
  imported_at timestamptz not null default now(),
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, source, source_type, source_id),
  foreign key (organization_id, import_run_id) references import_run (organization_id, id)
);
create index import_mapping_target_idx on import_mapping (organization_id, target_id);
create index import_mapping_run_idx on import_mapping (organization_id, import_run_id);

create table import_issue (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  import_run_id uuid not null,
  severity text not null check (severity in ('info','warning','error')),
  source_type text,
  source_id text,
  message text not null,
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, import_run_id) references import_run (organization_id, id) on delete cascade
);
create index import_issue_run_idx on import_issue (organization_id, import_run_id, severity);

create table import_credential (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  import_run_id uuid not null,
  -- AES-GCM-verschluesselter Token (Server-Schluessel); nie im Klartext, nach Abschluss/7 Tagen geloescht.
  encrypted_token bytea not null,
  expires_at timestamptz not null default (now() + interval '7 days'),
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  unique (import_run_id),
  foreign key (organization_id, import_run_id) references import_run (organization_id, id) on delete cascade
);
