# TOTP (2FA) für Admins — Enrollment, Recovery-Codes, Enforcement — Design

**Status:** Freigegeben (15.11.2026). Produktentscheidungen beschlossen (siehe unten). Umsetzung läuft.
**Ticket:** LIN-70 (abgespalten aus LIN-62 L-8; Mindestlänge-12 ist dort bereits umgesetzt, `2b1b679`).
**Audit-Bezug:** sec-auth.md L-8 („TOTP für Admins").
**Mockup:** `docs/superpowers/specs/assets/2026-11-15-lin-70-totp-mockup.html` (6 Screens, hell/dunkel, Lineal-Look).

## Intent / Warum

Admins (Org-Owner) halten die höchsten Rechte im Workspace: Nutzer sperren, Bot-Keys erzeugen, Projekte/Daten verwalten. Ein geleaktes Admin-Passwort kompromittiert den ganzen Mandanten. Passwort-Mindestlänge 12 (LIN-62) senkt das Risiko, beseitigt es nicht (Phishing, Reuse, Infostealer). Ein zweiter Faktor (TOTP) macht ein reines Passwort-Leak wertlos.

Trust-Modell: Angreifer kennt das Admin-Passwort (Phishing/Reuse), hat aber das gekoppelte Gerät nicht. Ziel: kein Login ohne den zweiten Faktor — **an jedem Pfad, der zu einer Browser-Session führt.**

## Goals

- Org-Owner können TOTP selbst einrichten: Secret erzeugen, per QR/`otpauth`-URI koppeln, erste Code-Bestätigung **vor** Aktivierung.
- Recovery-Codes bei Enrollment: einmalige Ersatzcodes, nur einmal angezeigt, gehasht gespeichert, Verbrauch markiert, neu erzeugbar.
- Zweiter Faktor wird an **beiden** Stellen geprüft, die eine Browser-Session begründen: `POST /auth/login` und der OAuth-Authorize-Consent-Flow (MCP/DCR laufen darüber).
- Pflicht (Enforcement) für Admins, mit definiertem Übergang für bestehende Konten/Sessions.
- Bot-User (Rolle `app`, LIN-22) bleiben funktionsfähig — sie handeln nur per API-Key, nie per Browser-Login, also kein TOTP und keine Aussperrung durch Enforcement.
- UI: Account→Sicherheit (Status/Verwaltung), Enrollment, Recovery-Code-Anzeige/-Regeneration, Login-Zweitschritt.

## Non-Goals / Anti-Goals

- **Kein** WebAuthn/Passkeys in dieser Stufe (eigenes, größeres Feature; TOTP zuerst, weil app-kompatibel und selbstverwaltet).
- **Kein** SMS-/E-Mail-OTP (schwächer, Zustellabhängigkeit).
- **Keine** 2FA-Pflicht für normale Mitglieder in dieser Stufe (Scope = Admins; später erweiterbar).
- **Kein** zweiter 2FA-Prüfpunkt im API-Key-Pfad — Bot-Keys sind bewusst ausgenommen (siehe Architektur).
- **Kein** Secret im Klartext at rest (gleiche Philosophie wie die Backup-age-Identity).

## Kernidee: 2FA sitzt an der Browser-Login-Stufe

Der heikelste Punkt aus dem Ticket ist die **2FA-Umgehung über den API-Pfad**. Lösung: Der zweite Faktor wird **nicht** pro Endpunkt verstreut, sondern **an der einen Stelle** eingehängt, die eine Browser-Session begründet — die Passwort-Verifikation. Weil der OAuth-Authorize-Flow (MCP/DCR) **denselben** Browser-Login durchläuft, bevor er den Authorize-Redirect ausstellt, ist dieser Pfad automatisch abgedeckt, ohne zweiten 2FA-Ort.

Die beiden Pfade, die **keine** interaktive Session begründen, bleiben außen vor — korrekterweise:
- **Bot-API-Keys** (`lin_api_…`, Rolle `app`): kein Browser, kein Passwort, kein TOTP. Enforcement darf sie nie sperren.
- **Bestehende OAuth-Access-Tokens**: schon ausgestellt; die 2FA-Prüfung sitzt bei der *Ausstellung* (Authorize), nicht bei jedem Token-Gebrauch.

## Architektur (entschieden)

### TOTP-Parameter
RFC 6238: HMAC-SHA-1, 6 Stellen, 30-s-Fenster, ±1 Schritt Toleranz. Kompatibel mit jeder Standard-App (Google Authenticator, 1Password, Aegis, …). `otpauth://totp/Lineal:<email>?secret=…&issuer=Lineal`.

### Secret-Speicherung
Pro Nutzer ein TOTP-Secret, **verschlüsselt at rest**. Aktivierung erst **nach** erfolgreicher erster Code-Bestätigung (kein halb eingerichteter Zustand). Neue DB-Spalten (Entwurf): `user_totp(user_id, secret_enc, confirmed_at, created_at)`.
→ *Schlüsselverwaltung ist eine offene Teilfrage (siehe Offen 4).*

### Recovery-Codes
10 Einmal-Codes, server-seitig nur **gehasht** (wie Passwörter, Argon2 via `spawn_blocking` analog LIN-62 L-1). Anzeige nur einmal. Verbrauch markiert. „Neu erzeugen" invalidiert alle alten. Entwurf: `user_recovery_code(user_id, code_hash, used_at)`.

### Login-Zweitschritt
Nach erfolgreichem Passwort und *wenn* der Nutzer 2FA aktiv hat: Zwischenzustand (noch **kein** `__Host-`-Session-Cookie), 6-stellige Eingabe oder Recovery-Code. Cookie wird **erst nach** bestandenem zweitem Schritt gesetzt (greift sauber in LIN-62 L-7). Fehlversuche zählen gegen denselben gestaffelten Lockout wie der Login (LIN-48), als eigener Zähler-Bucket.

### Enforcement
Admin-Konten ohne aktives 2FA sehen ein Pflicht-Banner. Übergang/Härte = **offene Entscheidung** (unten). „Deaktivieren" ist für erzwungene Rollen gesperrt, sonst ließe sich die Pflicht selbst abschalten.

## Beschlossene Entscheidungen (15.11.2026)

1. **Pflicht-Rolle:** **Nur Org-Owner.** Nur Owner sehen das Pflicht-Banner und bekommen „Deaktivieren" gesperrt. Mitglieder können TOTP freiwillig aktivieren (optionaler Login-Zweitschritt), haben aber keine Pflicht.
2. **Enforcement-Härte:** **Schonfrist 7 Tage / 3 Logins.** Owner ohne TOTP sehen ein Banner mit Countdown (min. aus Rest-Tagen und Rest-Logins); nach Ablauf harte Sperre — Login führt zwangsweise in die Einrichtung, kein anderer Zugang, bis TOTP aktiv ist.
3. **Totalverlust:** **Zweiter Owner (UI) + Host-CLI.** Ein anderer Owner kann im UI das TOTP eines Owners zurücksetzen (zählt als privilegierte Aktion, auditiert). Zusätzlich ein Server-CLI-Befehl (`lineal-server totp reset --user <email>`) als letzter Ausweg für den Ein-Owner-Fall.
4. **Secret-Verschlüsselung:** **Pluggable KeyProvider.** Default = **app-weiter Schlüssel** aus `.env`/Secret-File (konsistent mit heutiger Secret-Verwaltung, im Backup enthalten). **Zusätzlich OpenBAO** (Vault-kompatibel) als optionaler Backend — beim Eigentümer bereits eingerichtet. Konfiguration wählt den Provider; Secrets werden envelope-verschlüsselt, sodass ein Wechsel/Rotation des KEK ohne Neu-Enrollment möglich ist.

### KeyProvider-Design (zu 4)

Trait `TotpKeyProvider` mit zwei Implementierungen:
- `AppKeyProvider` — KEK aus `LINEAL_TOTP_KEK` (Secret-File/`.env`, 32 Byte base64). Default.
- `OpenBaoProvider` — KEK/Transit über OpenBAO (`LINEAL_TOTP_KEY_BACKEND=openbao`, Adresse/Token/Mount aus Config). Nutzt bevorzugt OpenBAO **Transit** (encrypt/decrypt bleibt im Dienst, KEK verlässt OpenBAO nie); Fallback: KEK als Secret lesen.

Envelope: pro Secret ein zufälliger DEK (XChaCha20-Poly1305), DEK mit dem KEK gewrappt. Gespeichert: `secret_enc`, `dek_wrapped`, `key_backend`, `key_version`. Rotation tauscht nur die DEK-Wraps, nicht die Secrets.

## Interaktionen mit bestehendem Code

- **LIN-62 L-7** `__Host-`-Cookie: wird erst nach 2FA gesetzt — Zwischenzustand braucht einen eigenen, kurzlebigen „pending-2FA"-Marker (nicht das volle Session-Cookie).
- **LIN-62 L-11** Cookie-Fehlerzähler / **LIN-48** gestaffelter Lockout: der 2FA-Zweitschritt braucht einen eigenen Fehlversuchs-Bucket.
- **LIN-62 L-6** Bot-Key-`created_by`/Revoke: Enforcement-Logik muss `app`-User sauber ausnehmen (Überschneidung dokumentieren).
- **LIN-22** Bots/App-User: Nachweis, dass API-Key-Pfad nie durch die 2FA-Pflicht blockiert wird.

## Vorgehen

Dieses Doc sammelt den Design-Stand. **Nicht mit Code beginnen**, bis die 4 offenen Punkte beantwortet sind. Danach: Plan (superpowers:writing-plans) mit Tasks je Fläche (Schema → Enrollment → Recovery → Login-Zweitschritt → OAuth-Pfad → Enforcement), red-first getestet wie der Rest der Sicherheits-Arbeit.
