-- Infrastruktur-Tabellen (S1): Sync-Sequenz, Outbox, Jobs.
-- Das fachliche Schema folgt ab 0010 (S2).

-- Workspaceweit monotone lastSyncId. Eine zentrale Sequenz ist monoton je Workspace
-- (docs/09 §6.3, Alternative bei Sequenz-Engpässen).
CREATE SEQUENCE sync_seq AS bigint START 1;

-- Transactional Outbox. Bewusst ohne FK auf organization (S2 legt das Schema später an).
CREATE TABLE outbox_event (
    event_id        uuid PRIMARY KEY,
    organization_id uuid        NOT NULL,
    origin          text        NOT NULL CHECK (origin IN ('user', 'system', 'import')),
    type            text        NOT NULL,
    payload         jsonb       NOT NULL,
    sync_id         bigint      NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    processed_at    timestamptz
);

CREATE INDEX outbox_event_pending_idx ON outbox_event (sync_id) WHERE processed_at IS NULL;
CREATE INDEX outbox_event_org_sync_idx ON outbox_event (organization_id, sync_id);

-- Zustellung pro Konsument (idempotent genau einmal).
CREATE TABLE outbox_delivery (
    consumer     text        NOT NULL,
    event_id     uuid        NOT NULL REFERENCES outbox_event (event_id) ON DELETE CASCADE,
    delivered_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, event_id)
);

-- Job-Queue (FOR UPDATE SKIP LOCKED); auch Grundlage periodischer Jobs.
CREATE TABLE job (
    id              uuid PRIMARY KEY,
    organization_id uuid,
    kind            text        NOT NULL,
    payload         jsonb       NOT NULL DEFAULT '{}'::jsonb,
    status          text        NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'done', 'failed')),
    run_at          timestamptz NOT NULL DEFAULT now(),
    attempts        integer     NOT NULL DEFAULT 0,
    max_attempts    integer     NOT NULL DEFAULT 5,
    unique_key      text,
    locked_by       text,
    locked_at       timestamptz,
    last_error      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    completed_at    timestamptz
);

CREATE UNIQUE INDEX job_unique_key_idx ON job (unique_key) WHERE unique_key IS NOT NULL;
CREATE INDEX job_due_idx ON job (run_at) WHERE status = 'queued';
