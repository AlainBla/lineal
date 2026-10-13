-- B8.26: Der Namenskonflikt beim Anlegen/Umbenennen darf die Namen privater Teams nicht
-- verraten (docs/02 §6.6). Die Eindeutigkeit gilt deshalb nur noch unter nicht-privaten Teams;
-- die Anwendung prüft zusätzlich gegen alle Teams, die der Akteur sehen darf (Admins: alle).
drop index if exists team_name_uq;
create unique index team_name_uq on team (organization_id, lower(name))
  where not trashed and visibility <> 'private';
