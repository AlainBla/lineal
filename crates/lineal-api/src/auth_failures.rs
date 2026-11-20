//! Brute-Force-Schutz: fehlgeschlagene Authentifizierungen zählen in eigenen, strengen
//! Zählern, unabhängig vom normalen Request-Bucket ([`crate::rate_limit`]).
//!
//! * je Client-IP: ungültige/abgelaufene Tokens und falsche Passwörter (20 pro Minute),
//! * je Paar (Ziel-Account, Client-IP) beim Login: 10 pro 15 Minuten. Ein Angreifer sperrt damit
//!   einen Nutzer nicht von dessen eigener IP aus,
//! * je Ziel-Account allein nur ein hohes Gesamtlimit (100 pro 15 Minuten) gegen verteilte Angriffe.
//!
//! Der Login reserviert seine Zähler **vor** der Passwortprüfung atomar ([`AuthFailures::reserve`])
//! und gibt sie bei Erfolg zurück ([`AuthFailures::release`]); parallele Versuche überschreiten
//! das Limit so nicht. Bei voller Tabelle lehnt `reserve` neue Schlüssel ab (429).
//!
//! Die Zähler sind feste Zeitfenster. Sie werden weder durch Verdrängung aktiver Einträge noch
//! durch Token-/Bucket-Wechsel zurückgesetzt; aufgeräumt werden nur abgelaufene Fenster. Ist die
//! Tabelle voll (Massen-Spoofing), teilen sich neue Schlüssel einen Überlauf-Zähler (mit
//! zehnfachem Limit) und es wird gewarnt.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

/// Fensterregel eines Zählers.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub limit: u32,
    pub window_ms: i64,
}

/// Fehlversuche je IP: 20 pro Minute.
pub const IP_RULE: Rule = Rule {
    limit: 20,
    window_ms: 60_000,
};
/// Anfragen mit ungültigem **Session-Cookie** (abgelaufen/widerrufen) je IP: getrennt vom
/// `ip:`-Login-Zähler gezählt (LIN-62 L-11), damit ein Browser mit alter Session hinter einer
/// Büro-NAT nicht den Login derselben IP aussperrt. Großzügig und mit derselben Fensterlänge wie
/// `IP_RULE` (wichtig: `purge_expired` nutzt für Nicht-`acct`-Schlüssel die IP-Fensterlänge).
pub const COOKIE_RULE: Rule = Rule {
    limit: 120,
    window_ms: 60_000,
};
/// Fehlversuche je Ziel-Account: 10 pro 15 Minuten.
pub const ACCOUNT_RULE: Rule = Rule {
    limit: 10,
    window_ms: 15 * 60_000,
};
/// Weiches Gesamtlimit je Ziel-Account über alle IPs: 100 pro 15 Minuten. Bekannte IPs dürfen es
/// passieren (S-23), damit ein verteilter Angriff den User nicht von der eigenen IP aussperrt.
pub const ACCOUNT_TOTAL_RULE: Rule = Rule {
    limit: 100,
    window_ms: 15 * 60_000,
};
/// Hartes Gesamtlimit je Ziel-Account: 1000 pro 15 Minuten. Zählt ALLE Fehlversuche – auch die per
/// bekannter IP durchgelassenen – und sperrt bei massivem verteiltem Angriff jeden (Sicherheitsnetz,
/// damit die „bekannte IP"-Ausnahme kein unbegrenztes Durchprobieren erlaubt).
pub const ACCOUNT_HARD_TOTAL_RULE: Rule = Rule {
    limit: 1000,
    window_ms: 15 * 60_000,
};
/// LIN-58: Passwort-Reset-Anfragen je E-Mail und je IP — 3 pro Stunde und 5 pro Tag (jeweils
/// eigener Zähler, da ein `Entry` genau ein Fenster trägt). Schnelle In-Memory-Sperre VOR jeder
/// DB-Arbeit; das neustart-feste Konto-Limit (Zeilen in `password_reset`) liegt zusätzlich darunter.
pub const RESET_HOUR_RULE: Rule = Rule {
    limit: 3,
    window_ms: 60 * 60_000,
};
pub const RESET_DAY_RULE: Rule = Rule {
    limit: 5,
    window_ms: 24 * 60 * 60_000,
};

/// Obergrenze gehaltener Schlüssel.
pub const MAX_KEYS: usize = 200_000;
const OVERFLOW_KEY: &str = "\0overflow";
const PURGE_INTERVAL_MS: i64 = 1_000;

#[derive(Debug, Clone, Copy)]
struct Entry {
    count: u32,
    window_start_ms: i64,
    /// Fensterlänge dieses Zählers. Wird beim Anlegen/Zurücksetzen aus der Regel übernommen, damit
    /// `purge_expired` jede Zeile mit IHRER Länge räumt — sonst würden langlebige Zähler (Stunden-/
    /// Tagesfenster des Reset-Limits, LIN-58) bei voller Tabelle fälschlich nach der IP-Fensterlänge
    /// (60 s) entfernt.
    window_ms: i64,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    last_purge_ms: i64,
    last_warn_ms: i64,
}

/// Senke für Audit-Ereignisse aus dem Zähler (LIN-58 Teil 3). Wird beim Erreichen des Account-
/// Gesamtlimits (verteilter Angriff) aufgerufen. Die Implementierung (im API-Adapter) muss sofort
/// zurückkehren — die eigentliche, asynchrone Audit-Schreibarbeit läuft losgelöst (`tokio::spawn`),
/// damit der Zähler-Lock nicht blockiert.
pub trait LockoutAudit: Send + Sync {
    /// `account` = Klartext-Adresse (zum Auflösen der Workspaces; wird nicht geloggt),
    /// `ip_count` = Zahl aktiver (Account, IP)-Paare im Fenster, `failures` = Stand des
    /// Gesamtzählers, `window_ms` = Fensterlänge, `allowed_known_ip` = für eine bekannte IP (S-23)
    /// durchgelassen.
    fn distributed_lockout(
        &self,
        account: &str,
        ip_count: u32,
        failures: u32,
        window_ms: i64,
        allowed_known_ip: bool,
    );
}

/// Prozessweiter Zähler für Fehlversuche.
pub struct AuthFailures {
    state: Mutex<State>,
    max_keys: usize,
    audit: Option<std::sync::Arc<dyn LockoutAudit>>,
}

impl Default for AuthFailures {
    fn default() -> Self {
        Self::with_max_keys(MAX_KEYS)
    }
}

impl AuthFailures {
    pub fn with_max_keys(max_keys: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            max_keys: max_keys.max(2),
            audit: None,
        }
    }

    /// Hängt die Audit-Senke für `auth.distributed_lockout` an (LIN-58 Teil 3).
    #[must_use]
    pub fn with_audit(mut self, audit: std::sync::Arc<dyn LockoutAudit>) -> Self {
        self.audit = Some(audit);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn active(e: &Entry, rule: Rule, now_ms: i64) -> bool {
        now_ms - e.window_start_ms < rule.window_ms
    }

    /// Sekunden bis zum Ende der Sperre, wenn `key` aktuell gesperrt ist.
    pub fn blocked_for(&self, key: &str, rule: Rule, now_ms: i64) -> Option<u64> {
        let state = self.lock();
        let overflow = state.entries.get(OVERFLOW_KEY);
        let own = state.entries.get(key);
        // Ein unbekannter Schlüssel in voller Tabelle fällt auf den Überlauf-Zähler zurück.
        let (entry, limit) = match own {
            Some(e) => (Some(e), rule.limit),
            None if state.entries.len() >= self.max_keys => (overflow, rule.limit * 10),
            None => (None, rule.limit),
        };
        let e = entry.filter(|e| Self::active(e, rule, now_ms))?;
        (e.count >= limit)
            .then(|| u64::try_from((rule.window_ms - (now_ms - e.window_start_ms)) / 1000 + 1).ok())
            .flatten()
    }

    /// Zählt einen Fehlversuch (atomar unter dem Lock).
    pub fn record(&self, key: &str, rule: Rule, now_ms: i64) {
        let mut state = self.lock();
        let state = &mut *state;
        if state.entries.len() >= self.max_keys && now_ms - state.last_purge_ms >= PURGE_INTERVAL_MS
        {
            Self::purge_expired(state, now_ms);
        }
        let key = if state.entries.len() >= self.max_keys && !state.entries.contains_key(key) {
            if now_ms - state.last_warn_ms >= 60_000 {
                state.last_warn_ms = now_ms;
                tracing::warn!("auth failure counter full; new keys share the overflow counter");
            }
            OVERFLOW_KEY
        } else {
            key
        };
        let e = state.entries.entry(key.to_owned()).or_insert(Entry {
            count: 0,
            window_start_ms: now_ms,
            window_ms: rule.window_ms,
        });
        if !Self::active(e, rule, now_ms) {
            *e = Entry {
                count: 0,
                window_start_ms: now_ms,
                window_ms: rule.window_ms,
            };
        }
        e.count = e.count.saturating_add(1);
        if e.count == rule.limit {
            log_limit_reached(key, rule);
        }
    }

    /// Reserviert atomar einen Versuch auf `key` (Zählen vor dem Prüfen). `Err(Sekunden)`, wenn
    /// das Limit erreicht ist oder die Tabelle voll ist und `key` neu wäre.
    pub fn reserve(&self, key: &str, rule: Rule, now_ms: i64) -> Result<(), u64> {
        self.reserve_counted(key, rule, now_ms).map(|_| ())
    }

    /// Wie [`Self::reserve`], liefert aber bei Erfolg den neuen Zählerstand (für die Erkennung des
    /// Limit-Übergangs; LIN-58 Teil 3).
    fn reserve_counted(&self, key: &str, rule: Rule, now_ms: i64) -> Result<u32, u64> {
        let mut state = self.lock();
        let state = &mut *state;
        if state.entries.len() >= self.max_keys && !state.entries.contains_key(key) {
            Self::purge_expired(state, now_ms);
            if state.entries.len() >= self.max_keys {
                tracing::warn!("auth failure counter full; rejecting a new login key");
                return Err(u64::try_from(rule.window_ms / 1000).unwrap_or(60));
            }
        }
        let e = state.entries.entry(key.to_owned()).or_insert(Entry {
            count: 0,
            window_start_ms: now_ms,
            window_ms: rule.window_ms,
        });
        if !Self::active(e, rule, now_ms) {
            *e = Entry {
                count: 0,
                window_start_ms: now_ms,
                window_ms: rule.window_ms,
            };
        }
        if e.count >= rule.limit {
            let left = (rule.window_ms - (now_ms - e.window_start_ms)) / 1000 + 1;
            return Err(u64::try_from(left).unwrap_or(1));
        }
        e.count += 1;
        if e.count == rule.limit {
            log_limit_reached(key, rule);
        }
        Ok(e.count)
    }

    /// Gibt eine Reservierung zurück (erfolgreicher Login).
    pub fn release(&self, key: &str) {
        if let Some(e) = self.lock().entries.get_mut(key) {
            e.count = e.count.saturating_sub(1);
        }
    }

    fn purge_expired(state: &mut State, now_ms: i64) {
        state.last_purge_ms = now_ms;
        // Nur abgelaufene Fenster entfernen; aktive Zähler bleiben (kein Zurücksetzen). Jede Zeile
        // trägt ihre eigene Fensterlänge, damit langlebige Zähler (Reset-Limit, LIN-58) nicht nach
        // der kurzen IP-Fensterlänge geräumt werden.
        state
            .entries
            .retain(|_, e| now_ms - e.window_start_ms < e.window_ms);
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // --- Kurzformen ---------------------------------------------------------------------

    pub fn ip_blocked(&self, ip_key: &str, now_ms: i64) -> Option<u64> {
        self.blocked_for(&format!("ip:{ip_key}"), IP_RULE, now_ms)
    }

    pub fn record_ip(&self, ip_key: &str, now_ms: i64) {
        self.record(&format!("ip:{ip_key}"), IP_RULE, now_ms);
    }

    /// Wie `ip_blocked`, aber für ungültige Session-Cookies (eigener, großzügiger Zähler, LIN-62
    /// L-11). Sperrt NICHT den Login-`ip:`-Zähler.
    pub fn cookie_blocked(&self, ip_key: &str, now_ms: i64) -> Option<u64> {
        self.blocked_for(&format!("cookie:{ip_key}"), COOKIE_RULE, now_ms)
    }

    pub fn record_cookie(&self, ip_key: &str, now_ms: i64) {
        self.record(&format!("cookie:{ip_key}"), COOKIE_RULE, now_ms);
    }

    /// Reserviert die Login-Zähler (IP, Paar Account+IP, Account gesamt) vor der Passwortprüfung.
    /// Bei `Err` ist nichts reserviert.
    ///
    /// S-23: Das Gesamtlimit je Account würde einen Account bei verteilten Fehlversuchen von ALLEN
    /// IPs sperren – auch von der eigenen. `ip_known` (jüngst erfolgreicher Login von dieser IP)
    /// lässt eine bekannte IP durch das Gesamtlimit; der Angriff über fremde IPs bleibt gesperrt.
    /// Ein so durchgelassener Fehlversuch erhöht das Gesamtlimit NICHT (das harte Paar-Limit
    /// 10/15min je Account+IP greift weiterhin).
    pub fn reserve_login(
        &self,
        account: &str,
        ip_key: Option<&str>,
        ip_known: bool,
        now_ms: i64,
    ) -> Result<LoginReservation, u64> {
        let ip = ip_key.unwrap_or("-");
        let mut held = Vec::new();
        // Harte Limits je IP und je Paar (Account, IP) – immer.
        for (key, rule) in [
            (format!("ip:{ip}"), IP_RULE),
            (pair_key(account, ip), ACCOUNT_RULE),
        ] {
            match self.reserve(&key, rule, now_ms) {
                Ok(()) => held.push(key),
                Err(wait) => {
                    for k in &held {
                        self.release(k);
                    }
                    return Err(wait);
                }
            }
        }
        // Hartes Gesamtlimit je Account – zählt auch per bekannter IP durchgelassene Fehlversuche.
        // LIN-58 Teil 3: Beim Übergang (count == limit) `auth.distributed_lockout` auditieren.
        let hard = account_hard_total_key(account);
        match self.reserve_counted(&hard, ACCOUNT_HARD_TOTAL_RULE, now_ms) {
            Ok(count) => {
                held.push(hard);
                if count == ACCOUNT_HARD_TOTAL_RULE.limit {
                    self.report_lockout(
                        account,
                        count,
                        ACCOUNT_HARD_TOTAL_RULE.window_ms,
                        false,
                        now_ms,
                    );
                }
            }
            Err(wait) => {
                for k in &held {
                    self.release(k);
                }
                return Err(wait);
            }
        }
        // Weiches Gesamtlimit je Account (verteilte Angriffe) – bekannte IPs dürfen hindurch.
        let total = account_total_key(account);
        match self.reserve_counted(&total, ACCOUNT_TOTAL_RULE, now_ms) {
            Ok(count) => {
                held.push(total);
                // Übergang: verteilter Angriff hat gerade das weiche Gesamtlimit erreicht.
                if count == ACCOUNT_TOTAL_RULE.limit {
                    self.report_lockout(
                        account,
                        count,
                        ACCOUNT_TOTAL_RULE.window_ms,
                        false,
                        now_ms,
                    );
                }
            }
            Err(wait) => {
                if ip_known {
                    // ~277: bekannte IP wird trotz erreichtem Gesamtlimit durchgelassen (S-23).
                    tracing::warn!(
                        "distributed login failures reached the account limit; a known IP was allowed through"
                    );
                    self.report_lockout(
                        account,
                        ACCOUNT_TOTAL_RULE.limit,
                        ACCOUNT_TOTAL_RULE.window_ms,
                        true,
                        now_ms,
                    );
                } else {
                    for k in &held {
                        self.release(k);
                    }
                    return Err(wait);
                }
            }
        }
        Ok(LoginReservation { keys: held })
    }

    /// Reserviert die Passwort-Reset-Limits (LIN-58): je E-Mail und je IP, jeweils 3/Stunde UND
    /// 5/Tag. Alles-oder-nichts (bei `Err` ist nichts reserviert); die Reservierungen werden NIE
    /// zurückgegeben (jede Anfrage zählt, unabhängig davon, ob ein Konto existiert). `ip = None`
    /// zählt nicht gegen ein IP-Limit, die E-Mail-Limits gelten trotzdem.
    pub fn reserve_password_reset(
        &self,
        email: &str,
        ip: Option<&str>,
        now_ms: i64,
    ) -> Result<(), u64> {
        let email_hash = account_hash(email);
        let mut keys: Vec<(String, Rule)> = vec![
            (format!("rpweh:{email_hash}"), RESET_HOUR_RULE),
            (format!("rpwed:{email_hash}"), RESET_DAY_RULE),
        ];
        if let Some(ip) = ip {
            keys.push((format!("rpwih:{ip}"), RESET_HOUR_RULE));
            keys.push((format!("rpwid:{ip}"), RESET_DAY_RULE));
        }
        let mut held: Vec<String> = Vec::new();
        for (key, rule) in keys {
            match self.reserve(&key, rule, now_ms) {
                Ok(()) => held.push(key),
                Err(wait) => {
                    // Zurückrollen: die bereits gezählten Teil-Reservierungen wieder freigeben, damit
                    // eine abgelehnte Anfrage das Kontingent nicht doppelt belastet.
                    for k in &held {
                        self.release(k);
                    }
                    return Err(wait);
                }
            }
        }
        Ok(())
    }

    /// Zahl der im Account-Fenster aktiven (Account, IP)-Paare ≈ Zahl verteilter Quell-IPs. Für die
    /// `ipCount`-Angabe im Lockout-Audit (LIN-58 Teil 3).
    fn distinct_ips_for_account(&self, account: &str, now_ms: i64) -> u32 {
        let prefix = format!("acct:{}|", account_hash(account));
        let state = self.lock();
        state
            .entries
            .iter()
            .filter(|(k, e)| {
                k.starts_with(&prefix) && now_ms - e.window_start_ms < ACCOUNT_RULE.window_ms
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Meldet einen verteilten Lockout an die Audit-Senke (falls angehängt).
    fn report_lockout(
        &self,
        account: &str,
        failures: u32,
        window_ms: i64,
        allowed_known_ip: bool,
        now_ms: i64,
    ) {
        if let Some(audit) = &self.audit {
            let ip_count = self.distinct_ips_for_account(account, now_ms);
            audit.distributed_lockout(account, ip_count, failures, window_ms, allowed_known_ip);
        }
    }

    /// Ob das Gesamtlimit des Accounts aktuell erreicht ist (für den „bekannte IP"-Pfad im Handler,
    /// damit die DB nur dann nach bekannten IPs gefragt wird).
    pub fn account_total_blocked(&self, account: &str, now_ms: i64) -> bool {
        self.blocked_for(&account_total_key(account), ACCOUNT_TOTAL_RULE, now_ms)
            .is_some()
    }
}

/// Reservierte Login-Zähler; bei Erfolg zurückgeben, bei Fehlschlag verfallen lassen.
#[derive(Debug)]
pub struct LoginReservation {
    keys: Vec<String>,
}

impl LoginReservation {
    /// Erfolgreicher Login (oder interner Fehler): Versuch zählt nicht.
    pub fn release(self, failures: &AuthFailures) {
        for k in &self.keys {
            failures.release(k);
        }
    }
}

/// Hash der normalisierten Adresse: Die Schlüssel entstehen vor jeder Prüfung der Eingabe und
/// bleiben im Speicher, ihre Größe darf also nicht vom Absender abhängen.
/// LIN-65 N13: Beim ERSTEN Erreichen eines Fehlversuch-Limits (Transition, einmal pro Fenster) auf
/// dem `security`-Ziel warnen — so wird Passwort-Spraying/Brute-Force in den Logs sichtbar (bisher
/// nur als 429/401) und kann alarmiert werden.
fn log_limit_reached(key: &str, rule: Rule) {
    tracing::warn!(
        target: "security",
        counter = %redacted_key(key),
        limit = rule.limit,
        window_ms = rule.window_ms,
        "authentication failure limit reached"
    );
}

/// Loggbare Kurzform eines Zaehler-Schluessels: der Typ und — nur bei IP-/Cookie-Zaehlern — die
/// Client-IP (betrieblich). Der (unsalted und damit umkehrbare) Account-Hash wird NIE geloggt,
/// damit aus den Logs keine Kennungen rekonstruierbar sind.
fn redacted_key(key: &str) -> String {
    let kind = key.split([':', '|']).next().unwrap_or("key");
    match key.split_once(':') {
        Some(("ip" | "cookie", ip)) => format!("{kind}:{ip}"),
        _ => kind.to_string(),
    }
}

fn account_hash(account: &str) -> String {
    format!("{:x}", Sha256::digest(account.trim().to_lowercase()))
}

fn pair_key(account: &str, ip: &str) -> String {
    format!("acct:{}|{ip}", account_hash(account))
}

fn account_total_key(account: &str) -> String {
    format!("acctt:{}", account_hash(account))
}

fn account_hard_total_key(account: &str) -> String {
    format!("accth:{}", account_hash(account))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// LIN-65 N13: Das Sicherheits-Log nennt nur Zaehler-Typ und (bei IP-/Cookie-Zaehlern) die IP;
    /// der umkehrbare Account-Hash taucht nie auf.
    #[test]
    fn redacted_key_never_leaks_account_hash() {
        assert_eq!(redacted_key("ip:203.0.113.7"), "ip:203.0.113.7");
        assert_eq!(redacted_key("cookie:203.0.113.7"), "cookie:203.0.113.7");
        assert_eq!(redacted_key("acct:deadbeefcafe|203.0.113.7"), "acct");
        assert_eq!(redacted_key("acctt:deadbeefcafe"), "acctt");
        assert_eq!(redacted_key("accth:deadbeefcafe"), "accth");
        assert_eq!(redacted_key("acctpw:1a2b3c"), "acctpw");
        assert_eq!(redacted_key(OVERFLOW_KEY), OVERFLOW_KEY);
    }

    #[test]
    fn cookie_failures_do_not_block_the_login_ip() {
        // LIN-62 L-11: Viele Anfragen mit ungültigem Session-Cookie von derselben IP dürfen den
        // `ip:`-Login-Zähler NICHT sperren (sonst 429 beim Login hinter NAT).
        let f = AuthFailures::default();
        for i in 0..100 {
            f.record_cookie("198.51.100.9", i);
        }
        assert!(
            f.ip_blocked("198.51.100.9", 100).is_none(),
            "Cookie-Fehler dürfen den Login-IP-Zähler nicht sperren"
        );
        // Der Cookie-Zähler selbst sperrt erst ab seinem eigenen (großzügigen) Limit.
        for i in 0..i64::from(COOKIE_RULE.limit) {
            f.record_cookie("203.0.113.5", i);
        }
        assert!(f.cookie_blocked("203.0.113.5", 1000).is_some());
    }

    #[test]
    fn login_keys_stay_small_for_huge_accounts() {
        let account = "a".repeat(2 * 1024 * 1024);
        assert!(pair_key(&account, "203.0.113.7").len() < 128);
        assert!(account_total_key(&account).len() < 128);
    }

    #[test]
    fn login_keys_ignore_case_and_outer_spaces() {
        assert_eq!(pair_key(" A@b.de ", "ip"), pair_key("a@b.de", "ip"));
        assert_eq!(account_total_key(" A@b.de "), account_total_key("a@b.de"));
        assert_ne!(pair_key("a@b.de", "ip1"), pair_key("a@b.de", "ip2"));
        assert_ne!(account_total_key("a@b.de"), account_total_key("c@b.de"));
    }

    #[test]
    fn ip_blocks_after_twenty_failures_and_recovers_after_the_window() {
        let f = AuthFailures::default();
        for _ in 0..20 {
            assert!(f.ip_blocked("v4:1.2.3.4", 1_000).is_none());
            f.record_ip("v4:1.2.3.4", 1_000);
        }
        let wait = f.ip_blocked("v4:1.2.3.4", 2_000).unwrap();
        assert!((1..=60).contains(&wait));
        assert!(f.ip_blocked("v4:1.2.3.5", 2_000).is_none());
        assert!(f.ip_blocked("v4:1.2.3.4", 1_000 + 61_000).is_none());
    }

    #[test]
    fn account_pair_blocks_only_the_attacking_ip_not_the_owner() {
        let f = AuthFailures::default();
        let attack = |ip: &str| f.reserve_login("Alice@Acme.example", Some(ip), false, 5_000);
        for _ in 0..10 {
            // Fehlversuche des Angreifers: Reservierungen bleiben bestehen.
            attack("v4:6.6.6.6").unwrap();
        }
        // Der Angreifer ist (je IP und je Paar) gesperrt ...
        assert!(attack("v4:6.6.6.6").is_err());
        // ... Alice von ihrer eigenen IP aber nicht (anderes Paar, andere IP).
        let own = f
            .reserve_login("alice@acme.example", Some("v4:1.1.1.1"), false, 5_100)
            .unwrap();
        own.release(&f);
        // Nach dem Fenster ist auch der Angreifer wieder frei.
        assert!(attack_after(&f, 5_000 + 15 * 60_000 + 1).is_ok());
    }

    fn attack_after(f: &AuthFailures, now: i64) -> Result<LoginReservation, u64> {
        f.reserve_login("alice@acme.example", Some("v4:6.6.6.6"), false, now)
    }

    #[test]
    fn distributed_attack_hits_the_account_total_limit() {
        let f = AuthFailures::default();
        // 100 Fehlversuche von je einer eigenen IP (je Paar nur 1, je IP nur 1) ...
        for i in 0..100 {
            f.reserve_login(
                "bob@acme.example",
                Some(&format!("v4:10.0.{}.{}", i / 250, i)),
                false,
                1_000,
            )
            .unwrap();
        }
        // ... erreichen das hohe Gesamtlimit des Accounts.
        assert!(
            f.reserve_login("bob@acme.example", Some("v4:10.9.9.9"), false, 1_000)
                .is_err()
        );
        // Andere Accounts sind unberührt.
        assert!(
            f.reserve_login("carol@acme.example", Some("v4:10.9.9.9"), false, 1_000)
                .is_ok()
        );
    }

    #[test]
    fn successful_login_gives_the_reservation_back() {
        let f = AuthFailures::default();
        for _ in 0..50 {
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), false, 1_000)
                .unwrap()
                .release(&f);
        }
        // Erfolgreiche Logins verbrauchen kein Kontingent.
        for _ in 0..10 {
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), false, 1_000)
                .unwrap();
        }
        assert!(
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), false, 1_000)
                .is_err()
        );
    }

    #[test]
    fn fifty_parallel_attempts_never_exceed_the_limit() {
        let f = Arc::new(AuthFailures::default());
        let granted: usize = (0..50)
            .map(|_| {
                let f = f.clone();
                std::thread::spawn(move || {
                    f.reserve_login("alice@acme.example", Some("v4:6.6.6.6"), false, 1_000)
                        .is_ok()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        assert_eq!(granted, 10, "je Paar höchstens 10 Versuche");
    }

    #[test]
    fn login_keys_are_rejected_when_the_table_is_full() {
        let f = AuthFailures::with_max_keys(30);
        let mut rejected = false;
        for i in 0..100 {
            if f.reserve_login(
                &format!("user{i}@acme.example"),
                Some("v4:7.7.7.7"),
                false,
                1_000,
            )
            .is_err()
            {
                rejected = true;
            }
        }
        assert!(
            rejected,
            "neue Schlüssel werden bei voller Tabelle abgelehnt"
        );
        assert!(f.len() <= 30);
    }

    #[test]
    fn full_table_never_resets_active_counters() {
        let f = AuthFailures::with_max_keys(50);
        for _ in 0..20 {
            f.record_ip("v4:6.6.6.6", 1_000);
        }
        assert!(f.ip_blocked("v4:6.6.6.6", 1_500).is_some());
        // Massen-Spoofing füllt die Tabelle.
        for i in 0..5_000 {
            f.record_ip(&format!("v4:fake{i}"), 1_000 + i64::from(i % 100));
        }
        assert!(f.len() <= 51, "Tabelle bleibt begrenzt: {}", f.len());
        // Der Zähler des Angreifers wurde nicht verdrängt oder zurückgesetzt.
        assert!(f.ip_blocked("v4:6.6.6.6", 2_000).is_some());
        // Nach Ablauf des Fensters ist er frei.
        assert!(f.ip_blocked("v4:6.6.6.6", 1_000 + 61_000).is_none());
    }

    #[test]
    fn concurrent_failures_are_counted_exactly() {
        let f = Arc::new(AuthFailures::default());
        let handles: Vec<_> = (0..50)
            .map(|_| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for _ in 0..2 {
                        f.record_ip("v4:9.9.9.9", 1_000);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(f.ip_blocked("v4:9.9.9.9", 1_000).is_some());
        let state = f.lock();
        assert_eq!(state.entries["ip:v4:9.9.9.9"].count, 100);
    }

    /// S-23: Ist das Gesamtlimit durch einen verteilten Angriff erreicht, bleibt eine unbekannte IP
    /// gesperrt, eine bekannte IP (jüngst erfolgreicher Login) kommt aber durch.
    #[test]
    fn account_total_lets_a_known_ip_through_but_blocks_unknown() {
        let f = AuthFailures::default();
        for i in 0..100 {
            f.reserve_login(
                "bob@acme.example",
                Some(&format!("v4:11.0.{}.{}", i / 250, i)),
                false,
                1_000,
            )
            .unwrap();
        }
        assert!(f.account_total_blocked("bob@acme.example", 1_000));
        // Unbekannte IP: weiterhin gesperrt.
        assert!(
            f.reserve_login("bob@acme.example", Some("v4:11.9.9.9"), false, 1_000)
                .is_err()
        );
        // Bekannte IP: darf das Gesamtlimit passieren (ihr Paar-Limit ist noch frei).
        assert!(
            f.reserve_login("bob@acme.example", Some("v4:11.9.9.9"), true, 1_000)
                .is_ok()
        );
    }

    /// Auch eine bekannte IP bleibt an das harte Paar-Limit (10/15min je Account+IP) gebunden – ein
    /// Angreifer von einer bekannten IP kann also trotzdem nicht durchprobieren.
    #[test]
    fn a_known_ip_is_still_bound_by_the_pair_limit() {
        let f = AuthFailures::default();
        for i in 0..100 {
            f.reserve_login(
                "eve@acme.example",
                Some(&format!("v4:12.0.{}.{}", i / 250, i)),
                false,
                1_000,
            )
            .unwrap();
        }
        for _ in 0..10 {
            f.reserve_login("eve@acme.example", Some("v4:12.9.9.9"), true, 1_000)
                .unwrap();
        }
        assert!(
            f.reserve_login("eve@acme.example", Some("v4:12.9.9.9"), true, 1_000)
                .is_err()
        );
    }

    /// LIN-58: Reset-Limit 3/Stunde und 5/Tag, je E-Mail UND je IP.
    #[test]
    fn password_reset_limit_is_three_per_hour_five_per_day_per_email_and_ip() {
        let f = AuthFailures::default();
        let hour = 60 * 60_000;
        // Erste 3 (Stundenlimit) ok, die 4. ist gesperrt.
        for _ in 0..3 {
            f.reserve_password_reset("alice@acme.example", Some("1.1.1.1"), 0)
                .unwrap();
        }
        assert!(
            f.reserve_password_reset("alice@acme.example", Some("1.1.1.1"), 0)
                .is_err(),
            "4. Anfrage in der Stunde ist gesperrt"
        );
        // Eine andere E-Mail von DERSELBEN IP ist auch gesperrt (IP-Stundenlimit erschöpft).
        assert!(
            f.reserve_password_reset("bob@acme.example", Some("1.1.1.1"), 0)
                .is_err(),
            "per-IP-Limit greift über E-Mails hinweg"
        );
        // Dieselbe E-Mail von einer ANDEREN IP: Stundenlimit der E-Mail ist ebenfalls erschöpft.
        assert!(
            f.reserve_password_reset("alice@acme.example", Some("2.2.2.2"), 0)
                .is_err(),
            "per-E-Mail-Limit greift über IPs hinweg"
        );
        // Nächste Stunde: Stundenlimit frisch, aber das Tageslimit (5) bleibt. Noch 2 ok, dann Schluss.
        f.reserve_password_reset("alice@acme.example", Some("1.1.1.1"), hour + 1)
            .unwrap();
        f.reserve_password_reset("alice@acme.example", Some("1.1.1.1"), hour + 1)
            .unwrap();
        assert!(
            f.reserve_password_reset("alice@acme.example", Some("1.1.1.1"), hour + 1)
                .is_err(),
            "6. Anfrage am Tag ist gesperrt (Tageslimit 5)"
        );
    }

    /// LIN-58: Eine abgelehnte Reset-Anfrage belastet das Kontingent nicht doppelt — die bereits
    /// gezählten Teil-Reservierungen werden zurückgerollt. Szenario: frische E-Mail auf einer
    /// erschöpften IP (E-Mail-Zähler werden reserviert, dann scheitert der IP-Zähler → Rollback des
    /// E-Mail-Zählers).
    #[test]
    fn a_rejected_reset_rolls_back_partial_reservations() {
        let f = AuthFailures::default();
        // IP 7.7.7.7 über E-Mail A erschöpfen (Stundenlimit 3).
        for _ in 0..3 {
            f.reserve_password_reset("a@acme.example", Some("7.7.7.7"), 0)
                .unwrap();
        }
        // Frische E-Mail B auf derselben IP: E-Mail-Zähler werden reserviert, dann scheitert der
        // IP-Zähler → Ablehnung, und die E-Mail-Teilreservierung muss zurückgerollt werden.
        assert!(
            f.reserve_password_reset("b@acme.example", Some("7.7.7.7"), 0)
                .is_err()
        );
        // Beweis des Rollbacks: B kann auf einer ANDEREN IP noch volle 3× — wäre der E-Mail-Zähler
        // oben nicht zurückgerollt worden, stünde er schon bei 1 und es wären nur 2.
        for _ in 0..3 {
            f.reserve_password_reset("b@acme.example", Some("8.8.8.8"), 0)
                .unwrap();
        }
        assert!(
            f.reserve_password_reset("b@acme.example", Some("8.8.8.8"), 0)
                .is_err(),
            "B hat jetzt sein Stundenlimit (3) erreicht"
        );
    }

    /// LIN-58: Langlebige Zähler (Reset-Tagesfenster) überleben das Räumen bei voller Tabelle — der
    /// frühere `purge_expired` hätte sie nach der IP-Fensterlänge (60 s) gelöscht.
    #[test]
    fn long_window_reset_keys_survive_table_pressure() {
        let f = AuthFailures::with_max_keys(50);
        // Tageszähler für eine E-Mail anlegen (3 Anfragen; Stundenlimit 3, Tageslimit 5).
        for _ in 0..3 {
            f.reserve_password_reset("victim@acme.example", Some("5.5.5.5"), 1_000)
                .unwrap();
        }
        // Tabelle mit kurzen IP-Zählern fluten (löst Purge aus), weit nach 60 s — der alte
        // `purge_expired` hätte den Tageszähler (window_start = 1_000) jetzt fälschlich geräumt.
        for i in 0..5_000 {
            f.record_ip(&format!("v4:fake{i}"), 100_000 + i64::from(i % 100));
        }
        // Nächste Stunde: das Stundenlimit ist frisch, der Tageszähler (window 24 h) steht noch bei 3.
        let next_hour = 1_000 + 60 * 60_000 + 1_000;
        f.reserve_password_reset("victim@acme.example", Some("5.5.5.5"), next_hour)
            .unwrap();
        f.reserve_password_reset("victim@acme.example", Some("5.5.5.5"), next_hour)
            .unwrap();
        assert!(
            f.reserve_password_reset("victim@acme.example", Some("5.5.5.5"), next_hour)
                .is_err(),
            "der Tageszähler wurde fälschlich geräumt (Tageslimit 5 müsste greifen)"
        );
    }

    /// LIN-58 Teil 3: Beim Erreichen des Account-Gesamtlimits (verteilter Angriff) wird die Audit-
    /// Senke genau einmal (Übergang) aufgerufen; eine durchgelassene bekannte IP ruft sie erneut
    /// mit `allowed_known_ip = true` auf.
    #[test]
    fn distributed_lockout_audit_fires_at_the_transition() {
        #[derive(Default)]
        struct Rec {
            calls: std::sync::Mutex<Vec<(u32, u32, i64, bool)>>,
        }
        impl LockoutAudit for Rec {
            fn distributed_lockout(
                &self,
                _account: &str,
                ip_count: u32,
                failures: u32,
                window_ms: i64,
                allowed_known_ip: bool,
            ) {
                self.calls
                    .lock()
                    .unwrap()
                    .push((ip_count, failures, window_ms, allowed_known_ip));
            }
        }
        let rec = Arc::new(Rec::default());
        let f = AuthFailures::default().with_audit(rec.clone());
        // 100 Fehlversuche von je eigener IP erreichen das weiche Gesamtlimit (ACCOUNT_TOTAL_RULE).
        for i in 0..100 {
            f.reserve_login(
                "bob@acme.example",
                Some(&format!(
                    "v4:10.{}.{}.{}",
                    i / 65536,
                    (i / 256) % 256,
                    i % 256
                )),
                false,
                1_000,
            )
            .unwrap();
        }
        {
            let calls = rec.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "genau ein Übergang: {calls:?}");
            let (ip_count, failures, window_ms, allowed) = calls[0];
            assert_eq!(failures, ACCOUNT_TOTAL_RULE.limit);
            assert_eq!(window_ms, ACCOUNT_TOTAL_RULE.window_ms);
            assert!(!allowed, "Übergang ist keine Durchlass-Meldung");
            assert!(ip_count >= 90, "ip_count ≈ Zahl verteilter IPs: {ip_count}");
        }
        // Eine bekannte IP wird durchgelassen (S-23) und erneut auditiert.
        f.reserve_login("bob@acme.example", Some("v4:10.9.9.9"), true, 1_000)
            .unwrap();
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "Durchlass-Meldung kommt hinzu");
        assert!(calls[1].3, "zweiter Aufruf markiert die durchgelassene IP");
    }

    /// Das harte Gesamtlimit zählt auch per bekannter IP durchgelassene Fehlversuche und sperrt bei
    /// massivem verteiltem Angriff selbst eine bekannte IP (Sicherheitsnetz, S-23/Review).
    #[test]
    fn the_hard_total_eventually_blocks_even_a_known_ip() {
        let f = AuthFailures::default();
        // 1000 Versuche von je eigener IP; als "bekannt" markiert, damit das weiche Limit sie
        // durchlässt und sie das harte Gesamtlimit füllen.
        for i in 0..1000 {
            f.reserve_login(
                "mallory@acme.example",
                Some(&format!(
                    "v4:13.{}.{}.{}",
                    i / 65536,
                    (i / 256) % 256,
                    i % 256
                )),
                true,
                1_000,
            )
            .unwrap();
        }
        assert!(
            f.reserve_login(
                "mallory@acme.example",
                Some("v4:13.255.255.255"),
                true,
                1_000
            )
            .is_err()
        );
    }
}
