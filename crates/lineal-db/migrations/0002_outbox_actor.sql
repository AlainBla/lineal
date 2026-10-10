-- Auslösender User eines Events (Self-Suppression bei Notifications, docs/10 §4).
ALTER TABLE outbox_event ADD COLUMN actor_id uuid;
