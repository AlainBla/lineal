-- LIN-25 Stufe 2: Ressourcen am Meilenstein. Dokumente, Links und Dateien gehoeren weiter dem Projekt,
-- der Meilenstein ist ein optionaler Bezug: Die Rechte folgen dem Projekt, und ohne Meilenstein
-- (geloescht, endgueltig entfernt) bleibt die Ressource am Projekt.
alter table document
  add column project_milestone_id uuid,
  add constraint document_milestone_needs_project
    check (project_milestone_id is null or project_id is not null),
  add constraint document_milestone_fk
    foreign key (organization_id, project_id, project_milestone_id)
    references project_milestone (organization_id, project_id, id)
    on delete set null (project_milestone_id);
create index document_milestone_idx on document (organization_id, project_milestone_id)
  where project_milestone_id is not null;

alter table entity_external_link
  add column project_milestone_id uuid,
  add constraint entity_external_link_milestone_needs_project
    check (project_milestone_id is null or project_id is not null),
  add constraint entity_external_link_milestone_fk
    foreign key (organization_id, project_id, project_milestone_id)
    references project_milestone (organization_id, project_id, id)
    on delete set null (project_milestone_id);
-- Dieselbe URL darf am Projekt und an einem Meilenstein haengen.
drop index entity_external_link_project_uq;
create unique index entity_external_link_project_uq
  on entity_external_link (project_id, project_milestone_id, url) nulls not distinct
  where project_id is not null;

create table project_file (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  project_id uuid not null,
  project_milestone_id uuid,
  url text not null,
  storage_key text not null,
  title text not null,
  content_type text,
  size bigint check (size >= 0),
  creator_id uuid,
  version int not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  archived_at timestamptz,
  unique (organization_id, id),
  unique (organization_id, storage_key),
  foreign key (organization_id, project_id) references project (organization_id, id) on delete cascade,
  foreign key (organization_id, project_id, project_milestone_id)
    references project_milestone (organization_id, project_id, id)
    on delete set null (project_milestone_id),
  foreign key (organization_id, creator_id) references "user" (organization_id, id)
);
create index project_file_project_idx on project_file (organization_id, project_id)
  where archived_at is null;
