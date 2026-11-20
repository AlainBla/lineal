-- LIN-58: Self-Service-Passwort-Reset. Eine Tabelle mit kurzlebigen, einmaligen Reset-Tokens.
--
-- Gespeichert wird NUR der SHA-256-Hash des Tokens (token_hash), nie der Klartext — genau wie bei
-- Sessions, API-Keys und Einladungen. Der Klartext steht einmalig im Mail-Link
-- ({base}/reset-password?token=lin_reset_…) und wird nie persistiert.
--
-- organization_id trägt die Zeile wie user_session/user_totp (Mandantentrennung); beim Löschen des
-- Workspace oder des Nutzers verschwindet alles mit. created_at bleibt auch nach dem Einlösen
-- erhalten (used_at wird gesetzt, die Zeile NICHT gelöscht), damit das neustart-feste Zähl-Limit je
-- Konto (Zeilen je Zeitfenster, analog zum Einladungs-Limit LIN-64) die Historie sieht.
create table password_reset (
    id uuid primary key,
    organization_id uuid not null references organization (id) on delete cascade,
    user_id uuid not null references "user" (id) on delete cascade,
    -- SHA-256 (hex) über das komplette Token inkl. Präfix `lin_reset_`. Global eindeutig.
    token_hash text not null unique,
    -- Anfragende IP (betrieblich/forensisch); nullbar, wenn nicht ermittelbar.
    requested_ip text,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    -- Einmaligkeit: gesetzt beim Einlösen ODER beim Entwerten durch eine neuere Anfrage.
    used_at timestamptz
);

-- Suche nach allen (auch verbrauchten) Tokens eines Nutzers: Entwerten alter Tokens und das
-- Zähl-Limit je Konto.
create index password_reset_user_idx on password_reset (organization_id, user_id);
