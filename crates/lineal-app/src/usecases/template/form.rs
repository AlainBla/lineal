//! Form-Templates und Default-Templates beim Anlegen (WP34, docs/06 §2.8).
//!
//! * [`render_form`] wertet die Antworten eines Form-Templates aus: Property-Felder (`title`,
//!   `priority`, `dueDate`, `labelGroup`) setzen Issue-Felder, alle anderen werden in
//!   Formularreihenfolge als Markdown in die Beschreibung geschrieben.
//! * [`App::default_template_for_issue`]/[`App::default_template_for_project`] liefern das
//!   Default-Template eines Teams für `useDefaultTemplate` der öffentlichen API. MCP wendet nie
//!   automatisch ein Default an.
//! * [`App::create_issue_with_form`] legt ein Issue aus einem (Form-)Template an.

use super::apply::TemplateIssueOutcome;
use super::{create_issue_tree, is_form, prepare_issue_template};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::repos::template::DefaultSlot;
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::usecases::issue::CreateIssueInput;
use chrono::NaiveDate;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Längste Antwort je Feld (Zeichen).
pub const MAX_ANSWER_CHARS: usize = 20_000;
/// Höchstzahl von Formularfeldern.
pub const MAX_FORM_FIELDS: usize = 100;

/// Antwort auf ein Formularfeld.
#[derive(Debug, Clone, PartialEq)]
pub struct FormAnswer {
    pub field_id: String,
    /// Text, Zahl (`priority`) oder Liste von Texten (`checkboxes`).
    pub value: Value,
}

/// Ergebnis der Auswertung eines Formulars.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderedForm {
    pub title: Option<String>,
    pub priority: Option<i64>,
    pub due_date: Option<NaiveDate>,
    /// Gewählte Labels der `labelGroup`-Felder (Name oder UUID).
    pub labels: Vec<String>,
    /// Markdown der übrigen Felder.
    pub description: String,
}

fn invalid(field_id: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg.into()).with_details(json!({ "fields": [field_id] }))
}

fn text_of(id: &str, v: &Value) -> AppResult<String> {
    let s = v
        .as_str()
        .ok_or_else(|| invalid(id, format!("the answer to `{id}` must be a string")))?;
    if s.chars().count() > MAX_ANSWER_CHARS {
        return Err(invalid(id, format!("the answer to `{id}` is too long")));
    }
    Ok(s.trim().to_owned())
}

fn priority_of(id: &str, v: &Value) -> AppResult<i64> {
    let n = match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "none" | "no priority" => Some(0),
            "urgent" => Some(1),
            "high" => Some(2),
            "medium" => Some(3),
            "low" => Some(4),
            other => other.parse().ok(),
        },
        _ => None,
    };
    n.filter(|n| (0..=4).contains(n))
        .ok_or_else(|| invalid(id, "priority must be 0-4 or none/urgent/high/medium/low"))
}

fn date_of(id: &str, v: &Value) -> AppResult<NaiveDate> {
    lineal_domain::rules::date::parse_iso_date(id, text_of(id, v)?.as_str())
        .map_err(|_| invalid(id, format!("`{id}` must be a date (YYYY-MM-DD)")))
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// Wertet die Antworten eines Form-Templates aus. Fehlende Pflichtfelder → `invalid_request` mit
/// `details.fields`; unbekannte Feld-IDs, ungültige Optionen und falsche Typen ebenfalls.
pub fn render_form(fields: &Value, answers: &[FormAnswer]) -> AppResult<RenderedForm> {
    let list = fields
        .as_array()
        .ok_or_else(|| AppError::invalid_request("the template has no form fields"))?;
    if list.len() > MAX_FORM_FIELDS {
        return Err(AppError::invalid_request("the form has too many fields"));
    }
    let known: BTreeSet<&str> = list
        .iter()
        .filter_map(|f| f.get("id").and_then(Value::as_str))
        .collect();
    let unknown: Vec<&str> = answers
        .iter()
        .map(|a| a.field_id.as_str())
        .filter(|id| !known.contains(id))
        .collect();
    if !unknown.is_empty() {
        return Err(AppError::invalid_request("unknown form field")
            .with_details(json!({ "fields": unknown })));
    }
    let answer_of = |id: &str| answers.iter().find(|a| a.field_id == id).map(|a| &a.value);

    let mut out = RenderedForm::default();
    let mut missing: Vec<String> = Vec::new();
    let mut sections: Vec<String> = Vec::new();
    for f in list {
        let id = f.get("id").and_then(Value::as_str).unwrap_or_default();
        let kind = f.get("kind").and_then(Value::as_str).unwrap_or_default();
        let label = f.get("label").and_then(Value::as_str).unwrap_or_default();
        let required = f.get("required").and_then(Value::as_bool).unwrap_or(false);
        if kind == "instructions" {
            if let Some(t) = f.get("text").and_then(Value::as_str)
                && !t.trim().is_empty()
            {
                sections.push(format!("### {label}\n\n{}", t.trim()));
            }
            continue;
        }
        let answer = answer_of(id).filter(|v| !is_empty(v));
        let Some(v) = answer else {
            if required {
                missing.push(id.to_owned());
            }
            continue;
        };
        let options: Vec<&str> = f
            .get("options")
            .and_then(Value::as_array)
            .map(|o| o.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        match kind {
            "title" => {
                let t = text_of(id, v)?;
                if t.chars().count() > 255 {
                    return Err(invalid(id, "the title must not exceed 255 characters"));
                }
                out.title = Some(t);
            }
            "priority" => out.priority = Some(priority_of(id, v)?),
            "dueDate" => out.due_date = Some(date_of(id, v)?),
            "labelGroup" => out.labels.push(text_of(id, v)?),
            "date" => sections.push(format!("### {label}\n\n{}", date_of(id, v)?)),
            "dropdown" => {
                let t = text_of(id, v)?;
                if !options.contains(&t.as_str()) {
                    return Err(invalid(id, format!("`{t}` is not an option of `{id}`")));
                }
                sections.push(format!("### {label}\n\n{t}"));
            }
            "checkboxes" => {
                let items: Vec<String> = match v {
                    Value::Array(a) => a
                        .iter()
                        .map(|x| text_of(id, x))
                        .collect::<AppResult<Vec<_>>>()?,
                    other => vec![text_of(id, other)?],
                };
                if let Some(bad) = items.iter().find(|i| !options.contains(&i.as_str())) {
                    return Err(invalid(id, format!("`{bad}` is not an option of `{id}`")));
                }
                let list: Vec<String> = items.iter().map(|i| format!("- {i}")).collect();
                sections.push(format!("### {label}\n\n{}", list.join("\n")));
            }
            "text" | "longText" => {
                sections.push(format!("### {label}\n\n{}", text_of(id, v)?));
            }
            other => {
                return Err(AppError::invalid_request(format!(
                    "unsupported form field kind `{other}`"
                )));
            }
        }
    }
    if !missing.is_empty() {
        return Err(
            AppError::invalid_request("required form fields are missing")
                .with_details(json!({ "fields": missing })),
        );
    }
    out.description = sections.join("\n\n");
    Ok(out)
}

impl App {
    /// Default-Template eines Teams für `useDefaultTemplate` (docs/06 §2.8): Mitglieder erhalten
    /// `defaultTemplateForMembers`, alle anderen `defaultTemplateForNonMembers`. Liefert die
    /// Template-ID oder `None`. Nur für die öffentliche API; MCP wendet nie ein Default an.
    pub async fn default_template_for_issue(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
    ) -> AppResult<Option<String>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        let d = tx.templates().team_defaults(team.id).await?;
        let slot = if snap.actor.is_member(team.id) {
            d.members
        } else {
            d.non_members
        };
        Ok(slot.map(|t| t.to_string()))
    }

    /// `defaultProjectTemplate` des Lead-Teams (docs/06 §2.8).
    pub async fn default_template_for_project(
        &self,
        ctx: &RequestCtx,
        lead_team_ref: &str,
    ) -> AppResult<Option<String>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, lead_team_ref, Visibility::Content).await?;
        let d = tx.templates().team_defaults(team.id).await?;
        let _ = DefaultSlot::Project;
        Ok(d.project.map(|t| t.to_string()))
    }

    /// Wie [`App::create_issue_from_template`], wertet aber bei einem Form-Template die Antworten
    /// aus (docs/06 §2.8): Titel, Priorität, Fälligkeit und Labels aus den Property-Feldern, der
    /// Rest als Markdown in der Beschreibung. Ausdrücklich übergebene Felder des Aufrufers haben
    /// Vorrang. Antworten zu einem Template ohne Formular → `invalid_request`.
    pub async fn create_issue_with_form(
        &self,
        ctx: &RequestCtx,
        template: &str,
        mut input: CreateIssueInput,
        answers: Vec<FormAnswer>,
        null_fields: &[String],
    ) -> AppResult<TemplateIssueOutcome> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (tpl, team, data) =
            prepare_issue_template(&mut *tx, &snap, ctx, template, &input).await?;
        if is_form(&tpl) {
            let fields = tpl
                .template_data
                .get("formFields")
                .cloned()
                .unwrap_or(Value::Null);
            let form = render_form(&fields, &answers)?;
            if input.title.is_none() {
                input.title = form.title;
            }
            if input.priority.is_none() {
                input.priority = form.priority;
            }
            if input.due_date.is_none() {
                input.due_date = form.due_date;
            }
            input.add_labels.extend(form.labels);
            if input.description.is_none() && !form.description.is_empty() {
                input.description = Some(form.description);
            }
        } else if !answers.is_empty() {
            return Err(AppError::invalid_request(
                "form answers were given, but the template is not a form",
            ));
        }
        let nulls: BTreeSet<String> = null_fields.iter().cloned().collect();
        let outcome = create_issue_tree(
            self,
            &mut *tx,
            &snap,
            ctx,
            &team,
            Some(&tpl),
            &data,
            input,
            &nulls,
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Value {
        json!([
            { "id": "t", "kind": "title", "label": "Title", "required": true },
            { "id": "p", "kind": "priority", "label": "Priority" },
            { "id": "d", "kind": "dueDate", "label": "Due" },
            { "id": "l", "kind": "labelGroup", "label": "Area", "labelGroupId": "00000000-0000-0000-0000-000000000000" },
            { "id": "i", "kind": "instructions", "label": "Hinweis", "text": "Bitte genau beschreiben." },
            { "id": "w", "kind": "longText", "label": "What happened?", "required": true },
            { "id": "s", "kind": "dropdown", "label": "Severity", "options": ["minor", "major"] },
            { "id": "c", "kind": "checkboxes", "label": "Browsers", "options": ["Firefox", "Chrome"] },
            { "id": "x", "kind": "date", "label": "Seen on" }
        ])
    }

    fn a(id: &str, v: Value) -> FormAnswer {
        FormAnswer {
            field_id: id.into(),
            value: v,
        }
    }

    #[test]
    fn property_fields_set_issue_fields_and_the_rest_becomes_markdown() {
        let r = render_form(
            &fields(),
            &[
                a("t", json!("Login kaputt")),
                a("p", json!("high")),
                a("d", json!("2026-11-01")),
                a("l", json!("Frontend")),
                a("w", json!("Der Button reagiert nicht.")),
                a("s", json!("major")),
                a("c", json!(["Firefox", "Chrome"])),
                a("x", json!("2026-10-12")),
            ],
        )
        .unwrap();
        assert_eq!(r.title.as_deref(), Some("Login kaputt"));
        assert_eq!(r.priority, Some(2));
        assert_eq!(r.due_date, NaiveDate::from_ymd_opt(2026, 11, 1));
        assert_eq!(r.labels, vec!["Frontend"]);
        assert_eq!(
            r.description,
            "### Hinweis\n\nBitte genau beschreiben.\n\n### What happened?\n\nDer Button reagiert nicht.\n\n### Severity\n\nmajor\n\n### Browsers\n\n- Firefox\n- Chrome\n\n### Seen on\n\n2026-10-12"
        );
    }

    #[test]
    fn missing_required_fields_are_listed() {
        let e = render_form(&fields(), &[a("p", json!(2))]).unwrap_err();
        assert_eq!(e.kind, crate::ErrorKind::InvalidRequest);
        let d = e.details;
        assert_eq!(d["fields"], json!(["t", "w"]));
        // Leere Antworten zählen als fehlend.
        let e = render_form(&fields(), &[a("t", json!("  ")), a("w", json!("x"))]).unwrap_err();
        assert_eq!(e.details["fields"], json!(["t"]));
    }

    #[test]
    fn invalid_answers_are_rejected() {
        let base = || vec![a("t", json!("T")), a("w", json!("W"))];
        for bad in [
            a("s", json!("critical")),
            a("c", json!(["Safari"])),
            a("p", json!(9)),
            a("d", json!("morgen")),
            a("nope", json!("x")),
        ] {
            let mut v = base();
            v.push(bad);
            assert_eq!(
                render_form(&fields(), &v).unwrap_err().kind,
                crate::ErrorKind::InvalidRequest
            );
        }
    }
}
