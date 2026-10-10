//! Statuswechsel und Lebenszyklus-Zeitstempel (docs/03 §1.6, docs/01 §6.7).

use crate::enums::{Priority, ProjectStatusType, StateType};
use crate::error::DomainError;
use crate::ids::UserId;
use crate::time::Timestamp;

/// Die vom Statuswechsel berührten Zeitstempel eines Issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IssueTimestamps {
    /// Eintritt in `triage`.
    pub started_triage_at: Option<Timestamp>,
    /// Verlassen von `triage`.
    pub triaged_at: Option<Timestamp>,
    /// Erster Start (bleibt bei `started`-Wechseln erhalten).
    pub started_at: Option<Timestamp>,
    /// Eintritt in `completed`.
    pub completed_at: Option<Timestamp>,
    /// Eintritt in `canceled` oder `duplicate`.
    pub canceled_at: Option<Timestamp>,
    /// Vom Auto-Close-Job gesetzt.
    pub auto_closed_at: Option<Timestamp>,
    /// Snooze (Triage) bis.
    pub snoozed_until_at: Option<Timestamp>,
    /// Snooze (Triage) gesetzt von.
    pub snoozed_by: Option<UserId>,
}

/// Herkunft eines Statuswechsels (docs/03 §1.6: `duplicate` nur über „Als Duplikat markieren“).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionSource {
    /// Normales Update (API, MCP, UI).
    Update,
    /// Die Aktion „Als Duplikat markieren“.
    MarkDuplicate,
}

/// Prüft die Einschränkungen eines Statuswechsels (docs/03 §1.6).
///
/// * `duplicate` nur über `MarkDuplicate` (sonst `invalid_request`),
/// * `requirePriorityToLeaveTriage`: Triage nur mit `priority != 0` verlassen
///   (Grund `priority_required`).
///
/// Gleiche Kategorie ist immer erlaubt (kein Wechsel).
pub fn check_transition(
    old: StateType,
    new: StateType,
    source: TransitionSource,
    require_priority_to_leave_triage: bool,
    priority: Priority,
) -> Result<(), DomainError> {
    if new == StateType::Duplicate
        && old != StateType::Duplicate
        && source != TransitionSource::MarkDuplicate
    {
        return Err(DomainError::validation(
            "state",
            "duplicate_requires_mark_duplicate",
        ));
    }
    if old == StateType::Triage
        && new != StateType::Triage
        && require_priority_to_leave_triage
        && priority == Priority::NoPriority
    {
        return Err(DomainError::validation("state", "priority_required"));
    }
    Ok(())
}

/// Wendet die Zeitstempelregeln eines Statuswechsels an (docs/03 §1.6, `applyStateChange`).
///
/// Die Zeitstempel beziehen sich auf **Kategorien**: Bleibt die Kategorie gleich
/// (z. B. „In Progress“ → „In Review“), kommt `timestamps` unverändert zurück.
pub fn apply_transition(
    old: StateType,
    new: StateType,
    now: Timestamp,
    timestamps: IssueTimestamps,
) -> IssueTimestamps {
    if old == new {
        return timestamps;
    }
    let mut t = timestamps;
    if old == StateType::Triage {
        t.triaged_at = Some(now);
        t.snoozed_until_at = None;
        t.snoozed_by = None;
    }
    if new == StateType::Triage {
        t.started_triage_at = Some(now);
        t.triaged_at = None;
    }
    if matches!(
        new,
        StateType::Triage | StateType::Backlog | StateType::Unstarted
    ) {
        t.started_at = None;
    }
    if new == StateType::Started && t.started_at.is_none() {
        t.started_at = Some(now);
    }
    if new == StateType::Completed {
        t.completed_at = Some(now);
        if t.started_at.is_none() {
            t.started_at = Some(now);
        }
    } else {
        t.completed_at = None;
    }
    if matches!(new, StateType::Canceled | StateType::Duplicate) {
        t.canceled_at = Some(now);
    } else {
        t.canceled_at = None;
        t.auto_closed_at = None;
    }
    t
}

/// Statuszeitstempel eines Projekts (docs/04 §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectTimestamps {
    /// Erster Start (bleibt erhalten).
    pub started_at: Option<Timestamp>,
    /// Abschluss.
    pub completed_at: Option<Timestamp>,
    /// Abbruch.
    pub canceled_at: Option<Timestamp>,
}

/// Zeitstempelregeln beim Wechsel des Projektstatus (docs/04 §2.4, docs/01 §6.7).
pub fn apply_project_transition(
    old: ProjectStatusType,
    new: ProjectStatusType,
    now: Timestamp,
    timestamps: ProjectTimestamps,
) -> ProjectTimestamps {
    if old == new {
        return timestamps;
    }
    let mut t = timestamps;
    match new {
        ProjectStatusType::Started => {
            if t.started_at.is_none() {
                t.started_at = Some(now);
            }
            t.completed_at = None;
            t.canceled_at = None;
        }
        ProjectStatusType::Completed => {
            t.completed_at = Some(now);
            t.canceled_at = None;
        }
        ProjectStatusType::Canceled => {
            t.canceled_at = Some(now);
            t.completed_at = None;
        }
        ProjectStatusType::Backlog | ProjectStatusType::Planned | ProjectStatusType::Paused => {
            t.completed_at = None;
            t.canceled_at = None;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use StateType::*;
    use chrono::{Duration, TimeZone, Utc};

    fn t0() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
    }
    fn later(h: i64) -> Timestamp {
        t0() + Duration::hours(h)
    }

    #[test]
    fn same_category_changes_nothing() {
        let ts = IssueTimestamps {
            started_at: Some(t0()),
            ..Default::default()
        };
        for s in StateType::ALL {
            assert_eq!(apply_transition(*s, *s, later(1), ts), ts);
        }
    }

    #[test]
    fn first_start_is_kept() {
        let a = apply_transition(Unstarted, Started, t0(), IssueTimestamps::default());
        assert_eq!(a.started_at, Some(t0()));
        // Completed -> Started: startedAt bleibt, completedAt weg.
        let done = apply_transition(Started, Completed, later(2), a);
        assert_eq!(done.completed_at, Some(later(2)));
        assert_eq!(done.started_at, Some(t0()));
        let reopened = apply_transition(Completed, Started, later(5), done);
        assert_eq!(reopened.started_at, Some(t0()));
        assert_eq!(reopened.completed_at, None);
    }

    #[test]
    fn jump_to_completed_sets_started_at() {
        let r = apply_transition(Backlog, Completed, t0(), IssueTimestamps::default());
        assert_eq!(r.started_at, Some(t0()));
        assert_eq!(r.completed_at, Some(t0()));
    }

    #[test]
    fn back_to_open_non_started_clears_started_at() {
        let ts = IssueTimestamps {
            started_at: Some(t0()),
            ..Default::default()
        };
        for target in [Triage, Backlog, Unstarted] {
            assert_eq!(
                apply_transition(Started, target, later(1), ts).started_at,
                None
            );
        }
        // Canceled behält startedAt (Festlegung gilt nur für Triage/Backlog/Unstarted).
        assert_eq!(
            apply_transition(Started, Canceled, later(1), ts).started_at,
            Some(t0())
        );
    }

    #[test]
    fn cancel_and_duplicate_set_canceled_at_and_reopen_clears_it_and_auto_close() {
        for closed in [Canceled, Duplicate] {
            let c = apply_transition(Backlog, closed, t0(), IssueTimestamps::default());
            assert_eq!(c.canceled_at, Some(t0()));
            let c = IssueTimestamps {
                auto_closed_at: Some(t0()),
                ..c
            };
            let r = apply_transition(closed, Backlog, later(3), c);
            assert_eq!(r.canceled_at, None);
            assert_eq!(r.auto_closed_at, None);
        }
        // canceled -> duplicate: bleibt in "cancelled-Familie", Zeitstempel neu, auto-close bleibt.
        let c = apply_transition(Backlog, Canceled, t0(), IssueTimestamps::default());
        let d = apply_transition(Canceled, Duplicate, later(1), c);
        assert_eq!(d.canceled_at, Some(later(1)));
    }

    #[test]
    fn triage_entry_and_exit() {
        let entered = apply_transition(Backlog, Triage, t0(), IssueTimestamps::default());
        assert_eq!(entered.started_triage_at, Some(t0()));
        assert_eq!(entered.triaged_at, None);
        let snoozed = IssueTimestamps {
            snoozed_until_at: Some(later(48)),
            snoozed_by: Some(UserId::nil()),
            ..entered
        };
        let left = apply_transition(Triage, Backlog, later(1), snoozed);
        assert_eq!(left.triaged_at, Some(later(1)));
        assert_eq!(left.snoozed_until_at, None);
        assert_eq!(left.snoozed_by, None);
        assert_eq!(left.started_triage_at, Some(t0())); // bleibt als Historie
        // Wiedereintritt überschreibt und leert triagedAt.
        let again = apply_transition(Backlog, Triage, later(9), left);
        assert_eq!(again.started_triage_at, Some(later(9)));
        assert_eq!(again.triaged_at, None);
    }

    #[test]
    fn full_matrix_invariants() {
        // Für jede Kategorienkombination: Invarianten des Zielzustands.
        for old in StateType::ALL {
            for new in StateType::ALL {
                let r = apply_transition(*old, *new, t0(), IssueTimestamps::default());
                assert_eq!(r.completed_at.is_some(), *new == Completed && old != new);
                assert_eq!(
                    r.canceled_at.is_some(),
                    matches!(new, Canceled | Duplicate) && old != new
                );
                if *new == Completed && old != new {
                    assert!(r.started_at.is_some());
                }
            }
        }
    }

    #[test]
    fn duplicate_only_via_mark_duplicate() {
        let upd = |o, n| check_transition(o, n, TransitionSource::Update, false, Priority::Low);
        assert!(upd(Backlog, Duplicate).is_err());
        assert!(upd(Started, Duplicate).is_err());
        assert!(upd(Duplicate, Duplicate).is_ok());
        assert!(upd(Duplicate, Backlog).is_ok());
        assert!(
            check_transition(
                Backlog,
                Duplicate,
                TransitionSource::MarkDuplicate,
                false,
                Priority::Low
            )
            .is_ok()
        );
    }

    #[test]
    fn priority_required_to_leave_triage() {
        let chk = |n, p| check_transition(Triage, n, TransitionSource::Update, true, p);
        assert_eq!(
            chk(Backlog, Priority::NoPriority),
            Err(DomainError::validation("state", "priority_required"))
        );
        assert!(chk(Backlog, Priority::High).is_ok());
        assert!(chk(Triage, Priority::NoPriority).is_ok());
        // Ausschalter
        assert!(
            check_transition(
                Triage,
                Backlog,
                TransitionSource::Update,
                false,
                Priority::NoPriority
            )
            .is_ok()
        );
        // Nur beim Verlassen von Triage.
        assert!(
            check_transition(
                Backlog,
                Started,
                TransitionSource::Update,
                true,
                Priority::NoPriority
            )
            .is_ok()
        );
    }

    #[test]
    fn project_status_timestamps() {
        use ProjectStatusType as P;
        let s = apply_project_transition(P::Planned, P::Started, t0(), Default::default());
        assert_eq!(s.started_at, Some(t0()));
        let paused = apply_project_transition(P::Started, P::Paused, later(1), s);
        assert_eq!(paused.started_at, Some(t0()));
        let again = apply_project_transition(P::Paused, P::Started, later(2), paused);
        assert_eq!(again.started_at, Some(t0()));
        let done = apply_project_transition(P::Started, P::Completed, later(3), again);
        assert_eq!(done.completed_at, Some(later(3)));
        let canceled = apply_project_transition(P::Completed, P::Canceled, later(4), done);
        assert_eq!(canceled.completed_at, None);
        assert_eq!(canceled.canceled_at, Some(later(4)));
        let back = apply_project_transition(P::Canceled, P::Backlog, later(5), canceled);
        assert_eq!((back.completed_at, back.canceled_at), (None, None));
        assert_eq!(back.started_at, Some(t0()));
    }
}
