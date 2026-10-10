//! policy: Bereichsmodule (Eigentümer laut docs/12).
//!
//! **API für alle Arbeitspakete** (Eigentümer: WP02):
//!
//! ```ignore
//! // einmal pro Request laden, mehrfach benutzen
//! let snap = policy::PolicySnapshot::load(tx, ctx).await?;
//! let visible = snap.visible_team_ids();                       // BTreeSet<TeamId>
//! let view = snap.require_team(ctx, policy::Action::CreateIssue, team_id)?; // not_found / forbidden
//! snap.require_teams(ctx, policy::Action::ManageProject, &[a, b])?;
//! snap.require_workspace(ctx, policy::Action::CreateTeam)?;
//! // oder kurz, mit eigenem Snapshot je Aufruf
//! let visible = policy::visible_team_ids(tx, ctx).await?;
//! policy::require_team(tx, ctx, policy::Action::UpdateIssue, team_id).await?;
//! ```
//!
//! Der reine Kern (`TeamTree`, `ActorView`, `compute_visible_team_ids`, `require`) hat keine I/O
//! und ist unit-getestet.

pub mod attachment;
pub mod auth;
pub mod comment;
pub mod cycle;
pub mod document;
pub mod initiative;
pub mod issue;
pub mod label;
pub mod notification;
pub mod org;
pub mod project;
pub mod state;
pub mod team;
pub mod template;
pub mod triage;
pub mod user;
pub mod view;
pub mod webhook;

pub use auth::{Action, Resource, require};
pub use org::{Limits, OrgPolicy, default_limits_json, default_security_settings_json, role_rank};
pub use team::{
    ActorView, PolicySnapshot, TeamPolicyView, TeamTree, compute_visible_team_ids, require_team,
    require_workspace, team_not_found, visibility_allowed_under_parent, visible_team_ids,
};
