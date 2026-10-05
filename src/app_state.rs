use crate::config::Config;
use anyhow::Result;
use std::time::Duration;

/// Everything a request handler or harvest worker needs. Cheap to share via `Arc`.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let http = http_client(&config.user_agent)?;
        Ok(Self { config, http })
    }
}

/// The one outbound HTTP client. All targets are fixed Wikimedia endpoints or
/// wiki hosts that passed [`crate::wiki::site`] validation, so no SSRF resolver is needed.
pub fn http_client(user_agent: &str) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .gzip(true)
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}
