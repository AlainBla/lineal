-- 2900 (WP32): Dokument-Abos wie Issue-Abos (docs/05 §6.3): Kommentierende werden Abonnenten.
alter table document_subscription drop constraint document_subscription_reason_check;
alter table document_subscription add constraint document_subscription_reason_check
  check (reason in ('creator','mention','comment','manual','addedByOther'));
