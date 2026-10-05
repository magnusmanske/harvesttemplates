//! Where candidate pages come from: the wiki replicas, or the API as a fallback.

use super::api::{MwApi, params};
use super::site::{NS_CATEGORY, NS_TEMPLATE, Site};
use crate::ids::ItemId;
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub id: u64,
    /// Full title with namespace prefix and spaces.
    pub title: String,
    pub item: Option<ItemId>,
    pub latest_revision: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_pages: usize,
    pub max_depth: u32,
    pub max_categories: usize,
}

/// Pages and subcategories directly in a set of categories.
#[derive(Debug, Default)]
pub struct CategoryStep {
    pub pages: Vec<u64>,
    /// Database keys, e.g. `Films_by_year`.
    pub subcategories: Vec<String>,
}

#[async_trait]
pub trait PageSource: Send + Sync + std::fmt::Debug {
    /// Pages in `namespace` transcluding the template (directly or through a
    /// redirect), most recently edited first. `template` is a database key.
    async fn transclusions(&self, site: &Site, template: &str, namespace: i32, limits: Limits) -> Result<Vec<Page>>;

    /// Members of the given categories (database keys).
    async fn category_step(&self, site: &Site, categories: &[String], namespace: i32) -> Result<CategoryStep>;

    /// Ids of pages in `namespace` in `category` or its subcategories, down to
    /// `depth` levels. Each category is visited once, so cycles are harmless.
    async fn category_members(
        &self,
        site: &Site,
        category: &str,
        namespace: i32,
        depth: u32,
        limits: Limits,
    ) -> Result<HashSet<u64>> {
        let depth = depth.min(limits.max_depth);
        let mut visited: HashSet<String> = HashSet::from([category.to_string()]);
        let mut level = vec![category.to_string()];
        let mut pages = HashSet::new();
        for current_depth in 0..=depth {
            let step = self.category_step(site, &level, namespace).await?;
            pages.extend(step.pages);
            if pages.len() > limits.max_pages {
                bail!("the category tree has more than {} pages; use a smaller depth", limits.max_pages);
            }
            if current_depth == depth {
                break;
            }
            level = step.subcategories.into_iter().filter(|c| visited.insert(c.clone())).collect();
            if visited.len() > limits.max_categories {
                bail!("the category tree has more than {} categories; use a smaller depth", limits.max_categories);
            }
            if level.is_empty() {
                break;
            }
        }
        Ok(pages)
    }
}

/// Reads from the MediaWiki API. Slower than the replicas but needs no database access.
#[derive(Debug, Clone)]
pub struct ApiSource {
    pub api: MwApi,
}

#[async_trait]
impl PageSource for ApiSource {
    async fn transclusions(&self, site: &Site, template: &str, namespace: i32, limits: Limits) -> Result<Vec<Page>> {
        let ns = namespace.to_string();
        let title = site.full_title(NS_TEMPLATE, template);
        let p = params(&[
            ("action", "query"),
            ("generator", "transcludedin"),
            ("titles", &title),
            ("gtinamespace", &ns),
            ("gtilimit", "max"),
            ("prop", "pageprops|info"),
            ("ppprop", "wikibase_item"),
        ]);
        let mut pages = Vec::new();
        self.api
            .query_continue(&site.host, &p, |json| {
                pages.extend(json["query"]["pages"].as_array().into_iter().flatten().filter_map(page_from_api));
                pages.len() <= limits.max_pages
            })
            .await?;
        if pages.len() > limits.max_pages {
            bail!("the template is used on more than {} pages; add a category filter", limits.max_pages);
        }
        pages.sort_by_key(|p| std::cmp::Reverse(p.latest_revision));
        Ok(pages)
    }

    async fn category_step(&self, site: &Site, categories: &[String], namespace: i32) -> Result<CategoryStep> {
        let mut step = CategoryStep::default();
        let namespaces = format!("{namespace}|{NS_CATEGORY}");
        for category in categories {
            let title = site.full_title(NS_CATEGORY, category);
            let p = params(&[
                ("action", "query"),
                ("list", "categorymembers"),
                ("cmtitle", &title),
                ("cmnamespace", &namespaces),
                ("cmprop", "ids|title"),
                ("cmlimit", "max"),
            ]);
            self.api
                .query_continue(&site.host, &p, |json| {
                    for member in json["query"]["categorymembers"].as_array().into_iter().flatten() {
                        add_member(&mut step, site, member, namespace);
                    }
                    true
                })
                .await?;
        }
        Ok(step)
    }
}

fn page_from_api(page: &Value) -> Option<Page> {
    Some(Page {
        id: page["pageid"].as_u64()?,
        title: page["title"].as_str()?.to_string(),
        item: page["pageprops"]["wikibase_item"].as_str().and_then(|q| q.parse().ok()),
        latest_revision: page["lastrevid"].as_u64().unwrap_or_default(),
    })
}

fn add_member(step: &mut CategoryStep, site: &Site, member: &Value, namespace: i32) {
    let ns = member["ns"].as_i64().unwrap_or(-1) as i32;
    if ns == namespace {
        step.pages.extend(member["pageid"].as_u64());
    }
    if let (NS_CATEGORY, Some(title)) = (ns, member["title"].as_str()) {
        step.subcategories.push(site.db_key(NS_CATEGORY, title));
    }
}

/// Uses `primary` (the replicas) and falls back to `fallback` (the API) when it fails.
#[derive(Debug)]
pub struct WithFallback<P, F> {
    pub primary: P,
    pub fallback: F,
}

#[async_trait]
impl<P: PageSource, F: PageSource> PageSource for WithFallback<P, F> {
    async fn transclusions(&self, site: &Site, template: &str, namespace: i32, limits: Limits) -> Result<Vec<Page>> {
        match self.primary.transclusions(site, template, namespace, limits).await {
            Err(e) if !is_limit_error(&e) => {
                tracing::warn!("replica failed for {}, using the API: {e:#}", site.dbname);
                self.fallback.transclusions(site, template, namespace, limits).await
            }
            result => result,
        }
    }

    async fn category_step(&self, site: &Site, categories: &[String], namespace: i32) -> Result<CategoryStep> {
        self.primary.category_step(site, categories, namespace).await
    }

    async fn category_members(
        &self,
        site: &Site,
        category: &str,
        namespace: i32,
        depth: u32,
        limits: Limits,
    ) -> Result<HashSet<u64>> {
        match self.primary.category_members(site, category, namespace, depth, limits).await {
            Err(e) if !is_limit_error(&e) => {
                tracing::warn!("replica failed for {}, using the API: {e:#}", site.dbname);
                self.fallback.category_members(site, category, namespace, depth, limits).await
            }
            result => result,
        }
    }
}

/// Limit errors are answers, not failures; retrying on the API would only be slower.
fn is_limit_error(e: &anyhow::Error) -> bool {
    e.to_string().contains("more than")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A category graph in memory: category → (pages, subcategories).
    #[derive(Debug)]
    struct Graph(HashMap<&'static str, (Vec<u64>, Vec<&'static str>)>);

    #[async_trait]
    impl PageSource for Graph {
        async fn transclusions(&self, _: &Site, _: &str, _: i32, _: Limits) -> Result<Vec<Page>> {
            Ok(vec![])
        }
        async fn category_step(&self, _: &Site, cats: &[String], _: i32) -> Result<CategoryStep> {
            let mut step = CategoryStep::default();
            for c in cats {
                let (pages, subs) = self.0.get(c.as_str()).cloned().unwrap_or_default();
                step.pages.extend(pages);
                step.subcategories.extend(subs.into_iter().map(String::from));
            }
            Ok(step)
        }
    }

    fn site() -> Site {
        Site {
            host: "x".into(),
            dbname: "xwiki".into(),
            lang: "en".into(),
            namespaces: Default::default(),
            template_prefixes: vec![],
            file_prefixes: vec![],
            category_prefixes: vec![],
            template_case_sensitive: false,
            edition: None,
        }
    }

    const LIMITS: Limits = Limits { max_pages: 100, max_depth: 30, max_categories: 100 };

    fn graph() -> Graph {
        Graph(HashMap::from([
            ("Root", (vec![1, 2], vec!["A", "B"])),
            ("A", (vec![3], vec!["Root", "C"])), // cycle back to Root
            ("B", (vec![2, 4], vec![])),
            ("C", (vec![5], vec![])),
        ]))
    }

    #[tokio::test]
    async fn walks_depth_and_survives_cycles() {
        let (g, site) = (graph(), site());
        for (depth, expected) in [(0, vec![1, 2]), (1, vec![1, 2, 3, 4]), (10, vec![1, 2, 3, 4, 5])] {
            let pages = g.category_members(&site, "Root", 0, depth, LIMITS).await.unwrap();
            assert_eq!(pages, HashSet::from_iter(expected), "depth {depth}");
        }
    }

    #[tokio::test]
    async fn enforces_limits() {
        let limits = Limits { max_pages: 3, ..LIMITS };
        let err = graph().category_members(&site(), "Root", 0, 5, limits).await.unwrap_err();
        assert!(is_limit_error(&err), "{err}");
    }
}
