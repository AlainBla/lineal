//! Manuelle Sortierung (docs/03 §12).

use crate::error::DomainError;
use std::str::FromStr;

/// Unterschreitet der Abstand zweier Nachbarn diesen Wert, wird neu nummeriert.
pub const MIN_GAP: f64 = 1e-6;

/// Größter zulässiger Betrag von `sortOrder`/`position` (B8.21): extreme Werte würden alle
/// folgenden Einträge auf denselben Wert zwingen.
pub const MAX_ABS: f64 = 1e12;

/// Prüft einen vom Aufrufer gelieferten Sortierwert (endlich, Betrag höchstens [`MAX_ABS`]).
pub fn validate(field: &str, v: f64) -> Result<f64, crate::DomainError> {
    if v.is_finite() && v.abs() <= MAX_ABS {
        Ok(v)
    } else {
        Err(crate::DomainError::validation(
            field,
            format!("must be a finite number between -{MAX_ABS:e} and {MAX_ABS:e}"),
        ))
    }
}

/// Neuer `sortOrder` zwischen zwei Nachbarn (docs/03 §12).
///
/// `a` ist der Nachbar **davor** (kleinerer Wert), `b` der Nachbar **danach**.
/// * beide: `(a + b) / 2`
/// * nur `b` (ganz oben einfügen): `b − 1`
/// * nur `a` (ganz unten einfügen): `a + 1`
/// * keiner (leere Liste): `0`
pub fn between(a: Option<f64>, b: Option<f64>) -> f64 {
    match (a, b) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        (None, Some(b)) => b - 1.0,
        (Some(a), None) => a + 1.0,
        (None, None) => 0.0,
    }
}

/// `true`, wenn der Abstand der Nachbarn zu klein für ein weiteres Einfügen ist
/// (Hintergrund-Neunummerierung nötig).
pub fn needs_renumber(a: f64, b: f64) -> bool {
    (b - a).abs() < MIN_GAP
}

/// Neue Werte `1, 2, …, n` mit Abstand 1 für die Neunummerierung.
pub fn renumbered(n: usize) -> Vec<f64> {
    (1..=n).map(|i| i as f64).collect()
}

/// `Team.setIssueSortOrderOnStateChange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrderOnStateChange {
    /// Ganz oben im Zielstatus (Default).
    #[default]
    Top,
    /// Ganz unten im Zielstatus.
    Bottom,
    /// Unverändert.
    None,
}

impl FromStr for SortOrderOnStateChange {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "top" => Ok(Self::Top),
            "bottom" => Ok(Self::Bottom),
            "none" => Ok(Self::None),
            o => Err(DomainError::validation(
                "setIssueSortOrderOnStateChange",
                format!("unknown value `{o}`"),
            )),
        }
    }
}

/// `sortOrder` nach einem Statuswechsel (docs/03 §12, `applySortOrderOnStateChange`).
///
/// Ein expliziter Wert (Drag & Drop) hat Vorrang. `min`/`max` sind die Extremwerte der
/// Issues im Zielstatus; ohne Issues dort (`None`) bleibt der Wert bei `top`/`bottom` 0 ± 0.
pub fn on_state_change(
    setting: SortOrderOnStateChange,
    explicit: Option<f64>,
    current: f64,
    min_in_target: Option<f64>,
    max_in_target: Option<f64>,
) -> f64 {
    if let Some(e) = explicit {
        return e;
    }
    match setting {
        SortOrderOnStateChange::Top => min_in_target.map_or(0.0, |m| m - 1.0),
        SortOrderOnStateChange::Bottom => max_in_target.map_or(0.0, |m| m + 1.0),
        SortOrderOnStateChange::None => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_cases() {
        assert_eq!(between(Some(1.0), Some(3.0)), 2.0);
        assert_eq!(between(Some(-4.0), Some(-3.0)), -3.5);
        assert_eq!(between(None, Some(5.0)), 4.0);
        assert_eq!(between(Some(5.0), None), 6.0);
        assert_eq!(between(None, None), 0.0);
    }

    #[test]
    fn repeated_halving_eventually_needs_renumber() {
        let (a, mut b) = (0.0, 1.0);
        let mut steps = 0;
        while !needs_renumber(a, b) {
            b = between(Some(a), Some(b));
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(steps, 20); // 2^-20 < 1e-6
        assert!(!needs_renumber(0.0, 1e-3));
    }

    #[test]
    fn renumber_has_gap_one() {
        assert_eq!(renumbered(3), [1.0, 2.0, 3.0]);
        assert!(renumbered(0).is_empty());
    }

    #[test]
    fn state_change_policy() {
        use SortOrderOnStateChange::{Bottom, Top};
        let none = SortOrderOnStateChange::None;
        assert_eq!(on_state_change(Top, None, 7.0, Some(2.0), Some(9.0)), 1.0);
        assert_eq!(
            on_state_change(Bottom, None, 7.0, Some(2.0), Some(9.0)),
            10.0
        );
        assert_eq!(on_state_change(none, None, 7.0, Some(2.0), Some(9.0)), 7.0);
        assert_eq!(
            on_state_change(Top, Some(3.5), 7.0, Some(2.0), Some(9.0)),
            3.5
        );
        assert_eq!(on_state_change(Top, None, 7.0, None, None), 0.0);
        assert_eq!("bottom".parse::<SortOrderOnStateChange>(), Ok(Bottom));
        assert!("up".parse::<SortOrderOnStateChange>().is_err());
    }
}
