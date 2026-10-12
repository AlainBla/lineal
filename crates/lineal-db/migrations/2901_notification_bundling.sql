-- 2901 (WP32): Bündelung gleichartiger Benachrichtigungen (docs/05 §6.6): Ereignisse desselben
-- Akteurs am selben Objekt innerhalb einer Minute landen in einer Notification. `bundled_types`
-- listet die Typen aller zusammengefassten Ereignisse (der spezifischste steht in `type`).
alter table notification add column bundled_types text[] not null default '{}';

-- Idempotenz für zusammengefasste Ereignisse: der `dedup_key` eines in eine bestehende
-- Notification eingegangenen Ereignisses wird hier vermerkt (eine erneute Zustellung ist dann ein No-op).
create table notification_merge (
  user_id uuid not null,
  dedup_key text not null,
  notification_id uuid not null references notification (id) on delete cascade,
  primary key (user_id, dedup_key)
);
create index notification_merge_notification_idx on notification_merge (notification_id);
