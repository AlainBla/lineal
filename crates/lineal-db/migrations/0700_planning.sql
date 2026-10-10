-- 0700 (WP07 Planung): Ergaenzungen fuer Cycles, Projekte, Initiativen und Status-Updates.
-- Das Schema aus 0014/0016 wird nur ergaenzt (docs/04).

-- Cycle: Phase-2-Marker des Wechsel-Jobs (docs/04 §1.5 `startedProcessedAt`) und eingefrorener Fortschritt
-- (`historyFrozen` ist `completed_at IS NOT NULL`).
alter table cycle
  add column started_processed_at timestamptz,
  add column progress double precision;

-- Team: Sub-Team kann die Vererbung des Cycle-Zeitplans abwaehlen (docs/04 §1.7 `inheritsCycles`).
alter table team
  add column inherits_cycles boolean not null default true,
  -- Fingerprint der Zeitplan-Einstellungen, zu denen die kuenftigen Cycles zuletzt berechnet wurden
  -- (Dauer, Cooldown, Starttag, Zeitzone); der Job berechnet bei Abweichung neu (docs/04 §1.4).
  add column cycle_schedule_fp text;

-- Team-Cooldown laut docs/04 §1.2 / A2: 0 bis 8 Wochen (0014 erlaubte nur 0..4).
alter table team drop constraint if exists team_cycle_cooldown_time_check;
alter table team add constraint team_cycle_cooldown_time_check check (cycle_cooldown_time between 0 and 8);

-- Erinnerungsmodus je Projekt/Initiative (docs/04 §5.4, A20).
alter table project
  add column update_reminder_mode text not null default 'followWorkspace'
    check (update_reminder_mode in ('followWorkspace', 'custom', 'never'));
alter table initiative
  add column update_reminder_mode text not null default 'followWorkspace'
    check (update_reminder_mode in ('followWorkspace', 'custom', 'never'));

-- Status-Updates: strukturierter Diff zum vorherigen Update (docs/04 §5.3).
alter table project_update
  add column diff jsonb,
  add column diff_markdown text,
  add column is_diff_hidden boolean not null default false;
alter table initiative_update
  add column diff jsonb,
  add column diff_markdown text,
  add column is_diff_hidden boolean not null default false;

-- Erinnerungs-Protokoll (docs/04 §5.4 `markSent`): je Ziel, faelligem Slot und Stufe genau einmal.
create table update_reminder_sent (
  organization_id uuid not null references organization (id) on delete cascade,
  target_type text not null check (target_type in ('project', 'initiative')),
  target_id uuid not null,
  due_at timestamptz not null,
  stage int not null check (stage between 0 and 2),
  sent_at timestamptz not null default now(),
  primary key (target_type, target_id, due_at, stage)
);

-- Aenderung eines Team-Keys: alte Projektkennungen (`P-OLD-n`) bleiben als Alias aufloesbar
-- (docs/04 §2.8). Die Kennung selbst wird aus Key und Nummer des Lead-Teams berechnet.
create function lineal_project_alias_on_team_key_change() returns trigger
language plpgsql as $$
begin
  insert into issue_identifier_alias (organization_id, identifier, entity_type, project_id, team_key)
  select p.organization_id, 'P-' || old.key || '-' || p.number, 'project', p.id, old.key
  from project p
  where p.lead_team_id = new.id
  on conflict (organization_id, identifier) do nothing;
  return new;
end;
$$;
create trigger team_key_changed_project_alias after update of key on team
  for each row when (old.key is distinct from new.key)
  execute function lineal_project_alias_on_team_key_change();

create index project_initiative_lookup_idx on initiative_to_project (organization_id, project_id, initiative_id);
create index initiative_parent_parent_idx on initiative_parent (organization_id, parent_id, child_id);
