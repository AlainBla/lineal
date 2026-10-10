-- 1100 (WP11): Volltext-Spalten für Projekte, Initiativen, Dokumente und Kommentare sowie ein Index
-- für die firmenweite Übersicht. Wie bei issue.search_vector (0015) generierte Spalten mit der
-- Konfiguration 'simple' und auf 100 000 Zeichen begrenzt (tsvector <= 1 MB); dadurch braucht es
-- keinen Index-Konsumenten, und die Berechtigung wird zur Abfragezeit angehängt (docs/10 §5.3).

alter table project add column search_vector tsvector generated always as (
  setweight(to_tsvector('simple', coalesce(name, '')), 'A') ||
  setweight(to_tsvector('simple', coalesce(description, '')), 'B') ||
  setweight(to_tsvector('simple', left(coalesce(content, ''), 100000)), 'C')
) stored;
create index project_search_idx on project using gin (search_vector);

alter table initiative add column search_vector tsvector generated always as (
  setweight(to_tsvector('simple', coalesce(name, '')), 'A') ||
  setweight(to_tsvector('simple', coalesce(description, '')), 'B') ||
  setweight(to_tsvector('simple', left(coalesce(content, ''), 100000)), 'C')
) stored;
create index initiative_search_idx on initiative using gin (search_vector);

alter table document add column search_vector tsvector generated always as (
  setweight(to_tsvector('simple', coalesce(title, '')), 'A') ||
  setweight(to_tsvector('simple', left(coalesce(content, ''), 100000)), 'B')
) stored;
create index document_search_idx on document using gin (search_vector);

alter table comment add column search_vector tsvector generated always as (
  setweight(to_tsvector('simple', left(coalesce(body, ''), 100000)), 'C')
) stored;
create index comment_search_idx on comment using gin (search_vector);

-- docs/14 §3: GROUP BY team_id, state_type über die Issue-Tabelle.
create index issue_team_state_type_idx on issue (organization_id, team_id, state_type)
  where not trashed and archived_at is null;
