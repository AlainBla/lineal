-- F.10 "Remind me" (docs/05 §6.10): persönliche Erinnerung an Issue, Projekt, Initiative oder
-- Dokument. Je Nutzer und Objekt höchstens eine; erneutes Setzen ersetzt Zeitpunkt und
-- setzt fired_at zurück. Der periodische Job erzeugt bei Fälligkeit eine Notification
-- (issueReminder, projectReminder, initiativeReminder, documentReminder).
create table reminder (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  target_type text not null check (target_type in ('issue','project','initiative','document')),
  issue_id uuid,
  project_id uuid,
  initiative_id uuid,
  document_id uuid,
  remind_at timestamptz not null,
  fired_at timestamptz,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade,
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, initiative_id) references initiative (organization_id, id) on delete cascade,
  foreign key (organization_id, document_id) references document (organization_id, id) on delete cascade,
  check (num_nonnulls(issue_id, project_id, initiative_id, document_id) = 1
    and case target_type
      when 'issue' then issue_id is not null
      when 'project' then project_id is not null
      when 'initiative' then initiative_id is not null
      when 'document' then document_id is not null
    end)
);
create unique index reminder_target_uq on reminder
  (organization_id, user_id, target_type, issue_id, project_id, initiative_id, document_id) nulls not distinct;
create index reminder_due_idx on reminder (remind_at) where fired_at is null;
