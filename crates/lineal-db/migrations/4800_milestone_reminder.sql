-- LIN-25 Stufe 4: "Remind me" auch am Meilenstein.
alter table reminder add column project_milestone_id uuid;
alter table reminder
  add constraint reminder_milestone_fk foreign key (organization_id, project_milestone_id)
    references project_milestone (organization_id, id) on delete cascade;
alter table reminder drop constraint reminder_target_type_check;
alter table reminder drop constraint reminder_check;
alter table reminder add constraint reminder_target_check check (
  target_type in ('issue','project','initiative','document','milestone')
  and num_nonnulls(issue_id, project_id, initiative_id, document_id, project_milestone_id) = 1
  and case target_type
    when 'issue' then issue_id is not null
    when 'project' then project_id is not null
    when 'initiative' then initiative_id is not null
    when 'document' then document_id is not null
    when 'milestone' then project_milestone_id is not null
  end
);
drop index reminder_target_uq;
create unique index reminder_target_uq on reminder
  (organization_id, user_id, target_type, issue_id, project_id, initiative_id, document_id,
   project_milestone_id) nulls not distinct;
