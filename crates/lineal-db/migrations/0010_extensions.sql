-- 0010: Extensions und generische Hilfsfunktionen (S2, Schema aus docs/01-domaenenmodell.md).
-- Konventionen fuer alle Migrationen 0010-0099:
--  * Tabellennamen singular snake_case; die Entitaet "User" heisst "user" und muss in SQL
--    IMMER in Anfuehrungszeichen stehen ("user"), weil user in PostgreSQL reserviert ist.
--  * Spalten snake_case (Team-Zaehler: last_issue_number, last_project_number, last_cycle_number).
--  * Enums als text + CHECK mit exakten Werten aus docs/00-konventionen.md §4.
--  * Jede Tabelle hat organization_id; FKs auf mandantengebundene Tabellen sind
--    zusammengesetzt (organization_id, x_id) -> ziel(organization_id, id), jede referenzierbare
--    Tabelle traegt dafuer UNIQUE (organization_id, id).
--  * Audit-Spalten: id uuid, version int default 1, created_at, updated_at, archived_at.
--    Reine Join-Tabellen: composite PK, organization_id, created_at.
--  * Annahme: updated_at/version werden vom Core gepflegt (kein Trigger), da updated_at nur
--    bei inhaltlichen Aenderungen steigen soll (docs/01 §0.2).
--  * Annahme: gen_random_uuid() ist ab PostgreSQL 13 eingebaut; pgcrypto wird nicht benoetigt.
--  * Annahme: PostgreSQL >= 15 (UNIQUE ... NULLS NOT DISTINCT, ON DELETE SET NULL (spalte));
--    docker-compose.yml pinnt postgres:16.

create extension if not exists pg_trgm;
create extension if not exists btree_gist;

-- Verbietet Aenderungen an der Spalte "type" (WorkflowState, ProjectStatus: Kategorie unveraenderlich).
create function lineal_forbid_type_change() returns trigger
language plpgsql as $$
begin
  if new.type is distinct from old.type then
    raise exception '%.type ist unveraenderlich', tg_table_name using errcode = '23514';
  end if;
  return new;
end;
$$;

-- Append-only-Tabellen (IssueHistory, DocumentContentHistory): UPDATE verboten.
create function lineal_forbid_update() returns trigger
language plpgsql as $$
begin
  raise exception '% ist append-only', tg_table_name using errcode = '23514';
end;
$$;
