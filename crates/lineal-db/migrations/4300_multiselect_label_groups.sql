-- B5.18: Projekt- und Initiative-Label-Gruppen sind immer multiSelect (docs/07 §3.4);
-- die Eindeutigkeit je Gruppe war ein Überbleibsel von singleSelect.
drop index if exists project_to_label_group_uq;
drop index if exists initiative_to_label_group_uq;
