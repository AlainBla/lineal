-- LIN-25 Stufe 4: Webhooks fuer Meilensteine und Kalender-Feed.
alter table webhook drop constraint webhook_resource_types_check;
alter table webhook add constraint webhook_resource_types_check check (
  cardinality(resource_types) >= 1 and resource_types <@ array[
    'Issue','Comment','Attachment','IssueLabel','Reaction','Project','ProjectMilestone','ProjectUpdate',
    'Cycle','Document','Initiative','InitiativeUpdate','User','IssueSLA']
);

-- Geheimer Link je Person. Kalender-Apps koennen sich nicht anmelden, der Token ersetzt die Session
-- und zeigt nur, was die Person gerade sehen darf. Gespeichert wird nur der Hash.
create table calendar_feed (
  organization_id uuid not null references organization (id) on delete cascade,
  user_id uuid not null,
  token_hash text not null,
  created_at timestamptz not null default now(),
  primary key (organization_id, user_id),
  foreign key (organization_id, user_id) references "user" (organization_id, id) on delete cascade
);
create unique index calendar_feed_token_hash_uq on calendar_feed (token_hash);
