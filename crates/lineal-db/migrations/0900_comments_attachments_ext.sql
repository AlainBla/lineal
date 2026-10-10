-- 0900 (WP09): Platzhalter geloeschter Kommentare und Upload-Reservierungen.

-- Top-Level-Kommentar mit Antworten: Body wird durch Platzhalter ersetzt, Zeitpunkt hier vermerkt (docs/03 §6.3).
alter table comment add column deleted_at timestamptz;

-- Reservierung eines Uploads (prepare): bindet Objektschluessel an Workspace, Issue und Ersteller.
-- finalize prueft Existenz, Ablauf, Zuordnung und genau einmalige Verwendung.
create table upload_reservation (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization (id) on delete cascade,
  issue_id uuid,
  creator_id uuid,
  storage_key text not null,
  filename text not null check (char_length(filename) between 1 and 255),
  content_type text not null,
  size bigint not null check (size > 0),
  title text,
  subtitle text,
  expires_at timestamptz not null,
  finalized_at timestamptz,
  created_at timestamptz not null default now(),
  unique (organization_id, id),
  unique (organization_id, storage_key),
  foreign key (organization_id, issue_id) references issue (organization_id, id) on delete cascade,
  foreign key (organization_id, creator_id) references "user" (organization_id, id)
);
create index upload_reservation_open_idx on upload_reservation (organization_id, expires_at) where finalized_at is null;
create index attachment_storage_key_idx on attachment (organization_id, storage_key) where storage_key is not null;
