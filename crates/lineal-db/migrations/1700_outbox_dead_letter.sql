-- 1700 (WP17): Fehlversuche und Dead-Letter je (Konsument, Event) dauerhaft in outbox_delivery.
-- Eine Zeile mit delivered_at IS NULL ist ein laufender Fehlversuchszähler (Event bleibt offen);
-- delivered_at IS NOT NULL bedeutet zugestellt. Dead-Letter: zugestellt mit last_error IS NOT NULL.
ALTER TABLE outbox_delivery
    ADD COLUMN attempts   integer NOT NULL DEFAULT 0,
    ADD COLUMN last_error text;
ALTER TABLE outbox_delivery ALTER COLUMN delivered_at DROP NOT NULL;
