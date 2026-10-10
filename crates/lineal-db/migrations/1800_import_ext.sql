-- 1800 (WP18): Ergaenzungen fuer den Linear-Import (docs/13). Nur additiv bzw. Check-Erweiterung.

-- Lease: genau ein Runner bearbeitet einen Lauf (Job und CLI), ohne dass der Checkpoint ueberschrieben wird.
alter table import_run add column lease_until timestamptz;

-- Ein abgebrochener Lauf ist kein Fehler: eigener Status `canceled` (docs/13 §6 importCancel).
alter table import_run drop constraint import_run_status_check;
alter table import_run add constraint import_run_status_check
  check (status in ('scanning','planned','running','paused','failed','completed','canceled'));

-- Runner-Suche (mandantenuebergreifend) ueber ausfuehrbare Laeufe.
create index import_run_runnable_idx on import_run (created_at) where status in ('scanning', 'running');
