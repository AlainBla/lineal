//! Brute-Force-Schutz: fehlgeschlagene Authentifizierungen zählen in eigenen, strengen
//! Zählern, unabhängig vom normalen Request-Bucket ([`crate::rate_limit`]).
//!
//! * je Client-IP: ungültige/abgelaufene Tokens und falsche Passwörter (20 pro Minute),
//! * je Ziel-Account beim Login: falsche Passwörter (10 pro 15 Minuten).
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
            state.last_purge_ms = now_ms;
            // Nur abgelaufene Fenster entfernen; aktive Zähler bleiben (kein Zurücksetzen).
            state.entries.retain(|k, e| {
                let w = if k.starts_with("acct:") {
                    ACCOUNT_RULE.window_ms
                } else {
                    IP_RULE.window_ms
                };
                now_ms - e.window_start_ms < w
            });
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

    pub fn account_blocked(&self, account: &str, now_ms: i64) -> Option<u64> {
        self.blocked_for(&account_key(account), ACCOUNT_RULE, now_ms)
    }

    pub fn record_account(&self, account: &str, now_ms: i64) {
        self.record(&account_key(account), ACCOUNT_RULE, now_ms);
    }
}

fn account_key(account: &str) -> String {
    format!("acct:{}", account.trim().to_lowercase())
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
    fn account_blocks_across_many_ips_and_ignores_case() {
        let f = AuthFailures::default();
        for i in 0..10 {
            // Jeder Versuch kommt von einer anderen IP: der Account-Zähler greift trotzdem.
            f.record_ip(&format!("v4:10.0.0.{i}"), 5_000);
            f.record_account("Alice@Acme.example", 5_000);
        }
        assert!(f.account_blocked("alice@acme.example", 6_000).is_some());
        assert!(f.account_blocked("bob@acme.example", 6_000).is_none());
        assert!(
            f.account_blocked("alice@acme.example", 5_000 + 15 * 60_000 + 1)
                .is_none()
        );
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
