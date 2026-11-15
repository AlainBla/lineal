-- LIN-70: TOTP-2FA für Org-Owner. Drei Tabellen:
--   user_totp          — ein (verschlüsseltes) TOTP-Secret pro Nutzer.
--   user_recovery_code — Einmal-Ersatzcodes, nur als keyed HMAC gespeichert.
--   user_totp_grace    — Schonfrist-Zähler für Owner, die noch kein TOTP haben.
-- Alle tragen organization_id (Mandantentrennung, wie user_session) und hängen
-- per Fremdschlüssel am Nutzer; beim Löschen des Nutzers verschwindet alles mit.

-- Secret wird envelope-verschlüsselt abgelegt: secret_enc = mit zufälligem DEK
-- verschlüsseltes Secret, dek_wrapped = DEK mit dem KEK gewrappt. key_backend
-- ('app'|'openbao') und key_version erlauben Rotation/Backend-Wechsel ohne
-- Neu-Enrollment. confirmed_at IS NULL = angefangen, aber noch nicht aktiv.
-- last_used_step: zuletzt akzeptierter TOTP-Zeitschritt, gegen Code-Replay
-- (RFC 6238 §5.2) — ein Code gilt nur, wenn sein Schritt größer ist.
create table user_totp (
    user_id uuid primary key references "user" (id) on delete cascade,
    organization_id uuid not null references organization (id) on delete cascade,
    secret_enc bytea not null,
    dek_wrapped bytea not null,
    key_backend text not null default 'app' check (key_backend in ('app', 'openbao')),
    key_version int not null default 1,
    confirmed_at timestamptz,
    last_used_step bigint,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
create index user_totp_org_idx on user_totp (organization_id);

-- Recovery-Codes: HMAC-SHA256, nie im Klartext. used_at markiert den Verbrauch
-- (Einmaligkeit); Neu-Erzeugung löscht die alten Zeilen und legt neue an.
create table user_recovery_code (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references "user" (id) on delete cascade,
    organization_id uuid not null references organization (id) on delete cascade,
    code_hash bytea not null,
    used_at timestamptz,
    created_at timestamptz not null default now()
);
create index user_recovery_code_user_idx on user_recovery_code (user_id);

-- Schonfrist: beim ersten Owner-Login ohne bestätigtes TOTP lazy angelegt.
-- Deadline = min(started_at + 7 Tage, 3 Logins). started_at + logins_since
-- treiben die harte Sperre; beim Enrollment wird die Zeile gelöscht.
create table user_totp_grace (
    user_id uuid primary key references "user" (id) on delete cascade,
    organization_id uuid not null references organization (id) on delete cascade,
    started_at timestamptz not null default now(),
    logins_since int not null default 0,
    created_at timestamptz not null default now()
);
