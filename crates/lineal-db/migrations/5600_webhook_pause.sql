-- LIN-89: Antwortet ein Webhook-Ziel mit HTTP 429, ist das Backpressure, kein Zustellfehler.
-- Die ganze Subscription wird bis zum `Retry-After`-Zeitpunkt (≤6h) pausiert: andere gequeuete
-- Zustellungen desselben Webhooks gehen in der Pause nicht raus. Standard NULL = nicht pausiert.
alter table webhook add column paused_until timestamptz;
-- Fällige, nicht pausierte Zustellungen zieht der Scheduler; der Teilindex beschleunigt die Abfrage.
create index webhook_paused_idx on webhook (organization_id, id) where paused_until is not null;
