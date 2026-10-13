//! `lineal import …` (WP18, docs/13 §6): Import aus Linear für große Workspaces ohne Browser-Timeouts.
//!
//! ```text
//! lineal import linear --org acme --token-env LINEAL_LINEAR_KEY --teams ENG,DES --dry-run
//! lineal import linear --org acme --token-env LINEAL_LINEAR_KEY --teams KOL --projects "Projekt 01,Projekt 02" \
//!     --target-team KOL=KP --dry-run
//! lineal import linear --org acme --token-env LINEAL_LINEAR_KEY --resume <runId>
//! lineal import linear-csv --org acme --file export.csv --team ENG
//! ```
//!
//! Das Token wird nur aus einer Umgebungsvariablen gelesen (nie als Argument) und nie ausgegeben.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use lineal_app::App;
use lineal_app::RequestCtx;
use lineal_app::ports::repos::import::ImportRun;
use lineal_app::usecases::import::{ImportScanInput, ImportStartInput, Secret};
use lineal_domain::ids::OrgId;
use lineal_import::engine::{ImportEngine, RunOutcome};
use lineal_import::{AppBlobSink, CsvSource, ImportSource, LinearSource, SourceEntity};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct ImportArgs {
    #[command(subcommand)]
    pub source: ImportSourceCmd,
}

#[derive(Debug, Subcommand)]
pub enum ImportSourceCmd {
    /// Aus der Linear-GraphQL-API importieren.
    Linear(LinearArgs),
    /// Aus einem Linear-CSV-Export importieren (nur Issues, Basisfelder).
    LinearCsv(CsvArgs),
}

#[derive(Debug, Args)]
pub struct LinearArgs {
    /// URL-Key des Ziel-Workspace in Lineal.
    #[arg(long)]
    pub org: String,
    /// Name der Umgebungsvariable mit dem Linear-API-Key.
    #[arg(long, default_value = "LINEAR_API_KEY")]
    pub token_env: String,
    /// Linear-Team-Keys (kommagetrennt); leer = alle.
    #[arg(long, value_delimiter = ',')]
    pub teams: Vec<String>,
    /// Ziel-Keys bei Kollisionen: `ENG=ENGX` (mehrfach oder kommagetrennt).
    #[arg(long, value_delimiter = ',')]
    pub key_map: Vec<String>,
    /// Nur diese Linear-Projekte (Name oder ID, kommagetrennt); leer = alle Projekte der Teams.
    #[arg(long, value_delimiter = ',')]
    pub projects: Vec<String>,
    /// In ein bestehendes Lineal-Team importieren statt ein neues anzulegen: `KOL=KP`
    /// (Linear-Team-Key = Key des Lineal-Teams; mehrfach oder kommagetrennt).
    #[arg(long, value_delimiter = ',')]
    pub target_team: Vec<String>,
    /// Probelauf: Mapping und Validierung ohne Schreiben.
    #[arg(long)]
    pub dry_run: bool,
    /// Nur Änderungen seit dem letzten erfolgreichen Lauf.
    #[arg(long)]
    pub delta: bool,
    /// Einen pausierten/fehlgeschlagenen Lauf fortsetzen.
    #[arg(long)]
    pub resume: Option<Uuid>,
}

#[derive(Debug, Args)]
pub struct CsvArgs {
    #[arg(long)]
    pub org: String,
    /// CSV-Datei (Linear-Export).
    #[arg(long)]
    pub file: PathBuf,
    /// Team-Key, auf den der Import beschränkt wird.
    #[arg(long)]
    pub team: String,
    /// Ziel-Key bei Kollision (`ENG=ENGX`).
    #[arg(long, value_delimiter = ',')]
    pub key_map: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: ImportArgs) -> Result<()> {
    let (_config, pool, app) = super::connect().await?;
    match args.source {
        ImportSourceCmd::Linear(a) => linear(&pool, &app, a).await,
        ImportSourceCmd::LinearCsv(a) => csv(&pool, &app, a).await,
    }
}

async fn org_by_key(pool: &sqlx::PgPool, key: &str) -> Result<OrgId> {
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM organization WHERE url_key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .context("organization lookup failed")?;
    id.map(OrgId)
        .ok_or_else(|| anyhow!("no workspace with url key `{key}`"))
}

fn key_map(pairs: &[String]) -> Result<BTreeMap<String, String>> {
    let mut m = BTreeMap::new();
    for p in pairs {
        let (a, b) = p
            .split_once('=')
            .ok_or_else(|| anyhow!("--key-map expects SOURCE=TARGET, got `{p}`"))?;
        m.insert(a.trim().to_owned(), b.trim().to_owned());
    }
    Ok(m)
}

/// Lineal-Team per Key (aktuell, nicht gelöscht) im Workspace.
async fn team_by_key(pool: &sqlx::PgPool, org: OrgId, key: &str) -> Result<Uuid> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM team WHERE organization_id = $1 AND upper(key) = upper($2) AND NOT trashed",
    )
    .bind(org.0)
    .bind(key)
    .fetch_optional(pool)
    .await
    .context("team lookup failed")?;
    id.ok_or_else(|| anyhow!("no Lineal team with key `{key}`"))
}

/// Projektangaben (Name ohne Beachtung der Schreibweise oder ID) gegen den Scan auflösen.
fn resolve_projects(run: &ImportRun, teams: &[String], wanted: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let all = run.stats["scan"]["teams"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for w in wanted {
        let mut hits: Vec<String> = Vec::new();
        for t in &all {
            let key = t["key"].as_str().unwrap_or("");
            if !teams.is_empty() && !teams.iter().any(|k| k.eq_ignore_ascii_case(key)) {
                continue;
            }
            for p in t["projects"].as_array().into_iter().flatten() {
                let id = p["id"].as_str().unwrap_or("");
                let name = p["name"].as_str().unwrap_or("");
                if (id == w || name.eq_ignore_ascii_case(w)) && !hits.iter().any(|h| h == id) {
                    hits.push(id.to_owned());
                }
            }
        }
        match hits.len() {
            0 => bail!("no project `{w}` in the selected teams"),
            1 => out.push(hits.remove(0)),
            _ => bail!("project name `{w}` is ambiguous; use the project id"),
        }
    }
    Ok(out)
}

fn app_err(e: lineal_app::AppError) -> anyhow::Error {
    // Fehlertexte der Use-Cases enthalten nie Zugangsdaten.
    anyhow!("{} ({})", e.message, e.details)
}

async fn linear(pool: &sqlx::PgPool, app: &App, a: LinearArgs) -> Result<()> {
    let org = org_by_key(pool, &a.org).await?;
    let ctx = RequestCtx::system(org, "cli-import");
    let raw = std::env::var(&a.token_env)
        .with_context(|| format!("environment variable `{}` is not set", a.token_env))?;
    let token = Secret::new(&raw).map_err(app_err)?;
    let source = LinearSource::from_env(&token).map_err(|e| anyhow!("{e}"))?;
    let sink = AppBlobSink { app };
    let engine = ImportEngine::new(app, &source, &sink);

    let run_id = if let Some(id) = a.resume {
        let run = app.import_get_run(&ctx, id).await.map_err(app_err)?;
        if matches!(run.status.as_str(), "paused" | "failed") {
            app.import_resume(&ctx, id).await.map_err(app_err)?;
        }
        id
    } else {
        let run = app
            .import_scan(
                &ctx,
                ImportScanInput {
                    source: "linear".into(),
                    token: raw.clone(),
                },
            )
            .await
            .map_err(app_err)?;
        engine.scan(org, run.id).await.map_err(app_err)?;
        let planned = app.import_get_run(&ctx, run.id).await.map_err(app_err)?;
        if planned.status != "planned" {
            bail!("scan failed: {}", planned.error.unwrap_or_default());
        }
        print_plan(&planned);
        let mut input = ImportStartInput {
            run_id: run.id,
            teams: a.teams.clone(),
            dry_run: a.dry_run,
            delta: a.delta,
            ..Default::default()
        };
        input.key_mapping = key_map(&a.key_map)?;
        input.projects = resolve_projects(&planned, &a.teams, &a.projects)?;
        for (linear_key, team_key) in key_map(&a.target_team)? {
            let id = team_by_key(pool, org, &team_key).await?;
            input.target_teams.insert(linear_key.to_uppercase(), id);
        }
        app.import_start(&ctx, input).await.map_err(app_err)?;
        run.id
    };
    execute(app, &engine, org, &ctx, run_id).await
}

async fn csv(pool: &sqlx::PgPool, app: &App, a: CsvArgs) -> Result<()> {
    let org = org_by_key(pool, &a.org).await?;
    let ctx = RequestCtx::system(org, "cli-import");
    let bytes =
        std::fs::read(&a.file).with_context(|| format!("cannot read {}", a.file.display()))?;
    let source = CsvSource::from_csv(&bytes, Some(&a.team)).map_err(|e| anyhow!(e))?;
    for w in source.warnings() {
        eprintln!("warning: {w}");
    }
    let teams = source
        .collect(SourceEntity::Teams, None, 10)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let scan = json!({ "teams": teams.iter().map(|t| json!({ "id": t["id"], "key": t["key"], "name": t["name"] })).collect::<Vec<_>>() });
    let run = app
        .import_register_scanned(&ctx, "linearCsv", scan)
        .await
        .map_err(app_err)?;
    let mut input = ImportStartInput {
        run_id: run.id,
        teams: vec![a.team.to_uppercase()],
        dry_run: a.dry_run,
        ..Default::default()
    };
    input.key_mapping = key_map(&a.key_map)?;
    app.import_start(&ctx, input).await.map_err(app_err)?;
    let sink = AppBlobSink { app };
    let engine = ImportEngine::new(app, &source, &sink);
    execute(app, &engine, org, &ctx, run.id).await
}

fn print_plan(run: &ImportRun) {
    let scan = &run.stats["scan"];
    println!(
        "Workspace: {}",
        scan["workspace"]["name"].as_str().unwrap_or("?")
    );
    if let Some(teams) = scan["teams"].as_array() {
        for t in teams {
            println!(
                "  team {:<8} {:<30} issues: {}{}",
                t["key"].as_str().unwrap_or("?"),
                t["name"].as_str().unwrap_or(""),
                t["issueCount"],
                if t["conflict"].as_bool().unwrap_or(false) {
                    "  KEY CONFLICT"
                } else {
                    ""
                }
            );
            for p in t["projects"].as_array().into_iter().flatten() {
                println!(
                    "      project {} ({}) status: {} issues: {}",
                    p["name"].as_str().unwrap_or("?"),
                    p["id"].as_str().unwrap_or("?"),
                    p["status"].as_str().unwrap_or("?"),
                    p["issueCount"]
                );
            }
        }
    }
}

async fn execute(
    app: &App,
    engine: &ImportEngine<'_>,
    org: OrgId,
    ctx: &RequestCtx,
    run_id: Uuid,
) -> Result<()> {
    let outcome = engine.run(org, run_id).await.map_err(app_err)?;
    let run = app.import_get_run(ctx, run_id).await.map_err(app_err)?;
    println!("run {run_id}: {:?} (status {})", outcome, run.status);
    if let Some(entities) = run.stats["entities"].as_object() {
        for (name, c) in entities {
            println!(
                "  {name:<18} created {} updated {} unchanged {} skipped {} conflicts {} errors {}",
                c["created"],
                c["updated"],
                c["unchanged"],
                c["skipped"],
                c["conflicts"],
                c["errors"]
            );
        }
    }
    if let Some(plan) = run.stats["plan"].as_object()
        && !plan.is_empty()
    {
        println!("  plan:");
        for (k, v) in plan {
            println!("    {k:<20} {v}");
        }
    }
    let (_, counts) = app
        .import_report(ctx, run_id, None, 1, 0)
        .await
        .map_err(app_err)?;
    for (sev, n) in counts {
        println!("  report {sev}: {n}");
    }
    if let RunOutcome::Failed(msg) = outcome {
        bail!("import failed: {msg} (resume with --resume {run_id})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned_run() -> ImportRun {
        let now = chrono::Utc::now();
        ImportRun {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            source: "linear".into(),
            status: "planned".into(),
            options: json!({}),
            checkpoint: json!({}),
            stats: json!({ "scan": { "teams": [
                { "key": "KOL", "projects": [
                    { "id": "p-1", "name": "Alpha" }, { "id": "p-2", "name": "Beta" } ] },
                { "key": "PLN", "projects": [
                    { "id": "p-1", "name": "Alpha" }, { "id": "p-3", "name": "Beta" } ] },
            ] } }),
            started_by: None,
            started_at: None,
            finished_at: None,
            error: None,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn projects_resolve_by_name_or_id_within_the_selected_teams() {
        let run = planned_run();
        let teams = vec!["KOL".to_owned()];
        assert_eq!(
            resolve_projects(&run, &teams, &["alpha".into(), "p-2".into()]).unwrap(),
            vec!["p-1", "p-2"]
        );
        // Ein Projekt über mehrere Teams zählt einmal; gleiche Namen in verschiedenen Projekten sind mehrdeutig.
        assert_eq!(
            resolve_projects(&run, &[], &["Alpha".into()]).unwrap(),
            vec!["p-1"]
        );
        assert!(resolve_projects(&run, &[], &["Beta".into()]).is_err());
        assert!(resolve_projects(&run, &teams, &["p-3".into()]).is_err());
    }
}
