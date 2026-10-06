//! Collecting a run's candidate pages, with the filters applied.

use super::job::Job;
use super::spec::SkipIf;
use crate::app_state::Clients;
use crate::ids::ItemId;
use crate::wiki::site::NS_CATEGORY;
use crate::wiki::{Limits, Page, PageSource};
use crate::wikitext::uppercase_first;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::HashSet;

/// Pages that use the template but are not candidates, by reason.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub not_in_category: usize,
    pub not_in_list: usize,
    pub not_in_petscan: usize,
    pub not_in_sparql: usize,
    pub no_item: usize,
    pub not_instance: usize,
    /// Pre-filtered via WDQS; the live check before each edit catches the rest.
    pub already_set: usize,
}

pub async fn candidates(
    job: &Job,
    clients: &Clients,
    source: &dyn PageSource,
    limits: Limits,
) -> Result<(Vec<Page>, Excluded)> {
    let spec = &job.spec;
    let mut pages = source.transclusions(&job.site, &job.template_key, spec.namespace, limits).await?;
    let mut excluded = Excluded::default();
    if !spec.category.trim().is_empty() {
        let category = job.site.db_key(NS_CATEGORY, &spec.category);
        let members = source.category_members(&job.site, &category, spec.namespace, spec.depth, limits).await?;
        excluded.not_in_category = retain(&mut pages, |p| members.contains(&p.id));
    }
    if !spec.manual_list.is_empty() {
        let wanted = manual_list(&spec.manual_list);
        let listed = |p: &Page| wanted.contains(&p.title) || p.item.is_some_and(|q| wanted.contains(&q.to_string()));
        excluded.not_in_list = retain(&mut pages, listed);
    }
    if let Some(psid) = spec.petscan {
        let result = clients.petscan.query(psid).await.with_context(|| format!("PetScan query {psid}"))?;
        excluded.not_in_petscan = match result.wiki.as_str() {
            wiki if wiki == job.site.dbname => retain(&mut pages, |p| result.page_ids.contains(&p.id)),
            "wikidatawiki" => retain(&mut pages, |p| p.item.is_some_and(|q| result.titles.contains(&q.to_string()))),
            wiki => bail!("PetScan query {psid} lists pages on {wiki}, not on {}", job.site.dbname),
        };
    }
    if !spec.sparql.trim().is_empty() {
        let items: HashSet<ItemId> =
            clients.wdqs.select_items(&spec.sparql, "item").await.context("SPARQL query")?.into_iter().collect();
        excluded.not_in_sparql = retain(&mut pages, |p| p.item.is_some_and(|q| items.contains(&q)));
    }
    excluded.no_item = retain(&mut pages, |p| p.item.is_some());
    if !spec.instance_of.is_empty() {
        let items: Vec<ItemId> = pages.iter().filter_map(|p| p.item).collect();
        let wanted = clients.wdqs.items_in_classes(&items, &spec.instance_of).await.context("instance-of filter")?;
        excluded.not_instance = retain(&mut pages, |p| p.item.is_some_and(|q| wanted.contains(&q)));
    }
    // With several properties, a page may matter for one even if it has another.
    if spec.skip_if == SkipIf::Property && spec.extra_properties.is_empty() {
        let items: Vec<ItemId> = pages.iter().filter_map(|p| p.item).collect();
        match clients.wdqs.items_with_property(job.property.id, &items).await {
            Ok(set) => excluded.already_set = retain(&mut pages, |p| !p.item.is_some_and(|q| set.contains(&q))),
            Err(e) => tracing::warn!("WDQS pre-filter failed, relying on live checks: {e:#}"),
        }
    }
    Ok((pages, excluded))
}

/// Titles (any spacing, lower-case first letter) and item ids (#109).
fn manual_list(entries: &[String]) -> HashSet<String> {
    entries
        .iter()
        .map(|e| e.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|e| !e.is_empty())
        .map(|e| uppercase_first(&e))
        .collect()
}

/// Keep pages matching `keep`; return how many were dropped.
fn retain(pages: &mut Vec<Page>, keep: impl Fn(&Page) -> bool) -> usize {
    let before = pages.len();
    pages.retain(keep);
    before - pages.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_list_normalisation() {
        let list = manual_list(&["paris_ france ".into(), "q42".into(), "".into()]);
        assert!(list.contains("Paris france"));
        assert!(list.contains("Q42"));
        assert_eq!(list.len(), 2);
    }
}
