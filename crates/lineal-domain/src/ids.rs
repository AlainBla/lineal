//! Typisierte IDs: je Entität ein Newtype über `Uuid` (Verwechslungen sind Kompilierfehler).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_types {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Neue zeitlich sortierbare ID (UUID v7).
            pub fn new_v7() -> Self { Self(Uuid::now_v7()) }
            /// Die Nil-UUID (nur für Systemkontexte und Tests).
            pub const fn nil() -> Self { Self(Uuid::nil()) }
            /// Zugrunde liegende UUID.
            pub const fn as_uuid(&self) -> Uuid { self.0 }
        }
        impl From<Uuid> for $name { fn from(u: Uuid) -> Self { Self(u) } }
        impl From<$name> for Uuid { fn from(v: $name) -> Self { v.0 } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> { Uuid::parse_str(s).map(Self) }
        }
    )+};
}

id_types! {
    /// Workspace (Schema: `Organization`).
    OrgId,
    UserId,
    TeamId,
    IssueId,
    /// Workflow-Status.
    StateId,
    LabelId,
    CycleId,
    ProjectId,
    MilestoneId,
    InitiativeId,
    CommentId,
    ReactionId,
    AttachmentId,
    ProjectUpdateId,
    InitiativeUpdateId,
    DocumentId,
    TemplateId,
    ViewId,
    NotificationId,
    WebhookId,
    ApiKeyId,
    InvitationId,
    IssueRelationId,
    /// Outbox-Event.
    EventId,
    /// Eintrag der Job-Tabelle.
    JobId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_is_transparent_and_roundtrips() {
        let id = IssueId::new_v7();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.0));
        assert_eq!(serde_json::from_str::<IssueId>(&json).unwrap(), id);
        assert_eq!(id.to_string().parse::<IssueId>().unwrap(), id);
    }
}
