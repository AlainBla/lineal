-- WP34: Planungs-Automationen (docs/04 §1.3 cycleNumberBase).
-- Zaehler, ab dem die Nummerierung nach einer Umbenennung ("Sprint 40") fortgesetzt wurde.
alter table team
  add column cycle_number_base int check (cycle_number_base is null or cycle_number_base >= 0);
