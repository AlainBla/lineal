-- 0800 (WP08): Idempotenzschluessel und Gruppenschluessel der Inbox (docs/05 §6.6, docs/10 §4.3).
-- dedup_key: Fan-out `event:<event_id>:<type>`, Faelligkeits-Erinnerungen `due:<issue>:<date>:<stage>`.
alter table notification add column dedup_key text;
create unique index notification_dedup_uq on notification (user_id, dedup_key);

-- Gruppenschluessel je Bezugsobjekt; Meilenstein-Typen gruppieren unter ihrem Projekt (project_id ist gesetzt).
alter table notification add column group_key text generated always as (
  case target_type
    when 'issue' then 'issue:' || issue_id::text
    when 'project' then 'project:' || project_id::text
    when 'initiative' then 'initiative:' || initiative_id::text
    when 'document' then 'document:' || document_id::text
    else 'notification:' || id::text
  end) stored;
create index notification_group_idx on notification (organization_id, user_id, group_key) where archived_at is null;
create index notification_email_idx on notification (created_at)
  where read_at is null and emailed_at is null and archived_at is null;
create index notification_archived_idx on notification (archived_at) where archived_at is not null;
