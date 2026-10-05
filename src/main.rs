use anyhow::Result;
use clap::Parser;
use harvesttemplates::{api, app_state::AppState, config::Config};
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
    state.store.migrate().await?;
    state.store.recover_after_restart().await?;
    api::serve(state, cli.port).await
}
