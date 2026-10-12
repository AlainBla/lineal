//! `lineal create-workspace`: Ersteinrichtung (Workspace + Owner) ohne bekannte Passwörter.
//!
//! Das Passwort wird nie als Argument übergeben (Prozessliste, Shell-History): entweder aus
//! `LINEAL_OWNER_PASSWORD` oder interaktiv ohne Echo, mit Wiederholung. Das Kommando ist
//! Betreiberzugang und von `LINEAL_WORKSPACE_CREATION` ausgenommen.

use anyhow::{Result, bail};
use clap::Args;
use lineal_app::App;
use lineal_app::usecases::org::CreateWorkspaceInput;
use std::io::IsTerminal;

#[derive(Debug, Args)]
pub struct CreateWorkspaceArgs {
    /// Anzeigename des Workspace.
    #[arg(long)]
    pub name: String,
    /// URL-Key (3-32 Zeichen a-z, 0-9, -); Standard: aus dem Namen abgeleitet.
    #[arg(long)]
    pub url_key: Option<String>,
    /// E-Mail-Adresse des Owners (Login).
    #[arg(long)]
    pub owner_email: String,
    /// Name des Owners.
    #[arg(long)]
    pub owner_name: String,
}

/// Legt Workspace und Owner an und liefert die Login-URL.
pub async fn execute(app: &App, args: &CreateWorkspaceArgs, password: String) -> Result<String> {
    let created = app
        .create_workspace(CreateWorkspaceInput {
            name: args.name.clone(),
            url_key: args.url_key.clone(),
            owner_name: args.owner_name.clone(),
            owner_email: args.owner_email.clone(),
            owner_password: Some(password),
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(format!(
        "Workspace {:?} (urlKey {}) created with owner {} (user id {}).\nLogin: {}/login\n\
         To let this user create further workspaces via GraphQL, set \
         LINEAL_WORKSPACE_CREATION=admins and add the user id to LINEAL_INSTANCE_ADMINS.",
        created.org.name,
        created.org.url_key,
        created.owner.email,
        created.owner.id.0,
        app.config.base_url.trim_end_matches('/')
    ))
}

/// Passwort aus der Umgebung oder interaktiv (ohne Echo, zweimal).
fn read_password(env: Option<String>) -> Result<String> {
    if let Some(p) = env.filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    if !std::io::stdin().is_terminal() {
        bail!("no password: set LINEAL_OWNER_PASSWORD or run in an interactive terminal");
    }
    let first = rpassword::prompt_password("Owner password: ")?;
    let second = rpassword::prompt_password("Repeat password: ")?;
    if first != second {
        bail!("passwords do not match");
    }
    Ok(first)
}

pub async fn run(args: CreateWorkspaceArgs) -> Result<()> {
    let (_config, _pool, app) = super::connect().await?;
    let password = read_password(std::env::var("LINEAL_OWNER_PASSWORD").ok())?;
    println!("{}", execute(&app, &args, password).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_from_env_is_used_without_prompt() {
        assert_eq!(
            read_password(Some("s3cret-pass".into())).unwrap(),
            "s3cret-pass"
        );
    }
}
