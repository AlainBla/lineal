//! Feste Enums aus docs/00 §4. Serde-Schreibweise exakt wie im Original-Schema.

use crate::error::DomainError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

macro_rules! string_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $var:ident => $s:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vdoc])* #[serde(rename = $s)] $var),+
        }

        impl $name {
            /// Alle Varianten in Definitionsreihenfolge.
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            /// Wire-/DB-Schreibweise.
            pub const fn as_str(&self) -> &'static str {
                match self { $($name::$var => $s),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }

        impl FromStr for $name {
            type Err = DomainError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($s => Ok($name::$var),)+
                    other => Err(DomainError::validation(
                        stringify!($name),
                        format!("unknown value `{other}`"),
                    )),
                }
            }
        }
    };
}

string_enum! {
    /// Statuskategorie (`WorkflowState.type`), feste Reihenfolge.
    StateType {
        Triage => "triage",
        Backlog => "backlog",
        Unstarted => "unstarted",
        Started => "started",
        Completed => "completed",
        Canceled => "canceled",
        Duplicate => "duplicate",
    }
}

impl StateType {
    /// „Offen“ = triage | backlog | unstarted | started.
    pub const fn is_open(&self) -> bool {
        matches!(
            self,
            Self::Triage | Self::Backlog | Self::Unstarted | Self::Started
        )
    }
    /// „Geschlossen“ = completed | canceled | duplicate.
    pub const fn is_closed(&self) -> bool {
        !self.is_open()
    }
}

string_enum! {
    /// `TeamVisibility`.
    TeamVisibility { Public => "public", Private => "private", Restricted => "restricted" }
}

string_enum! {
    /// `UserRoleType`.
    UserRole {
        Owner => "owner",
        Admin => "admin",
        User => "user",
        Guest => "guest",
        App => "app",
    }
}

string_enum! {
    /// `issueEstimationType`.
    EstimationType {
        NotUsed => "notUsed",
        Exponential => "exponential",
        Fibonacci => "fibonacci",
        Linear => "linear",
        TShirt => "tShirt",
    }
}

string_enum! {
    /// `IssueRelationType`.
    RelationType {
        Blocks => "blocks",
        Related => "related",
        Duplicate => "duplicate",
        Similar => "similar",
    }
}

string_enum! {
    /// `ProjectStatusType`.
    ProjectStatusType {
        Backlog => "backlog",
        Planned => "planned",
        Started => "started",
        Paused => "paused",
        Completed => "completed",
        Canceled => "canceled",
    }
}

string_enum! {
    /// Initiative-Status.
    InitiativeStatus {
        Proposed => "Proposed",
        Planned => "Planned",
        Active => "Active",
        Completed => "Completed",
        Canceled => "Canceled",
    }
}

string_enum! {
    /// `health` von Status-Updates.
    Health { OnTrack => "onTrack", AtRisk => "atRisk", OffTrack => "offTrack" }
}

string_enum! {
    /// Label-Gruppentyp `groupType`.
    LabelGroupType { SingleSelect => "singleSelect", MultiSelect => "multiSelect" }
}

string_enum! {
    /// SLA-Zähltyp `slaType`.
    SlaType { All => "all", OnlyBusinessDays => "onlyBusinessDays" }
}

string_enum! {
    /// View-Typ.
    ViewType { Issue => "Issue", Project => "Project", Initiative => "Initiative" }
}

string_enum! {
    /// Template-Typ.
    TemplateType { Issue => "issue", Project => "project", Document => "document" }
}

string_enum! {
    /// Auflösung von Zieldaten bei Projekt/Initiative.
    DateResolution { Month => "month", Quarter => "quarter", HalfYear => "halfYear", Year => "year" }
}

/// Priorität (`priority: Int`, 0 = keine … 4 = Low). Serde: Ganzzahl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum Priority {
    /// 0 – No priority (sortiert zuletzt).
    NoPriority,
    /// 1 – Urgent (sortiert zuerst).
    Urgent,
    /// 2 – High.
    High,
    /// 3 – Medium.
    Medium,
    /// 4 – Low.
    Low,
}

impl Priority {
    /// Numerischer Wert.
    pub const fn value(&self) -> u8 {
        match self {
            Self::NoPriority => 0,
            Self::Urgent => 1,
            Self::High => 2,
            Self::Medium => 3,
            Self::Low => 4,
        }
    }
    /// Anzeigename (`name` in `{ "value": 2, "name": "High" }`).
    pub const fn name(&self) -> &'static str {
        match self {
            Self::NoPriority => "No priority",
            Self::Urgent => "Urgent",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
    /// Sortierschlüssel für Listen: Urgent zuerst, „No priority“ zuletzt.
    pub const fn sort_key(&self) -> u8 {
        match self {
            Self::NoPriority => 5,
            other => other.value(),
        }
    }
}

impl TryFrom<u8> for Priority {
    type Error = DomainError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Ok(match v {
            0 => Self::NoPriority,
            1 => Self::Urgent,
            2 => Self::High,
            3 => Self::Medium,
            4 => Self::Low,
            _ => {
                return Err(DomainError::validation(
                    "priority",
                    "must be between 0 and 4",
                ));
            }
        })
    }
}

impl From<Priority> for u8 {
    fn from(p: Priority) -> Self {
        p.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap()
    }

    #[test]
    fn state_type_wire_and_order() {
        let names: Vec<_> = StateType::ALL.iter().map(StateType::as_str).collect();
        assert_eq!(
            names,
            [
                "triage",
                "backlog",
                "unstarted",
                "started",
                "completed",
                "canceled",
                "duplicate"
            ]
        );
        assert!(StateType::Started.is_open() && StateType::Duplicate.is_closed());
        assert_eq!("started".parse::<StateType>().unwrap(), StateType::Started);
        assert!("Started".parse::<StateType>().is_err());
    }

    #[test]
    fn camel_case_enums_are_exact() {
        assert_eq!(wire(&EstimationType::TShirt), "\"tShirt\"");
        assert_eq!(wire(&EstimationType::NotUsed), "\"notUsed\"");
        assert_eq!(wire(&Health::OffTrack), "\"offTrack\"");
        assert_eq!(wire(&LabelGroupType::SingleSelect), "\"singleSelect\"");
        assert_eq!(wire(&SlaType::OnlyBusinessDays), "\"onlyBusinessDays\"");
        assert_eq!(wire(&DateResolution::HalfYear), "\"halfYear\"");
        assert_eq!(wire(&InitiativeStatus::Active), "\"Active\"");
        assert_eq!(wire(&ViewType::Initiative), "\"Initiative\"");
        assert_eq!(wire(&UserRole::App), "\"app\"");
    }

    #[test]
    fn serde_matches_as_str_for_all() {
        fn check<T: Serialize + Copy>(all: &[T], f: impl Fn(&T) -> &'static str) {
            for v in all {
                assert_eq!(wire(v), format!("\"{}\"", f(v)));
            }
        }
        check(StateType::ALL, StateType::as_str);
        check(TeamVisibility::ALL, TeamVisibility::as_str);
        check(EstimationType::ALL, EstimationType::as_str);
        check(RelationType::ALL, RelationType::as_str);
        check(ProjectStatusType::ALL, ProjectStatusType::as_str);
        check(TemplateType::ALL, TemplateType::as_str);
    }

    #[test]
    fn priority_is_int() {
        assert_eq!(wire(&Priority::High), "2");
        assert_eq!(
            serde_json::from_str::<Priority>("4").unwrap(),
            Priority::Low
        );
        assert!(serde_json::from_str::<Priority>("5").is_err());
        assert_eq!(Priority::High.name(), "High");
        assert!(Priority::Urgent.sort_key() < Priority::NoPriority.sort_key());
    }
}
