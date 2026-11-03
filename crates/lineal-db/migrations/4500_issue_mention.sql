-- 4500 (LIN-12): Rückverweise („Mentioned in“). Eine Erwähnung `ENG-123` in der Beschreibung eines
-- Issues oder in einem Kommentar legt keine `related`-Relation mehr an (docs/03 §4.2), sondern
-- eine Zeile hier. Die Zeilen werden beim Speichern von Beschreibung/Kommentar ersetzt, beim
-- Löschen des Kommentars entfernt und mit Quelle/Ziel per Cascade gelöscht. Sichtbarkeit wird erst
-- beim Lesen geprüft. Gespeichert (statt zur Lesezeit berechnet), weil `search_vector` eine
-- workspace-abhängige Textsuchkonfiguration und Stemming nutzt und `ENG-123` darin in Teiltoken
-- zerfällt: eine exakte Treffersuche wäre ein Scan mit Nachfilterung.

create table issue_mention (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  source_issue_id uuid not null,
  -- NULL = Beschreibung des Quell-Issues, sonst der erwähnende Kommentar.
  source_comment_id uuid,
  target_issue_id uuid not null,
  created_at timestamptz not null default now(),
  foreign key (organization_id, source_issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, source_comment_id) references comment (organization_id, id) on delete cascade,
  foreign key (organization_id, target_issue_id) references issue (organization_id, id) on delete cascade
);
create unique index issue_mention_description_uq on issue_mention (source_issue_id, target_issue_id)
  where source_comment_id is null;
create unique index issue_mention_comment_uq on issue_mention (source_comment_id, target_issue_id)
  where source_comment_id is not null;
create index issue_mention_target_idx on issue_mention (organization_id, target_issue_id, created_at);
create index issue_mention_source_comment_idx on issue_mention (source_comment_id)
  where source_comment_id is not null;

-- Bestand: Kennungen aus vorhandenen Beschreibungen und Issue-Kommentaren nachtragen (Näherung der
-- Anwendungslogik; Code-Spans werden hier nicht erkannt). Aliase alter Kennungen zählen mit.
create temporary table _lin12_ident on commit drop as
select organization_id, id as issue_id, upper(identifier) as ident from issue
union
select organization_id, issue_id, upper(identifier) from issue_identifier_alias
 where entity_type = 'issue' and issue_id is not null;

insert into issue_mention (organization_id, source_issue_id, source_comment_id, target_issue_id, created_at)
select distinct i.organization_id, i.id, null::uuid, t.issue_id, i.updated_at
  from issue i
  cross join lateral regexp_matches(
    left(coalesce(i.description, ''), 100000),
    '(?<![A-Za-z0-9-])([A-Za-z][A-Za-z0-9]{0,9}-[0-9]{1,9})(?![A-Za-z0-9-])', 'g') m
  join _lin12_ident t on t.organization_id = i.organization_id and t.ident = upper(m[1])
 where t.issue_id <> i.id
on conflict do nothing;

insert into issue_mention (organization_id, source_issue_id, source_comment_id, target_issue_id, created_at)
select distinct c.organization_id, c.issue_id, c.id, t.issue_id, c.created_at
  from comment c
  cross join lateral regexp_matches(
    left(coalesce(c.body, ''), 100000),
    '(?<![A-Za-z0-9-])([A-Za-z][A-Za-z0-9]{0,9}-[0-9]{1,9})(?![A-Za-z0-9-])', 'g') m
  join _lin12_ident t on t.organization_id = c.organization_id and t.ident = upper(m[1])
 where c.issue_id is not null and c.deleted_at is null and t.issue_id <> c.issue_id
on conflict do nothing;
