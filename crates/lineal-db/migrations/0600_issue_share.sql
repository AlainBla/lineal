-- 0600 (WP06): Issue-Sharing (docs/02 §6.4, docs/07 §3.17). Ein User erhält Zugriff auf ein einzelnes Issue,
-- ohne Zugriff auf dessen Team zu haben. Das Modell ist in docs/01 nur ein Erweiterungspunkt
-- (`inheritsSharedAccess`); die Tabelle ist die einfachste Umsetzung.

create table issue_share (
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid not null,
  user_id uuid not null,
  shared_by_id uuid,
  created_at timestamptz not null default now(),
  primary key (issue_id, user_id),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, user_id) references "user" (organization_id, id),
  foreign key (organization_id, shared_by_id) references "user" (organization_id, id)
);
create index issue_share_user_idx on issue_share (organization_id, user_id);

-- Zusammenfassen von History-Einträgen (docs/03 §8.2) liest den jüngsten Eintrag je Issue.
create index issue_history_issue_desc_idx on issue_history (organization_id, issue_id, created_at desc);
