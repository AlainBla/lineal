-- 3100 (WP34): Volltextkonfiguration je Workspace (simple | german | english, Default simple).
-- Die generierten search_vector-Spalten (0015, 1100) hatten die Konfiguration fest eingebaut. Eine
-- generierte Spalte kann nicht von einer anderen Tabelle abhängen, daher werden sie zu normalen
-- Spalten, die ein BEFORE-Trigger aus der Konfiguration des Workspace füllt. Beim Wechsel der
-- Konfiguration baut ein AFTER-Trigger auf organization alle Vektoren des Workspace neu auf.

alter table organization add column fulltext_config text not null default 'simple'
  check (fulltext_config in ('simple', 'german', 'english'));

create function lineal_ts_config(org uuid) returns regconfig
  language sql stable as $$
  select coalesce(
    (select case fulltext_config when 'german' then 'german' when 'english' then 'english'
                else 'simple' end::regconfig
       from organization where id = org),
    'simple'::regconfig)
$$;

create function lineal_tsv_issue(c regconfig, title text, description text) returns tsvector
  language sql immutable as $$
  select setweight(to_tsvector(c, coalesce(title, '')), 'A') ||
         setweight(to_tsvector(c, left(coalesce(description, ''), 100000)), 'B')
$$;
create function lineal_tsv_project(c regconfig, name text, description text, content text)
  returns tsvector language sql immutable as $$
  select setweight(to_tsvector(c, coalesce(name, '')), 'A') ||
         setweight(to_tsvector(c, coalesce(description, '')), 'B') ||
         setweight(to_tsvector(c, left(coalesce(content, ''), 100000)), 'C')
$$;
create function lineal_tsv_document(c regconfig, title text, content text) returns tsvector
  language sql immutable as $$
  select setweight(to_tsvector(c, coalesce(title, '')), 'A') ||
         setweight(to_tsvector(c, left(coalesce(content, ''), 100000)), 'B')
$$;
create function lineal_tsv_comment(c regconfig, body text) returns tsvector
  language sql immutable as $$
  select setweight(to_tsvector(c, left(coalesce(body, ''), 100000)), 'C')
$$;

-- Spalten ersetzen (der GIN-Index fällt mit der Spalte weg).
alter table issue drop column search_vector;
alter table issue add column search_vector tsvector;
alter table project drop column search_vector;
alter table project add column search_vector tsvector;
alter table initiative drop column search_vector;
alter table initiative add column search_vector tsvector;
alter table document drop column search_vector;
alter table document add column search_vector tsvector;
alter table comment drop column search_vector;
alter table comment add column search_vector tsvector;

update issue set search_vector =
  lineal_tsv_issue(lineal_ts_config(organization_id), title, description);
update project set search_vector =
  lineal_tsv_project(lineal_ts_config(organization_id), name, description, content);
update initiative set search_vector =
  lineal_tsv_project(lineal_ts_config(organization_id), name, description, content);
update document set search_vector =
  lineal_tsv_document(lineal_ts_config(organization_id), title, content);
update comment set search_vector =
  lineal_tsv_comment(lineal_ts_config(organization_id), body);

create index issue_search_idx on issue using gin (search_vector);
create index project_search_idx on project using gin (search_vector);
create index initiative_search_idx on initiative using gin (search_vector);
create index document_search_idx on document using gin (search_vector);
create index comment_search_idx on comment using gin (search_vector);

create function lineal_trg_issue_tsv() returns trigger language plpgsql as $$
begin
  new.search_vector := lineal_tsv_issue(lineal_ts_config(new.organization_id), new.title, new.description);
  return new;
end $$;
create trigger issue_tsv_trg before insert or update of title, description on issue
  for each row execute function lineal_trg_issue_tsv();

create function lineal_trg_project_tsv() returns trigger language plpgsql as $$
begin
  new.search_vector := lineal_tsv_project(lineal_ts_config(new.organization_id), new.name, new.description, new.content);
  return new;
end $$;
create trigger project_tsv_trg before insert or update of name, description, content on project
  for each row execute function lineal_trg_project_tsv();
create trigger initiative_tsv_trg before insert or update of name, description, content on initiative
  for each row execute function lineal_trg_project_tsv();

create function lineal_trg_document_tsv() returns trigger language plpgsql as $$
begin
  new.search_vector := lineal_tsv_document(lineal_ts_config(new.organization_id), new.title, new.content);
  return new;
end $$;
create trigger document_tsv_trg before insert or update of title, content on document
  for each row execute function lineal_trg_document_tsv();

create function lineal_trg_comment_tsv() returns trigger language plpgsql as $$
begin
  new.search_vector := lineal_tsv_comment(lineal_ts_config(new.organization_id), new.body);
  return new;
end $$;
create trigger comment_tsv_trg before insert or update of body on comment
  for each row execute function lineal_trg_comment_tsv();

-- Wechsel der Konfiguration: alle Vektoren des Workspace neu aufbauen (nur search_vector wird
-- geschrieben, daher feuern die Zeilen-Trigger oben nicht erneut).
create function lineal_trg_org_fulltext() returns trigger language plpgsql as $$
declare c regconfig := (case new.fulltext_config when 'german' then 'german'
  when 'english' then 'english' else 'simple' end)::regconfig;
begin
  update issue set search_vector = lineal_tsv_issue(c, title, description)
    where organization_id = new.id;
  update project set search_vector = lineal_tsv_project(c, name, description, content)
    where organization_id = new.id;
  update initiative set search_vector = lineal_tsv_project(c, name, description, content)
    where organization_id = new.id;
  update document set search_vector = lineal_tsv_document(c, title, content)
    where organization_id = new.id;
  update comment set search_vector = lineal_tsv_comment(c, body)
    where organization_id = new.id;
  return new;
end $$;
create trigger organization_fulltext_trg after update of fulltext_config on organization
  for each row when (old.fulltext_config is distinct from new.fulltext_config)
  execute function lineal_trg_org_fulltext();
