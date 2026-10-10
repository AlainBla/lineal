//! Scope und Fortschritt von Cycles, Projekten und Meilensteinen (docs/01 §6.3).

use crate::enums::{EstimationType, StateType};

/// Eingabe je Issue; Punkte richten sich nach den Einstellungen des **eigenen** Teams.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IssueProgressInput {
    /// Statuskategorie des Issues.
    pub state_type: StateType,
    /// Gesetzter Schätzwert (`None` = ungeschätzt).
    pub estimate: Option<f64>,
    /// Effektive Schätzskala des Teams.
    pub estimation_type: EstimationType,
    /// `Team.defaultIssueEstimate` (Default 1).
    pub default_estimate: f64,
}

/// Kennzahlen einer Issue-Menge.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stats {
    /// Summe der Punkte aller relevanten Issues (ohne `canceled`/`duplicate`).
    pub scope: f64,
    /// Punkte der `completed`-Issues.
    pub completed: f64,
    /// Punkte der `started`-Issues.
    pub started: f64,
    /// `(completed + 0,25 × started) / scope`, 0 bei leerem Scope.
    pub progress: f64,
}

/// Punkte eines Issues (docs/01 §6.3): `notUsed` → 1, sonst Schätzwert, sonst Team-Default.
pub fn issue_points(i: &IssueProgressInput) -> f64 {
    if i.estimation_type == EstimationType::NotUsed {
        return 1.0;
    }
    i.estimate.unwrap_or(i.default_estimate)
}

/// Scope, erledigt, begonnen und Fortschritt (docs/01 §6.3).
pub fn stats(issues: &[IssueProgressInput]) -> Stats {
    let mut s = Stats::default();
    for i in issues {
        if matches!(i.state_type, StateType::Canceled | StateType::Duplicate) {
            continue;
        }
        let p = issue_points(i);
        s.scope += p;
        match i.state_type {
            StateType::Completed => s.completed += p,
            StateType::Started => s.started += p,
            _ => {}
        }
    }
    s.progress = if s.scope == 0.0 {
        0.0
    } else {
        (s.completed + 0.25 * s.started) / s.scope
    };
    s
}

/// Fortschritt 0..=1 (docs/01 §6.3): erledigt + 0,25 × begonnen, geteilt durch den Scope.
pub fn progress(issues: &[IssueProgressInput]) -> f64 {
    stats(issues).progress
}

#[cfg(test)]
mod tests {
    use super::*;
    use StateType::*;

    fn inp(
        state_type: StateType,
        estimate: Option<f64>,
        ty: EstimationType,
        def: f64,
    ) -> IssueProgressInput {
        IssueProgressInput {
            state_type,
            estimate,
            estimation_type: ty,
            default_estimate: def,
        }
    }
    fn plain(s: StateType) -> IssueProgressInput {
        inp(s, None, EstimationType::NotUsed, 1.0)
    }

    #[test]
    fn example_1_cycle_without_estimates() {
        let mut v = vec![plain(Completed); 5];
        v.extend(vec![plain(Started); 4]);
        v.push(plain(Unstarted));
        let s = stats(&v);
        assert_eq!((s.scope, s.completed, s.started), (10.0, 5.0, 4.0));
        assert!((s.progress - 0.60).abs() < 1e-12);
        assert!((progress(&v) - 0.60).abs() < 1e-12);
    }

    #[test]
    fn example_2_mixed_team_scales() {
        let fib = EstimationType::Fibonacci;
        let v = [
            inp(Completed, Some(3.0), fib, 1.0),                    // ENG-120
            inp(Started, Some(5.0), fib, 1.0),                      // ENG-121
            inp(Unstarted, None, fib, 1.0),                         // ENG-122 -> Default 1
            inp(Completed, None, EstimationType::NotUsed, 1.0),     // DES-14 -> 1
            inp(Canceled, Some(8.0), EstimationType::NotUsed, 1.0), // DES-15 ausgeschlossen
        ];
        let s = stats(&v);
        assert_eq!((s.scope, s.completed, s.started), (10.0, 4.0, 5.0));
        assert!((s.progress - 0.525).abs() < 1e-12);
    }

    #[test]
    fn empty_and_all_excluded_is_zero() {
        assert_eq!(progress(&[]), 0.0);
        assert_eq!(progress(&[plain(Canceled), plain(Duplicate)]), 0.0);
    }

    #[test]
    fn points_rules() {
        assert_eq!(
            issue_points(&inp(Started, Some(8.0), EstimationType::NotUsed, 3.0)),
            1.0
        );
        assert_eq!(
            issue_points(&inp(Started, Some(8.0), EstimationType::Linear, 3.0)),
            8.0
        );
        assert_eq!(
            issue_points(&inp(Started, None, EstimationType::Linear, 3.0)),
            3.0
        );
        // Explizites 0 (allowZero) ist nicht "ungeschätzt".
        assert_eq!(
            issue_points(&inp(Started, Some(0.0), EstimationType::Linear, 3.0)),
            0.0
        );
    }

    #[test]
    fn all_completed_is_one_and_triage_counts_in_scope() {
        assert_eq!(progress(&[plain(Completed), plain(Completed)]), 1.0);
        assert_eq!(
            progress(&[plain(Completed), plain(Triage), plain(Backlog)]),
            1.0 / 3.0
        );
    }

    #[test]
    fn zero_point_scope_does_not_divide_by_zero() {
        let z = inp(Completed, Some(0.0), EstimationType::Linear, 1.0);
        assert_eq!(progress(&[z]), 0.0);
    }
}
