-- 1200 (WP12): Triage-Verantwortung, Automationen, SLA. Nur Ergänzungen.
-- Die Strukturen `Issue`/`Team` im Port gehören anderen Paketen; die neuen Spalten werden
-- ausschließlich über `TriageRepo` gelesen und geschrieben.

-- Parent/Child-Automation (docs/03 §3.4): `autoClosedByParentClosing`.
alter table issue add column auto_closed_by_parent_closing boolean not null default false;

-- SLA (docs/03 §11): internes Regelkennzeichen und Meldemarken (docs/10 §3: "bereits gemeldet").
-- Manuelles SLA = SLA gesetzt, aber `sla_rule_id is null`.
alter table issue add column sla_rule_id uuid;
alter table issue add column sla_set_notified_at timestamptz;
alter table issue add column sla_high_risk_notified_at timestamptz;
alter table issue add column sla_breached_notified_at timestamptz;

-- SLA je Team: aktivierbar, Arbeitswoche (docs/03 §11.2).
alter table team add column sla_enabled boolean not null default false;
alter table team add column sla_work_week text not null default 'monToFri'
  check (sla_work_week in ('monToFri', 'sunToThu'));

-- SLA-Regeln: Name und Startmodus (docs/03 §11.2).
alter table sla_rule add column name text not null default '';
alter table sla_rule add column start_mode text not null default 'issueCreation'
  check (start_mode in ('issueCreation', 'ruleMatch'));

alter table issue add constraint issue_sla_rule_fk
  foreign key (organization_id, sla_rule_id) references sla_rule (organization_id, id)
  on delete set null (sla_rule_id);

-- Triage-Verantwortung: gespeicherter aktueller Verantwortlicher (docs/10 §3 "Triage-Rotation-Schichtwechsel").
alter table triage_responsibility add column current_user_id uuid;
alter table triage_responsibility add constraint triage_responsibility_current_user_fk
  foreign key (organization_id, current_user_id) references "user" (organization_id, id)
  on delete set null (current_user_id);

-- Auto-Close-/Archive-/SLA-Läufe.
create index issue_open_updated_idx on issue (team_id, updated_at)
  where state_type in ('triage', 'backlog', 'unstarted') and archived_at is null and not trashed;
create index issue_sla_pending_idx on issue (sla_breaches_at)
  where sla_breaches_at is not null and completed_at is null and canceled_at is null
    and archived_at is null and not trashed;
create index issue_snoozed_triage_idx on issue (snoozed_until_at)
  where snoozed_until_at is not null and state_type = 'triage';
