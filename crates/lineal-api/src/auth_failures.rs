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
/// Fehlversuche je Ziel-Account: 10 pro 15 Minuten.
pub const ACCOUNT_RULE: Rule = Rule {
    limit: 10,
    window_ms: 15 * 60_000,
};
/// Gesamtlimit je Ziel-Account über alle IPs: 100 pro 15 Minuten.
pub const ACCOUNT_TOTAL_RULE: Rule = Rule {
    limit: 100,
    window_ms: 15 * 60_000,
};
/// Obergrenze gehaltener Schlüssel.
pub const MAX_KEYS: usize = 200_000;
const OVERFLOW_KEY: &str = "\0overflow";
const PURGE_INTERVAL_MS: i64 = 1_000;

#[derive(Debug, Clone, Copy)]
struct Entry {
    count: u32,
    window_start_ms: i64,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    last_purge_ms: i64,
    last_warn_ms: i64,
}

/// Prozessweiter Zähler für Fehlversuche.
pub struct AuthFailures {
    state: Mutex<State>,
    max_keys: usize,
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
        }
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
        });
        if !Self::active(e, rule, now_ms) {
            *e = Entry {
                count: 0,
                window_start_ms: now_ms,
            };
        }
        e.count = e.count.saturating_add(1);
    }

    /// Reserviert atomar einen Versuch auf `key` (Zählen vor dem Prüfen). `Err(Sekunden)`, wenn
    /// das Limit erreicht ist oder die Tabelle voll ist und `key` neu wäre.
    pub fn reserve(&self, key: &str, rule: Rule, now_ms: i64) -> Result<(), u64> {
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
        });
        if !Self::active(e, rule, now_ms) {
            *e = Entry {
                count: 0,
                window_start_ms: now_ms,
            };
        }
        if e.count >= rule.limit {
            let left = (rule.window_ms - (now_ms - e.window_start_ms)) / 1000 + 1;
            return Err(u64::try_from(left).unwrap_or(1));
        }
        e.count += 1;
        Ok(())
    }

    /// Gibt eine Reservierung zurück (erfolgreicher Login).
    pub fn release(&self, key: &str) {
        if let Some(e) = self.lock().entries.get_mut(key) {
            e.count = e.count.saturating_sub(1);
        }
    }

    fn purge_expired(state: &mut State, now_ms: i64) {
        state.last_purge_ms = now_ms;
        // Nur abgelaufene Fenster entfernen; aktive Zähler bleiben (kein Zurücksetzen).
        state.entries.retain(|k, e| {
            let w = if k.starts_with("acct") {
                ACCOUNT_RULE.window_ms
            } else {
                IP_RULE.window_ms
            };
            now_ms - e.window_start_ms < w
        });
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

    /// Reserviert die Login-Zähler (IP, Paar Account+IP, Account gesamt) vor der Passwortprüfung.
    /// Bei `Err` ist nichts reserviert.
    pub fn reserve_login(
        &self,
        account: &str,
        ip_key: Option<&str>,
        now_ms: i64,
    ) -> Result<LoginReservation, u64> {
        let ip = ip_key.unwrap_or("-");
        let keys = [
            (format!("ip:{ip}"), IP_RULE),
            (pair_key(account, ip), ACCOUNT_RULE),
            (account_total_key(account), ACCOUNT_TOTAL_RULE),
        ];
        let mut held = Vec::new();
        for (key, rule) in keys {
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
        Ok(LoginReservation { keys: held })
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

fn pair_key(account: &str, ip: &str) -> String {
    format!("acct:{}|{ip}", account.trim().to_lowercase())
}

fn account_total_key(account: &str) -> String {
    format!("acctt:{}", account.trim().to_lowercase())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        let attack = |ip: &str| f.reserve_login("Alice@Acme.example", Some(ip), 5_000);
        for _ in 0..10 {
            // Fehlversuche des Angreifers: Reservierungen bleiben bestehen.
            attack("v4:6.6.6.6").unwrap();
        }
        // Der Angreifer ist (je IP und je Paar) gesperrt ...
        assert!(attack("v4:6.6.6.6").is_err());
        // ... Alice von ihrer eigenen IP aber nicht (anderes Paar, andere IP).
        let own = f
            .reserve_login("alice@acme.example", Some("v4:1.1.1.1"), 5_100)
            .unwrap();
        own.release(&f);
        // Nach dem Fenster ist auch der Angreifer wieder frei.
        assert!(attack_after(&f, 5_000 + 15 * 60_000 + 1).is_ok());
    }

    fn attack_after(f: &AuthFailures, now: i64) -> Result<LoginReservation, u64> {
        f.reserve_login("alice@acme.example", Some("v4:6.6.6.6"), now)
    }

    #[test]
    fn distributed_attack_hits_the_account_total_limit() {
        let f = AuthFailures::default();
        // 100 Fehlversuche von je einer eigenen IP (je Paar nur 1, je IP nur 1) ...
        for i in 0..100 {
            f.reserve_login(
                "bob@acme.example",
                Some(&format!("v4:10.0.{}.{}", i / 250, i)),
                1_000,
            )
            .unwrap();
        }
        // ... erreichen das hohe Gesamtlimit des Accounts.
        assert!(
            f.reserve_login("bob@acme.example", Some("v4:10.9.9.9"), 1_000)
                .is_err()
        );
        // Andere Accounts sind unberührt.
        assert!(
            f.reserve_login("carol@acme.example", Some("v4:10.9.9.9"), 1_000)
                .is_ok()
        );
    }

    #[test]
    fn successful_login_gives_the_reservation_back() {
        let f = AuthFailures::default();
        for _ in 0..50 {
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), 1_000)
                .unwrap()
                .release(&f);
        }
        // Erfolgreiche Logins verbrauchen kein Kontingent.
        for _ in 0..10 {
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), 1_000)
                .unwrap();
        }
        assert!(
            f.reserve_login("alice@acme.example", Some("v4:1.1.1.1"), 1_000)
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
                    f.reserve_login("alice@acme.example", Some("v4:6.6.6.6"), 1_000)
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
            if f.reserve_login(&format!("user{i}@acme.example"), Some("v4:7.7.7.7"), 1_000)
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
}
