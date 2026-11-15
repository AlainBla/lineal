//! `lineal totp reset --user <email>` (LIN-70): setzt das TOTP eines Nutzers auf
//! dem Host zurueck — der letzte Ausweg, wenn Geraet und Recovery-Codes verloren
//! sind und kein zweiter Owner reset kann. Loescht Secret, Recovery-Codes und
//! Schonfrist mandantengebunden.

use anyhow::Result;
use clap::{Args, Subcommand};
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct TotpArgs {
    #[command(subcommand)]
    pub command: TotpCommand,
}

#[derive(Debug, Subcommand)]
pub enum TotpCommand {
    /// TOTP (2FA) eines Nutzers zuruecksetzen.
    Reset {
        /// E-Mail des Nutzers.
        #[arg(long)]
        user: String,
    },
}

pub async fn run(args: TotpArgs) -> Result<()> {
    match args.command {
        TotpCommand::Reset { user } => reset(&user).await,
    }
}

async fn reset(email: &str) -> Result<()> {
    let (_config, pool, _app) = super::connect(false).await?;
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, organization_id FROM \"user\" WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_all(&pool)
            .await?;
    if rows.is_empty() {
        println!("Kein Nutzer mit E-Mail {email} gefunden.");
        return Ok(());
    }
    let mut reset_count = 0u64;
    for (user_id, org_id) in &rows {
        sqlx::query("DELETE FROM user_recovery_code WHERE user_id = $1 AND organization_id = $2")
            .bind(user_id)
            .bind(org_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM user_totp_grace WHERE user_id = $1 AND organization_id = $2")
            .bind(user_id)
            .bind(org_id)
            .execute(&pool)
            .await?;
        let r = sqlx::query("DELETE FROM user_totp WHERE user_id = $1 AND organization_id = $2")
            .bind(user_id)
            .bind(org_id)
            .execute(&pool)
            .await?;
        if r.rows_affected() > 0 {
            reset_count += 1;
        }
    }
    println!("TOTP fuer {reset_count} Konto(en) mit E-Mail {email} zurueckgesetzt.");
    Ok(())
}
