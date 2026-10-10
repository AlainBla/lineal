//! Label-Gruppen-Regeln (docs/03 §5.5).

use crate::enums::LabelGroupType;
use crate::error::DomainError;
use crate::ids::LabelId;
use std::collections::HashSet;

/// Eine Label-Gruppe mit ihren Kind-Labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelGroup {
    /// Gruppen-Label.
    pub id: LabelId,
    /// `singleSelect` erlaubt höchstens ein Kind pro Issue.
    pub group_type: LabelGroupType,
    /// Kinder der Gruppe.
    pub children: Vec<LabelId>,
}

/// Prüft die Gruppenregeln für die neue Labelmenge und liefert die bereinigte Menge
/// (Reihenfolge erhalten, Duplikate entfernt).
///
/// * `added = None` (ersetzendes `labels`): mehr als ein Kind einer `singleSelect`-Gruppe
///   → Fehler `single_select_violation`.
/// * `added = Some(neu)` (`addLabels`): ein neu hinzugefügtes Kind **ersetzt** die bereits
///   gesetzten Geschwister seiner `singleSelect`-Gruppe (UI-Verhalten). Werden zwei
///   Geschwister gleichzeitig hinzugefügt, bleibt es ein Fehler.
pub fn check_group_constraints(
    selected: &[LabelId],
    groups: &[LabelGroup],
    added: Option<&[LabelId]>,
) -> Result<Vec<LabelId>, DomainError> {
    let mut seen = HashSet::new();
    let mut result: Vec<LabelId> = selected
        .iter()
        .copied()
        .filter(|l| seen.insert(*l))
        .collect();

    for group in groups
        .iter()
        .filter(|g| g.group_type == LabelGroupType::SingleSelect)
    {
        let members: Vec<LabelId> = result
            .iter()
            .copied()
            .filter(|l| group.children.contains(l))
            .collect();
        if members.len() <= 1 {
            continue;
        }
        let violation = || {
            DomainError::validation(
                "labels",
                format!("single_select_violation (group {})", group.id),
            )
        };
        let Some(added) = added else {
            return Err(violation());
        };
        let newly: Vec<LabelId> = members
            .iter()
            .copied()
            .filter(|l| added.contains(l))
            .collect();
        match newly.as_slice() {
            [keep] => result.retain(|l| *l == *keep || !members.contains(l)),
            _ => return Err(violation()),
        }
    }
    Ok(result)
}

/// `addLabels`: vereinigt `current` mit `to_add` und wendet [`check_group_constraints`] mit
/// Ersetzungsverhalten an.
pub fn add_labels(
    current: &[LabelId],
    to_add: &[LabelId],
    groups: &[LabelGroup],
) -> Result<Vec<LabelId>, DomainError> {
    let mut all = current.to_vec();
    all.extend_from_slice(to_add);
    check_group_constraints(&all, groups, Some(to_add))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<LabelId> {
        (0..n).map(|_| LabelId::new_v7()).collect()
    }

    /// Gruppe `Platform` (singleSelect): iOS, Android; Gruppe `Area` (multiSelect): A1, A2.
    struct Fx {
        ios: LabelId,
        android: LabelId,
        a1: LabelId,
        a2: LabelId,
        bug: LabelId,
        groups: Vec<LabelGroup>,
    }
    fn fx() -> Fx {
        let l = ids(7);
        Fx {
            ios: l[0],
            android: l[1],
            a1: l[2],
            a2: l[3],
            bug: l[4],
            groups: vec![
                LabelGroup {
                    id: l[5],
                    group_type: LabelGroupType::SingleSelect,
                    children: vec![l[0], l[1]],
                },
                LabelGroup {
                    id: l[6],
                    group_type: LabelGroupType::MultiSelect,
                    children: vec![l[2], l[3]],
                },
            ],
        }
    }

    #[test]
    fn valid_sets_pass_unchanged_and_dedupe() {
        let f = fx();
        let sel = [f.ios, f.a1, f.a2, f.bug, f.ios];
        assert_eq!(
            check_group_constraints(&sel, &f.groups, None).unwrap(),
            vec![f.ios, f.a1, f.a2, f.bug]
        );
        assert_eq!(
            check_group_constraints(&[], &f.groups, None).unwrap(),
            vec![]
        );
    }

    #[test]
    fn replace_mode_rejects_two_children_of_single_select() {
        let f = fx();
        let err = check_group_constraints(&[f.ios, f.android], &f.groups, None).unwrap_err();
        assert!(
            matches!(err, DomainError::Validation { ref field, ref reason }
            if field == "labels" && reason.starts_with("single_select_violation"))
        );
    }

    #[test]
    fn multi_select_allows_many() {
        let f = fx();
        assert!(check_group_constraints(&[f.a1, f.a2], &f.groups, None).is_ok());
    }

    #[test]
    fn add_labels_replaces_sibling_in_single_select_group() {
        let f = fx();
        let r = add_labels(&[f.ios, f.bug, f.a1], &[f.android], &f.groups).unwrap();
        assert_eq!(r, vec![f.bug, f.a1, f.android]);
        // Gleiches Label erneut hinzufügen: unverändert.
        let r = add_labels(&[f.ios, f.bug], &[f.ios], &f.groups).unwrap();
        assert_eq!(r, vec![f.ios, f.bug]);
        // Multi-select ergänzt nur.
        let r = add_labels(&[f.a1], &[f.a2], &f.groups).unwrap();
        assert_eq!(r, vec![f.a1, f.a2]);
    }

    #[test]
    fn add_two_siblings_at_once_is_still_a_violation() {
        let f = fx();
        assert!(add_labels(&[], &[f.ios, f.android], &f.groups).is_err());
    }

    #[test]
    fn preexisting_violation_without_added_member_is_rejected() {
        // Bereits inkonsistente Daten (z. B. Altbestand) werden nicht still repariert.
        let f = fx();
        assert!(
            check_group_constraints(&[f.ios, f.android, f.bug], &f.groups, Some(&[f.bug])).is_err()
        );
    }
}
