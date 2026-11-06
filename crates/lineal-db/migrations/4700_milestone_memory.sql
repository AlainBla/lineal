-- LIN-25 Stufe 3: Gedaechtnis des Meilensteins. `done` ist kein berechneter Status mehr: Abschliessen
-- ist eine bewusste Handlung, `completed_at` haelt sie fest. `snapshot` friert die Issue-Liste ein.
alter table project_milestone
  add column completed_at timestamptz,
  add column completed_by_id uuid,
  add column result_note text,
  add column snapshot jsonb,
  add constraint project_milestone_completed_by_fk
    foreign key (organization_id, completed_by_id) references "user" (organization_id, id);

create table project_milestone_date_change (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  project_milestone_id uuid not null,
  from_date date,
  to_date date,
  actor_id uuid,
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  foreign key (organization_id, project_milestone_id) references project_milestone (organization_id, id) on delete cascade,
  foreign key (organization_id, actor_id) references "user" (organization_id, id)
);
create index project_milestone_date_change_idx
  on project_milestone_date_change (organization_id, project_milestone_id, created_at);

-- Bestehende Meilensteine, die bisher als done galten: Zeitpunkt des letzten erledigten Issues,
-- ohne Snapshot (dieselbe Regel gilt nach einem Import, siehe complete_finished_milestones).
update project_milestone m set completed_at = d.last_completed
from (
  select i.organization_id, i.project_milestone_id, max(coalesce(i.completed_at, i.updated_at)) as last_completed
  from issue i
  where i.project_milestone_id is not null and not i.trashed
    and i.state_type not in ('canceled', 'duplicate')
  group by i.organization_id, i.project_milestone_id
  having bool_and(i.state_type = 'completed')
) d
where m.organization_id = d.organization_id and m.id = d.project_milestone_id
  and m.archived_at is null and m.completed_at is null;
