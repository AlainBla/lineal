//! Notification-Typen: Kategorie, Rangfolge, Dringlichkeit und Textdarstellung (docs/05 §6.2, §6.9).

use crate::ports::repos::notification::NotificationView;

/// Die elf Kategorien (docs/01 §5.4).
pub const CATEGORIES: [&str; 11] = [
    "assignments",
    "mentions",
    "commentsAndReplies",
    "reactions",
    "statusChanges",
    "subscriptions",
    "triage",
    "reminders",
    "documentChanges",
    "postsAndUpdates",
    "system",
];

/// Kategorie eines Notification-Typs; unbekannte Typen gelten als `system`.
pub fn category_of(kind: &str) -> &'static str {
    match kind {
        "issueAssignedToYou"
        | "issueUnassignedFromYou"
        | "projectAddedAsLead"
        | "projectAddedAsMember"
        | "initiativeAddedAsOwner"
        | "documentAddedAsOwner"
        | "documentRemovedAsOwner" => "assignments",
        "issueStatusChangedAll"
        | "issueCreated"
        | "issueAddedToView"
        | "issueSubscribed"
        | "issueUnsubscribed"
        | "documentSubscribed"
        | "documentUnsubscribed" => "subscriptions",
        "issueStatusChanged"
        | "issueReopened"
        | "issuePriorityUrgent"
        | "issueBlocking"
        | "issueUnblocked" => "statusChanges",
        "issueDue" | "issueSlaHighRisk" | "issueSlaBreached" => "reminders",
        "issueAddedToTriage" | "triageResponsibilityIssueAddedToTriage" => "triage",
        "projectUpdateCreated" | "initiativeUpdateCreated" => "postsAndUpdates",
        "documentMoved" | "documentDeleted" | "documentRestored" => "documentChanges",
        k if k.ends_with("Mention") => "mentions",
        k if k.contains("NewComment") || k.ends_with("ThreadResolved") => "commentsAndReplies",
        k if k.ends_with("Reaction") => "reactions",
        k if k.ends_with("Reminder") || k.ends_with("Prompt") => "reminders",
        k if k.ends_with("ContentChange") => "documentChanges",
        _ => "system",
    }
}

/// Rangfolge bei mehreren Gründen für denselben Empfänger (kleiner = spezifischer):
/// Mention vor Zuweisung vor Kommentar vor Statuswechsel vor Abo-Ereignis (docs/05 §6.1 Nr. 4).
pub fn rank_of(kind: &str) -> u8 {
    match category_of(kind) {
        "mentions" => 0,
        "assignments" => 1,
        "commentsAndReplies" | "reactions" => 2,
        "statusChanges" | "triage" | "reminders" => 3,
        "subscriptions" => 5,
        _ => 4,
    }
}

/// Typen, die immer als Priorität gelten (docs/05 §6.7).
pub fn is_always_priority(kind: &str) -> bool {
    matches!(
        kind,
        "issuePriorityUrgent" | "issueSlaBreached" | "issueSlaHighRisk"
    )
}

/// Dringlichkeit für die E-Mail-Verzögerung (docs/05 §6.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    High,
    Medium,
    Low,
}

/// Dringlichkeit eines Typs.
pub fn urgency_of(kind: &str) -> Urgency {
    if is_always_priority(kind) {
        return Urgency::High;
    }
    match category_of(kind) {
        "assignments" | "mentions" | "triage" => Urgency::Medium,
        _ => Urgency::Low,
    }
}

/// Standard für Kanäle ohne gespeicherte Einstellung (docs/05 §6.9).
pub fn channel_default(category: &str, channel: &str) -> bool {
    match channel {
        "email" => matches!(category, "assignments" | "mentions"),
        "desktop" | "mobile" => true,
        _ => false,
    }
}

/// Default-Typen eines Objekt-Abos (docs/05 §6.3).
pub fn default_subscription_types(
    kind: crate::ports::repos::notification::ObjectKind,
    context: Option<&str>,
) -> Vec<String> {
    use crate::ports::repos::notification::ObjectKind as K;
    let v: &[&str] = match (kind, context) {
        (K::Project, _) => &["projectUpdateCreated"],
        (K::Initiative, _) => &["initiativeUpdateCreated"],
        (K::Team, Some("triage")) => &["issueAddedToTriage"],
        (K::Team, Some("activeIssues" | "backlog")) => &["issueCreated"],
        (K::Team, Some("activeCycle" | "upcomingCycle")) => &["issueAddedToView"],
        (K::Team, _) => &["issueSlaHighRisk", "issueSlaBreached"],
        (K::Cycle, _) => &["issueAddedToView"],
        (K::Label, _) => &["issueCreated"],
    };
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// Gerenderter Text einer Notification (Englisch, docs/07 `get_notifications`).
pub struct Rendered {
    pub title: String,
    pub subtitle: String,
    pub url: String,
}

/// Rendert Titel, Untertitel und Link. `base` ist `<baseUrl>/<urlKey>`.
pub fn render(v: &NotificationView, base: &str) -> Rendered {
    let n = &v.n;
    let who = v.actor_name.clone().unwrap_or_else(|| "Someone".to_owned());
    let ident = v.issue_identifier.clone().unwrap_or_default();
    let anchor = n
        .comment_id
        .map(|c| format!("#comment-{c}"))
        .unwrap_or_default();
    let (title, url) = if let (Some(i), Some(t)) = (&v.issue_identifier, &v.issue_title) {
        (t.clone(), format!("{base}/issue/{i}{anchor}"))
    } else if let (Some(id), Some(name)) = (n.project_id, &v.project_name) {
        (name.clone(), format!("{base}/project/{id}{anchor}"))
    } else if let (Some(id), Some(name)) = (n.initiative_id, &v.initiative_name) {
        (name.clone(), format!("{base}/initiative/{id}{anchor}"))
    } else if let (Some(id), Some(t)) = (n.document_id, &v.document_title) {
        (t.clone(), format!("{base}/document/{id}{anchor}"))
    } else {
        ("Notification".to_owned(), format!("{base}/inbox"))
    };
    let emoji = n.reaction_emoji.clone().unwrap_or_default();
    let subtitle = match n.kind.as_str() {
        "issueAssignedToYou" => format!("{who} assigned {ident} to you"),
        "issueUnassignedFromYou" => format!("{who} unassigned you from {ident}"),
        "issueMention" => format!("{who} mentioned you in {ident}"),
        "issueCommentMention" => format!("{who} mentioned you in a comment on {ident}"),
        "issueNewComment" => format!("{who} commented on {ident}"),
        "issueThreadResolved" => format!("{who} resolved a thread on {ident}"),
        "issueCommentReaction" => format!("{who} reacted {emoji} to your comment on {ident}"),
        "issueEmojiReaction" => format!("{who} reacted {emoji} to {ident}"),
        "issueStatusChanged" | "issueStatusChangedAll" => {
            format!("{who} changed the status of {ident}")
        }
        "issueReopened" => format!("{who} reopened {ident}"),
        "issuePriorityUrgent" => format!("{ident} was marked as urgent"),
        "issueBlocking" => format!("{ident} is blocked by another issue"),
        "issueUnblocked" => format!("{ident} is no longer blocked"),
        "issueDue" => format!("{ident} is due soon"),
        "issueAutoArchiveWarning" => {
            format!("{ident} will be archived automatically within a week")
        }
        "issueAutoArchived" => format!("{ident} was archived automatically"),
        "issueSlaHighRisk" => format!("{ident} is at risk of breaching its SLA"),
        "issueSlaBreached" => format!("{ident} breached its SLA"),
        "issueReminder" => format!("Reminder for {ident}"),
        "issueSubscribed" => format!("{who} subscribed you to {ident}"),
        "issueUnsubscribed" => format!("{who} unsubscribed you from {ident}"),
        "issueCreated" => format!("{who} created {ident}"),
        "issueAddedToTriage" => format!("{ident} was added to triage"),
        "projectUpdateCreated" => format!("{who} posted a project update"),
        "projectNewComment" | "projectCommentMention" => format!("{who} commented on a project"),
        other => format!("{who}: {other}"),
    };
    Rendered {
        title,
        subtitle,
        url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_follow_the_catalog() {
        assert_eq!(category_of("issueAssignedToYou"), "assignments");
        assert_eq!(category_of("issueCommentMention"), "mentions");
        assert_eq!(category_of("projectUpdateMention"), "mentions");
        assert_eq!(category_of("issueNewComment"), "commentsAndReplies");
        assert_eq!(category_of("issueCommentReaction"), "reactions");
        assert_eq!(category_of("issueStatusChanged"), "statusChanges");
        assert_eq!(category_of("issueStatusChangedAll"), "subscriptions");
        assert_eq!(category_of("issueDue"), "reminders");
        assert_eq!(category_of("projectUpdateCreated"), "postsAndUpdates");
        assert_eq!(category_of("documentContentChange"), "documentChanges");
        assert_eq!(category_of("somethingNew"), "system");
        for k in [
            "issueMention",
            "issueNewComment",
            "issueSubscribed",
            "issueDue",
        ] {
            assert!(CATEGORIES.contains(&category_of(k)));
        }
    }

    #[test]
    fn rank_prefers_mention_over_assignment_over_comment_over_status() {
        let order = [
            "issueCommentMention",
            "issueAssignedToYou",
            "issueNewComment",
            "issueStatusChanged",
            "issueCreated",
        ];
        let ranks: Vec<u8> = order.iter().map(|k| rank_of(k)).collect();
        assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{ranks:?}");
    }

    #[test]
    fn urgency_classes() {
        assert_eq!(urgency_of("issueSlaBreached"), Urgency::High);
        assert_eq!(urgency_of("issueCommentMention"), Urgency::Medium);
        assert_eq!(urgency_of("issueNewComment"), Urgency::Low);
    }
}
