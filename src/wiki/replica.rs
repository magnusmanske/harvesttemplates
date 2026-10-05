use super::pages::{CategoryStep, Limits, Page, PageSource};
use super::site::{NS_CATEGORY, Site};
use crate::config::{DbUser, ReplicaConfig};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use dashmap::DashMap;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, OptsBuilder, Pool, PoolConstraints, PoolOpts};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IN_LIST_CHUNK: usize = 500;

/// One lazily created connection pool per wiki replica.
pub struct Replicas {
    config: ReplicaConfig,
    user: DbUser,
    pools: DashMap<String, Pool>,
}

impl std::fmt::Debug for Replicas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Replicas").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Replicas {
    pub fn new(config: ReplicaConfig, user: DbUser) -> Self {
        Self { config, user, pools: DashMap::new() }
    }

    async fn conn(&self, dbname: &str) -> Result<Conn> {
        let pool = self.pools.entry(dbname.to_string()).or_insert_with(|| self.new_pool(dbname)).clone();
        tokio::time::timeout(CONNECT_TIMEOUT, pool.get_conn())
            .await
            .with_context(|| format!("timeout connecting to the {dbname} replica"))?
            .with_context(|| format!("cannot connect to the {dbname} replica"))
    }

    fn new_pool(&self, dbname: &str) -> Pool {
        let c = &self.config;
        let (host, port) = match c.overrides.get(dbname).and_then(|o| o.rsplit_once(':')) {
            Some((host, port)) => (host.to_string(), port.parse().unwrap_or(c.port)),
            None => (c.host_pattern.replace("{dbname}", dbname), c.port),
        };
        let constraints = PoolConstraints::new(0, c.max_connections_per_wiki.max(1)).unwrap_or_default();
        let opts = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(port)
            .user(Some(&self.user.name))
            .pass(Some(self.user.password.expose()))
            .db_name(Some(format!("{dbname}_p")))
            .setup(vec!["SET SESSION max_statement_time = 300"])
            .pool_opts(
                PoolOpts::default().with_constraints(constraints).with_inactive_connection_ttl(Duration::from_secs(30)),
            );
        Pool::new(opts)
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
        let mut conn = self.conn(&site.dbname).await?;
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
        let mut conn = self.conn(&site.dbname).await?;
        let mut step = CategoryStep::default();
        for chunk in categories.chunks(IN_LIST_CHUNK) {
            let sql = CATEGORY_SQL.replace("{}", &vec!["?"; chunk.len()].join(","));
            let mut args: Vec<mysql_async::Value> = chunk.iter().map(|c| c.as_str().into()).collect();
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

    /// Needs `config.json` with a replica override (SSH tunnel) for enwiki.
    #[tokio::test]
    #[ignore = "requires database / external services — run with cargo test -- --ignored"]
    async fn replica_matches_api_on_enwiki() {
        let config = Config::load("config.json".as_ref()).unwrap();
        let http = crate::app_state::http_client(&config.user_agent).unwrap();
        let api = ApiSource { api: MwApi::new(http) };
        let replicas = Replicas::new(config.replicas, config.db_user);
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
