use crate::auth::{FileSessionStore, OAuth, TokenCache};
use crate::config::Config;
use crate::harvest::ActiveRuns;
use crate::storage::Store;
use crate::wiki::{ApiSource, Limits, MwApi, PageSource, Replicas, WithFallback};
use crate::wikidata::{Wdqs, Wikidata, wdqs};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// Read-only clients for the wikis, Wikidata and WDQS.
#[derive(Debug, Clone)]
pub struct Clients {
    pub mw: MwApi,
    pub wikidata: Wikidata,
    pub wdqs: Wdqs,
}

impl Clients {
    pub fn new(http: &reqwest::Client) -> Self {
        let mw = MwApi::new(http.clone());
        Self { wikidata: Wikidata { api: mw.clone() }, wdqs: Wdqs::new(http.clone(), wdqs::ENDPOINT), mw }
    }

    /// Everything pointed at one mock server.
    pub fn mocked(url: &str) -> Self {
        let http = reqwest::Client::new();
        let mw = MwApi::with_base_url(http.clone(), url.to_string());
        Self { wikidata: Wikidata { api: mw.clone() }, wdqs: Wdqs::new(http, url), mw }
    }

    pub const fn services(&self) -> crate::constraints::Services<'_> {
        crate::constraints::Services { wdqs: &self.wdqs, mw: &self.mw }
    }
}

/// Everything a request handler or harvest worker needs. Shared via `Arc`.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub clients: Clients,
    pub wikidata_api_url: String,
    pub pages: Arc<dyn PageSource>,
    pub oauth: OAuth,
    pub sessions: FileSessionStore,
    pub store: Store,
    pub runs: Arc<ActiveRuns>,
    pub tokens: Arc<TokenCache>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let http = http_client(&config.user_agent)?;
        let clients = Clients::new(&http);
        let fallback = ApiSource { api: clients.mw.clone() };
        let pages = WithFallback { primary: Replicas::new(config.replicas.clone(), config.db_user.clone()), fallback };
        let session_dir = &config.server.session_dir;
        let sessions = FileSessionStore::new(session_dir.clone())
            .with_context(|| format!("cannot create session directory {}", session_dir.display()))?;
        let store = Store::new(&config.tool_db, &config.db_user);
        Ok(Self {
            store,
            runs: Arc::default(),
            tokens: Arc::default(),
            clients,
            wikidata_api_url: crate::auth::edit::WIKIDATA_API.to_string(),
            pages: Arc::new(pages),
            oauth: OAuth::new(http, &config.oauth),
            sessions,
            config,
        })
    }
}

impl AppState {
    pub const fn limits(&self) -> Limits {
        let h = &self.config.harvest;
        Limits { max_pages: h.max_candidates, max_depth: h.max_category_depth, max_categories: h.max_categories }
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
