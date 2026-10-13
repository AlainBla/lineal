-- 4000 (Paket 7, B7.05): Aufbewahrung von job/outbox_event. Die periodische Löschung
-- (lineal-jobs, Job `purge`) sucht erledigte Jobs bzw. verarbeitete Events nach Zeitstempel;
-- ohne diese Teilindizes wäre jeder Lauf ein Sequential Scan über die wachsenden Tabellen.
create index if not exists job_done_completed_idx on job (completed_at) where status = 'done';
create index if not exists job_failed_run_idx on job (run_at) where status = 'failed';
create index if not exists outbox_event_processed_idx on outbox_event (processed_at)
  where processed_at is not null;
