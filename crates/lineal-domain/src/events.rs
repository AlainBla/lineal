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

macro_rules! domain_events {
    ($($(#[$doc:meta])* $var:ident { $($f:ident : $t:ty),* $(,)? }),+ $(,)?) => {
        /// Alle Ereignisse, die Use-Cases in die Outbox schreiben.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "type")]
        pub enum DomainEvent {
            $($(#[$doc])* $var { $($f: $t),* }),+
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
    IssueCreated { issue_id: IssueId, team_id: TeamId, mentioned_user_ids: Vec<UserId> },
    /// `changed`: geänderte Feldnamen (camelCase, z. B. `assigneeId`, `stateId`, `description`);
    /// `mentioned_user_ids`: neu hinzugekommene @-Erwähnungen in der Beschreibung.
    IssueUpdated { issue_id: IssueId, team_id: TeamId, changed: Vec<String>, mentioned_user_ids: Vec<UserId> },
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
    ProjectUpdated { project_id: ProjectId },
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
    InitiativeUpdated { initiative_id: InitiativeId },
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_type_tag() {
        let ev = DomainEvent::IssueCreated {
            issue_id: IssueId::new_v7(),
            team_id: TeamId::new_v7(),
            mentioned_user_ids: vec![],
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
