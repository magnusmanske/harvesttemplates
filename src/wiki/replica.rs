use super::pages::{CategoryStep, Limits, Page, PageSource};
use super::site::{NS_CATEGORY, Site};
use crate::config::ReplicaConfig;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::path::Path;
use std::time::Duration;
use wikimisc::mysql_async::Conn;
use wikimisc::mysql_async::prelude::Queryable;
use wikimisc::replicas::Replicas as SharedReplicas;
use wikimisc::toolforge::db::Cluster;

const MAX_STATEMENT_TIME: Duration = Duration::from_secs(300);
const IN_LIST_CHUNK: usize = 500;

/// The tables each query reads, so the connection goes to the database that has
/// up-to-date copies of them: some wikis moved tables to extension databases,
/// e.g. Commons its links tables, and left frozen copies behind.
const TRANSCLUSION_TABLES: &[&str] = &["linktarget", "templatelinks", "page", "page_props"];
const CATEGORY_TABLES: &[&str] = &["linktarget", "categorylinks", "page"];

/// Connections to the wiki replicas. Commons keeps `templatelinks` and
/// `page_props` in different databases, so its transclusions fail here and
/// come from the API instead (see `WithFallback`).
#[derive(Debug)]
pub struct Replicas(SharedReplicas);

impl Replicas {
    /// `credentials` is the `replica.my.cnf` with the database user and password.
    pub fn new(config: &ReplicaConfig, credentials: &Path) -> Result<Self> {
        let replicas = SharedReplicas::builder(Cluster::Analytics)
            .my_cnf_path(credentials)
            .pool_max(config.max_connections)
            .max_statement_time(MAX_STATEMENT_TIME)
            .build()
            .with_context(|| format!("cannot read the replica credentials from {}", credentials.display()))?;
        Ok(Self(replicas))
    }

    /// A connection to the database of `dbname` that has all of `tables`.
    async fn conn(&self, dbname: &str, tables: &[&str]) -> Result<Conn> {
        self.0.conn_for_dbname(dbname, tables).await
    }
}

const TRANSCLUSIONS_SQL: &str = "SELECT page_id, page_title, page_latest, pp_value
    FROM linktarget
    JOIN templatelinks ON tl_target_id = lt_id
    JOIN page ON page_id = tl_from
    LEFT JOIN page_props ON pp_page = page_id AND pp_propname = 'wikibase_item'
    WHERE lt_namespace = 10 AND lt_title = ? AND tl_from_namespace = ?
    LIMIT ?";

const CATEGORY_SQL: &str = "SELECT page_id, page_namespace, page_title
    FROM linktarget
    JOIN categorylinks ON cl_target_id = lt_id
    JOIN page ON page_id = cl_from
    WHERE lt_namespace = 14 AND lt_title IN ({}) AND (page_namespace = ? OR cl_type = 'subcat')";

#[async_trait]
impl PageSource for Replicas {
    async fn transclusions(&self, site: &Site, template: &str, namespace: i32, limits: Limits) -> Result<Vec<Page>> {
        let mut conn = self.conn(&site.dbname, TRANSCLUSION_TABLES).await?;
        let rows: Vec<(u64, Vec<u8>, u64, Option<Vec<u8>>)> =
            conn.exec(TRANSCLUSIONS_SQL, (template, namespace, limits.max_pages + 1)).await?;
        if rows.len() > limits.max_pages {
            bail!("the template is used on more than {} pages; add a category filter", limits.max_pages);
        }
        let mut pages: Vec<Page> = rows
            .into_iter()
            .map(|(id, title, latest_revision, item)| Page {
                id,
                title: site.full_title(namespace, &String::from_utf8_lossy(&title)),
                item: item.and_then(|q| String::from_utf8_lossy(&q).parse().ok()),
                latest_revision,
            })
            .collect();
        pages.sort_by_key(|p| std::cmp::Reverse(p.latest_revision));
        Ok(pages)
    }

    async fn category_step(&self, site: &Site, categories: &[String], namespace: i32) -> Result<CategoryStep> {
        let mut conn = self.conn(&site.dbname, CATEGORY_TABLES).await?;
        let mut step = CategoryStep::default();
        for chunk in categories.chunks(IN_LIST_CHUNK) {
            let sql = CATEGORY_SQL.replace("{}", &vec!["?"; chunk.len()].join(","));
            let mut args: Vec<wikimisc::mysql_async::Value> = chunk.iter().map(|c| c.as_str().into()).collect();
            args.push(namespace.into());
            let rows: Vec<(u64, i32, Vec<u8>)> = conn.exec(sql, args).await?;
            for (id, ns, title) in rows {
                if ns == namespace {
                    step.pages.push(id);
                }
                if ns == NS_CATEGORY {
                    step.subcategories.push(String::from_utf8_lossy(&title).into_owned());
                }
            }
        }
        Ok(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::wiki::{ApiSource, MwApi, Site};

    const LIMITS: Limits = Limits { max_pages: 200_000, max_depth: 5, max_categories: 1000 };

    #[test]
    fn queries_go_to_the_databases_with_live_tables() {
        use wikimisc::toolforge::db::ExtensionDb;
        // Commons moved its links tables, and left frozen copies behind
        assert_eq!(ExtensionDb::for_tables("commonswiki", CATEGORY_TABLES).unwrap(), Some(ExtensionDb::Links));
        // page_props stayed in the core database, so this falls back to the API
        assert!(ExtensionDb::for_tables("commonswiki", TRANSCLUSION_TABLES).is_err());
        assert_eq!(ExtensionDb::for_tables("enwiki", TRANSCLUSION_TABLES).unwrap(), None);
    }

    /// Needs `config.json` and replica access (on Toolforge, or tunnels in local mode).
    #[tokio::test]
    #[ignore = "requires database / external services — run with cargo test -- --ignored"]
    async fn replica_matches_api_on_enwiki() {
        let config = Config::load("config.json".as_ref()).unwrap();
        let http = wikimisc::http::client(&config.user_agent).unwrap();
        let api = ApiSource { api: MwApi::new(http) };
        let replicas = Replicas::new(&config.replicas, &config.db_credentials).unwrap();
        let site = Site::load(&api.api, "en.wikipedia.org").await.unwrap();

        let template = site.db_key(10, "Template:Coord missing");
        let from_db = replicas.transclusions(&site, &template, 0, LIMITS).await.unwrap();
        let from_api = api.transclusions(&site, &template, 0, LIMITS).await.unwrap();
        assert!(!from_db.is_empty());
        let diff = from_db.len().abs_diff(from_api.len());
        assert!(diff * 100 < from_db.len(), "db {} vs api {}", from_db.len(), from_api.len());
        assert!(from_db.iter().filter(|p| p.item.is_some()).count() * 2 > from_db.len());

        let category = site.db_key(14, "Category:Airports in Berlin");
        let from_db = replicas.category_members(&site, &category, 0, 2, LIMITS).await.unwrap();
        let from_api = api.category_members(&site, &category, 0, 2, LIMITS).await.unwrap();
        assert!(!from_db.is_empty());
        assert_eq!(from_db, from_api);
    }
}
