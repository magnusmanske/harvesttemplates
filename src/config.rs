//! Typed configuration, loaded from a JSON file that is never committed.
//! See `config.json.template` for the shape.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A string that never shows up in `Debug` output or logs.
/// Serialisable because OAuth tokens live in (owner-only) session files.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    /// MySQL `user`/`password` in `my.cnf` format, as Toolforge provides them.
    /// Relative paths are relative to the config file.
    #[serde(default = "default_db_credentials")]
    pub db_credentials: PathBuf,
    pub tool_db: DbConfig,
    pub replicas: ReplicaConfig,
    /// OAuth 2 client: `application_key`, `application_secret`, `callback_url`.
    #[serde(default = "default_oauth_file")]
    pub oauth_file: PathBuf,
    #[serde(default)]
    pub harvest: HarvestConfig,
    /// Sent with every outbound request, per the Wikimedia User-Agent policy.
    pub user_agent: String,
    /// Read from `db_credentials`.
    #[serde(skip)]
    pub db_user: DbUser,
    /// Read from `oauth_file`.
    #[serde(skip)]
    pub oauth: OauthConfig,
}

fn default_db_credentials() -> PathBuf {
    "replica.my.cnf".into()
}

fn default_oauth_file() -> PathBuf {
    "oauth.ini".into()
}

#[derive(Debug, Clone, Default)]
pub struct DbUser {
    pub name: String,
    pub password: Secret,
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
    pub host: String,
    #[serde(default = "default_mysql_port")]
    pub port: u16,
    /// `{user}` is replaced by the database user, e.g. `{user}__harvesttemplates`.
    pub database: String,
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
    #[serde(default)]
    pub overrides: HashMap<String, String>,
    #[serde(default = "default_max_connections")]
    pub max_connections_per_wiki: usize,
}

const fn default_mysql_port() -> u16 {
    3306
}

#[derive(Debug, Clone, Default)]
pub struct OauthConfig {
    pub client_id: String,
    pub client_secret: Secret,
    /// As registered with the OAuth 2 client; its path is served by the login callback.
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
    /// Load the config and the credential files it points to.
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("cannot read config file {}", path.display()))?;
        let mut config: Self =
            serde_json::from_str(&text).with_context(|| format!("invalid config file {}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let db = read_key_values(&dir.join(&config.db_credentials))?;
        config.db_user = DbUser {
            name: required(&db, "user")?,
            password: Secret(required(&db, "password")?),
        };
        config.tool_db.database = config.tool_db.database.replace("{user}", &config.db_user.name);
        let oauth = read_key_values(&dir.join(&config.oauth_file))?;
        config.oauth = OauthConfig {
            client_id: required(&oauth, "application_key")?,
            client_secret: Secret(required(&oauth, "application_secret")?),
            callback_url: required(&oauth, "callback_url")?,
        };
        Ok(config)
    }
}

/// `key = value` lines of an ini/cnf file. Sections and comments are ignored,
/// surrounding quotes removed.
fn read_key_values(path: &Path) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with(['#', ';', '[']))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches(['"', '\'']).to_string()))
        .collect())
}

fn required(values: &HashMap<String, String>, key: &str) -> Result<String> {
    values
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
        .with_context(|| format!("credentials file lacks '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_and_credential_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, include_str!("../config.json.template")).unwrap();
        std::fs::write(
            dir.path().join("replica.my.cnf"),
            "[client]\nuser = s1234\npassword = 'pw=1'\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("oauth.ini"),
            "; OAuth 2\napplication_key=abc\napplication_secret=\"def\"\ncallback_url=\"https://x.toolforge.org/callback\"\n",
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.db_user.name, "s1234");
        assert_eq!(cfg.db_user.password.expose(), "pw=1");
        assert_eq!(cfg.tool_db.database, "s1234__harvesttemplates");
        assert_eq!(
            (cfg.oauth.client_id.as_str(), cfg.oauth.client_secret.expose()),
            ("abc", "def")
        );
        assert_eq!(cfg.oauth.callback_url, "https://x.toolforge.org/callback");
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("pw=1") && !dbg.contains("def"), "{dbg}");
    }

    #[test]
    fn missing_credentials_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, include_str!("../config.json.template")).unwrap();
        let err = Config::load(&path).unwrap_err().to_string();
        assert!(err.contains("replica.my.cnf"), "{err}");
    }
}
