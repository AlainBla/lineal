//! Domain-Events (docs/09 §6). Felder minimal: betroffene IDs. Serialisierbar für die Outbox.

use crate::ids::*;
use serde::{Deserialize, Serialize};

/// Herkunft eines Events (Spalte `origin`, siehe docs/13 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventOrigin {
    /// Vom Benutzer ausgelöst.
    User,
    /// Systemintern (Jobs, Automationen).
    System,
    /// Durch den Linear-Import.
    Import,
}

impl EventOrigin {
    /// Wire-/DB-Schreibweise.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
            Self::Import => "import",
        }
    }
}

impl std::str::FromStr for EventOrigin {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            "import" => Ok(Self::Import),
            other => Err(crate::DomainError::validation(
                "origin",
                format!("unknown value `{other}`"),
            )),
        }
    }
}

/// Alte Werte einer Issue-Änderung (`IssueUpdated.previous`), damit Konsumenten Übergänge erkennen
/// (`issueUnassignedFromYou`, `issueReopened`, `issueUnblocked`, docs/05 §6.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuePrevious {
    /// Bisheriger Assignee; gilt nur, wenn `assigneeId` in `changed` steht (`None` = war unzugewiesen).
    #[serde(default)]
    pub assignee_id: Option<UserId>,
    /// Bisheriger Statustyp (`triage`, `backlog`, `unstarted`, `started`, `completed`, `canceled`,
    /// `duplicate`); gilt nur, wenn `stateId` in `changed` steht.
    #[serde(default)]
    pub state_type: Option<String>,
    /// Neue Werte; der Notification-Handler wertet den Ereignis-Zustand aus, nicht den aktuellen
    /// (B6.09). `None` bei Altdaten. Geboxt, damit `DomainEvent` klein bleibt (Debug-Stack).
    #[serde(default)]
    pub new: Option<Box<IssueNewValues>>,
}

/// Neue Werte der in `changed` genannten Felder (siehe [`IssuePrevious::new`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueNewValues {
    /// Neuer Assignee; gilt nur, wenn `assigneeId` in `changed` steht (`None` = jetzt unzugewiesen).
    #[serde(default)]
    pub assignee_id: Option<UserId>,
    /// Neuer Statustyp; gilt nur, wenn `stateId` in `changed` steht.
    #[serde(default)]
    pub state_type: Option<String>,
    /// Neue Priorität; gilt nur, wenn `priority` in `changed` steht.
    #[serde(default)]
    pub priority: Option<i32>,
    /// Neu gesetzte Labels (nur wenn `labelIds` in `changed` steht).
    #[serde(default)]
    pub labels_added: Vec<LabelId>,
}

impl IssuePrevious {
    /// Alte und neue Werte für die in `changed` genannten Felder; `None`, wenn weder `assigneeId`,
    /// `stateId`, `priority` noch `labelIds` betroffen ist.
    pub fn of(changed: &[String], old: (Option<UserId>, &str), new: IssueNow) -> Option<Self> {
        let has = |f: &str| changed.iter().any(|c| c == f);
        (has("assigneeId") || has("stateId") || has("priority") || has("labelIds")).then(|| Self {
            assignee_id: has("assigneeId").then_some(old.0).flatten(),
            state_type: has("stateId").then(|| old.1.to_owned()),
            new: Some(Box::new(IssueNewValues {
                assignee_id: has("assigneeId").then_some(new.assignee_id).flatten(),
                state_type: has("stateId").then(|| new.state_type.to_owned()),
                priority: has("priority").then_some(new.priority),
                labels_added: if has("labelIds") {
                    new.labels_added
                } else {
                    Vec::new()
                },
            })),
        })
    }
}

/// Neue Werte einer Issue-Änderung für [`IssuePrevious::of`].
pub struct IssueNow<'a> {
    pub assignee_id: Option<UserId>,
    pub state_type: &'a str,
    pub priority: i32,
    pub labels_added: Vec<LabelId>,
}

/// Zustand des Issues bei der Anlage (`IssueCreated.initial`): Der Handler darf nicht den aktuellen
/// Zustand lesen, wenn das Issue direkt danach geändert wurde (B6.09).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueInitial {
    #[serde(default)]
    pub assignee_id: Option<UserId>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub state_type: String,
    #[serde(default)]
    pub label_ids: Vec<LabelId>,
}

macro_rules! domain_events {
    ($($(#[$doc:meta])* $var:ident { $($(#[$fm:meta])* $f:ident : $t:ty),* $(,)? }),+ $(,)?) => {
        /// Alle Ereignisse, die Use-Cases in die Outbox schreiben.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "type")]
        pub enum DomainEvent {
            $($(#[$doc])* $var { $($(#[$fm])* $f: $t),* }),+
        }

        impl DomainEvent {
            /// Name der Variante (`type` im Event-Format).
            pub const fn type_name(&self) -> &'static str {
                match self { $(Self::$var { .. } => stringify!($var)),+ }
            }
            /// Namen aller Varianten.
            pub const TYPE_NAMES: &'static [&'static str] = &[$(stringify!($var)),+];
        }
    };
}

domain_events! {
    /// `mentioned_user_ids`: @-Erwähnungen in der Beschreibung (für Notifications, WP08).
    /// `initial`: Zustand bei der Anlage (Assignee, Priorität, Statustyp, Labels); `None` bei Altdaten.
    IssueCreated { issue_id: IssueId, team_id: TeamId, mentioned_user_ids: Vec<UserId>, #[serde(default)] initial: Option<Box<IssueInitial>> },
    /// `changed`: geänderte Feldnamen (camelCase, z. B. `assigneeId`, `stateId`, `description`);
    /// `mentioned_user_ids`: neu hinzugekommene @-Erwähnungen in der Beschreibung.
    /// `previous`: alte Werte der Felder `assigneeId`/`stateId` (WP32, nur wenn sie in `changed` stehen;
    /// `None` = nicht erfasst, z. B. bei Altdaten oder Systemänderungen ohne Bedeutung für Notifications).
    IssueUpdated { previous: Option<IssuePrevious>, issue_id: IssueId, team_id: TeamId, changed: Vec<String>, mentioned_user_ids: Vec<UserId> },
    IssueRemoved { issue_id: IssueId, team_id: TeamId },
    IssueRelationCreated { relation_id: IssueRelationId, issue_id: IssueId },
    IssueRelationRemoved { relation_id: IssueRelationId, issue_id: IssueId },
    /// `mentioned_user_ids`: @-Erwähnungen im Kommentartext (für Notifications, WP08).
    CommentCreated { comment_id: CommentId, mentioned_user_ids: Vec<UserId> },
    /// `mentioned_user_ids`: neu hinzugekommene @-Erwähnungen.
    CommentUpdated { comment_id: CommentId, mentioned_user_ids: Vec<UserId> },
    CommentRemoved { comment_id: CommentId },
    IssueLabelCreated { label_id: LabelId },
    IssueLabelUpdated { label_id: LabelId },
    IssueLabelRemoved { label_id: LabelId },
    ReactionCreated { reaction_id: ReactionId },
    ReactionRemoved { reaction_id: ReactionId },
    AttachmentCreated { attachment_id: AttachmentId },
    AttachmentUpdated { attachment_id: AttachmentId },
    AttachmentRemoved { attachment_id: AttachmentId },
    ProjectCreated { project_id: ProjectId },
    /// `changed`: geänderte Feldnamen (camelCase, z. B. `leadId`); `previous_lead_id`: bisheriger Lead,
    /// gilt nur, wenn `leadId` in `changed` steht (B6.02). Altdaten: leer bzw. `None`.
    ProjectUpdated { project_id: ProjectId, #[serde(default)] changed: Vec<String>, #[serde(default)] previous_lead_id: Option<UserId> },
    ProjectRemoved { project_id: ProjectId },
    ProjectMilestoneCreated { milestone_id: MilestoneId, project_id: ProjectId },
    ProjectMilestoneUpdated { milestone_id: MilestoneId, project_id: ProjectId },
    ProjectMilestoneRemoved { milestone_id: MilestoneId, project_id: ProjectId },
    ProjectUpdateCreated { update_id: ProjectUpdateId, project_id: ProjectId },
    ProjectUpdateUpdated { update_id: ProjectUpdateId, project_id: ProjectId },
    ProjectUpdateRemoved { update_id: ProjectUpdateId, project_id: ProjectId },
    CycleCreated { cycle_id: CycleId, team_id: TeamId },
    CycleUpdated { cycle_id: CycleId, team_id: TeamId },
    CycleRemoved { cycle_id: CycleId, team_id: TeamId },
    DocumentCreated { document_id: DocumentId },
    DocumentUpdated { document_id: DocumentId },
    DocumentRemoved { document_id: DocumentId },
    InitiativeCreated { initiative_id: InitiativeId },
    /// `changed`: geänderte Feldnamen (camelCase, z. B. `ownerId`); `previous_owner_id`: bisheriger
    /// Owner, gilt nur, wenn `ownerId` in `changed` steht (B6.02). Altdaten: leer bzw. `None`.
    InitiativeUpdated { initiative_id: InitiativeId, #[serde(default)] changed: Vec<String>, #[serde(default)] previous_owner_id: Option<UserId> },
    InitiativeRemoved { initiative_id: InitiativeId },
    InitiativeUpdateCreated { update_id: InitiativeUpdateId, initiative_id: InitiativeId },
    InitiativeUpdateUpdated { update_id: InitiativeUpdateId, initiative_id: InitiativeId },
    InitiativeUpdateRemoved { update_id: InitiativeUpdateId, initiative_id: InitiativeId },
    UserCreated { user_id: UserId },
    UserUpdated { user_id: UserId },
    TeamCreated { team_id: TeamId },
    TeamUpdated { team_id: TeamId },
    TeamRemoved { team_id: TeamId },
    NotificationCreated { notification_id: NotificationId, user_id: UserId },
    IssueSlaSet { issue_id: IssueId },
    IssueSlaHighRisk { issue_id: IssueId },
    IssueSlaBreached { issue_id: IssueId },
    OAuthAuthorizationRevoked { client_id: String },
    // --- vom Orchestrator vor Welle 2 ergänzt ---
    WorkflowStateCreated { state_id: StateId, team_id: TeamId },
    WorkflowStateUpdated { state_id: StateId, team_id: TeamId },
    WorkflowStateRemoved { state_id: StateId, team_id: TeamId },
    TeamMembershipCreated { team_id: TeamId, user_id: UserId },
    TeamMembershipRemoved { team_id: TeamId, user_id: UserId },
    OrganizationUpdated {},
    ApiKeyCreated { user_id: UserId },
    ApiKeyRevoked { user_id: UserId },
    IssueSubscriptionCreated { issue_id: IssueId, user_id: UserId },
    IssueSubscriptionRemoved { issue_id: IssueId, user_id: UserId },
    TemplateCreated { template_id: TemplateId },
    TemplateUpdated { template_id: TemplateId },
    TemplateRemoved { template_id: TemplateId },
    CustomViewCreated { view_id: ViewId },
    CustomViewUpdated { view_id: ViewId },
    CustomViewRemoved { view_id: ViewId },
    /// Triage-Aktion (accept/decline/duplicate/snooze/move) auf einem Issue (WP12).
    IssueTriaged { issue_id: IssueId, team_id: TeamId, action: String },
    // --- Paket 6 (B4.03): Favoriten sind persönlich; nur an `user_id` zu synchronisieren ---
    FavoriteCreated { favorite_id: uuid::Uuid, user_id: UserId },
    FavoriteUpdated { favorite_id: uuid::Uuid, user_id: UserId },
    FavoriteRemoved { favorite_id: uuid::Uuid, user_id: UserId },
    // --- vom Orchestrator vor Welle 4 ergänzt (Sync des Inbox-Zustands, WP15) ---
    /// Gelesen/ungelesen/gesnoozt geändert; nur an `user_id` zu synchronisieren.
    NotificationUpdated { notification_id: NotificationId, user_id: UserId },
    /// Archiviert (einzeln oder per „archive all“); nur an `user_id` zu synchronisieren.
    NotificationArchived { notification_id: NotificationId, user_id: UserId },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hält `DomainEvent` klein: es liegt in vielen Futures, und Debug-Builds legen jeden
    /// Zwischenwert auf den Stack (Stack-Überlauf beim Issue-Anlegen in 2-MiB-Testthreads).
    #[test]
    fn domain_event_stays_small() {
        assert!(std::mem::size_of::<DomainEvent>() <= 136);
    }

    #[test]
    fn roundtrip_and_type_tag() {
        let ev = DomainEvent::IssueCreated {
            issue_id: IssueId::new_v7(),
            team_id: TeamId::new_v7(),
            mentioned_user_ids: vec![],
            initial: None,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "IssueCreated");
        assert_eq!(ev.type_name(), "IssueCreated");
        assert_eq!(serde_json::from_value::<DomainEvent>(v).unwrap(), ev);
    }

    #[test]
    fn covers_sla_and_oauth_events() {
        for n in [
            "IssueSlaSet",
            "IssueSlaHighRisk",
            "IssueSlaBreached",
            "OAuthAuthorizationRevoked",
        ] {
            assert!(DomainEvent::TYPE_NAMES.contains(&n));
        }
    }

    #[test]
    fn origin_parses() {
        assert_eq!(
            "import".parse::<EventOrigin>().unwrap(),
            EventOrigin::Import
        );
        assert!("x".parse::<EventOrigin>().is_err());
    }
}
