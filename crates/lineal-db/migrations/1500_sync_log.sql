-- 1500 (WP15): Sync-Log fuer den Echtzeit-Delta-Strom (docs/09 §5, docs/11 §3.7).
--
-- `id` ist der Stream-Cursor (`lastSyncId` des Clients): streng monoton in Commit-Reihenfolge.
-- Er stammt aus `sync_log_seq` und wird nur unter einem Advisory-Lock vergeben, der bis zum Commit
-- gehalten wird; so wird eine hoehere id nie vor einer niedrigeren sichtbar (kein Ueberspringen bei
-- parallelen Konsumenten). `sync_id` ist die lastSyncId der urspruenglichen Schreib-Transaktion
-- (mehrere Zeilen/Events koennen sie teilen; sie ist nur Information).
CREATE SEQUENCE sync_log_seq AS bigint START 1;

CREATE TABLE sync_log (
    id              bigint      PRIMARY KEY,
    organization_id uuid        NOT NULL,
    sync_id         bigint      NOT NULL,
    event_id        uuid        NOT NULL,
    n               smallint    NOT NULL DEFAULT 0,
    -- I insert, U update, A archive, V unarchive, D delete, C covering, G sync-group change
    action          text        NOT NULL CHECK (action IN ('I', 'U', 'A', 'V', 'D', 'C', 'G')),
    model_name      text        NOT NULL,
    model_id        uuid        NOT NULL,
    data            jsonb       NOT NULL DEFAULT '{}'::jsonb,
    -- Berechtigungsschluessel: `team:<uuid>`, `org`, `user:<uuid>` (docs/09 §5.6)
    sync_groups     text[]      NOT NULL,
    origin          text        NOT NULL,
    actor_id        uuid,
    created_at      timestamptz NOT NULL DEFAULT now(),
    -- Idempotenz: die Zustellung an den Konsumenten ist at-least-once.
    UNIQUE (event_id, n)
);
CREATE INDEX sync_log_org_id_idx ON sync_log (organization_id, id);
CREATE INDEX sync_log_created_idx ON sync_log (created_at);
CREATE INDEX sync_log_model_idx ON sync_log (organization_id, model_name, model_id, id DESC);

-- Wasserzeichen der Aufbewahrung: alles bis einschliesslich `purged_up_to` wurde geloescht.
-- Ein Client mit `since < purged_up_to` bekommt `resetRequired`.
CREATE TABLE sync_log_purged (
    organization_id uuid   PRIMARY KEY,
    purged_up_to    bigint NOT NULL
);

-- RFC-3339 mit Millisekunden in UTC (identisch zum GraphQL-Format der Zeitstempel).
CREATE FUNCTION sync_ts(t timestamptz) RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$ SELECT to_char(t AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') $$;
