use crate::auth::{FileSessionStore, OAuth};
use crate::config::Config;
use crate::wiki::{ApiSource, MwApi, PageSource, Replicas, WithFallback};
use crate::wikidata::{Wdqs, Wikidata, wdqs};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// Everything a request handler or harvest worker needs. Shared via `Arc`.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub mw: MwApi,
    pub wikidata: Wikidata,
    pub wikidata_api_url: String,
    pub wdqs: Wdqs,
    pub pages: Arc<dyn PageSource>,
    pub oauth: OAuth,
    pub sessions: FileSessionStore,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let http = http_client(&config.user_agent)?;
        let mw = MwApi::new(http.clone());
        let pages = WithFallback {
            primary: Replicas::new(config.replicas.clone()),
            fallback: ApiSource { api: mw.clone() },
        };
        let sessions =
            FileSessionStore::new(config.server.session_dir.clone()).with_context(|| {
                format!(
                    "cannot create session directory {}",
                    config.server.session_dir.display()
                )
            })?;
        Ok(Self {
            wikidata: Wikidata { api: mw.clone() },
            wikidata_api_url: crate::auth::edit::WIKIDATA_API.to_string(),
            wdqs: Wdqs::new(http.clone(), wdqs::ENDPOINT),
            pages: Arc::new(pages),
            oauth: OAuth::new(http, &config.oauth),
            sessions,
            mw,
            config,
        })
    }
}

/// The one outbound HTTP client. All targets are fixed Wikimedia endpoints or
/// wiki hosts built by [`crate::wiki::site::host_for`], so no SSRF resolver is needed.
pub fn http_client(user_agent: &str) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .gzip(true)
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}
