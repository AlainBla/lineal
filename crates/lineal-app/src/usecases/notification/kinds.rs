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

/// Alle bekannten Notification-Typen (docs/05 §6.2); nur sie sind als Abo-Typ zulässig.
pub const KNOWN_KINDS: &[&str] = &[
    "issueAssignedToYou",
    "issueUnassignedFromYou",
    "issueMention",
    "issueCommentMention",
    "issueNewComment",
    "issueThreadResolved",
    "issueCommentReaction",
    "issueEmojiReaction",
    "issueStatusChanged",
    "issueStatusChangedAll",
    "issueReopened",
    "issuePriorityUrgent",
    "issueBlocking",
    "issueUnblocked",
    "issueDue",
    "issueAutoArchiveWarning",
    "issueAutoArchived",
    "issueSlaHighRisk",
    "issueSlaBreached",
    "issueReminder",
    "projectReminder",
    "initiativeReminder",
    "documentReminder",
    "issueSubscribed",
    "issueShared",
    "invitationAccepted",
    "issueUnsubscribed",
    "issueCreated",
    "issueAddedToView",
    "issueAddedToTriage",
    "triageResponsibilityIssueAddedToTriage",
    "projectAddedAsLead",
    "projectAddedAsMember",
    "initiativeAddedAsOwner",
    "projectMention",
    "initiativeMention",
    "projectUpdateMention",
    "initiativeUpdateMention",
    "projectMilestoneMention",
    "projectUpdateCreated",
    "projectUpdatePrompt",
    "initiativeUpdatePrompt",
    "projectUpdateReaction",
    "initiativeUpdateReaction",
    "projectCommentReaction",
    "projectMilestoneCommentReaction",
    "projectUpdateCommentReaction",
    "initiativeCommentReaction",
    "initiativeUpdateCommentReaction",
    "documentCommentReaction",
    "projectNewComment",
    "projectCommentMention",
    "projectMilestoneNewComment",
    "projectMilestoneCommentMention",
    "projectUpdateNewComment",
    "projectUpdateCommentMention",
    "initiativeNewComment",
    "initiativeCommentMention",
    "initiativeUpdateNewComment",
    "initiativeUpdateCommentMention",
    "initiativeUpdateCreated",
    "documentNewComment",
    "documentCommentMention",
    "documentMention",
    "documentSubscribed",
    "documentUnsubscribed",
    "documentAddedAsOwner",
    "documentRemovedAsOwner",
    "documentMoved",
    "documentDeleted",
    "documentRestored",
    "documentContentChange",
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
        | "issueShared"
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

/// Dringlichkeit einer gespeicherten Notification (docs/05 §6.9): berücksichtigt alle
/// gebündelten Typen und die Priorität des Issues (`issueAssignedToYou` bei Urgent-Issues ist
/// hoch). Ohne `immediate_urgent` (Einstellung `emailImmediateUrgent`) gilt "hoch" nur als "mittel".
pub fn urgency_of_view(v: &NotificationView, immediate_urgent: bool) -> Urgency {
    let own = [v.n.kind.clone()];
    let kinds: &[String] = if v.n.bundled_types.is_empty() {
        &own
    } else {
        &v.n.bundled_types
    };
    let mut best = Urgency::Low;
    for k in kinds.iter().chain(std::iter::once(&v.n.kind)) {
        let u = if k == "issueAssignedToYou" && v.issue_priority == Some(1) {
            Urgency::High
        } else {
            urgency_of(k)
        };
        best = match (best, u) {
            (Urgency::High, _) | (_, Urgency::High) => Urgency::High,
            (Urgency::Medium, _) | (_, Urgency::Medium) => Urgency::Medium,
            _ => Urgency::Low,
        };
    }
    if best == Urgency::High && !immediate_urgent {
        Urgency::Medium
    } else {
        best
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
        (K::User, _) => &["issueAddedToView"],
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
    let title = if n.kind == "invitationAccepted" {
        "Invitation accepted".to_owned()
    } else {
        title
    };
    let emoji = n.reaction_emoji.clone().unwrap_or_default();
    let subject = if v.issue_identifier.is_some() {
        ident.clone()
    } else if n.project_id.is_some() {
        "a project".to_owned()
    } else if n.initiative_id.is_some() {
        "an initiative".to_owned()
    } else {
        "a document".to_owned()
    };
    let status_text = |who: &str| match v.issue_state_type.as_deref() {
        Some("completed") => format!("{ident} was marked as completed by {who}"),
        Some("canceled") => format!("{ident} was canceled by {who}"),
        Some("duplicate") => format!("{ident} was marked as a duplicate by {who}"),
        _ => match &v.issue_state_name {
            Some(state) => format!("{who} moved {ident} to {state}"),
            None => format!("{who} changed the status of {ident}"),
        },
    };
    let subtitle = match n.kind.as_str() {
        "issueAssignedToYou" => format!("{who} assigned {ident} to you"),
        "issueUnassignedFromYou" => format!("{who} unassigned you from {ident}"),
        "issueMention" => format!("{who} mentioned you in {ident}"),
        "issueCommentMention" => format!("{who} mentioned you in a comment on {ident}"),
        "issueNewComment" => format!("{who} commented on {ident}"),
        "issueThreadResolved" => format!("{who} resolved a thread on {ident}"),
        "issueCommentReaction" => format!("{who} reacted {emoji} to your comment on {ident}"),
        "issueEmojiReaction" => format!("{who} reacted {emoji} to {ident}"),
        "issueStatusChanged" => status_text(&who),
        "issueStatusChangedAll" => match &v.issue_state_name {
            Some(state) => format!("{who} moved {ident} to {state}"),
            None => format!("{who} changed the status of {ident}"),
        },
        "documentMention" => format!("{who} mentioned you in a document"),
        "issueReopened" => format!("{who} reopened {ident}"),
        "issuePriorityUrgent" => format!("{ident} was marked as urgent"),
        "issueBlocking" => format!("{ident} is blocked by another issue"),
        "issueUnblocked" => format!("{ident} is no longer blocked"),
        "issueDue" => match v.issue_due_date {
            Some(due) if due < n.created_at.date_naive() => format!("{ident} is overdue"),
            Some(_) => format!("{ident} is due tomorrow"),
            None => format!("{ident} is due soon"),
        },
        "issueAutoArchiveWarning" => {
            format!("{ident} will be archived automatically within a week")
        }
        "issueAutoArchived" => format!("{ident} was archived automatically"),
        "issueSlaHighRisk" => format!("{ident} is at risk of breaching its SLA"),
        "issueSlaBreached" => format!("{ident} breached its SLA"),
        "issueReminder" => format!("Reminder for {ident}"),
        "projectReminder" => "Reminder for a project".to_owned(),
        "initiativeReminder" => "Reminder for an initiative".to_owned(),
        "documentReminder" => "Reminder for a document".to_owned(),
        "issueSubscribed" => format!("{who} subscribed you to {ident}"),
        "issueShared" => format!("{who} shared {ident} with you"),
        "invitationAccepted" => format!("{who} accepted your invitation"),
        "issueUnsubscribed" => format!("{who} unsubscribed you from {ident}"),
        "issueCreated" => format!("{who} created {ident}"),
        "issueAddedToView" => format!("{ident} was added to a view you follow"),
        "issueAddedToTriage" | "triageResponsibilityIssueAddedToTriage" => {
            format!("{ident} was added to triage")
        }
        "projectAddedAsLead" => format!("{who} made you lead of this project"),
        "projectAddedAsMember" => format!("{who} added you to this project"),
        "initiativeAddedAsOwner" => format!("{who} made you owner of this initiative"),
        "projectMention" => format!("{who} mentioned you in a project"),
        "initiativeMention" => format!("{who} mentioned you in an initiative"),
        "projectUpdateMention" => format!("{who} mentioned you in a project update"),
        "initiativeUpdateMention" => format!("{who} mentioned you in an initiative update"),
        "projectMilestoneMention" => format!("{who} mentioned you in a project milestone"),
        "projectUpdateCreated" => format!("{who} posted a project update"),
        "projectUpdatePrompt" => "Time to post a project update".to_owned(),
        "initiativeUpdatePrompt" => "Time to post an initiative update".to_owned(),
        "projectUpdateReaction" => format!("{who} reacted {emoji} to your project update"),
        "initiativeUpdateReaction" => format!("{who} reacted {emoji} to your initiative update"),
        "projectCommentReaction" => format!("{who} reacted {emoji} to your comment on a project"),
        "projectMilestoneCommentReaction" => {
            format!("{who} reacted {emoji} to your comment on a project milestone")
        }
        "projectUpdateCommentReaction" => {
            format!("{who} reacted {emoji} to your comment on a project update")
        }
        "initiativeCommentReaction" => {
            format!("{who} reacted {emoji} to your comment on an initiative")
        }
        "initiativeUpdateCommentReaction" => {
            format!("{who} reacted {emoji} to your comment on an initiative update")
        }
        "documentCommentReaction" => format!("{who} reacted {emoji} to your comment on a document"),
        "projectNewComment" => format!("{who} commented on a project"),
        "projectCommentMention" => format!("{who} mentioned you in a project comment"),
        "projectMilestoneNewComment" => format!("{who} commented on a project milestone"),
        "projectMilestoneCommentMention" => {
            format!("{who} mentioned you in a project milestone comment")
        }
        "projectUpdateNewComment" => format!("{who} commented on a project update"),
        "projectUpdateCommentMention" => {
            format!("{who} mentioned you in a comment on a project update")
        }
        "initiativeNewComment" => format!("{who} commented on an initiative"),
        "initiativeCommentMention" => format!("{who} mentioned you in an initiative comment"),
        "initiativeUpdateNewComment" => format!("{who} commented on an initiative update"),
        "initiativeUpdateCommentMention" => {
            format!("{who} mentioned you in a comment on an initiative update")
        }
        "initiativeUpdateCreated" => format!("{who} posted an initiative update"),
        "documentNewComment" => format!("{who} commented on a document"),
        "documentCommentMention" => format!("{who} mentioned you in a document comment"),
        // Unbekannter Typ: kein roher Schlüssel im Text.
        _ => format!("New activity on {subject}"),
    };
    // Gebündelte Ereignisse (docs/05 §6.6): der spezifischste Typ bleibt, der Rest wird gezählt.
    let subtitle = match n.bundled_types.len() {
        0 | 1 => subtitle,
        more => format!("{subtitle} (+{} more)", more - 1),
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

    fn view(kind: &str) -> NotificationView {
        use crate::ports::repos::notification::Notification;
        let now = chrono::Utc::now();
        NotificationView {
            n: Notification {
                id: lineal_domain::ids::NotificationId::new_v7(),
                user_id: lineal_domain::ids::UserId::new_v7(),
                kind: kind.to_owned(),
                category: category_of(kind).to_owned(),
                target_type: "issue".into(),
                actor_id: None,
                issue_id: None,
                team_id: None,
                project_id: None,
                project_update_id: None,
                project_milestone_id: None,
                initiative_id: None,
                initiative_update_id: None,
                document_id: None,
                comment_id: None,
                parent_comment_id: None,
                reaction_emoji: None,
                read_at: None,
                snoozed_until_at: None,
                unsnoozed_at: None,
                emailed_at: None,
                is_priority: false,
                group_key: String::new(),
                bundled_types: vec![],
                created_at: now,
                updated_at: now,
                archived_at: None,
            },
            actor_name: Some("Alice".into()),
            issue_identifier: Some("ENG-1".into()),
            issue_team_id: None,
            issue_title: Some("T".into()),
            issue_priority: None,
            issue_due_date: None,
            issue_state_name: None,
            issue_state_type: None,
            project_name: None,
            initiative_name: None,
            document_title: None,
            recipient_email: "x@y.z".into(),
        }
    }

    #[test]
    fn texts_never_show_raw_type_keys() {
        for k in [
            "projectUpdatePrompt",
            "issueAddedToView",
            "projectCommentReaction",
            "triageResponsibilityIssueAddedToTriage",
            "somethingNew",
        ] {
            let r = render(&view(k), "https://x/acme");
            assert!(!r.subtitle.contains(k), "{}", r.subtitle);
            assert!(!r.subtitle.contains("Someone:"), "{}", r.subtitle);
        }
    }

    #[test]
    fn status_and_due_texts() {
        let mut v = view("issueStatusChanged");
        v.issue_state_type = Some("completed".into());
        assert_eq!(
            render(&v, "b").subtitle,
            "ENG-1 was marked as completed by Alice"
        );
        let mut d = view("issueDue");
        let day = d.n.created_at.date_naive();
        d.issue_due_date = Some(day - chrono::Duration::days(2));
        assert_eq!(render(&d, "b").subtitle, "ENG-1 is overdue");
        d.issue_due_date = Some(day + chrono::Duration::days(1));
        assert_eq!(render(&d, "b").subtitle, "ENG-1 is due tomorrow");
    }

    #[test]
    fn urgency_classes() {
        assert_eq!(urgency_of("issueSlaBreached"), Urgency::High);
        assert_eq!(urgency_of("issueCommentMention"), Urgency::Medium);
        assert_eq!(urgency_of("issueNewComment"), Urgency::Low);
    }
}
