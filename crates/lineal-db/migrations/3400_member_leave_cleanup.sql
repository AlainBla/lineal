-- B1.15 (docs/02 §4.2, Effekte des Austritts): Bestandsbereinigung. Der Austritt aus einem
-- nicht öffentlichen Team (restricted, private) ließ Assignee, Issue-Abos und Projekt-Lead stehen.
-- Neu ausgeführt wird das beim Austritt im Use-Case; hier einmalig für vorhandene Daten:
--  1. offene Issues eines Nicht-Mitglieds in nicht öffentlichen Teams: assignee = null,
--  2. Issue-Abos von Nicht-Mitgliedern auf Issues nicht öffentlicher Teams entfernen,
--  3. Projekt-Lead ohne Zugriff (kein Projektteam öffentlich oder mit Mitgliedschaft) leeren.
-- Für die Bestandsfälle werden keine History-Einträge geschrieben (Akteur und Zeitpunkt des
-- Austritts sind nicht rekonstruierbar).

update issue i
   set assignee_id = null, version = i.version + 1, updated_at = now()
 where i.assignee_id is not null
   and not i.trashed
   and i.archived_at is null
   and i.state_type in ('triage', 'backlog', 'unstarted', 'started')
   and exists (select 1 from team t where t.id = i.team_id and t.visibility <> 'public')
   and not exists (select 1 from team_membership m
                    where m.team_id = i.team_id and m.user_id = i.assignee_id);

delete from issue_subscription s
 using issue i, team t
 where s.issue_id = i.id
   and t.id = i.team_id
   and t.visibility <> 'public'
   and not exists (select 1 from team_membership m
                    where m.team_id = i.team_id and m.user_id = s.user_id);

update project p
   set lead_id = null, version = p.version + 1, updated_at = now()
 where p.lead_id is not null
   and not exists (
     select 1
       from project_team pt
       join team t on t.id = pt.team_id
      where pt.project_id = p.id
        and (t.visibility = 'public'
             or exists (select 1 from team_membership m
                         where m.team_id = t.id and m.user_id = p.lead_id)));
