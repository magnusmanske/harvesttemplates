//! Typed configuration, loaded from a JSON file that is never committed.
//! See `config.json.template` for the shape.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A string that never shows up in `Debug` output or logs.
#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    pub tool_db: DbConfig,
    pub replicas: ReplicaConfig,
    pub oauth: OauthConfig,
    #[serde(default)]
    pub harvest: HarvestConfig,
    /// Sent with every outbound request, per the Wikimedia User-Agent policy.
    pub user_agent: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub html_dir: PathBuf,
    pub session_dir: PathBuf,
    pub session_lifetime_days: i64,
    /// Set to `false` only for plain-http local development.
    pub cookie_secure: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            html_dir: "html".into(),
            session_dir: "sessions".into(),
            session_lifetime_days: 30,
            cookie_secure: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DbConfig {
    /// `mysql://user:password@host:port/database`
    pub url: Secret,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

const fn default_max_connections() -> usize {
    4
}

/// Wiki replicas. The host is derived from the wiki's dbname, so one entry
/// covers all ~900 wikis. `overrides` maps a dbname to `host:port`, which is
/// how SSH tunnels are wired up for local development.
#[derive(Debug, Clone, Deserialize)]
pub struct ReplicaConfig {
    pub host_pattern: String,
    #[serde(default = "default_mysql_port")]
    pub port: u16,
    pub user: String,
    pub password: Secret,
    #[serde(default)]
    pub overrides: HashMap<String, String>,
    #[serde(default = "default_max_connections")]
    pub max_connections_per_wiki: usize,
}

const fn default_mysql_port() -> u16 {
    3306
}

#[derive(Debug, Clone, Deserialize)]
pub struct OauthConfig {
    pub consumer_key: String,
    pub consumer_secret: Secret,
    /// Absolute URL of `/api/auth/callback` as registered with the consumer.
    pub callback_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HarvestConfig {
    /// Minimum delay between two edits of the same run.
    pub edit_interval_ms: u64,
    pub max_active_runs_per_user: usize,
    pub max_candidates: usize,
    pub max_category_depth: u32,
    pub max_categories: usize,
}

impl Default for HarvestConfig {
    fn default() -> Self {
        Self {
            edit_interval_ms: 1000,
            max_active_runs_per_user: 2,
            max_candidates: 500_000,
            max_category_depth: 30,
            max_categories: 20_000,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("invalid config file {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let text = include_str!("../config.json.template");
        let cfg: Config = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.replicas.port, 3306);
        assert!(cfg.server.cookie_secure);
    }

    #[test]
    fn secrets_are_redacted_in_debug() {
        let text = include_str!("../config.json.template");
        let cfg: Config = serde_json::from_str(text).unwrap();
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("PASSWORD"), "{dbg}");
        assert!(!dbg.contains("CONSUMER_SECRET"), "{dbg}");
    }
}
