//! History mit Zusammenfassen (docs/03 §8.2).
//!
//! `issue_history` ist append-only (Trigger verbietet UPDATE). „Fortschreiben“ des letzten
//! Eintrags heißt deshalb: verrechnen und den alten Eintrag durch einen neuen ersetzen
//! (`created_at` rückt dabei auf „jetzt“, das entspricht dem `updatedAt` des Originals).

use crate::error::AppResult;
use crate::ports::Tx;
use crate::ports::repos::issue::{HistoryEntry, HistorySource, Issue};
use chrono::{DateTime, Duration, Utc};
use lineal_domain::ids::{LabelId, UserId};
use serde_json::{Map, Value, json};
use uuid::Uuid;

/// Fenster für das Zusammenfassen desselben Akteurs.
pub const COALESCE_WINDOW_SECS: i64 = 60;
/// Erstellungsfenster des Creators.
pub const CREATION_WINDOW_SECS: i64 = 180;

/// Inhalt eines History-Eintrags.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Delta {
    /// `{ feld: { from, to } }`.
    pub changes: Map<String, Value>,
    /// `created`, `descriptionUpdated`, `trashed`, `restored`, `archived`, `unarchived`.
    pub flags: Vec<String>,
    pub added_labels: Vec<LabelId>,
    pub removed_labels: Vec<LabelId>,
    pub relation_changes: Vec<Value>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
            && self.flags.is_empty()
            && self.added_labels.is_empty()
            && self.removed_labels.is_empty()
            && self.relation_changes.is_empty()
    }

    /// Trägt eine Feldänderung ein (nur, wenn sich der Wert unterscheidet).
    pub fn set(&mut self, field: &str, from: Value, to: Value) {
        if from != to {
            self.changes
                .insert(field.to_owned(), json!({ "from": from, "to": to }));
        }
    }

    pub fn flag(&mut self, flag: &str) {
        if !self.flags.iter().any(|f| f == flag) {
            self.flags.push(flag.to_owned());
        }
    }

    pub fn from_entry(e: &HistoryEntry) -> Self {
        Self {
            changes: e.changes.as_object().cloned().unwrap_or_default(),
            flags: e.flags.clone(),
            added_labels: e.added_label_ids.clone().unwrap_or_default(),
            removed_labels: e.removed_label_ids.clone().unwrap_or_default(),
            relation_changes: e
                .relation_changes
                .as_ref()
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

fn opposite(flag: &str) -> Option<&'static str> {
    match flag {
        "trashed" => Some("restored"),
        "restored" => Some("trashed"),
        "archived" => Some("unarchived"),
        "unarchived" => Some("archived"),
        _ => None,
    }
}

fn rel_key(r: &Value) -> (String, String) {
    (
        r.get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        r.get("identifier")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    )
}

/// Verrechnet `new` in `old`: je Feld bleibt das ursprüngliche `from`, `to` wird überschrieben;
/// ist danach `from == to`, entfällt das Feld. Label-Mengen werden verrechnet.
pub fn merge(old: &Delta, new: &Delta) -> Delta {
    let mut out = old.clone();
    for (k, v) in &new.changes {
        let to = v.get("to").cloned().unwrap_or(Value::Null);
        match out.changes.get(k) {
            Some(prev) => {
                let from = prev.get("from").cloned().unwrap_or(Value::Null);
                if from == to {
                    out.changes.remove(k);
                } else {
                    out.changes
                        .insert(k.clone(), json!({ "from": from, "to": to }));
                }
            }
            None => {
                out.changes.insert(k.clone(), v.clone());
            }
        }
    }
    for f in &new.flags {
        match opposite(f) {
            Some(o) if out.flags.iter().any(|x| x == o) => out.flags.retain(|x| x != o),
            _ => {
                if !out.flags.contains(f) {
                    out.flags.push(f.clone());
                }
            }
        }
    }
    // Labels: netto.
    let mut added: Vec<LabelId> = Vec::new();
    let mut removed: Vec<LabelId> = Vec::new();
    for l in old.added_labels.iter().chain(&new.added_labels) {
        if !added.contains(l) {
            added.push(*l);
        }
    }
    for l in old.removed_labels.iter().chain(&new.removed_labels) {
        if !removed.contains(l) {
            removed.push(*l);
        }
    }
    // Hinzugefügt und wieder entfernt (oder umgekehrt) hebt sich auf.
    let both: Vec<LabelId> = added
        .iter()
        .copied()
        .filter(|l| removed.contains(l))
        .collect();
    added.retain(|l| !both.contains(l));
    removed.retain(|l| !both.contains(l));
    out.added_labels = added;
    out.removed_labels = removed;
    // Relationen: gegenläufige Aktion gleicher Relation hebt sich auf.
    for r in &new.relation_changes {
        let (t, i) = rel_key(r);
        let action = r.get("action").and_then(Value::as_str).unwrap_or("");
        let inverse = out.relation_changes.iter().position(|o| {
            rel_key(o) == (t.clone(), i.clone())
                && o.get("action").and_then(Value::as_str).unwrap_or("") != action
        });
        match inverse {
            Some(pos) => {
                out.relation_changes.remove(pos);
            }
            None => out.relation_changes.push(r.clone()),
        }
    }
    out
}

fn to_entry(
    id: Uuid,
    issue: &Issue,
    actor: Option<UserId>,
    source: HistorySource,
    d: &Delta,
    now: DateTime<Utc>,
) -> HistoryEntry {
    HistoryEntry {
        id,
        issue_id: issue.id,
        actor_id: actor,
        actor_name: None,
        source,
        changes: Value::Object(d.changes.clone()),
        added_label_ids: (!d.added_labels.is_empty()).then(|| d.added_labels.clone()),
        removed_label_ids: (!d.removed_labels.is_empty()).then(|| d.removed_labels.clone()),
        relation_changes: (!d.relation_changes.is_empty())
            .then(|| Value::Array(d.relation_changes.clone())),
        flags: d.flags.clone(),
        created_at: now,
    }
}

/// Schreibt (oder verrechnet) einen History-Eintrag; leere Deltas schreiben nichts.
pub async fn record(
    tx: &mut dyn Tx,
    new_id: Uuid,
    issue: &Issue,
    actor: Option<UserId>,
    source: HistorySource,
    delta: &Delta,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if delta.is_empty() {
        return Ok(());
    }
    // Erstellungsfenster: Änderungen des Creators gelten als Teil der Erstellung (Teamwechsel
    // bleiben sichtbar: die alte Kennung ist nur aus `teamId` plus Alias rekonstruierbar).
    if actor.is_some()
        && !delta.changes.contains_key("teamId")
        && actor == issue.creator_id
        && now - issue.created_at <= Duration::seconds(CREATION_WINDOW_SECS)
    {
        return Ok(());
    }
    if let Some(last) = tx.issues().last_history(issue.id).await?
        && actor.is_some()
        && last.actor_id == actor
        && last.source == source
        && !last.flags.iter().any(|f| f == "created")
        && now - last.created_at <= Duration::seconds(COALESCE_WINDOW_SECS)
    {
        let merged = merge(&Delta::from_entry(&last), delta);
        tx.issues().delete_history(last.id).await?;
        if merged.is_empty() {
            return Ok(());
        }
        let entry = to_entry(new_id, issue, actor, source, &merged, now);
        return tx.issues().insert_history(&entry).await;
    }
    let entry = to_entry(new_id, issue, actor, source, delta, now);
    tx.issues().insert_history(&entry).await
}

/// Der `created`-Eintrag.
pub async fn record_created(
    tx: &mut dyn Tx,
    id: Uuid,
    issue: &Issue,
    actor: Option<UserId>,
    source: HistorySource,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let mut d = Delta::default();
    d.flag("created");
    let entry = to_entry(id, issue, actor, source, &d, now);
    tx.issues().insert_history(&entry).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(field: &str, from: Value, to: Value) -> Delta {
        let mut x = Delta::default();
        x.set(field, from, to);
        x
    }

    #[test]
    fn keeps_original_from_and_latest_to() {
        let m = merge(
            &d("priority", json!(0), json!(2)),
            &d("priority", json!(2), json!(3)),
        );
        assert_eq!(m.changes["priority"], json!({ "from": 0, "to": 3 }));
    }

    #[test]
    fn field_changed_back_disappears() {
        let m = merge(
            &d("title", json!("A"), json!("B")),
            &d("title", json!("B"), json!("A")),
        );
        assert!(m.is_empty());
    }

    #[test]
    fn unrelated_fields_accumulate() {
        let m = merge(
            &d("title", json!("A"), json!("B")),
            &d("priority", json!(0), json!(1)),
        );
        assert_eq!(m.changes.len(), 2);
    }

    #[test]
    fn labels_net_out() {
        let (a, b, c) = (LabelId::new_v7(), LabelId::new_v7(), LabelId::new_v7());
        let old = Delta {
            added_labels: vec![a, b],
            removed_labels: vec![c],
            ..Default::default()
        };
        let new = Delta {
            added_labels: vec![c],
            removed_labels: vec![a],
            ..Default::default()
        };
        let m = merge(&old, &new);
        assert_eq!(m.added_labels, vec![b]);
        assert!(m.removed_labels.is_empty());
    }

    #[test]
    fn relation_added_then_removed_cancels() {
        let add = json!({ "type": "blocks", "identifier": "ENG-2", "action": "added" });
        let rem = json!({ "type": "blocks", "identifier": "ENG-2", "action": "removed" });
        let old = Delta {
            relation_changes: vec![add],
            ..Default::default()
        };
        let new = Delta {
            relation_changes: vec![rem],
            ..Default::default()
        };
        assert!(merge(&old, &new).is_empty());
    }

    #[test]
    fn trash_then_restore_cancels() {
        let mut a = Delta::default();
        a.flag("trashed");
        let mut b = Delta::default();
        b.flag("restored");
        assert!(merge(&a, &b).is_empty());
    }
}
