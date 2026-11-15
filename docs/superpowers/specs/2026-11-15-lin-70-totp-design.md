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
10 Einmal-Codes, server-seitig nur **gehasht**. **Kein Argon2** (im Login-Pfad würden bis zu 10× Argon2 pro Versuch ≈ DoS): Codes sind CSPRNG-Zufall mit ≥40 Bit Entropie, daher genügt **keyed HMAC-SHA256 + konstantzeitiger Vergleich**. Anzeige nur einmal. Verbrauch markiert. „Neu erzeugen" invalidiert alle alten. Entwurf: `user_recovery_code(id, user_id, code_hash, used_at, created_at)`.

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
- `AppKeyProvider` (**Default**) — KEK wird per **HKDF-SHA256 aus `session_secret`** abgeleitet (Domain-Info `"totp-kek-v1"`), genau wie `import.rs` heute at-rest-Token ableitet. **Kein neues Pflicht-Secret**, funktioniert in Dev, ist im Backup enthalten. Optionaler Override `LINEAL_TOTP_KEK` (32 Byte base64), falls ein von der Session getrennter Schlüssel gewünscht ist.
- `OpenBaoProvider` — KEK/Transit über OpenBAO (`LINEAL_TOTP_KEY_BACKEND=openbao`, Adresse/Token/Mount aus Config). Nutzt OpenBAO **Transit** (encrypt/decrypt bleibt im Dienst, KEK verlässt OpenBAO nie). **Achtung:** mit diesem Backend wird OpenBAO zur **Login-Abhängigkeit** für TOTP-aktive Owner. In CI nicht verifizierbar → manueller Test gegen die Instanz des Eigentümers, nicht als „verifiziert" melden.

Envelope: pro Secret ein zufälliger DEK (XChaCha20-Poly1305), DEK mit dem KEK gewrappt. Gespeichert: `secret_enc`, `dek_wrapped`, `key_backend`, `key_version`. Rotation tauscht nur die DEK-Wraps, nicht die Secrets.

**KEK-Drift:** Eine Rotation von `session_secret` (Default-Backend) macht bestehende TOTP-Secrets ohne Re-Wrap unlesbar — dasselbe Verhalten wie bei den heutigen `import.rs`-Token. Muss dokumentiert sein; Re-Wrap-Pfad bei Bedarf über die Key-Version.

## Interaktionen mit bestehendem Code

- **LIN-62 L-7** `__Host-`-Cookie: wird erst nach 2FA gesetzt — Zwischenzustand braucht einen eigenen, kurzlebigen „pending-2FA"-Marker (nicht das volle Session-Cookie).
- **LIN-62 L-11** Cookie-Fehlerzähler / **LIN-48** gestaffelter Lockout: der 2FA-Zweitschritt braucht einen eigenen Fehlversuchs-Bucket.
- **LIN-62 L-6** Bot-Key-`created_by`/Revoke: Enforcement-Logik muss `app`-User sauber ausnehmen (Überschneidung dokumentieren).
- **LIN-22** Bots/App-User: Nachweis, dass API-Key-Pfad nie durch die 2FA-Pflicht blockiert wird.

## Sicherheits-Verfeinerungen (advisor-Review, 15.11.)

**A · Pending-Token mit `scope`.** `App::login` gibt ein Enum zurück: `Session` (kein TOTP) · `TotpRequired` · `EnrollRequired`. Der Pending-Token (stateless, HMAC über `user_id|org|scope|exp`, 5 min) trägt einen **Scope**:
- `login` — Owner/Nutzer mit aktivem TOTP → `/auth/login/totp` prüft den Code, mintet dann die Session.
- `enroll` — Owner **ohne** TOTP, Schonfrist abgelaufen → die Enroll-Endpunkte akzeptieren diesen Token **statt** einer Session; `enroll-confirm` mintet die volle Session. So hat der gesperrte Owner überhaupt eine Credential, um sich einzurichten (sonst Deadlock). Enroll-Endpunkte haben damit **zwei Auth-Modi**: Session (freiwillig aus den Einstellungen) oder `enroll`-Scope-Token (erzwungen).

**B · Harte Sperre braucht Session-Widerruf.** Sessions leben 14 Tage, Schonfrist 7 — ohne Eingriff liefe der Zugang 14 Tage weiter. **Default:** der periodische Job **widerruft die Sessions** von Ownern, deren Deadline ohne bestätigtes TOTP abgelaufen ist. Zusätzlich **Default:** `enroll-confirm` widerruft die **übrigen** Sessions des Nutzers (GitHub-Stil, `revoke_other_sessions` existiert).

**C · Code-Replay verhindern (RFC 6238 §5.2).** Ein erfolgreich benutzter Code darf im selben Fenster nicht erneut gelten. Spalte `last_used_step` auf `user_totp`; akzeptiere nur `step > last_used_step`. Per Test gepinnt.

**D · Per-Konto-Lockout am Zweitschritt.** Stateless Token = kein Pro-Versuch-Zustand → ein Angreifer mit dem Passwort könnte über rotierende IPs gegen `/auth/login/totp` bruteforcen. TOTP- **und** Recovery-Code-Fehler laufen über einen `auth_failures`-Konto-Bucket (Schlüssel = `user_id`, z. B. 10 Fehler → 15 min), analog zur Brute-Force-Reservierung in `rest/auth.rs:103-125`. Audit `totp_failed`. Ein verbrauchter Pending-Token bleibt seine 5 min gültig (braucht trotzdem einen frischen Code) — akzeptiert.

**Weitere Festlegungen:**
- **Secret-Erzeugung:** echter CSPRNG (neue `rand`-Quelle in lineal-app), **nicht** aus UUIDv4 (Version/Variant-Bits). 20-Byte-Secret (RFC-6238-üblich).
- **Grace „3 Logins":** per Test pinnen (3. Login passiert, 4. blockt — Off-by-one festnageln).
- **`/login?next=` vs `search.redirect`:** heutige Diskrepanz (Map §9) beheben, damit der Redirect nach Passwort→TOTP→Session zurück zu `/oauth/authorize` überlebt. **E2E-Test:** anonymes `GET /oauth/authorize` → Login → TOTP → Authorize erfolgreich. Das macht „OAuth automatisch gedeckt" praktisch wahr, nicht nur theoretisch.
- **TOTP-Core:** hand-gerollt (hmac + sha1), verifiziert gegen **RFC 6238 Appendix B** (8-stellig, Secret `12345678901234567890`), konstantzeitiger Vergleich.

## Vorgehen

Dieses Doc sammelt den Design-Stand. **Nicht mit Code beginnen**, bis die 4 offenen Punkte beantwortet sind. Danach: Plan (superpowers:writing-plans) mit Tasks je Fläche (Schema → Enrollment → Recovery → Login-Zweitschritt → OAuth-Pfad → Enforcement), red-first getestet wie der Rest der Sicherheits-Arbeit.
