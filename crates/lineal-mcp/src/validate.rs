//! `validate` (WP04): Tool-Katalog (Snapshot + Erweiterungen) und Schema-Validierung der
//! `tools/call`-Argumente gegen die unveränderten `inputSchema`s (docs/07 §6.1).

use crate::dispatch::{COMPAT_TOOLS, EXTENSION_TOOLS, SNAPSHOT_JSON};
use jsonschema::Validator;
use lineal_app::AppError;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Ein Tool in `tools/list`.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: String,
    /// Byte-/strukturidentisch zum Snapshot (`parameters` dort) bei kompatiblen Tools.
    pub input_schema: Value,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

impl ToolDef {
    /// Eintrag für `tools/list` (`name`, `description`, `inputSchema`, `annotations`).
    pub fn to_list_entry(&self) -> Value {
        let mut annotations = json!({ "readOnlyHint": self.read_only });
        if self.destructive {
            annotations["destructiveHint"] = Value::Bool(true);
        }
        if self.idempotent {
            annotations["idempotentHint"] = Value::Bool(true);
        }
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": annotations,
        })
    }
}

/// Katalog aller Tools in `tools/list`-Reihenfolge samt kompilierten Validatoren.
pub struct Catalog {
    tools: Vec<ToolDef>,
    validators: HashMap<&'static str, Validator>,
}

impl Catalog {
    /// Alle Tools: 55 kompatible (Snapshot-Reihenfolge), dann Erweiterungen.
    pub fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Tool nach Name.
    pub fn tool(&self, name: &str) -> Option<&ToolDef> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// Schema der Erweiterung `get_workspace_overview` (docs/14 §4), im Stil des Snapshots.
fn overview_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "additionalProperties": false,
        "properties": {
            "includeSubTeams": {
                "default": true,
                "description": "Include sub-teams in each team's figures",
                "type": "boolean"
            },
            "period": {
                "default": "7d",
                "description": "Reporting period: 7d | 30d",
                "enum": ["7d", "30d"],
                "type": "string"
            },
            "teams": {
                "description": "Restrict to these teams (name, key, or ID)",
                "items": { "type": "string" },
                "type": "array"
            }
        },
        "type": "object"
    })
}

/// Beschreibung und Schema der Meilenstein-Erweiterungen (docs/07, LIN-25).
fn milestone_tool(name: &str) -> (&'static str, Value) {
    let base = |props: Value, required: Value| {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "additionalProperties": false,
            "properties": props,
            "required": required,
            "type": "object"
        })
    };
    let project = json!({
        "description": "Project name, ID, identifier (e.g., P-ENG-123), or slug",
        "type": "string"
    });
    let milestone = json!({ "description": "Milestone name or ID", "type": "string" });
    match name {
        "get_milestone_resources" => (
            "List the documents, links and files of a milestone (Lineal extension)",
            base(
                json!({ "project": project, "milestone": milestone }),
                json!(["project", "milestone"]),
            ),
        ),
        "save_milestone_resource" => (
            "Add a resource to a milestone: a link (`url`, `title`), an existing project document (`document`) or an uploaded file (`assetUrl` from prepare_milestone_upload). Exactly one of url, document or assetUrl (Lineal extension)",
            base(
                json!({
                    "project": project,
                    "milestone": milestone,
                    "url": { "description": "Link URL (http or https)", "format": "uri", "type": "string" },
                    "title": { "description": "Link or file title", "type": "string" },
                    "document": { "description": "Document ID of a document in this project", "type": "string" },
                    "assetUrl": { "description": "assetUrl returned by prepare_milestone_upload", "type": "string" }
                }),
                json!(["project", "milestone"]),
            ),
        ),
        "delete_milestone_resource" => (
            "Remove a resource from a milestone. Links and files are deleted, documents stay in the project (Lineal extension)",
            base(
                json!({
                    "project": project,
                    "type": { "description": "Resource type", "enum": ["link", "document", "file"], "type": "string" },
                    "id": { "description": "Resource ID", "type": "string" }
                }),
                json!(["project", "type", "id"]),
            ),
        ),
        "complete_milestone" => (
            "Complete a milestone deliberately: freezes its issue list, handles open issues and stores a result note. Completing never happens automatically (Lineal extension)",
            base(
                json!({
                    "project": project,
                    "milestone": milestone,
                    "openIssues": {
                        "description": "What happens to open issues: keep them, remove them from the milestone, or move them to `moveTo`",
                        "enum": ["keep", "remove", "move"],
                        "type": "string"
                    },
                    "moveTo": { "description": "Target milestone name or ID when openIssues is move", "type": "string" },
                    "note": { "description": "Result note as Markdown", "type": "string" }
                }),
                json!(["project", "milestone", "openIssues"]),
            ),
        ),
        "convert_milestone_to_project" => (
            "Turn an open milestone into a project with the same teams and priority. Its issues and resources move along, the milestone is removed (Lineal extension)",
            base(
                json!({ "project": project, "milestone": milestone }),
                json!(["project", "milestone"]),
            ),
        ),
        "reopen_milestone" => (
            "Reopen a completed milestone. Its result note stays, the frozen issue list is dropped (Lineal extension)",
            base(
                json!({ "project": project, "milestone": milestone }),
                json!(["project", "milestone"]),
            ),
        ),
        _ => (
            "Prepare a direct file upload for a milestone. PUT the raw bytes to uploadRequest.url with all headers verbatim, then call save_milestone_resource with assetUrl (Lineal extension)",
            base(
                json!({
                    "filename": { "description": "Filename for the upload", "type": "string" },
                    "contentType": { "description": "MIME type", "type": "string" },
                    "size": { "description": "Exact file size in bytes", "exclusiveMinimum": 0, "type": "integer" }
                }),
                json!(["filename", "contentType", "size"]),
            ),
        ),
    }
}

const OVERVIEW_DESCRIPTION: &str = "Company-wide overview of the Lineal workspace across all teams visible to the user: open issues, active cycles, project health and workload per team (Lineal extension)";

fn build_catalog() -> Catalog {
    let snapshot: Value =
        serde_json::from_str(SNAPSHOT_JSON).expect("docs/mcp-tools.snapshot.json is valid JSON");
    let mut tools = Vec::new();
    for (name, _) in COMPAT_TOOLS {
        let entry = &snapshot[name];
        tools.push(def(
            name,
            entry["description"].as_str().unwrap_or_default().to_owned(),
            entry["parameters"].clone(),
        ));
    }
    for (name, _) in EXTENSION_TOOLS {
        if name == "get_workspace_overview" {
            tools.push(def(
                name,
                OVERVIEW_DESCRIPTION.to_owned(),
                overview_schema(),
            ));
        } else {
            let (description, schema) = milestone_tool(name);
            tools.push(def(name, description.to_owned(), schema));
        }
    }
    let validators = tools
        .iter()
        .map(|t| {
            let v = jsonschema::validator_for(&t.input_schema)
                .unwrap_or_else(|e| panic!("invalid inputSchema of {}: {e}", t.name));
            (t.name, v)
        })
        .collect();
    Catalog { tools, validators }
}

fn def(name: &'static str, description: String, input_schema: Value) -> ToolDef {
    let read_only = name.starts_with("list_")
        || name.starts_with("get_")
        || name == "extract_images"
        || name == "search_documentation";
    let retire = name.starts_with("retire_");
    ToolDef {
        name,
        description,
        input_schema,
        read_only,
        destructive: name.starts_with("delete_") || retire || name == "unshare_issue",
        idempotent: name == "mark_notification" || retire || name.starts_with("restore_"),
    }
}

/// Der (einmalig gebaute) Katalog.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}

/// Validiert `args` gegen das `inputSchema` von `tool`. Verstöße → `invalid_request` mit
/// `details.path` (JSON-Pointer der ersten Verletzung) und `details.errors` (max. 10).
/// Unbekanntes Tool → `Ok` (der Dispatcher meldet es als `-32601`).
pub fn validate_args(tool: &str, args: &Value) -> Result<(), AppError> {
    let cat = catalog();
    let Some(def) = cat.tool(tool) else {
        return Ok(());
    };
    let Some(validator) = cat.validators.get(def.name) else {
        return Ok(());
    };
    let errors: Vec<(String, String)> = validator
        .iter_errors(args)
        .take(10)
        .map(|e| (e.instance_path.to_string(), e.to_string()))
        .collect();
    let Some((path, message)) = errors.first().cloned() else {
        return Ok(());
    };
    let shown = if path.is_empty() { "(root)" } else { &path };
    Err(
        AppError::invalid_request(format!("Invalid arguments for {tool}: {shown}: {message}"))
            .with_details(json!({
                "path": path,
                "errors": errors
                    .iter()
                    .map(|(p, m)| json!({ "path": p, "message": m }))
                    .collect::<Vec<_>>(),
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::ErrorKind;

    #[test]
    fn catalog_has_55_compat_plus_extension() {
        let c = catalog();
        assert_eq!(c.tools().len(), 63);
        assert_eq!(c.tools()[55].name, "get_workspace_overview");
        assert!(c.tool("get_issue").is_some_and(|t| t.read_only));
        assert!(c.tool("delete_comment").is_some_and(|t| t.destructive));
        assert!(c.tool("mark_notification").is_some_and(|t| t.idempotent));
    }

    #[test]
    fn valid_and_invalid_arguments() {
        assert!(validate_args("list_teams", &json!({})).is_ok());
        assert!(validate_args("list_teams", &json!({ "limit": 250 })).is_ok());

        let e = validate_args("list_teams", &json!({ "limit": 300 })).expect_err("max");
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(e.details["path"], "/limit");

        let e = validate_args("list_teams", &json!({ "bogus": 1 })).expect_err("additional");
        assert_eq!(e.kind, ErrorKind::InvalidRequest);

        let e = validate_args("list_teams", &json!({ "orderBy": "name" })).expect_err("enum");
        assert_eq!(e.details["path"], "/orderBy");

        let e = validate_args("get_issue", &json!({})).expect_err("required");
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert!(e.message.contains("id"), "{}", e.message);
    }

    #[test]
    fn extension_schema_validates() {
        assert!(validate_args("get_workspace_overview", &json!({ "period": "30d" })).is_ok());
        assert!(validate_args("get_workspace_overview", &json!({ "period": "1y" })).is_err());
    }
}
