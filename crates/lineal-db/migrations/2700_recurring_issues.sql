-- 2700 (WP30): wiederkehrende Issues (docs/03 §13). Nur Ergaenzungen.
--
-- Eine Regel verweist auf ein Issue-Template des Teams und erzeugt daraus periodisch Issues
-- (Job `recurring_issues`, Akteur `system`). `next_run_at` ist der naechste Lauf (UTC, aus Rhythmus,
-- Uhrzeit und Team-Zeitzone berechnet); `recurring_issue_run` macht jede Erzeugung je Zeitpunkt
-- eindeutig (idempotent bei doppeltem Lauf).
create table recurring_issue (
  id uuid primary key,
  organization_id uuid not null references organization (id) on delete cascade,
  team_id uuid not null,
  template_id uuid not null,
  frequency text not null check (frequency in ('daily', 'weekly', 'monthly')),
  -- Alle N Tage/Wochen/Monate (ab start_date).
  interval_count int not null default 1 check (interval_count between 1 and 365),
  -- ISO-Wochentag 1 (Montag) bis 7 (Sonntag); nur weekly.
  weekday int check (weekday between 1 and 7),
  -- 1..31 (kuerzere Monate: letzter Tag); nur monthly.
  day_of_month int check (day_of_month between 1 and 31),
  -- Lokale Uhrzeit in der Team-Zeitzone, Minuten seit Mitternacht.
  minute_of_day int not null check (minute_of_day between 0 and 1439),
  -- Ankerdatum der Intervallzaehlung (lokales Datum in der Team-Zeitzone).
  start_date date not null,
  paused boolean not null default false,
  next_run_at timestamptz not null,
  last_run_at timestamptz,
  last_error text,
  last_error_at timestamptz,
  creator_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, team_id) references team (organization_id, id) on delete cascade,
  foreign key (organization_id, template_id) references template (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id) on delete set null (creator_id),
  check (frequency = 'weekly' or weekday is null),
  check (frequency = 'monthly' or day_of_month is null)
);
create index recurring_issue_team_idx on recurring_issue (organization_id, team_id);
create index recurring_issue_due_idx on recurring_issue (next_run_at) where not paused;

create table recurring_issue_run (
  recurring_issue_id uuid not null references recurring_issue (id) on delete cascade,
  occurrence_at timestamptz not null,
  issue_id uuid,
  created_at timestamptz not null default now(),
  primary key (recurring_issue_id, occurrence_at)
);
