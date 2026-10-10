//! Zuordnung Tool-Name → Handler-Gruppe und Dispatch. Gemeinsame Datei: nur S1 / Orchestrator.
//!
//! `tools/list` liefert den Inhalt von `docs/mcp-tools.snapshot.json` (55 kompatible Tools,
//! docs/07 §6.1) und danach die Lineal-Erweiterungen aus [`EXTENSION_TOOLS`] (docs/14 §4).

use crate::handlers;
use lineal_app::{App, AppResult, RequestCtx};
use serde_json::Value;

/// Der Schema-Snapshot der kompatiblen Tools, zur Build-Zeit eingebettet.
pub const SNAPSHOT_JSON: &str = include_str!("../../../docs/mcp-tools.snapshot.json");

/// Handler-Gruppe (je eine Datei in `handlers/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    TeamsUsers,
    Issues,
    Statuses,
    Labels,
    Comments,
    Attachments,
    Projects,
    Milestones,
    Cycles,
    Initiatives,
    StatusUpdates,
    Views,
    Inbox,
    Triage,
    Documents,
    Templates,
    Sharing,
    Workspace,
}

impl Group {
    /// Dateiname der Gruppe in `handlers/`.
    pub const fn name(&self) -> &'static str {
        match self {
            Group::TeamsUsers => "teams_users",
            Group::Issues => "issues",
            Group::Statuses => "statuses",
            Group::Labels => "labels",
            Group::Comments => "comments",
            Group::Attachments => "attachments",
            Group::Projects => "projects",
            Group::Milestones => "milestones",
            Group::Cycles => "cycles",
            Group::Initiatives => "initiatives",
            Group::StatusUpdates => "status_updates",
            Group::Views => "views",
            Group::Inbox => "inbox",
            Group::Triage => "triage",
            Group::Documents => "documents",
            Group::Templates => "templates",
            Group::Sharing => "sharing",
            Group::Workspace => "workspace",
        }
    }
}

/// Die 55 mit Linear kompatiblen Tools in Snapshot-Reihenfolge.
pub const COMPAT_TOOLS: [(&str, Group); 55] = [
    ("list_teams", Group::TeamsUsers),
    ("get_team", Group::TeamsUsers),
    ("list_users", Group::TeamsUsers),
    ("get_user", Group::TeamsUsers),
    ("get_workspace", Group::TeamsUsers),
    ("list_issues", Group::Issues),
    ("get_issue", Group::Issues),
    ("save_issue", Group::Issues),
    ("list_issue_statuses", Group::Statuses),
    ("get_issue_status", Group::Statuses),
    ("list_issue_labels", Group::Labels),
    ("save_issue_label", Group::Labels),
    ("retire_issue_label", Group::Labels),
    ("restore_issue_label", Group::Labels),
    ("list_comments", Group::Comments),
    ("save_comment", Group::Comments),
    ("delete_comment", Group::Comments),
    ("list_projects", Group::Projects),
    ("get_project", Group::Projects),
    ("save_project", Group::Projects),
    ("list_project_labels", Group::Labels),
    ("save_project_label", Group::Labels),
    ("retire_project_label", Group::Labels),
    ("restore_project_label", Group::Labels),
    ("list_milestones", Group::Milestones),
    ("get_milestone", Group::Milestones),
    ("save_milestone", Group::Milestones),
    ("list_cycles", Group::Cycles),
    ("list_initiatives", Group::Initiatives),
    ("get_initiative", Group::Initiatives),
    ("save_initiative", Group::Initiatives),
    ("list_initiative_labels", Group::Labels),
    ("save_initiative_label", Group::Labels),
    ("retire_initiative_label", Group::Labels),
    ("restore_initiative_label", Group::Labels),
    ("get_status_updates", Group::StatusUpdates),
    ("save_status_update", Group::StatusUpdates),
    ("delete_status_update", Group::StatusUpdates),
    ("list_custom_views", Group::Views),
    ("get_notifications", Group::Inbox),
    ("mark_notification", Group::Inbox),
    ("get_triage_responsibility", Group::Triage),
    ("list_documents", Group::Documents),
    ("get_document", Group::Documents),
    ("save_document", Group::Documents),
    ("list_templates", Group::Templates),
    ("get_template", Group::Templates),
    ("prepare_attachment_upload", Group::Attachments),
    ("create_attachment_from_upload", Group::Attachments),
    ("get_attachment", Group::Attachments),
    ("delete_attachment", Group::Attachments),
    ("extract_images", Group::Attachments),
    ("share_issue", Group::Sharing),
    ("unshare_issue", Group::Sharing),
    ("search_documentation", Group::Documents),
];

/// Lineal-eigene Erweiterungen (bewusst über den Linear-MCP hinaus), erscheinen in
/// `tools/list` nach den kompatiblen Tools.
pub const EXTENSION_TOOLS: [(&str, Group); 1] = [("get_workspace_overview", Group::Workspace)];

/// Alle Tools: kompatible, dann Erweiterungen.
pub fn all_tools() -> impl Iterator<Item = (&'static str, Group)> {
    COMPAT_TOOLS.iter().chain(EXTENSION_TOOLS.iter()).copied()
}

/// Handler-Gruppe eines Tools.
pub fn group_for(tool: &str) -> Option<Group> {
    all_tools().find(|(name, _)| *name == tool).map(|(_, g)| g)
}

/// Verteilt einen `tools/call` an den Handler. `None` = unbekanntes Tool (JSON-RPC `-32601`).
pub async fn dispatch(
    app: &App,
    ctx: &RequestCtx,
    tool: &str,
    args: Value,
) -> Option<AppResult<Value>> {
    let group = group_for(tool)?;
    Some(match group {
        Group::TeamsUsers => handlers::teams_users::handle(app, ctx, tool, args).await,
        Group::Issues => handlers::issues::handle(app, ctx, tool, args).await,
        Group::Statuses => handlers::statuses::handle(app, ctx, tool, args).await,
        Group::Labels => handlers::labels::handle(app, ctx, tool, args).await,
        Group::Comments => handlers::comments::handle(app, ctx, tool, args).await,
        Group::Attachments => handlers::attachments::handle(app, ctx, tool, args).await,
        Group::Projects => handlers::projects::handle(app, ctx, tool, args).await,
        Group::Milestones => handlers::milestones::handle(app, ctx, tool, args).await,
        Group::Cycles => handlers::cycles::handle(app, ctx, tool, args).await,
        Group::Initiatives => handlers::initiatives::handle(app, ctx, tool, args).await,
        Group::StatusUpdates => handlers::status_updates::handle(app, ctx, tool, args).await,
        Group::Views => handlers::views::handle(app, ctx, tool, args).await,
        Group::Inbox => handlers::inbox::handle(app, ctx, tool, args).await,
        Group::Triage => handlers::triage::handle(app, ctx, tool, args).await,
        Group::Documents => handlers::documents::handle(app, ctx, tool, args).await,
        Group::Templates => handlers::templates::handle(app, ctx, tool, args).await,
        Group::Sharing => handlers::sharing::handle(app, ctx, tool, args).await,
        Group::Workspace => handlers::workspace::handle(app, ctx, tool, args).await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::ErrorKind;
    use lineal_testkit::{SeedUser, ctx_for, fake_app};
    use std::collections::BTreeSet;

    fn snapshot_names() -> BTreeSet<String> {
        let v: Value = serde_json::from_str(SNAPSHOT_JSON).unwrap();
        let map = v.as_object().unwrap();
        for (key, tool) in map {
            assert_eq!(tool["name"], *key, "snapshot key and name differ");
        }
        map.keys().cloned().collect()
    }

    #[test]
    fn dispatch_knows_exactly_the_55_snapshot_tools() {
        let snapshot = snapshot_names();
        assert_eq!(snapshot.len(), 55);
        let compat: Vec<&str> = COMPAT_TOOLS.iter().map(|(n, _)| *n).collect();
        let compat_set: BTreeSet<String> = compat.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            compat.len(),
            compat_set.len(),
            "duplicate tool names in COMPAT_TOOLS"
        );
        assert_eq!(compat_set, snapshot);
    }

    #[test]
    fn extension_is_separate_from_snapshot() {
        assert_eq!(EXTENSION_TOOLS.map(|(n, _)| n), ["get_workspace_overview"]);
        assert!(!snapshot_names().contains("get_workspace_overview"));
        assert_eq!(group_for("get_workspace_overview"), Some(Group::Workspace));
        let all: Vec<_> = all_tools().map(|(n, _)| n).collect();
        assert_eq!(all.len(), 56);
        assert_eq!(all.last(), Some(&"get_workspace_overview"));
    }

    #[tokio::test]
    async fn every_tool_dispatches_to_a_not_implemented_handler() {
        let app = fake_app();
        let ctx = ctx_for(SeedUser::Alice);
        for (name, group) in all_tools() {
            // WP02 hat teams_users und statuses implementiert (eigene Contract-Tests).
            if matches!(group, Group::TeamsUsers | Group::Statuses) {
                continue;
            }
            let res = dispatch(&app, &ctx, name, Value::Null).await.expect(name);
            let err = res.expect_err(name);
            assert_eq!(err.kind, ErrorKind::Internal, "{name}");
            assert_eq!(err.message, "not implemented", "{name}");
        }
        assert!(
            dispatch(&app, &ctx, "no_such_tool", Value::Null)
                .await
                .is_none()
        );
    }
}
