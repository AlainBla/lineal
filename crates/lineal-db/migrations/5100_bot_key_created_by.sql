-- LIN-62 L-6: Herkunft eines API-Keys festhalten, damit beim Sperren eines Admins die von IHM
-- erzeugten Bot-Keys mitwiderrufen werden. Bisher hing ein Bot-Key nur am Bot-User (`user_id`),
-- nicht am Ersteller, sodass ein gesperrter Admin seinen Bot-Zugang behielt.
--
-- Nullable und ohne Backfill: der Ersteller bestehender Keys ist nur im Audit-Log belegt, nicht
-- rekonstruierbar. `on delete set null`: wird ein Ersteller gelöscht, gilt sein Key wieder als
-- "ohne bekannten Ersteller", statt den Löschvorgang per Fremdschlüssel zu blockieren.
alter table api_key
    add column created_by uuid null references "user" (id) on delete set null;

-- Der Widerruf beim Sperren filtert auf (organization_id, created_by); der vorhandene Index liegt
-- auf (organization_id, user_id).
create index api_key_created_by_idx on api_key (organization_id, created_by);
