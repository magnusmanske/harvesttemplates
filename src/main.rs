use anyhow::Result;
use clap::{Parser, Subcommand};
use harvesttemplates::{api, app_state::AppState, config::Config, legacy_shares};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

/// HarvestTemplates web service.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Path to the JSON configuration file.
    #[arg(long, default_value = "config.json")]
    config: PathBuf,
    /// Port to listen on. Toolforge sets `PORT`.
    #[arg(long, env = "PORT", default_value_t = 8000)]
    port: u16,
    /// Development only: treat every request as logged in as this user (cannot edit).
    #[arg(long)]
    dev_user: Option<String>,
    /// Without a command, run the web service.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Import the old tool's shared queries, keeping their ids. Safe to repeat.
    ImportLegacyShares {
        /// Fetch and check everything, but store nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();
    if let Some(name) = &cli.dev_user {
        harvesttemplates::auth::session::enable_dev_user(name)?;
        tracing::warn!("development mode: everyone is logged in as {name}");
    }
    let config = Config::load(&cli.config)?;
    let state = Arc::new(AppState::new(config)?);
    if let Some(Command::ImportLegacyShares { dry_run }) = cli.command {
        return import_legacy_shares(&state, dry_run).await;
    }
    state.store.migrate().await?;
    state.store.recover_after_restart().await?;
    api::serve(state, cli.port).await
}

async fn import_legacy_shares(state: &AppState, dry_run: bool) -> Result<()> {
    let http = harvesttemplates::app_state::http_client(&state.config.user_agent)?;
    if !dry_run {
        state.store.migrate().await?;
    }
    let report = legacy_shares::import(&http, (!dry_run).then_some(&state.store)).await?;
    println!("listed: {}, imported: {}, already there: {}", report.listed, report.imported, report.already_there);
    for (id, reason) in &report.unusable {
        println!("skipped htid {id}: {reason}");
    }
    Ok(())
}
