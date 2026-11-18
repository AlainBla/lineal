-- LIN-76: Herkunft eines Kommentars festhalten, damit die Kommentar-Karte „via MCP“/„via API“
-- anzeigen kann (docs/01 §3.6, docs/03 §8.2) – analog zu `issue_history.source` (0015).
--
-- Not null mit Default `user`: bestehende Kommentare werden als vom Autor von Hand erstellt
-- gewertet; ihre tatsächliche Herkunft wurde nie gespeichert und ist nicht rekonstruierbar.
alter table comment
    add column source text not null default 'user'
        check (source in ('user','automation','api','mcp','import','system'));
