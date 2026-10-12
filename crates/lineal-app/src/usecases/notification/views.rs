//! View-Abos (docs/05 §4.7): Schnittstelle zwischen dem Notification-Fan-out (WP32) und dem
//! Datenmodell der View-Abonnements (WP31).
//!
//! Der Fan-out kennt keine View-Filter. Er fragt für jedes Issue-Ereignis eine
//! [`ViewSubscriptionSource`], welche Abonnenten abonnierter Views das Ereignis betrifft, und
//! liefert die Notifications selbst (Self-Suppression, Sichtbarkeit je Empfänger, Bündelung,
//! Idempotenz). Die Quelle entscheidet nur, **wer** welchen Typ bekommt:
//!
//! | Typ | Bedingung |
//! |---|---|
//! | `issueAddedToView` | das Issue erfüllt den View-Filter jetzt, vorher nicht (oder wurde neu angelegt) |
//! | `issueStatusChanged` | ein Issue der View wechselt nach `completed`/`canceled` |
//! | `issueStatusChangedAll` | jeder Statuswechsel eines Issues der View |
//!
//! Die Filterauswertung läuft mit den Rechten des Abonnenten. Als Vorher-Werte stehen
//! [`ViewIssueEvent::previous`] (Assignee, Statustyp) zur Verfügung.
//!
//! Implementierung: [`ViewSubscriptions`] (WP35), eingehängt in `lineal_jobs::registry`.
//!
//! # Regeln (WP35)
//!
//! * Je Ereignis werden die aktiven Abos des Workspace je benötigtem Typ einmal geladen
//!   (`ViewRepo::subscribed_views`). Je Abonnent wird die View mit **dessen** Rechten geladen
//!   (`get_view`; unsichtbar, archiviert oder keine Issue-View → übersprungen).
//! * Filterprüfung: eine gezielte Listenabfrage auf genau dieses Issue (`{ id: { eq } }` UND
//!   `combined_filter` inkl. Team-/Projekt-Scope) mit Sichtbarkeit, `isMe` und Zeitzone des
//!   Abonnenten. Ergebnis je (Abonnent, View) pro Ereignis gemerkt.
//! * `issueAddedToView`: bei neuen Issues, die den Filter erfüllen. Bei Änderungen ist das
//!   „vorher nicht“ nur teilweise rekonstruierbar (`previous` kennt nur Assignee und Statustyp).
//!   Konservative Regel: das Issue erfüllt den Filter jetzt UND mindestens ein geändertes Feld
//!   kommt im Filter oder Scope der View vor (Änderungen an nicht gefilterten Feldern können ein
//!   Issue nie in die View bringen) UND das Ereignis ist kein reiner Statuswechsel innerhalb
//!   desselben Statustyps. Ein Issue, das in der View blieb, obwohl sich ein gefiltertes Feld
//!   änderte (z. B. Priorität 1 → 2 bei „Priorität ≤ 2“), kann dadurch einmal zusätzlich
//!   gemeldet werden; die 1-Minuten-Bündelung des Fan-outs begrenzt das.
//! * `issueStatusChanged`: Statuswechsel nach `completed`/`canceled` (Vorher-Statustyp bekannt:
//!   nur bei echtem Typwechsel), `issueStatusChangedAll`: jeder Statuswechsel. Das Issue muss die
//!   View nach der Änderung erfüllen; Views, die abgeschlossene Issues ausschließen, melden das
//!   Schließen daher nicht.

use crate::context::{Actor, RequestCtx};
use crate::error::AppResult;
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::issue::IssueListFilter;
use crate::ports::repos::notification::IssueFacts;
use crate::ports::repos::user::PageRequest;
use crate::ports::repos::view::{FilterEval, ViewModel};
use crate::usecases::view::{build_eval, combined_filter, view_access};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::events::IssuePrevious;
use lineal_domain::ids::{OrgId, UserId, ViewId};
use serde_json::{Value, json};
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};

/// Ein Issue-Ereignis aus Sicht der View-Abos.
#[derive(Debug, Clone, Copy)]
pub struct ViewIssueEvent<'a> {
    /// Auswertungszeitpunkt des Fan-outs (relative Datumsfilter).
    pub now: DateTime<Utc>,
    /// Zustand des Issues nach dem Ereignis.
    pub issue: &'a IssueFacts,
    /// Das Issue wurde gerade angelegt.
    pub is_new: bool,
    /// Geänderte Felder (camelCase, `IssueUpdated.changed`); bei neuen Issues leer.
    pub changed: &'a [String],
    /// Alte Werte (Assignee, Statustyp), soweit erfasst.
    pub previous: Option<&'a IssuePrevious>,
}

/// Liefert die Empfänger von View-Benachrichtigungen zu einem Issue-Ereignis.
#[async_trait]
pub trait ViewSubscriptionSource: Send + Sync {
    /// Paare aus Abonnent und Notification-Typ (`issueAddedToView`, `issueStatusChanged`,
    /// `issueStatusChangedAll`). Mehrfache Nennungen sind erlaubt; es gewinnt der spezifischste
    /// Typ je Empfänger. Der Akteur und nicht sichtbare Empfänger werden vom Fan-out entfernt.
    async fn recipients(
        &self,
        tx: &mut dyn Tx,
        event: &ViewIssueEvent<'_>,
    ) -> AppResult<Vec<(UserId, &'static str)>>;
}

const ADDED: &str = "issueAddedToView";
const STATUS: &str = "issueStatusChanged";
const STATUS_ALL: &str = "issueStatusChangedAll";

/// Datenbankgestützte [`ViewSubscriptionSource`] auf Basis von `notification_subscription`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewSubscriptions;

/// Rechte und Auswertungskontext eines Abonnenten (je Ereignis einmal aufgebaut).
struct Viewer {
    ctx: RequestCtx,
    snap: PolicySnapshot,
    eval: FilterEval,
}

/// Alle Schlüssel eines Filter-JSON (Feld- und Operatornamen, beliebig tief).
fn collect_keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.insert(k.clone());
                collect_keys(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_keys(x, out)),
        _ => {}
    }
}

/// Kommt ein geändertes Feld (`stateId`, `labelIds` …) im Filter vor (`state`, `labels` …)?
fn filter_uses_changed(filter: &Value, changed: &[String]) -> bool {
    let mut keys = BTreeSet::new();
    collect_keys(filter, &mut keys);
    changed.iter().any(|c| {
        keys.contains(c.as_str())
            || c.strip_suffix("Ids")
                .is_some_and(|b| keys.contains(b) || keys.contains(format!("{b}s").as_str()))
            || c.strip_suffix("Id").is_some_and(|b| keys.contains(b))
    })
}

#[async_trait]
impl ViewSubscriptionSource for ViewSubscriptions {
    async fn recipients(
        &self,
        tx: &mut dyn Tx,
        event: &ViewIssueEvent<'_>,
    ) -> AppResult<Vec<(UserId, &'static str)>> {
        let f = event.issue;
        let state_changed = !event.is_new && event.changed.iter().any(|c| c == "stateId");
        let prev_type = event.previous.and_then(|p| p.state_type.as_deref());
        let type_changed = prev_type.is_none_or(|p| p != f.state_type);
        let closed_now = matches!(f.state_type.as_str(), "completed" | "canceled");
        let mut wanted: Vec<&'static str> = Vec::new();
        if event.is_new || !event.changed.is_empty() {
            wanted.push(ADDED);
        }
        if state_changed && type_changed {
            wanted.push(STATUS_ALL);
            if closed_now {
                wanted.push(STATUS);
            }
        }
        let mut subs = Vec::new();
        for ntype in &wanted {
            for s in tx.views().subscribed_views(ntype).await? {
                if s.active {
                    subs.push((s, *ntype));
                }
            }
        }
        if subs.is_empty() {
            return Ok(Vec::new());
        }
        let now = event.now;
        let Some(org) = tx.orgs().get().await?.map(|o| o.id) else {
            return Ok(Vec::new());
        };
        let mut viewers: HashMap<UserId, Option<Viewer>> = HashMap::new();
        // (Abonnent, View) -> geladene View (None: unsichtbar/ungeeignet), Treffer lazy.
        let mut views: HashMap<(UserId, ViewId), Option<Loaded>> = HashMap::new();
        let only_same_type_status =
            state_changed && !type_changed && matches!(event.changed, [c] if c == "stateId");
        let mut out = Vec::new();
        for (sub, ntype) in subs {
            let user = sub.subscriber_id;
            if let Entry::Vacant(slot) = viewers.entry(user) {
                slot.insert(load_viewer(tx, org, user, now).await?);
            }
            let Some(Some(viewer)) = viewers.get(&user) else {
                continue;
            };
            let key = (user, sub.view_id);
            if let Entry::Vacant(slot) = views.entry(key) {
                slot.insert(load_view(tx, viewer, sub.view_id, f).await?);
            }
            let Some(Some(loaded)) = views.get_mut(&key) else {
                continue;
            };
            // Neu in der View kann ein Issue nur durch ein im Filter/Scope verwendetes Feld
            // gekommen sein; ein reiner Statuswechsel innerhalb desselben Typs ändert nichts.
            if ntype == ADDED
                && !event.is_new
                && (only_same_type_status || !filter_uses_changed(&loaded.filter, event.changed))
            {
                continue;
            }
            let hit = match loaded.hit {
                Some(h) => h,
                None => {
                    let h = matches_issue(tx, viewer, &loaded.filter, f).await?;
                    loaded.hit = Some(h);
                    h
                }
            };
            if hit {
                out.push((user, ntype));
            }
        }
        Ok(out)
    }
}

/// Eine für den Abonnenten sichtbare Issue-View.
struct Loaded {
    /// `combined_filter` (View-Filter und Scope) ohne Issue-Einschränkung.
    filter: Value,
    /// Ergebnis der Einzelabfrage, sobald nötig.
    hit: Option<bool>,
}

/// Rechte des Abonnenten; `None`, wenn er inaktiv oder unbekannt ist.
async fn load_viewer(
    tx: &mut dyn Tx,
    org: OrgId,
    user: UserId,
    now: DateTime<Utc>,
) -> AppResult<Option<Viewer>> {
    let Some(u) = tx.notifications().fanout_users(&[user]).await?.pop() else {
        return Ok(None);
    };
    if !u.active {
        return Ok(None);
    }
    let mut ctx = RequestCtx::system(org, "view-subscriptions");
    ctx.actor = Actor::User {
        id: user,
        role: u.role,
    };
    let snap = PolicySnapshot::load(tx, &ctx).await?;
    let eval = build_eval(tx, &snap, &ctx, now, false, true).await?;
    Ok(Some(Viewer { ctx, snap, eval }))
}

/// Lädt die View mit den Rechten des Abonnenten. `None`: View für ihn unsichtbar, archiviert,
/// keine Issue-View oder das Issue liegt in keinem für ihn sichtbaren Team.
async fn load_view(
    tx: &mut dyn Tx,
    viewer: &Viewer,
    view: ViewId,
    issue: &IssueFacts,
) -> AppResult<Option<Loaded>> {
    let access = view_access(&viewer.snap, &viewer.ctx)?;
    let Some(v) = tx.views().get_view(&access, view).await? else {
        return Ok(None);
    };
    if v.archived_at.is_some() || v.model != ViewModel::Issue {
        return Ok(None);
    }
    if !viewer.snap.visible_team_ids().contains(&issue.team_id) {
        return Ok(None);
    }
    Ok(Some(Loaded {
        filter: combined_filter(&v, Vec::new()),
        hit: None,
    }))
}

/// Erfüllt genau dieses Issue den Filter (Einzelabfrage mit `id`-Filter, Rechte des Abonnenten)?
async fn matches_issue(
    tx: &mut dyn Tx,
    viewer: &Viewer,
    filter: &Value,
    issue: &IssueFacts,
) -> AppResult<bool> {
    let only = json!({ "and": [filter, { "id": { "eq": issue.id.to_string() } }] });
    let Ok(ast) = lineal_domain::rules::filter::from_json_value(&only) else {
        return Ok(false);
    };
    let f = IssueListFilter {
        team_ids: viewer.snap.visible_team_ids().into_iter().collect(),
        ast: Some(Box::new(ast)),
        eval: Some(viewer.eval.clone()),
        ..Default::default()
    };
    let page = PageRequest {
        limit: 1,
        ..Default::default()
    };
    Ok(!tx.issues().list(&f, &page).await?.items.is_empty())
}
