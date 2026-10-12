//! Validiert alle Abfragen des Linear-Adapters gegen das öffentliche Linear-GraphQL-Schema
//! (`tests/data/linear-schema.graphql`, aus github.com/linear/linear, MIT-Lizenz).
//! Prüft Feldnamen, Argumente, Typen und Verbindungsform; die Komplexität wird separat
//! über eine Punkteschätzung begrenzt (Linear lehnt Abfragen > 10 000 Punkte ab).

use apollo_compiler::{ExecutableDocument, Schema, validation::Valid};
use lineal_import::adapter::linear::{WORKSPACE_QUERY, build_query};
use lineal_import::port::{PageQuery, Scope, SourceEntity};

fn schema() -> Valid<Schema> {
    let text = include_str!("data/linear-schema.graphql");
    Schema::parse_and_validate(text, "linear-schema.graphql").expect("Linear schema is valid")
}

/// Fehlermeldungen (ohne Quelltext-Ausschnitt) einer Abfrage; leer = gültig.
fn errors(schema: &Valid<Schema>, name: &str, query: &str) -> Vec<String> {
    match ExecutableDocument::parse_and_validate(schema, query, name) {
        Ok(_) => Vec::new(),
        Err(e) => e
            .errors
            .iter()
            .map(|d| format!("{name}: {}", d.error))
            .collect(),
    }
}

fn validate(schema: &Valid<Schema>, name: &str, query: &str) {
    let errs = errors(schema, name, query);
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

#[test]
fn workspace_query_matches_schema() {
    validate(&schema(), "workspace", WORKSPACE_QUERY);
}

#[test]
fn every_entity_query_matches_schema_with_and_without_filter() {
    let schema = schema();
    let mut all = Vec::new();
    for e in SourceEntity::ALL {
        for after in [None, Some("2026-03-01T10:00:00Z".parse().unwrap())] {
            for scope in [
                Scope::default(),
                Scope {
                    teams: vec!["11111111-aaaa-4bbb-8ccc-000000000001".into()],
                    projects: vec!["22222222-aaaa-4bbb-8ccc-000000000002".into()],
                },
            ] {
                let body = build_query(&PageQuery {
                    entity: e,
                    cursor: None,
                    updated_after: after,
                    limit: 100,
                    scope,
                });
                all.extend(errors(&schema, e.as_str(), body["query"].as_str().unwrap()));
            }
        }
    }
    all.sort();
    all.dedup();
    assert!(all.is_empty(), "{}", all.join("\n"));
}

/// Schätzt die Komplexität nach Linears Regel (Feld 0,1 Punkte, Objekt 1 Punkt, Verbindung
/// mal `first`; Variablen werden aufgelöst).
fn cost(schema: &Schema, set: &apollo_compiler::executable::SelectionSet, first: f64) -> f64 {
    use apollo_compiler::ast::Value;
    use apollo_compiler::executable::Selection;
    let mut total = 0.0;
    for sel in &set.selections {
        let Selection::Field(f) = sel else {
            panic!("fragments and inline fragments are not used by the adapter");
        };
        if f.name.as_str() == "__typename" {
            continue;
        }
        let ty = f.ty().inner_named_type();
        let is_object = schema
            .types
            .get(ty)
            .is_some_and(|t| t.is_object() || t.is_interface() || t.is_union());
        if !is_object {
            total += 0.1;
            continue;
        }
        let mut mult = 1.0;
        if ty.as_str().ends_with("Connection") {
            mult = 50.0; // Standard ohne `first`
            if let Some(a) = f.arguments.iter().find(|a| a.name.as_str() == "first") {
                mult = match a.value.as_ref() {
                    Value::Int(i) => i.try_to_f64().unwrap_or(50.0),
                    Value::Variable(_) => first,
                    _ => 50.0,
                };
            }
        }
        total += mult * (1.0 + cost(schema, &f.selection_set, first));
    }
    total
}

#[test]
fn every_entity_query_stays_far_below_the_complexity_limit() {
    let schema = schema();
    let mut over = Vec::new();
    for e in SourceEntity::ALL {
        let body = build_query(&PageQuery {
            entity: e,
            cursor: None,
            updated_after: None,
            limit: 100,
            scope: Scope::default(),
        });
        let first = body["variables"]["first"].as_f64().unwrap();
        let doc = ExecutableDocument::parse_and_validate(
            &schema,
            body["query"].as_str().unwrap(),
            e.as_str(),
        )
        .expect("valid");
        let op = doc.operations.get(None).expect("anonymous operation");
        let points = cost(&schema, &op.selection_set, first);
        // Linear lehnt ab 10 000 ab; wir halten die Hälfte als Sicherheitsabstand ein.
        if points > 5_000.0 {
            over.push(format!(
                "{}: {points:.0} points at first={first}",
                e.as_str()
            ));
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
}
