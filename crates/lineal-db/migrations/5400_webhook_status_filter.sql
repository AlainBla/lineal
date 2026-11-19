-- LIN-77: Opt-in-Filter je Webhook – Issue-`update`-Events nur zustellen, wenn sich der Status
-- (`stateId`) geaendert hat. Standard false: bestehende Webhooks liefern unveraendert.
alter table webhook add column issue_status_changes_only boolean not null default false;
