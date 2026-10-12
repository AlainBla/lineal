-- 2500 (WP25): Audit-Log (docs/15). Append-only; Aufbewahrung ueber eine eigene Funktion.
--
-- Mandantentrennung wie bei den uebrigen Tabellen: jede Abfrage filtert auf organization_id
-- (PgTx-Repo). Bewusst ohne Fremdschluessel auf organization/user: Eintraege ueberleben
-- Loeschungen (ein Workspace-Loeschlauf waere sonst durch den Trigger blockiert).
create table audit_log (
  id              uuid        primary key,
  organization_id uuid        not null,
  occurred_at     timestamptz not null default now(),
  actor_user_id   uuid,
  actor_kind      text        not null check (actor_kind in ('user', 'app', 'system')),
  -- session | api_key | oauth | cli; null bei Systemaktionen ohne Zugangsdaten
  credential_kind text        check (credential_kind in ('session', 'api_key', 'oauth', 'cli')),
  action          text        not null check (char_length(action) between 1 and 100),
  target_type     text        not null check (char_length(target_type) between 1 and 50),
  target_id       uuid,
  details         jsonb       not null default '{}'::jsonb check (jsonb_typeof(details) = 'object'),
  ip              inet,
  request_id      text        not null default '' check (char_length(request_id) <= 200)
);

create index audit_log_org_occurred_idx on audit_log (organization_id, occurred_at desc, id desc);
create index audit_log_org_actor_idx on audit_log (organization_id, actor_user_id, occurred_at desc);

-- Append-only: UPDATE und TRUNCATE sind immer verboten; DELETE nur aus der Aufbewahrungsfunktion
-- (Sitzungsschalter `lineal.audit_purge`, nur transaktionslokal gesetzt).
create function audit_log_guard() returns trigger
language plpgsql as $$
begin
  if tg_op = 'DELETE' and current_setting('lineal.audit_purge', true) = 'on' then
    return old;
  end if;
  raise exception 'audit_log ist append-only' using errcode = '23514';
end;
$$;

create trigger audit_log_append_only before update or delete on audit_log
  for each row execute function audit_log_guard();
create trigger audit_log_no_truncate before truncate on audit_log
  for each statement execute function audit_log_guard();

-- Aufbewahrung je Workspace: `securitySettings.auditLogRetentionDays` (Default 365, Minimum 30).
-- Ungueltige Werte fallen auf 365 zurueck. Liefert die Anzahl geloeschter Eintraege.
create function audit_log_purge_expired(p_now timestamptz) returns bigint
language plpgsql as $$
declare
  n bigint;
begin
  perform set_config('lineal.audit_purge', 'on', true);
  with cutoff as (
    select o.id as org_id,
           p_now - make_interval(days => case
             when coalesce(o.security_settings->>'auditLogRetentionDays', '') ~ '^[0-9]{1,6}$'
               then greatest(30, (o.security_settings->>'auditLogRetentionDays')::int)
             else 365
           end) as before
    from organization o
  ),
  deleted as (
    delete from audit_log a
    using cutoff c
    where a.organization_id = c.org_id and a.occurred_at < c.before
    returning 1
  )
  select count(*) into n from deleted;
  perform set_config('lineal.audit_purge', 'off', true);
  return n;
end;
$$;
